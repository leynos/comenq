//! Task orchestration for comenqd.
//!
//! Coordinates the listener, queue writer, and worker tasks, applying
//! exponential backoff on failure and handling graceful shutdown.

use crate::config::Config;
use anyhow::Result;
use backon::{ExponentialBackoff, ExponentialBuilder};
use octocrab::Octocrab;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::fs;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::{mpsc, watch};
use yaque::{Sender, channel};

use crate::listener::run_listener;
use crate::worker::{WorkerControl, WorkerHooks, build_octocrab, run_worker};

/// Asynchronously create the queue directory and any missing parents.
pub async fn ensure_queue_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).await?;
    Ok(())
}

/// Build a jittered exponential backoff with no maximum attempt count.
///
/// The minimum delay is provided by the caller to allow environment-specific
/// tuning.
pub(crate) fn backoff(min_delay: Duration) -> ExponentialBackoff {
    backon::BackoffBuilder::build(
        ExponentialBuilder::default()
            .with_jitter()
            .with_min_delay(min_delay)
            .without_max_times(),
    )
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
    let (queue_tx, _) = channel(&config.queue_path)?; // drop unused receiver
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
                _ = sigint.recv() => { let _ = shutdown_tx.send(()); }
                _ = sigterm.recv() => { let _ = shutdown_tx.send(()); }
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
                let delay = listener_backoff.next().expect("backoff should yield a duration");
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
                let delay = worker_backoff.next().expect("backoff should yield a duration");
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
                let delay = writer_backoff.next().expect("backoff should yield a duration");
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {},
                    _ = shutdown_rx.changed() => {
                        listener.abort();
                        worker.abort();
                        writer.abort();
                        break;
                    }
                }
                listener.abort();
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
                        listener = spawn_listener(cfg.clone(), client_tx.clone(), shutdown_rx.clone());
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
