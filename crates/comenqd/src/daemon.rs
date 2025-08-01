//! Asynchronous daemon tasks for comenqd.
//!
//! This module provides the run function used by `main` which spawns the
//! Unix socket listener and the queue worker.

use crate::config::Config;
use anyhow::Result;
use comenq_lib::CommentRequest;
use octocrab::Octocrab;
use std::fs as stdfs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::fs;
use tokio::io::AsyncReadExt;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, watch};
use yaque::{Receiver, Sender, channel};

fn build_octocrab(token: &str) -> Result<Octocrab> {
    Ok(Octocrab::builder()
        .personal_token(token.to_string())
        .build()?)
}

fn prepare_listener(path: &Path) -> Result<UnixListener> {
    if stdfs::metadata(path).is_ok() {
        stdfs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    stdfs::set_permissions(path, stdfs::Permissions::from_mode(0o660))?;
    Ok(listener)
}

async fn ensure_queue_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).await?;
    Ok(())
}

/// Forward bytes from a channel into the persistent queue.
///
/// The queue writer decouples the listener from the queue, ensuring a
/// single writer for the `yaque` queue. It reads raw JSON payloads from the
/// provided [`mpsc::UnboundedReceiver`] and attempts to enqueue each item
/// using the [`yaque::Sender`]. Errors are logged and the loop continues so
/// the daemon remains responsive.
///
/// # Parameters
/// - `sender`: queue writer from `yaque`.
/// - `rx`: receiver for payloads from client handlers.
///
/// # Errors
/// Returns an [`anyhow::Error`] if:
/// - the receiver channel (`rx`) is closed unexpectedly,
/// - the queue sender encounters an I/O error while enqueuing,
/// - or if the sender fails while awaiting shutdown.
///
/// # Examples
/// ```no_run
/// use yaque::channel;
/// use tokio::sync::mpsc;
/// # async fn docs() -> anyhow::Result<()> {
/// let (queue_tx, _rx) = channel("/tmp/q")?;
/// let (tx, rx) = mpsc::unbounded_channel();
/// tokio::spawn(async move { comenqd::daemon::queue_writer(queue_tx, rx).await? });
/// tx.send(Vec::new()).unwrap();
/// # Ok(())
/// # }
/// ```
pub async fn queue_writer(
    mut sender: Sender,
    mut rx: mpsc::UnboundedReceiver<Vec<u8>>,
) -> Result<()> {
    while let Some(bytes) = rx.recv().await {
        if let Err(e) = sender.send(bytes).await {
            tracing::error!(error = %e, "Queue enqueue failed");
        }
    }
    Ok(())
}

/// Start the daemon with the provided configuration.
pub async fn run(config: Config) -> Result<()> {
    ensure_queue_dir(&config.queue_path).await?;
    tracing::info!(queue = %config.queue_path.display(), "Queue directory prepared");
    let octocrab = Arc::new(build_octocrab(&config.github_token)?);
    let (queue_tx, rx) = channel(&config.queue_path)?;
    let (client_tx, client_rx) = mpsc::unbounded_channel();
    let cfg = Arc::new(config);
    let (shutdown_tx, shutdown_rx) = watch::channel(());

    let writer = tokio::spawn(queue_writer(queue_tx, client_rx));
    let listener = tokio::spawn(run_listener(cfg.clone(), client_tx, shutdown_rx));
    let worker = tokio::spawn(run_worker(cfg.clone(), rx, octocrab));

    tokio::select! {
        res = listener => match res {
            Ok(inner) => inner?,
            Err(e) => return Err(e.into()),
        },
        res = worker => match res {
            Ok(inner) => inner?,
            Err(e) => return Err(e.into()),
        },
    }

    let _ = shutdown_tx.send(());
    writer.await??;

    Ok(())
}

/// Listen on the Unix socket and spawn a handler for each client.
///
/// The listener accepts connections on the path configured in [`Config`]. Each
/// connection is handled concurrently by [`handle_client`], forwarding valid
/// requests to the queue writer. The function exits when the `shutdown` watch
/// channel is triggered.
///
/// # Parameters
/// - `config`: shared daemon configuration.
/// - `tx`: channel used to forward request bytes to [`queue_writer`].
/// - `shutdown`: signal to terminate the listener loop.
///
/// # Errors
/// Returns an error if the socket cannot be created or if accepting a
/// connection fails after retries. Exiting due to a shutdown signal is normal
/// and not treated as an error.
pub async fn run_listener(
    config: Arc<Config>,
    tx: mpsc::UnboundedSender<Vec<u8>>,
    mut shutdown: watch::Receiver<()>,
) -> Result<()> {
    let listener = prepare_listener(&config.socket_path)?;

    loop {
        tokio::select! {
            res = listener.accept() => match res {
                Ok((stream, _)) => {
                    let tx_clone = tx.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_client(stream, tx_clone).await {
                            tracing::warn!(error = %e, "Client handling failed");
                        }
                    });
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to accept client connection");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            },
            _ = shutdown.changed() => {
                break;
            }
        }
    }
    Ok(())
}

