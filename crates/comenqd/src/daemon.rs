//! Daemon tasks for comenqd.
//!
//! This module implements the Unix socket listener and the worker that
//! processes queued comment requests. It posts comments with a timeout and
//! applies the configured cooldown period between requests.
use crate::config::Config;
use anyhow::Result;
use comenq_lib::CommentRequest;
use octocrab::Octocrab;
use std::fs as stdfs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::fs;
use tokio::io::AsyncReadExt;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Notify, mpsc, watch};
use yaque::{Receiver, Sender, channel};

/// Errors returned when posting a comment to GitHub.
#[derive(Debug, Error)]
enum PostCommentError {
    /// The GitHub API request failed.
    #[error(transparent)]
    Api(#[from] octocrab::Error),
    /// The request timed out.
    #[error("timeout")]
    Timeout,
}

/// Constructs an authenticated Octocrab GitHub client using a personal access token.
///
/// # Arguments
///
/// * `token` - A GitHub personal access token used for authentication.
///
/// # Returns
///
/// Returns an `Octocrab` client instance on success, or an error if the client could not be built.
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

/// Asynchronously creates the queue directory and all necessary parent directories if they do not exist.
async fn ensure_queue_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).await?;
    Ok(())
}

/// Attempts to post a comment to a GitHub pull request, enforcing a 10-second timeout.
///
/// Returns `Ok(())` if the comment is successfully posted. If the GitHub API returns an error,
/// returns `PostCommentError::Api`. If the operation does not complete within 10 seconds,
/// returns `PostCommentError::Timeout`.
async fn post_comment(
    octocrab: &Octocrab,
    request: &CommentRequest,
) -> Result<(), PostCommentError> {
    let issues = octocrab.issues(&request.owner, &request.repo);
    let fut = issues.create_comment(request.pr_number, &request.body);
    match tokio::time::timeout(Duration::from_secs(10), fut).await {
        Ok(res) => res.map(|_| ()).map_err(PostCommentError::Api),
        Err(_) => Err(PostCommentError::Timeout),
    }
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
/// ```rust,no_run
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
    let mut worker = Worker::spawn(cfg.clone(), rx, octocrab);

    tokio::select! {
        res = listener => match res {
            Ok(inner) => inner?,
            Err(e) => return Err(e.into()),
        },
        res = worker.join_handle() => match res {
            Ok(inner) => inner?,
            Err(e) => return Err(e.into()),
        },
    }
    let _ = shutdown_tx.send(());
    match tokio::time::timeout(Duration::from_secs(30), worker.shutdown()).await {
        Ok(res) => res?,
        Err(_) => {
            tracing::error!(
                "worker shutdown timed out after 30 seconds; possible resource leak or deadlock"
            );
            return Err(anyhow::anyhow!(
                "worker shutdown timed out after 30 seconds"
            ));
        }
    }
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

/// Awaitable hooks emitted by [`Worker`].
pub struct WorkerSignals {
    enqueued: Arc<Notify>,
    drained: Arc<Notify>,
}

impl WorkerSignals {
    /// Wait until the worker observes a new queue item.
    pub async fn on_enqueued(&self) {
        self.enqueued.notified().await;
    }

    /// Wait until the queue is empty and the worker is idle.
    pub async fn on_drained(&self) {
        self.drained.notified().await;
    }
}

/// Background task that processes queued comment requests.
pub struct Worker {
    shutdown: watch::Sender<()>,
    handle: tokio::task::JoinHandle<Result<()>>,
}

impl Worker {
    /// Spawn a worker without exposing progress signals.
    pub fn spawn(config: Arc<Config>, rx: Receiver, octocrab: Arc<Octocrab>) -> Self {
        let (tx, rx_shutdown) = watch::channel(());
        let handle = tokio::spawn(worker_loop(config, rx, octocrab, rx_shutdown, None));
        Self {
            shutdown: tx,
            handle,
        }
    }

    /// Spawn a worker and return progress signals for tests.
    pub fn spawn_with_signals(
        config: Arc<Config>,
        rx: Receiver,
        octocrab: Arc<Octocrab>,
    ) -> (Self, WorkerSignals) {
        let (shutdown_tx, shutdown_rx) = watch::channel(());
        let enqueued = Arc::new(Notify::new());
        let drained = Arc::new(Notify::new());
        let handle = tokio::spawn(worker_loop(
            config,
            rx,
            octocrab,
            shutdown_rx,
            Some((enqueued.clone(), drained.clone())),
        ));
        (
            Self {
                shutdown: shutdown_tx,
                handle,
            },
            WorkerSignals { enqueued, drained },
        )
    }

    /// Signal shutdown and wait for the worker task to finish.
    pub async fn shutdown(self) -> Result<()> {
        let _ = self.shutdown.send(());
        self.handle.await?
    }

    /// Borrow the underlying join handle.
    pub fn join_handle(&mut self) -> &mut tokio::task::JoinHandle<Result<()>> {
        &mut self.handle
    }
}

async fn worker_loop(
    config: Arc<Config>,
    mut rx: Receiver,
    octocrab: Arc<Octocrab>,
    mut shutdown: watch::Receiver<()>,
    signals: Option<(Arc<Notify>, Arc<Notify>)>,
) -> Result<()> {
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            res = rx.recv() => {
                let guard = match res {
                    Ok(g) => g,
                    Err(e) => {
                        tracing::info!(error = %e, "Worker receiver closed; exiting");
                        return Err(e.into());
                    }
                };
                if let Some((ref enq, _)) = signals {
                    enq.notify_one();
                }
                let request: CommentRequest = serde_json::from_slice(&guard)?;
                match post_comment(&octocrab, &request).await {
                    Ok(_) => { guard.commit()?; }
                    Err(PostCommentError::Api(e)) => {
                        tracing::error!(
                            error = %e,
                            owner = %request.owner,
                            repo = %request.repo,
                            pr = request.pr_number,
                            "GitHub API call failed",
                        );
                    }
                    Err(PostCommentError::Timeout) => {
                        tracing::error!(
                            owner = %request.owner,
                            repo = %request.repo,
                            pr = request.pr_number,
                            "GitHub API call timed out",
                        );
                    }
                }
                if let Some((_, ref drained)) = signals
                    && stdfs::read_dir(&config.queue_path)?.next().is_none()
                {
                    drained.notify_one();
                }
                if sleep_or_shutdown(
                    &mut shutdown,
                    Duration::from_secs(config.cooldown_period_seconds),
                )
                .await
                {
                    break;
                }
            }
        }
    }
    if let Some((_, ref drained)) = signals
        && stdfs::read_dir(&config.queue_path)?.next().is_none()
    {
        drained.notify_one();
    }
    Ok(())
}

