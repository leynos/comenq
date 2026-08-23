//! Persistent queue writing and recovery supervision.
//!
//! Keeps accepted client payloads outside the restartable writer task, so a
//! task panic or sender-open failure cannot silently discard buffered work.

use super::observability::log_task_failure;
use super::{Config, Path, metrics, sleep_or_shutdown};
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tracing::Instrument;
use yaque::Sender;

/// Maximum consecutive queue-writer recovery attempts before shutdown.
pub(super) const MAX_WRITER_RESTARTS: usize = 5;

/// Build the finite retry schedule used only for queue-writer recovery.
pub(super) fn writer_backoff(min_delay: Duration) -> impl Iterator<Item = Duration> {
    super::backoff(min_delay).take(MAX_WRITER_RESTARTS)
}

/// Recovery state owned by the writer supervisor.
///
/// At most one writer task may call `recv` at a time: the supervisor always
/// awaits or aborts the previous task before spawning a replacement. The
/// receiver lock therefore protects hand-off across task cancellation. It is
/// intentionally held while awaiting a message so a received payload is
/// recorded in `pending` before any replacement can access the receiver.
pub(super) struct QueueWriterState {
    pub(super) receiver: Arc<tokio::sync::Mutex<mpsc::Receiver<Vec<u8>>>>,
    pub(super) pending: Arc<tokio::sync::Mutex<Option<Vec<u8>>>>,
}

/// Reason the queue writer stopped processing client requests.
pub(super) enum QueueWriterExit {
    /// All client senders were dropped, so no recovery is required.
    ClientChannelClosed,
    /// Persistent enqueue failed and the retained payload needs recovery.
    EnqueueFailed(QueueWriterState),
}

impl QueueWriterState {
    pub(super) fn new(receiver: mpsc::Receiver<Vec<u8>>) -> Self {
        Self {
            receiver: Arc::new(tokio::sync::Mutex::new(receiver)),
            pending: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }
}

impl Clone for QueueWriterState {
    fn clone(&self) -> Self {
        Self {
            receiver: Arc::clone(&self.receiver),
            pending: Arc::clone(&self.pending),
        }
    }
}

#[tracing::instrument(skip_all, fields(task = "writer", queue = %cfg.queue_path.display()))]
pub(super) async fn supervise_writer<I, O>(
    mut handle: tokio::task::JoinHandle<QueueWriterExit>,
    recovery_state: QueueWriterState,
    mut backoff: I,
    cfg: Arc<Config>,
    mut open_sender: O,
    shutdown_tx: watch::Sender<()>,
    mut shutdown: watch::Receiver<()>,
) where
    I: Iterator<Item = Duration>,
    O: FnMut(&Path) -> anyhow::Result<Sender>,
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
                let state = match res {
                    Ok(QueueWriterExit::ClientChannelClosed) => break,
                    Ok(QueueWriterExit::EnqueueFailed(state)) => state,
                    Err(e) => {
                        // Only log join failures here; queue_writer logs enqueue errors.
                        log_task_failure::<(), _>("writer", &Err(e));
                        recovery_state.clone()
                    }
                };
                restart_attempt = restart_attempt.saturating_add(1);
                metrics::record_task_restart("writer");
                let Some(delay) = backoff.next() else {
                    tracing::error!(
                        task = "writer",
                        attempt = restart_attempt,
                        queue = %cfg.queue_path.display(),
                        "Queue writer restart limit reached; shutting down daemon",
                    );
                    let _ = shutdown_tx.send(());
                    break;
                };
                tracing::warn!(
                    task = "writer",
                    attempt = restart_attempt,
                    delay_ms = delay.as_millis(),
                    queue = %cfg.queue_path.display(),
                    "Scheduling task restart",
                );
                if sleep_or_shutdown(&mut shutdown, delay).await {
                    break;
                }
                match open_sender(&cfg.queue_path) {
                    Ok(queue_tx) => {
                        tracing::debug!(
                            task = "writer",
                            attempt = restart_attempt,
                            side = "sender",
                            queue = %cfg.queue_path.display(),
                            "Queue side reopened",
                        );
                        handle = spawn_queue_writer(queue_tx, state, &cfg.queue_path, restart_attempt);
                    }
                    Err(e) => {
                        tracing::error!(
                            task = "writer",
                            attempt = restart_attempt,
                            side = "sender",
                            queue = %cfg.queue_path.display(),
                            error = %e,
                            "Queue sender creation failed",
                        );
                        handle = tokio::spawn(async move { QueueWriterExit::EnqueueFailed(state) });
                    }
                }
            }
        }
    }
}

/// Forward bytes from a channel into the persistent queue.
///
/// The daemon's supervised writer retains failed payloads in its recovery
/// state. This standalone helper preserves the established public API used by
/// tests and callers that do not need restart supervision.
pub async fn queue_writer(
    mut sender: Sender,
    mut receiver: mpsc::Receiver<Vec<u8>>,
) -> mpsc::Receiver<Vec<u8>> {
    while let Some(bytes) = receiver.recv().await {
        metrics::record_client_channel_depth(receiver.len());
        if let Err(e) = sender.send(&bytes).await {
            metrics::record_queue_writer_failure();
            tracing::error!(error = %e, "Queue enqueue failed");
            break;
        }
    }
    receiver
}

pub(super) fn spawn_queue_writer(
    sender: Sender,
    state: QueueWriterState,
    queue_path: &Path,
    attempt: u64,
) -> tokio::task::JoinHandle<QueueWriterExit> {
    let span = tracing::info_span!(
        "queue_writer",
        task = "writer",
        queue = %queue_path.display(),
        attempt,
    );
    tokio::spawn(run_queue_writer(sender, state).instrument(span))
}

pub(super) async fn run_queue_writer(sender: Sender, state: QueueWriterState) -> QueueWriterExit {
    run_queue_writer_with_after_enqueue(sender, state, || std::future::ready(())).await
}

async fn run_queue_writer_with_after_enqueue<F, Fut>(
    mut sender: Sender,
    state: QueueWriterState,
    mut after_enqueue: F,
) -> QueueWriterExit
where
    F: FnMut() -> Fut,
    Fut: Future<Output = ()>,
{
    loop {
        let bytes = {
            let mut pending = state.pending.lock().await;
            match pending.as_ref() {
                Some(bytes) => bytes.clone(),
                None => {
                    let mut receiver = state.receiver.lock().await;
                    let Some(bytes) = receiver.recv().await else {
                        return QueueWriterExit::ClientChannelClosed;
                    };
                    let depth = receiver.len();
                    drop(receiver);
                    metrics::record_client_channel_depth(depth);
                    *pending = Some(bytes.clone());
                    bytes
                }
            }
        };
        if let Err(e) = sender.send(&bytes).await {
            metrics::record_queue_writer_failure();
            tracing::error!(error = %e, "Queue enqueue failed");
            return QueueWriterExit::EnqueueFailed(state);
        }
        after_enqueue().await;
        *state.pending.lock().await = None;
    }
}

#[cfg(test)]
mod tests;