/// Read a single request from `stream` and forward it to the queue.
///
/// Expects the client to send a JSON encoded [`CommentRequest`] and then close
/// the connection. The request is re-encoded to bytes and sent over `tx` for the
/// queue writer to persist. If the channel has been closed an error is
/// returned.
///
/// # Parameters
/// - `stream`: client connection on the Unix socket.
/// - `tx`: channel to the queue writer task.
///
/// # Errors
/// Fails if reading from the socket or parsing JSON fails, or if the queue
/// writer has shut down.
async fn handle_client(mut stream: UnixStream, tx: mpsc::UnboundedSender<Vec<u8>>) -> Result<()> {
    let mut buffer = Vec::new();
    stream.read_to_end(&mut buffer).await?;
    let request: CommentRequest = serde_json::from_slice(&buffer)?;
    let bytes = serde_json::to_vec(&request)?;
    tx.send(bytes)
        .map_err(|_| anyhow::anyhow!("queue writer dropped"))?;
    Ok(())
}

/// Dequeue requests and post comments to GitHub with a cooldown.
///
/// The worker continuously reads entries from the `yaque` queue and posts each
/// comment through the provided [`Octocrab`] instance. Successful posts commit
/// the queue entry, removing it from disk. Failures leave the message
/// uncommitted so it is retried on the next loop iteration.
///
/// A cooldown period, configured via [`Config`], is enforced **between all
/// requests**, regardless of success or failure. After each attempt the worker
/// waits for the cooldown duration before handling the next queue item. There
/// is no exponential backoff; failed requests are retried after the same
/// cooldown period.
///
/// # Parameters
/// - `config`: shared daemon configuration.
/// - `rx`: receiver half of the queue channel.
/// - `octocrab`: authenticated GitHub client.
///
/// # Errors
/// Propagates I/O and serialization errors from queue operations and any error
/// returned by the GitHub client.
pub async fn run_worker(
    config: Arc<Config>,
    mut rx: Receiver,
    octocrab: Arc<Octocrab>,
) -> Result<()> {
    loop {
        let guard = rx.recv().await?;
        let request: CommentRequest = serde_json::from_slice(&guard)?;

        let issues = octocrab.issues(&request.owner, &request.repo);
        let post = issues.create_comment(request.pr_number, &request.body);
        match tokio::time::timeout(Duration::from_secs(10), post).await {
            Ok(Ok(_)) => {
                guard.commit()?;
            }
            Ok(Err(e)) => {
                tracing::error!(
                    error = %e,
                    owner = %request.owner,
                    repo = %request.repo,
                    pr = request.pr_number,
                    "GitHub API call failed"
                );
            }
            Err(_) => {
                tracing::error!(
                    owner = %request.owner,
                    repo = %request.repo,
                    pr = request.pr_number,
                    "GitHub API call timed out"
                );
            }
        }

        tokio::time::sleep(Duration::from_secs(config.cooldown_period_seconds)).await;
    }
}

