//! Daemon tasks for comenqd.
//!
//! This module implements the Unix socket listener and the worker that
//! processes queued comment requests. It posts comments with a timeout and
//! applies the configured cooldown period between requests.
use crate::config::Config;
use anyhow::Result;
use backon::{ExponentialBackoff, ExponentialBuilder};
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
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::{Notify, mpsc, watch};
use yaque::{Receiver, Sender, channel};

const GITHUB_API_TIMEOUT_SECS: u64 = 30;

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

/// Builds a jittered exponential backoff with no maximum attempt count.
///
/// The minimum delay is provided by the caller to allow environment-specific
/// tuning.
fn backoff(min_delay: Duration) -> ExponentialBackoff {
    backon::BackoffBuilder::build(
        ExponentialBuilder::default()
            .with_jitter()
            .with_min_delay(min_delay)
            .without_max_times(),
    )
}

/// Attempts to post a comment to a GitHub pull request, enforcing a 30-second timeout.
///
/// Returns `Ok(())` if the comment is successfully posted. If the GitHub API returns an error,
/// returns `PostCommentError::Api`. If the operation does not complete within 30 seconds,
/// returns `PostCommentError::Timeout`.
async fn post_comment(
    octocrab: &Octocrab,
    request: &CommentRequest,
) -> Result<(), PostCommentError> {
    let issues = octocrab.issues(&request.owner, &request.repo);
    let fut = issues.create_comment(request.pr_number, &request.body);
    // Coverage instrumentation slows the async GitHub client; allow a generous
    // window before treating the call as timed out.
    match tokio::time::timeout(Duration::from_secs(GITHUB_API_TIMEOUT_SECS), fut).await {
        Ok(res) => res.map(|_| ()).map_err(PostCommentError::Api),
        Err(_) => Err(PostCommentError::Timeout),
    }
}

/// Forward bytes from a channel into the persistent queue.
///
/// The queue writer decouples the listener from the queue, ensuring a
/// single writer for the `yaque` queue. It reads raw JSON payloads from the
/// provided [`mpsc::UnboundedReceiver`] and attempts to enqueue each item
/// using the [`yaque::Sender`]. On enqueue failure the error is logged and the
/// loop terminates so a supervising task can recreate the sender.
///
/// When the loop terminates the receiver is returned so a supervising task can
/// resume consumption without losing any buffered requests.
///
/// # Parameters
/// - `sender`: queue sender from `yaque`.
/// - `rx`: receiver for payloads from client handlers.
///
/// # Examples
/// ```rust,no_run
/// use yaque::channel;
/// use tokio::sync::mpsc;
/// # async fn docs() -> anyhow::Result<()> {
/// let (queue_tx, _rx) = channel("/tmp/q")?;
/// let (tx, rx) = mpsc::unbounded_channel();
/// tokio::spawn(async move { comenqd::daemon::queue_writer(queue_tx, rx).await });
/// tx.send(Vec::new()).unwrap();
/// # Ok(())
/// # }
/// ```
pub async fn queue_writer(
    mut sender: Sender,
    mut rx: mpsc::UnboundedReceiver<Vec<u8>>,
) -> mpsc::UnboundedReceiver<Vec<u8>> {
    while let Some(bytes) = rx.recv().await {
        if let Err(e) = sender.send(bytes).await {
            tracing::error!(error = %e, "Queue enqueue failed");
            break;
        }
    }
    rx
}

