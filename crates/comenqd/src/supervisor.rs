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
#[cfg(unix)]
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::{mpsc, watch};
use yaque::{Receiver, Sender};

use crate::listener::run_listener;
use crate::metrics;
use crate::worker::{WorkerControl, WorkerHooks, build_octocrab, run_worker};

mod observability;
mod writer;

use observability::log_task_failure;
pub use writer::queue_writer;
use writer::{QueueWriterState, spawn_queue_writer, supervise_writer, writer_backoff};

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

/// Fallback delay used if a backoff iterator is unexpectedly exhausted.
///
/// The builders here never cap attempts, so exhaustion should not occur;
/// the fallback keeps restart pacing sane without panicking.
pub(crate) const BACKOFF_FALLBACK_DELAY: Duration = Duration::from_secs(1);

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
pub(super) async fn sleep_or_shutdown(shutdown: &mut watch::Receiver<()>, d: Duration) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(d) => false,
        _ = shutdown.changed() => true,
    }
}

/// Supervise a task that returns `Result<()>` and respawn it on failure.
#[tracing::instrument(skip_all, fields(task = name))]
async fn supervise_task<F, I>(
    name: &'static str,
    mut handle: tokio::task::JoinHandle<anyhow::Result<()>>,
    mut backoff: I,
    mut spawn_fn: F,
    mut shutdown: watch::Receiver<()>,
) where
    F: FnMut(u64) -> tokio::task::JoinHandle<anyhow::Result<()>>,
    I: Iterator<Item = Duration>,
{
    let mut restart_attempt = 0_u64;
    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                let grace = tokio::time::sleep(Duration::from_millis(100));
                tokio::select! {
                    _ = &mut handle => {}
                    _ = grace => handle.abort(),
                }
                break;
            }
            res = &mut handle => {
                if matches!(&res, Ok(Ok(_))) {
                    // Normal completion; do not respawn.
                    break;
                }
                log_task_failure(name, &res);
                restart_attempt = restart_attempt.saturating_add(1);
                metrics::record_task_restart(name);
                let delay = backoff.next().unwrap_or(BACKOFF_FALLBACK_DELAY);
                tracing::warn!(
                    task = name,
                    attempt = restart_attempt,
                    delay_ms = delay.as_millis(),
                    "Scheduling task restart",
                );
                if sleep_or_shutdown(&mut shutdown, delay).await {
                    break;
                }
                handle = spawn_fn(restart_attempt);
                tracing::debug!(
                    task = name,
                    attempt = restart_attempt,
                    "Task restarted",
                );
            }
        }
    }
}

/// Start the daemon with the provided configuration.
#[tracing::instrument(skip(config), fields(queue = %config.queue_path.display()))]
pub async fn run(config: Config) -> Result<()> {
    ensure_queue_dir(&config.queue_path).await?;
    tracing::info!(queue = %config.queue_path.display(), "Queue directory prepared");
    let octocrab = Arc::new(build_octocrab(&config.github_token)?);
    // Open only the sender here; the worker opens the matching receiver.
    // Opening a full channel() in both places would contend for yaque's
    // per-side lock files and leave the worker in a permanent restart loop.
    let queue_tx = Sender::open(&config.queue_path)?;
    tracing::debug!(
        task = "writer",
        attempt = 0,
        side = "sender",
        queue = %config.queue_path.display(),
        "Queue side opened",
    );
    let (client_tx_initial, client_rx) = mpsc::channel(config.client_channel_capacity);
    let client_tx = Arc::new(tokio::sync::Mutex::new(client_tx_initial));
    let cfg = Arc::new(config);
    let (shutdown_tx, shutdown_rx) = watch::channel(());

    // Initial task spawns and backoff builders.
    let writer_state = QueueWriterState::new(client_rx);
    let writer = spawn_queue_writer(queue_tx, writer_state.clone());
    let listener = spawn_listener(cfg.clone(), client_tx.clone(), shutdown_rx.clone());
    let worker = spawn_worker(cfg.clone(), octocrab.clone(), shutdown_rx.clone(), 0);
    let min_delay = Duration::from_millis(cfg.restart_min_delay_ms);
    let listener_backoff = backoff(min_delay);
    let worker_backoff = backoff(min_delay);
    let writer_backoff = writer_backoff(min_delay);

    // Convert SIGINT and SIGTERM into a shutdown signal.
    #[cfg(unix)]
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
            |_| {
                let cfg = cfg.clone();
                let client_tx = client_tx_clone.clone();
                let shutdown_listener = shutdown_listener.clone();
                spawn_listener(cfg, client_tx, shutdown_listener)
            },
            shutdown_listener.clone(),
        ),
        supervise_task(
            "worker",
            worker,
            worker_backoff,
            |attempt| {
                spawn_worker(
                    cfg.clone(),
                    octocrab.clone(),
                    shutdown_worker.clone(),
                    attempt,
                )
            },
            shutdown_worker.clone(),
        ),
        supervise_writer(
            writer,
            writer_state,
            writer_backoff,
            cfg.clone(),
            |path| Sender::open(path).map_err(anyhow::Error::from),
            shutdown_tx,
            shutdown_rx,
        ),
    );

    Ok(())
}

fn spawn_listener(
    cfg: Arc<Config>,
    client_tx: Arc<tokio::sync::Mutex<mpsc::Sender<Vec<u8>>>>,
    shutdown: watch::Receiver<()>,
) -> tokio::task::JoinHandle<anyhow::Result<()>> {
    tokio::spawn(run_listener(cfg, client_tx, shutdown))
}

fn spawn_worker(
    cfg: Arc<Config>,
    octocrab: Arc<Octocrab>,
    shutdown: watch::Receiver<()>,
    attempt: u64,
) -> tokio::task::JoinHandle<anyhow::Result<()>> {
    tokio::spawn(async move {
        // Open only the receiver; the queue writer owns the sender side.
        let rx = Receiver::open(&cfg.queue_path)?;
        tracing::debug!(
            task = "worker",
            attempt,
            side = "receiver",
            queue = %cfg.queue_path.display(),
            "Queue side opened",
        );
        let control = WorkerControl::new(shutdown, WorkerHooks::default());
        run_worker(cfg, rx, octocrab, control).await
    })
}

#[cfg(test)]
mod tests;
