//! Task orchestration for comenqd.
//!
//! Coordinates the listener and worker tasks, applying exponential backoff on
//! failure and handling graceful shutdown.

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
use tokio::sync::watch;

use crate::listener::run_listener;
use crate::queue::SharedQueue;
use crate::store::StoreError;
use crate::worker::{WorkerControl, WorkerHooks, build_octocrab, run_worker};

#[derive(Debug, Error)]
pub enum SupervisorError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Octocrab(#[from] octocrab::Error),
    #[error(transparent)]
    Store(#[from] StoreError),
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
                let delay = backoff.next().unwrap_or(BACKOFF_FALLBACK_DELAY);
                if sleep_or_shutdown(&mut shutdown, delay).await {
                    break;
                }
                backoff = backoff_builder();
                handle = spawn_fn();
            }
        }
    }
}

/// Start the daemon with the provided configuration.
pub async fn run(config: Config) -> Result<()> {
    ensure_queue_dir(&config.queue_path).await?;
    tracing::info!(queue = %config.queue_path.display(), "Queue directory prepared");
    let octocrab = Arc::new(build_octocrab(&config.github_token)?);
    let cfg = Arc::new(config);
    let queue = SharedQueue::open(cfg.clone())?;
    let (shutdown_tx, shutdown_rx) = watch::channel(());

    // Initial task spawns and backoff builders.
    let listener = spawn_listener(queue.clone(), shutdown_rx.clone());
    let worker = spawn_worker(queue.clone(), octocrab.clone(), shutdown_rx.clone());
    let min_delay = Duration::from_millis(cfg.restart_min_delay_ms);
    let listener_backoff = backoff(min_delay);
    let worker_backoff = backoff(min_delay);

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
    let shutdown_listener = shutdown_rx.clone();
    let shutdown_worker = shutdown_rx;
    let listener_queue = queue.clone();
    tokio::join!(
        supervise_task(
            "listener",
            listener,
            listener_backoff,
            || spawn_listener(listener_queue.clone(), shutdown_listener.clone()),
            shutdown_listener.clone(),
            || backoff(min_delay),
        ),
        supervise_task(
            "worker",
            worker,
            worker_backoff,
            || spawn_worker(queue.clone(), octocrab.clone(), shutdown_worker.clone()),
            shutdown_worker.clone(),
            || backoff(min_delay),
        ),
    );

    Ok(())
}

fn spawn_listener(
    queue: Arc<SharedQueue>,
    shutdown: watch::Receiver<()>,
) -> tokio::task::JoinHandle<anyhow::Result<()>> {
    tokio::spawn(run_listener(queue, shutdown))
}

fn spawn_worker(
    queue: Arc<SharedQueue>,
    octocrab: Arc<Octocrab>,
    shutdown: watch::Receiver<()>,
) -> tokio::task::JoinHandle<anyhow::Result<()>> {
    let control = WorkerControl::new(shutdown, WorkerHooks::default());
    tokio::spawn(run_worker(queue, octocrab, control))
}

/// Log any failure from a supervised task.
///
/// Accepts the task name and the result yielded when awaiting its
/// [`JoinHandle`](tokio::task::JoinHandle). This is a no-op when the task
/// completes successfully. On failure it logs the error and tags it with
/// `kind` to distinguish an `inner_error` from a `join_error`.
fn log_task_failure<T, E>(task: &str, res: &std::result::Result<anyhow::Result<T>, E>)
where
    E: std::fmt::Display,
{
    match res {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => tracing::error!(
            task = task,
            kind = "inner_error",
            error = %e,
            "Task failed"
        ),
        Err(e) => tracing::error!(
            task = task,
            kind = "join_error",
            error = %e,
            "Task failed"
        ),
    }
}

#[cfg(test)]
mod tests;