/// Start the daemon with the provided configuration.
pub async fn run(config: Config) -> Result<()> {
    ensure_queue_dir(&config.queue_path).await?;
    tracing::info!(queue = %config.queue_path.display(), "Queue directory prepared");
    let octocrab = Arc::new(build_octocrab(&config.github_token)?);
    // Drop the unused receiver since yaque lacks a sender-only constructor.
    let (queue_tx, _) = channel(&config.queue_path)?;
    let (mut client_tx, client_rx) = mpsc::unbounded_channel();
    let cfg = Arc::new(config);
    let (shutdown_tx, mut shutdown_rx) = watch::channel(());

    // Initial task spawns.
    let mut writer = tokio::spawn(queue_writer(queue_tx, client_rx));
    let mut listener = spawn_listener(cfg.clone(), client_tx.clone(), shutdown_rx.clone());
    let mut worker = spawn_worker(cfg.clone(), octocrab.clone(), shutdown_rx.clone());
    let min_delay = Duration::from_millis(cfg.restart_min_delay_ms);
    let mut listener_backoff = backoff(min_delay);
    let mut worker_backoff = backoff(min_delay);
    let mut writer_backoff = backoff(min_delay);

    // Convert SIGINT and SIGTERM into a shutdown signal.
    {
        let shutdown_tx = shutdown_tx.clone();
        tokio::spawn(async move {
            let mut sigint = match signal(SignalKind::interrupt()) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!(error = %e, "Failed to install SIGINT handler");
                    let _ = shutdown_tx.send(());
                    return;
                }
            };
            let mut sigterm = match signal(SignalKind::terminate()) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!(error = %e, "Failed to install SIGTERM handler");
                    let _ = shutdown_tx.send(());
                    return;
                }
            };

            tokio::select! {
                _ = sigint.recv() => {
                    let _ = shutdown_tx.send(());
                }
                _ = sigterm.recv() => {
                    let _ = shutdown_tx.send(());
                }
            }
        });
    }

    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                listener.abort();
                worker.abort();
                writer.abort();
                break;
            }
            res = &mut listener => {
                log_listener_failure(&res);
                let delay = listener_backoff
                    .next()
                    .expect("backoff should yield a duration");
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {},
                    _ = shutdown_rx.changed() => {
                        listener.abort();
                        worker.abort();
                        writer.abort();
                        break;
                    }
                }
                listener = spawn_listener(cfg.clone(), client_tx.clone(), shutdown_rx.clone());
                listener_backoff = backoff(min_delay);
            }
            res = &mut worker => {
                log_worker_failure(&res);
                let delay = worker_backoff
                    .next()
                    .expect("backoff should yield a duration");
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {},
                    _ = shutdown_rx.changed() => {
                        listener.abort();
                        worker.abort();
                        writer.abort();
                        break;
                    }
                }
                worker = spawn_worker(cfg.clone(), octocrab.clone(), shutdown_rx.clone());
                worker_backoff = backoff(min_delay);
            }
            res = &mut writer => {
                log_writer_failure(&res);
                let delay = writer_backoff
                    .next()
                    .expect("backoff should yield a duration");
                // Allow shutdown to pre-empt the restart delay.
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {},
                    _ = shutdown_rx.changed() => {
                        listener.abort();
                        worker.abort();
                        writer.abort();
                        break;
                    }
                }
                // Stop accepting new connections before respawning the writer.
                // Moving the receiver between tasks is safe, but pausing accepts
                // avoids growing an in-memory backlog while the writer is down.
                listener.abort();
                // Recreate a queue sender for the restarted writer, reusing the
                // existing receiver where possible to avoid dropping buffered
                // requests. If the writer panicked the receiver is lost and a
                // new channel is created, potentially dropping pending items.
                let rx = match res {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::error!(error = %e, "Writer task panicked");
                        let pair = mpsc::unbounded_channel();
                        client_tx = pair.0;
                        pair.1
                    }
                };
                match channel(&cfg.queue_path) {
                    Ok((queue_tx, _)) => {
                        writer = tokio::spawn(queue_writer(queue_tx, rx));
                        listener =
                            spawn_listener(cfg.clone(), client_tx.clone(), shutdown_rx.clone());
                        writer_backoff = backoff(min_delay);
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "Queue sender creation failed");
                        let _ = shutdown_tx.send(());
                        break;
                    }
                }
            }
        }
    }

    // Close the client sender so the queue writer can exit cleanly.
    drop(client_tx);
    // Gracefully await all tasks with a timeout; ignore outcomes since shutdown is in progress.
    let _ = tokio::time::timeout(Duration::from_secs(10), async {
        let _ = listener.await;
        let _ = worker.await;
        let _ = writer.await;
    })
    .await;
    Ok(())
}

fn spawn_listener(
    cfg: Arc<Config>,
    tx: mpsc::UnboundedSender<Vec<u8>>,
    shutdown: watch::Receiver<()>,
) -> tokio::task::JoinHandle<Result<()>> {
    tokio::spawn(run_listener(cfg, tx, shutdown))
}