#[cfg(test)]
mod tests {
    //! Tests for the daemon tasks.
    use super::*;
    use tempfile::tempdir;
    mod fs {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/support/fs.rs"
        ));
    }
    use tokio::io::AsyncWriteExt;
    use tokio::net::{UnixListener, UnixStream};
    use tokio::sync::{mpsc, watch};
    use tokio::time::{Duration, sleep};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn setup_run_worker(status: u16) -> (MockServer, Arc<Config>, Receiver, Arc<Octocrab>) {
        let dir = tempdir().expect("tempdir");
        let cfg = Arc::new(Config {
            github_token: "t".into(),
            socket_path: dir.path().join("sock"),
            queue_path: dir.path().join("q"),
            cooldown_period_seconds: 0,
        });
        let (sender, rx) = channel(&cfg.queue_path).expect("channel");
        let req = CommentRequest {
            owner: "o".into(),
            repo: "r".into(),
            pr_number: 1,
            body: "b".into(),
        };
        sender
            .send(serde_json::to_vec(&req).expect("serialize"))
            .await
            .expect("send");

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/o/r/issues/1/comments"))
            .respond_with(ResponseTemplate::new(status).set_body_raw("{}", "application/json"))
            .mount(&server)
            .await;

        let octo = Arc::new(
            Octocrab::builder()
                .personal_token("t".to_string())
                .base_uri(server.uri())
                .expect("base_uri")
                .build()
                .expect("build octocrab"),
        );

        (server, cfg, rx, octo)
    }

    #[tokio::test]
    async fn ensure_queue_dir_creates_directory() {
        let dir = tempdir().expect("Failed to create temporary directory");
        let path = dir.path().join("queue");
        ensure_queue_dir(&path)
            .await
            .expect("Failed to ensure queue directory");
        assert!(path.is_dir());
    }

    #[tokio::test]
    async fn run_creates_queue_directory() {
        let dir = tempdir().expect("Failed to create temporary directory");
        let cfg = Config {
            github_token: "t".into(),
            socket_path: dir.path().join("sock"),
            queue_path: dir.path().join("q"),
            cooldown_period_seconds: 1,
        };

        assert!(!cfg.queue_path.exists());

        let handle = tokio::spawn(run(cfg.clone()));

        assert!(
            fs::wait_for_path(&cfg.queue_path, 2000).await,
            "queue directory not created"
        );

        handle.abort();
        assert!(cfg.queue_path.is_dir(), "queue directory not created");
    }

    #[tokio::test]
    async fn prepare_listener_sets_permissions() {
        let dir = tempdir().expect("tempdir");
        let sock = dir.path().join("sock");
        stdfs::write(&sock, b"stale").expect("create stale file");

        let listener = prepare_listener(&sock).expect("prepare listener");
        drop(listener);

        let meta = stdfs::metadata(&sock).expect("metadata");
        assert_eq!(meta.permissions().mode() & 0o777, 0o660);
    }

    #[tokio::test]
    async fn handle_client_enqueues_request() {
        let dir = tempdir().expect("tempdir");
        let queue_path = dir.path().join("q");
        let (sender, mut receiver) = channel(&queue_path).expect("channel");
        let (client_tx, mut writer_rx) = mpsc::unbounded_channel();
        let writer = tokio::spawn(queue_writer(sender, writer_rx));

        let (mut client, server) = UnixStream::pair().expect("pair");
        let handle = tokio::spawn(handle_client(server, client_tx));

        let req = CommentRequest {
            owner: "o".into(),
            repo: "r".into(),
            pr_number: 1,
            body: "b".into(),
        };
        let payload = serde_json::to_vec(&req).expect("serialize");
        client.write_all(&payload).await.expect("write");
        client.shutdown().await.expect("shutdown");
        handle.await.expect("join").expect("client");
        drop(writer); // stop queue writer

        let guard = receiver.recv().await.expect("recv");
        let stored: CommentRequest = serde_json::from_slice(&guard).expect("parse");
        assert_eq!(stored, req);
    }

    #[tokio::test]
    async fn run_listener_accepts_connections() {
        let dir = tempdir().expect("tempdir");
        let cfg = Arc::new(Config {
            github_token: "t".into(),
            socket_path: dir.path().join("sock"),
            queue_path: dir.path().join("q"),
            cooldown_period_seconds: 1,
        });

        let (sender, mut receiver) = channel(&cfg.queue_path).expect("channel");
        let (client_tx, writer_rx) = mpsc::unbounded_channel();
        let (shutdown_tx, shutdown_rx) = watch::channel(());
        let writer = tokio::spawn(queue_writer(sender, writer_rx));

        let listener_task = tokio::spawn(run_listener(cfg.clone(), client_tx, shutdown_rx));

        assert!(
            fs::wait_for_path(&cfg.socket_path, 100).await,
            "socket file not created"
        );

        let mut stream = UnixStream::connect(&cfg.socket_path)
            .await
            .expect("connect");
        let req = CommentRequest {
            owner: "o".into(),
            repo: "r".into(),
            pr_number: 1,
            body: "b".into(),
        };
        let payload = serde_json::to_vec(&req).expect("serialize");
        stream.write_all(&payload).await.expect("write");
        stream.shutdown().await.expect("shutdown");

        let guard = receiver.recv().await.expect("recv");
        let stored: CommentRequest = serde_json::from_slice(&guard).expect("parse");
        assert_eq!(stored, req);

        listener_task.abort();
        let _ = shutdown_tx.send(());
        drop(writer);
    }

    #[tokio::test]
    async fn run_worker_commits_on_success() {
        let (server, cfg, rx, octo) = setup_run_worker(201).await;
        let h = tokio::spawn(run_worker(cfg.clone(), rx, octo));
        sleep(Duration::from_millis(50)).await;
        h.abort();

        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        assert_eq!(std::fs::read_dir(&cfg.queue_path).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn run_worker_requeues_on_error() {
        let (server, cfg, rx, octo) = setup_run_worker(500).await;
        let h = tokio::spawn(run_worker(cfg.clone(), rx, octo));
        sleep(Duration::from_millis(50)).await;
        h.abort();

        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        assert!(std::fs::read_dir(&cfg.queue_path).unwrap().count() > 0);
    }
}
