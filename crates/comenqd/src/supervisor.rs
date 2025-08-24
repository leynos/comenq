//! Task orchestration for comenqd.
//!
//! Coordinates the listener, queue writer, and worker tasks, applying
//! exponential backoff on failure and handling graceful shutdown.

use crate::config::Config;
use backon::{ExponentialBackoff, ExponentialBuilder};
use octocrab::Octocrab;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::fs;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::{mpsc, watch};
use yaque::{Sender, channel};

use crate::listener::run_listener;
use crate::worker::{WorkerControl, WorkerHooks, build_octocrab, run_worker};

#[derive(Debug, Error)]
pub enum SupervisorError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Octocrab(#[from] octocrab::Error),
}

pub type Result<T> = std::result::Result<T, SupervisorError>;

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

/// Sleep for `d` or return early if `shutdown` is triggered.
///
/// Returns `true` if a shutdown occurred.
async fn sleep_or_shutdown(shutdown: &mut watch::Receiver<()>, d: Duration) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(d) => false,
        _ = shutdown.changed() => true,
    }
}

/// Supervise a task that returns `Result<()>` and respawn it on failure.
async fn supervise_task<F, B>(
    name: &str,
    mut handle: tokio::task::JoinHandle<anyhow::Result<()>>,
    mut backoff: ExponentialBackoff,
    mut spawn_fn: F,
    mut shutdown: watch::Receiver<()>,
    mut backoff_builder: B,
) where
    F: FnMut() -> tokio::task::JoinHandle<anyhow::Result<()>>,
    B: FnMut() -> ExponentialBackoff,
{
    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                handle.abort();
                break;
            }
            res = &mut handle => {
                match &res {
                    Ok(Ok(())) => tracing::warn!(task = name, "exited"),
                    Ok(Err(e)) => tracing::error!(task = name, error = %e),
                    Err(e) => tracing::error!(task = name, error = %e),
                }
                let delay = backoff.next().expect("backoff should yield a duration");
                if sleep_or_shutdown(&mut shutdown, delay).await {
                    break;
                }
                backoff = backoff_builder();
                handle = spawn_fn();
            }
        }
    }
}

async fn supervise_writer<B>(
    mut handle: tokio::task::JoinHandle<mpsc::Receiver<Vec<u8>>>,
    mut backoff: ExponentialBackoff,
    mut backoff_builder: B,
    cfg: Arc<Config>,
    client_tx: Arc<std::sync::Mutex<mpsc::Sender<Vec<u8>>>>,
    shutdown_tx: watch::Sender<()>,
    mut shutdown: watch::Receiver<()>,
) where
    B: FnMut() -> ExponentialBackoff,
{
    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                handle.abort();
                break;
            }
            res = &mut handle => {
                let rx = match res {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::error!(task = "writer", error = %e);
                        let pair = mpsc::channel(cfg.client_channel_capacity);
                        *client_tx.lock().unwrap() = pair.0;
                        pair.1
                    }
                };
                let delay = backoff.next().expect("backoff should yield a duration");
                if sleep_or_shutdown(&mut shutdown, delay).await {
                    break;
                }
                backoff = backoff_builder();
                match channel(&cfg.queue_path) {
                    Ok((queue_tx, _)) => {
                        handle = tokio::spawn(queue_writer(queue_tx, rx));
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
/// let (tx, rx) = mpsc::channel(1);
/// tokio::spawn(async move { comenqd::daemon::queue_writer(queue_tx, rx).await });
/// tx.send(Vec::new()).await.unwrap();
/// # Ok(())
/// # }
/// ```
pub async fn queue_writer(
    mut sender: Sender,
    mut rx: mpsc::Receiver<Vec<u8>>,
) -> mpsc::Receiver<Vec<u8>> {
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
    let (client_tx_initial, client_rx) = mpsc::channel(config.client_channel_capacity);
    let client_tx = Arc::new(std::sync::Mutex::new(client_tx_initial));
    let cfg = Arc::new(config);
    let (shutdown_tx, shutdown_rx) = watch::channel(());

    // Initial task spawns and backoff builders.
    let writer = tokio::spawn(queue_writer(queue_tx, client_rx));
    let listener_tx = client_tx.clone();
    let listener = spawn_listener(
        cfg.clone(),
        listener_tx.lock().unwrap().clone(),
        shutdown_rx.clone(),
    );
    let worker = spawn_worker(cfg.clone(), octocrab.clone(), shutdown_rx.clone());
    let min_delay = Duration::from_millis(cfg.restart_min_delay_ms);
    let listener_backoff = backoff(min_delay);
    let worker_backoff = backoff(min_delay);
    let writer_backoff = backoff(min_delay);

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

    // Supervise tasks concurrently.
    let client_tx_clone = client_tx.clone();
    let shutdown_listener = shutdown_rx.clone();
    let shutdown_worker = shutdown_rx.clone();
    tokio::join!(
        supervise_task(
            "listener",
            listener,
            listener_backoff,
            || {
                let tx = client_tx_clone.lock().unwrap().clone();
                spawn_listener(cfg.clone(), tx, shutdown_listener.clone())
            },
            shutdown_listener.clone(),
            || backoff(min_delay),
        ),
        supervise_task(
            "worker",
            worker,
            worker_backoff,
            || spawn_worker(cfg.clone(), octocrab.clone(), shutdown_worker.clone()),
            shutdown_worker.clone(),
            || backoff(min_delay),
        ),
        supervise_writer(
            writer,
            writer_backoff,
            || backoff(min_delay),
            cfg.clone(),
            client_tx,
            shutdown_tx.clone(),
            shutdown_rx,
        ),
    );

    Ok(())
}

fn spawn_listener(
    cfg: Arc<Config>,
    tx: mpsc::Sender<Vec<u8>>,
    shutdown: watch::Receiver<()>,
) -> tokio::task::JoinHandle<anyhow::Result<()>> {
    tokio::spawn(run_listener(cfg, tx, shutdown))
}

fn spawn_worker(
    cfg: Arc<Config>,
    octocrab: Arc<Octocrab>,
    shutdown: watch::Receiver<()>,
) -> tokio::task::JoinHandle<anyhow::Result<()>> {
    let cfg_clone = cfg.clone();
    tokio::spawn(async move {
        let (_tx, rx) = channel(&cfg_clone.queue_path)?;
        let control = WorkerControl::new(shutdown, WorkerHooks::default());
        run_worker(cfg_clone, rx, octocrab, control).await
    })
}

// Logging helpers are no longer required; supervision handles reporting.