fn spawn_worker(
    cfg: Arc<Config>,
    octocrab: Arc<Octocrab>,
    shutdown: watch::Receiver<()>,
) -> tokio::task::JoinHandle<Result<()>> {
    let cfg_clone = cfg.clone();
    tokio::spawn(async move {
        // Obtain a fresh queue receiver each time the worker is spawned.
        // The sender persists across restarts.
        let (_tx, rx) = channel(&cfg_clone.queue_path)?;
        let control = WorkerControl::new(shutdown, WorkerHooks::default());
        run_worker(cfg_clone, rx, octocrab, control).await
    })
}

fn log_listener_failure(res: &Result<Result<()>, tokio::task::JoinError>) {
    match res {
        Ok(Ok(())) => tracing::warn!("Listener exited unexpectedly"),
        Ok(Err(e)) => tracing::error!(error = %e, "Listener task failed"),
        Err(e) => tracing::error!(error = %e, "Listener task panicked"),
    }
}

fn log_worker_failure(res: &Result<Result<()>, tokio::task::JoinError>) {
    match res {
        Ok(Ok(())) => tracing::warn!("Worker exited unexpectedly"),
        Ok(Err(e)) => tracing::error!(error = %e, "Worker task failed"),
        Err(e) => tracing::error!(error = %e, "Worker task panicked"),
    }
}

fn log_writer_failure(res: &Result<mpsc::UnboundedReceiver<Vec<u8>>, tokio::task::JoinError>) {
    match res {
        Ok(_) => tracing::warn!("Writer exited unexpectedly"),
        Err(e) => tracing::error!(error = %e, "Writer task panicked"),
    }
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
    let min_delay = Duration::from_millis(config.restart_min_delay_ms);
    let mut accept_backoff = backoff(min_delay);

    loop {
        tokio::select! {
            res = listener.accept() => match res {
                Ok((stream, _)) => {
                    accept_backoff = backoff(min_delay);
                    let tx_clone = tx.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_client(stream, tx_clone).await {
                            tracing::warn!(error = %e, "Client handling failed");
                        }
                    });
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to accept client connection");
                    let delay = accept_backoff
                        .next()
                        .expect("backoff should yield a duration");
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {},
                        _ = shutdown.changed() => break,
                    }
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

/// Hooks used to observe worker progress during tests.
///
/// Each field is optional. When present the [`Notify`] is signalled at key
/// points in the worker's lifecycle.
#[derive(Default)]
pub struct WorkerHooks {
    /// Signalled when a request is retrieved from the queue.
    pub enqueued: Option<Arc<Notify>>,
    /// Signalled after the worker completes processing of a request.
    pub idle: Option<Arc<Notify>>,
    /// Signalled when the queue is empty and the worker is idle.
    pub drained: Option<Arc<Notify>>,
}

impl WorkerHooks {
    fn notify_enqueued(&self) {
        if let Some(n) = &self.enqueued {
            n.notify_waiters();
        }
    }

    fn notify_idle(&self) {
        if let Some(n) = &self.idle {
            n.notify_waiters();
        }
    }

    #[cfg(test)]
    fn notify_drained_if_empty(&self, queue_path: &Path) -> std::io::Result<()> {
        if let Some(n) = &self.drained {
            // Ignore sentinel files left by the queue implementation and
            // consider the directory empty when no other files remain.
            let empty = !stdfs::read_dir(queue_path)?
                .filter_map(Result::ok)
                .any(|e| {
                    let name = e.file_name();
                    let name = name.to_string_lossy();
                    name != "version" && name != "recv.lock"
                });
            if empty {
                n.notify_waiters();
            }
        }
        Ok(())
    }

    async fn wait_or_shutdown(secs: u64, shutdown: &mut watch::Receiver<()>) {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(secs)) => {},
            _ = shutdown.changed() => {},
        }
    }
}

/// Controls the worker task.
///
/// Bundles the shutdown signal and optional test hooks to keep the worker API
/// concise.
pub struct WorkerControl {
    /// Watch channel used to signal graceful shutdown.
    pub shutdown: watch::Receiver<()>,
    /// Hooks for observing worker progress during tests.
    pub hooks: WorkerHooks,
}

impl WorkerControl {
    /// Create a new [`WorkerControl`].
    pub fn new(shutdown: watch::Receiver<()>, hooks: WorkerHooks) -> Self {
        Self { shutdown, hooks }
    }
}