/// Sleep for `dur` unless a shutdown signal arrives.
///
/// Returns `true` if shutdown was triggered.
async fn sleep_or_shutdown(shutdown: &mut watch::Receiver<()>, dur: Duration) -> bool {
    tokio::select! {
        _ = shutdown.changed() => true,
        _ = tokio::time::sleep(dur) => false,
    }
}

#[cfg(test)]
mod tests {
    //! Tests for the daemon tasks.
    use super::*;
    use octocrab::Octocrab;
    use rstest::{fixture, rstest};
    use std::fs as stdfs;
    use std::future::Future;
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;
    use tempfile::{TempDir, tempdir};
    use test_support::{octocrab_for, temp_config};
    use tokio::io::AsyncWriteExt;
    use tokio::net::UnixStream;
    use tokio::sync::{mpsc, watch};
    use tokio::time::{sleep, timeout};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use yaque::{Receiver, Sender};

    async fn wait_for_file(path: &Path, tries: u32, delay: Duration) -> bool {
        for _ in 0..tries {
            if path.exists() {
                return true;
            }
            sleep(delay).await;
        }
        path.exists()
    }

    async fn wait_for<F>(future: F, msg: &str)
    where
        F: Future<Output = ()>,
    {
        timeout(Duration::from_secs(120), future)
            .await
            .unwrap_or_else(|_| panic!("{}", msg));
    }

    /// Context dependencies for worker tests.
    struct WorkerTestContext {
        server: MockServer,
        cfg: Arc<Config>,
        sender: Sender,
        rx: Receiver,
        req: Vec<u8>,
        octo: Arc<Octocrab>,
        // Hold the directory to ensure temporary paths remain valid.
        _dir: TempDir,
    }

    /// Fixture: 2s cooldown from `temp_config` affords time for queue updates
    /// during coverage runs.
    #[fixture]
    async fn worker_test_context(#[default(201)] status: u16) -> WorkerTestContext {
        let dir = tempdir().expect("tempdir");
        let cfg = Arc::new(Config::from(temp_config(&dir).with_cooldown(2)));
        let (sender, rx) = channel(&cfg.queue_path).expect("channel");
        let req = CommentRequest {
            owner: "o".into(),
            repo: "r".into(),
            pr_number: 1,
            body: "b".into(),
        };
        let req = serde_json::to_vec(&req).expect("serialize");

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/o/r/issues/1/comments"))
            .respond_with(
                ResponseTemplate::new(status).set_body_json(&serde_json::json!({
                    "id": 1,
                    "body": "b",
                })),
            )
            .mount(&server)
            .await;

        let octo = octocrab_for(&server);

        WorkerTestContext {
            server,
            cfg,
            sender,
            rx,
            req,
            octo,
            _dir: dir,
        }
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
        let (client_tx, writer_rx) = mpsc::unbounded_channel();
        let writer = tokio::spawn(queue_writer(sender, writer_rx));

        let (mut client, server) = UnixStream::pair().expect("pair");
        let handle = tokio::spawn(handle_client(server, client_tx.clone()));
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
        drop(client_tx); // close writer channel
        writer.await.expect("writer join").expect("queue_writer");
        let guard = receiver.recv().await.expect("recv");
        let stored: CommentRequest = serde_json::from_slice(&guard).expect("parse");
        assert_eq!(stored, req);
    }