/// Processes queued comment requests and posts them to GitHub, enforcing a cooldown between attempts.
///
/// Continuously receives comment requests from the persistent queue, attempts to post each comment to GitHub using the provided client, and commits successfully processed entries to remove them from the queue. Failed requests remain in the queue for retry. A fixed cooldown period, specified in the configuration, is applied after each attempt regardless of outcome. There is no exponential backoff; all retries use the same cooldown interval.
///
/// # Errors
///
/// Returns errors from queue operations, deserialization, or GitHub client failures.
///
/// # Examples
///
/// ```rust,ignore
/// use std::sync::Arc;
/// # use comenqd::daemon::{run_worker, WorkerControl, WorkerHooks};
/// # use comenqd::Config;
/// # use yaque::Receiver;
/// # use octocrab::Octocrab;
/// # async fn example() -> anyhow::Result<()> {
/// // Construct a Config instance here (omitted for brevity).
/// let config = Arc::new(/* Config */ unimplemented!());
/// let rx: Receiver = /* obtain from yaque */ unimplemented!();
/// let octocrab = Arc::new(Octocrab::builder().build()?);
/// let (_tx, shutdown) = watch::channel(());
/// let control = WorkerControl::new(shutdown, WorkerHooks::default());
/// run_worker(config, rx, octocrab, control).await?;
/// # Ok(())
/// # }
/// ```
pub async fn run_worker(
    config: Arc<Config>,
    mut rx: Receiver,
    octocrab: Arc<Octocrab>,
    mut control: WorkerControl,
) -> Result<()> {
    let hooks = &mut control.hooks;
    let shutdown = &mut control.shutdown;
    loop {
        let guard = tokio::select! {
            res = rx.recv() => res?,
            _ = shutdown.changed() => break,
        };
        hooks.notify_enqueued();
        let request: CommentRequest = match serde_json::from_slice(&guard) {
            Ok(req) => req,
            Err(e) => {
                tracing::error!(error = %e, "Failed to deserialise queued request; dropping");
                if let Err(commit_err) = guard.commit() {
                    tracing::error!(
                        error = %commit_err,
                        "Failed to commit malformed queue entry",
                    );
                }
                // Maintain hook semantics for tests even on malformed input.
                hooks.notify_idle();
                #[cfg(test)]
                if let Err(check_err) = hooks.notify_drained_if_empty(&config.queue_path) {
                    tracing::warn!(
                        error = %check_err,
                        "Queue emptiness check failed after drop",
                    );
                }
                WorkerHooks::wait_or_shutdown(config.cooldown_period_seconds, shutdown).await;
                continue;
            }
        };

        match post_comment(&octocrab, &request).await {
            Ok(_) => {
                guard.commit()?;
            }
            Err(PostCommentError::Api(e)) => {
                tracing::error!(
                    error = %e,
                    owner = %request.owner,
                    repo = %request.repo,
                    pr = request.pr_number,
                    "GitHub API call failed"
                );
            }
            Err(PostCommentError::Timeout) => {
                tracing::error!(
                    owner = %request.owner,
                    repo = %request.repo,
                    pr = request.pr_number,
                    "GitHub API call timed out"
                );
            }
        }

        hooks.notify_idle();
        #[cfg(test)]
        hooks.notify_drained_if_empty(&config.queue_path)?;
        WorkerHooks::wait_or_shutdown(config.cooldown_period_seconds, shutdown).await;
    }
    #[cfg(test)]
    hooks.notify_drained_if_empty(&config.queue_path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Tests for the daemon tasks.
    use super::*;
    use octocrab::Octocrab;
    use rstest::{fixture, rstest};
    use std::fs as stdfs;
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;
    use tempfile::{TempDir, tempdir};
    use test_support::{octocrab_for, temp_config};
    use tokio::io::AsyncWriteExt;
    use tokio::net::UnixStream;
    use tokio::sync::{Notify, mpsc, watch};
    use tokio::time::{sleep, timeout};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use yaque::Receiver;

    const TEST_COOLDOWN_SECONDS: u64 = 60;

    #[cfg(test)]
    #[expect(
        unexpected_cfgs,
        reason = "unit tests: detect non-standard coverage cfg flags across environments"
    )]
    fn is_coverage_run() -> bool {
        // Detect if running under coverage instrumentation
        std::env::var("CARGO_LLVM_COV_TARGET_DIR").is_ok()
            || std::env::var("RUSTFLAGS").map_or(false, |flags| flags.contains("coverage"))
            || cfg!(coverage_nightly)
            || cfg!(coverage)
    }

    #[cfg(test)]
    fn coverage_timeout_multiplier() -> u32 {
        if is_coverage_run() { 10 } else { 1 }
    }

    async fn wait_for_file(path: &Path, tries: u32, delay: Duration) -> bool {
        for _ in 0..tries {
            if path.exists() {
                return true;
            }
            sleep(delay).await;
        }
        path.exists()
    }

    /// Context dependencies for worker tests.
    struct WorkerTestContext {
        server: MockServer,
        cfg: Arc<Config>,
        rx: Receiver,
        octo: Arc<Octocrab>,
        // Hold the directory to ensure temporary paths remain valid.
        _dir: TempDir,
    }

    /// Fixture: 1s cooldown from `temp_config` throttles retries for deterministic tests.
    #[fixture]
    async fn worker_test_context(#[default(201)] status: u16) -> WorkerTestContext {
        let dir = tempdir().expect("tempdir");
        // Use a long cooldown so retries do not fire before the test shuts the
        // worker down. This keeps the number of HTTP attempts deterministic even
        // under heavy instrumentation.
        let cfg = Arc::new(Config::from(
            temp_config(&dir).with_cooldown(TEST_COOLDOWN_SECONDS),
        ));
        let (mut sender, rx) = channel(&cfg.queue_path).expect("channel");
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
            rx,
            octo,
            _dir: dir,
        }
    }

    /// Collect diagnostics about the queue state and server requests.
    async fn diagnose_queue_state(
        cfg: &Config,
        server: &MockServer,
        expected_files: usize,
    ) -> String {
        let queue_files = stdfs::read_dir(&cfg.queue_path)
            .map(|entries| entries.count())
            .unwrap_or(0);
        let server_requests = server.received_requests().await.unwrap_or_default().len();

        let mut output = format!(
            "Queue directory contains {} files (expected {})\n",
            queue_files, expected_files
        );
        output.push_str(&format!(
            "Mock server received {} requests\n",
            server_requests
        ));

        if let Ok(entries) = stdfs::read_dir(&cfg.queue_path) {
            output.push_str("Remaining queue files:\n");
            for (i, entry) in entries.enumerate() {
                if let Ok(entry) = entry {
                    let name = entry.file_name();
                    output.push_str(&format!("  {}. {}\n", i + 1, name.to_string_lossy()));

                    if let Ok(metadata) = entry.metadata() {
                        output.push_str(&format!("     Size: {} bytes\n", metadata.len()));
                        if let Ok(modified) = metadata.modified() {
                            if let Ok(elapsed) = modified.elapsed() {
                                output.push_str(&format!(
                                    "     Age: {:.1}s ago\n",
                                    elapsed.as_secs_f32()
                                ));
                            }
                        }
                    }
                }
            }
        }

        output
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
        let cfg = Config::from(temp_config(&dir).with_cooldown(1));
        assert!(!cfg.queue_path.exists());
        let handle = tokio::spawn(run(cfg.clone()));
        wait_for_file(&cfg.queue_path, 200, Duration::from_millis(10)).await;
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
        let (client_tx, writer_rx) = mpsc::unbounded_channel();
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
        let cfg = Arc::new(Config::from(temp_config(&dir).with_cooldown(1)));
        let (sender, mut receiver) = channel(&cfg.queue_path).expect("channel");
        let (client_tx, writer_rx) = mpsc::unbounded_channel();
        let (shutdown_tx, shutdown_rx) = watch::channel(());
        let writer = tokio::spawn(queue_writer(sender, writer_rx));
        let listener_task = tokio::spawn(run_listener(cfg.clone(), client_tx, shutdown_rx));
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
        let _ = shutdown_tx.send(());
        let _ = tokio::time::timeout(Duration::from_secs(10), async {
            let _ = listener_task.await;
            let _ = writer.await;
        })
        .await;
    }

    #[rstest]
    #[tokio::test]
    async fn run_worker_commits_on_success(
        #[future]
        #[from(worker_test_context)]
        ctx: WorkerTestContext,
    ) {
        let ctx = ctx.await;
        let server = Arc::new(ctx.server);
        let drained = Arc::new(Notify::new());
        let drained_for_wait = Arc::clone(&drained);
        let drained_notified = drained_for_wait.notified();
        let (shutdown_tx, shutdown_rx) = watch::channel(());
        let control = WorkerControl {
            shutdown: shutdown_rx,
            hooks: WorkerHooks {
                enqueued: None,
                idle: None,
                drained: Some(drained),
            },
        };
        let h = tokio::spawn(run_worker(ctx.cfg.clone(), ctx.rx, ctx.octo, control));

        let drain_timeout = Duration::from_secs(30 * coverage_timeout_multiplier() as u64);
        match timeout(drain_timeout, drained_notified).await {
            Ok(_) => println!("Worker drained notification received successfully"),
            Err(_) => {
                let diagnostics = diagnose_queue_state(&ctx.cfg, &server, 0).await;
                eprintln!(
                    "Timeout waiting for worker drained notification after {} seconds",
                    drain_timeout.as_secs()
                );
                eprintln!(
                    "Coverage mode: {}, Timeout used: {}s",
                    is_coverage_run(),
                    30 * coverage_timeout_multiplier()
                );
                eprintln!("{}", diagnostics);
                panic!("worker drained: QUEUE CLEANUP FAILURE");
            }
        }
        shutdown_tx.send(()).expect("send shutdown");
        let join_timeout = Duration::from_secs(30 * coverage_timeout_multiplier() as u64);
        match timeout(join_timeout, h).await {
            Ok(join_result) => {
                join_result.expect("join worker").expect("worker result");
                println!("\u{2713} Worker task completed successfully");
            }
            Err(_) => {
                let diagnostics = diagnose_queue_state(&ctx.cfg, &server, 0).await;
                eprintln!(
                    "\u{274C} Worker join timeout after {}s",
                    join_timeout.as_secs()
                );
                eprintln!("{}", diagnostics);
                panic!(
                    "join worker: timeout in success test after {}s",
                    join_timeout.as_secs()
                );
            }
        }
        assert_eq!(server.received_requests().await.expect("requests").len(), 1);
        let data_files = stdfs::read_dir(&ctx.cfg.queue_path)
            .expect("read queue directory")
            .filter_map(Result::ok)
            .filter(|e| {
                let name = e.file_name();
                let name = name.to_string_lossy();
                name != "version" && name != "recv.lock"
            })
            .count();
        assert_eq!(
            data_files, 0,
            "Queue data files should be empty after successful processing",
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
        let ctx = ctx.await;
        let server = Arc::new(ctx.server);
        let (shutdown_tx, shutdown_rx) = watch::channel(());
        let enqueued = Arc::new(Notify::new());
        let enqueued_for_wait = Arc::clone(&enqueued);
        let enqueued_notified = enqueued_for_wait.notified();
        let control = WorkerControl {
            shutdown: shutdown_rx,
            hooks: WorkerHooks {
                enqueued: Some(enqueued),
                idle: None,
                drained: None,
            },
        };
        let h = tokio::spawn(run_worker(ctx.cfg.clone(), ctx.rx, ctx.octo, control));

        // The unit value from `timeout` is irrelevant; `expect` handles expiry.
        let _ = timeout(
            Duration::from_secs(30 * coverage_timeout_multiplier() as u64),
            enqueued_notified,
        )
        .await
        .expect("worker picked up job");
        shutdown_tx.send(()).expect("send shutdown");
        let join_timeout = Duration::from_secs(45 * coverage_timeout_multiplier() as u64);
        match timeout(join_timeout, h).await {
            Ok(join_result) => {
                join_result.expect("join worker").expect("worker result");
                println!("\u{2713} Worker task completed with error handling");
            }
            Err(_) => {
                let diagnostics = diagnose_queue_state(&ctx.cfg, &server, 1).await;
                eprintln!(
                    "\u{274C} Worker join timeout after {}s",
                    join_timeout.as_secs()
                );
                eprintln!("{}", diagnostics);
                panic!(
                    "join worker: timeout in error test after {}s",
                    join_timeout.as_secs()
                );
            }
        }
        assert_eq!(
            server.received_requests().await.expect("requests").len(),
            1,
            "worker should attempt the request exactly once before shutdown",
        );
        assert!(
            stdfs::read_dir(&ctx.cfg.queue_path)
                .expect("read queue directory")
                .count()
                > 0,
            "Queue should retain job after API failure",
        );
    }
}