    #[tokio::test]
    async fn run_listener_accepts_connections() {
        let dir = tempdir().expect("tempdir");
        let cfg = Arc::new(Config::from(temp_config(&dir).with_cooldown(1)));
        let (sender, mut receiver) = channel(&cfg.queue_path).expect("channel");
        let (client_tx, writer_rx) = mpsc::unbounded_channel();
        let (shutdown_tx, shutdown_rx) = watch::channel(());
        let writer = tokio::spawn(queue_writer(sender, writer_rx));
        let listener_task = tokio::spawn(run_listener(cfg.clone(), client_tx.clone(), shutdown_rx));
        wait_for_file(&cfg.socket_path, 10, Duration::from_millis(10)).await;
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
        drop(client_tx); // close writer channel
        writer.await.expect("writer join").expect("queue_writer");
    }

    #[rstest]
    #[tokio::test]
    async fn run_worker_commits_on_success(
        #[future]
        #[from(worker_test_context)]
        ctx: WorkerTestContext,
    ) {
        let WorkerTestContext {
            server,
            cfg,
            mut sender,
            rx,
            req,
            octo,
            ..
        } = ctx.await;
        let (worker, signals) = Worker::spawn_with_signals(cfg.clone(), rx, octo);
        let enqueued = signals.on_enqueued();
        let drained = signals.on_drained();
        sender.send(req.clone()).await.expect("send request");
        drop(sender);
        wait_for(enqueued, "worker did not start processing").await;
        wait_for(drained, "worker did not drain").await;
        worker.shutdown().await.expect("shutdown");
        assert_eq!(server.received_requests().await.expect("requests").len(), 1);
        assert_eq!(
            stdfs::read_dir(&cfg.queue_path)
                .expect("read queue directory")
                .count(),
            0,
            "Queue should be empty after successful processing",
        );
    }

    #[rstest]
    #[tokio::test]
    async fn run_worker_requeues_on_error(
        #[future]
        #[with(500)]
        #[from(worker_test_context)]
        ctx: WorkerTestContext,
    ) {
        let WorkerTestContext {
            server,
            cfg,
            mut sender,
            rx,
            req,
            octo,
            ..
        } = ctx.await;
        let (worker, signals) = Worker::spawn_with_signals(cfg.clone(), rx, octo);
        let enqueued = signals.on_enqueued();
        sender.send(req.clone()).await.expect("send request");
        drop(sender);
        wait_for(enqueued, "worker did not start processing").await;
        worker.shutdown().await.expect("shutdown");
        assert!(server.received_requests().await.expect("requests").len() >= 1);
        assert!(
            stdfs::read_dir(&cfg.queue_path)
                .expect("read queue directory")
                .count()
                > 0,
            "Queue should retain job after API failure",
        );
    }

    #[rstest]
    #[tokio::test]
    async fn run_worker_processes_multiple_requests(
        #[future]
        #[with(201)]
        #[from(worker_test_context)]
        ctx: WorkerTestContext,
    ) {
        let WorkerTestContext {
            server,
            cfg,
            mut sender,
            rx,
            req,
            octo,
            ..
        } = ctx.await;
        let (worker, signals) = Worker::spawn_with_signals(cfg.clone(), rx, octo);
        let enqueued = signals.on_enqueued();
        let drained = signals.on_drained();
        for _ in 0..3 {
            sender.send(req.clone()).await.expect("send request");
        }
        drop(sender);
        wait_for(enqueued, "worker did not start processing").await;
        wait_for(drained, "worker did not drain").await;
        worker.shutdown().await.expect("shutdown");
        assert_eq!(
            server.received_requests().await.expect("requests").len(),
            3,
            "Worker should process all queued requests",
        );
        assert_eq!(
            stdfs::read_dir(&cfg.queue_path)
                .expect("read queue directory")
                .count(),
            0,
            "Queue should be empty after processing multiple requests",
        );
    }

    #[rstest]
    #[tokio::test]
    async fn worker_signals_handle_bursty_load(
        #[future]
        #[with(201)]
        #[from(worker_test_context)]
        ctx: WorkerTestContext,
    ) {
        let WorkerTestContext {
            server,
            cfg,
            mut sender,
            rx,
            req,
            octo,
            ..
        } = ctx.await;
        let (worker, signals) = Worker::spawn_with_signals(cfg.clone(), rx, octo);
        let drained = signals.on_drained();
        let enqueued: Vec<_> = (0..2).map(|_| signals.on_enqueued()).collect();
        for _ in 0..2 {
            sender.send(req.clone()).await.expect("send request");
        }
        drop(sender);
        for (idx, fut) in enqueued.into_iter().enumerate() {
            wait_for(fut, &format!("request {} not observed", idx + 1)).await;
        }
        wait_for(drained, "worker did not drain").await;
        worker.shutdown().await.expect("shutdown");
        assert_eq!(
            server.received_requests().await.expect("requests").len(),
            2,
            "Worker should process both queued requests",
        );
    }
}
