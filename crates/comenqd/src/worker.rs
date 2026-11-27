//! Queue worker for comenqd.
//!
//! Watches the shared queue and posts the head comment once its estimated
//! posting time arrives. Cooldowns always run in full; each entry's random
//! flutter was fixed when it was enqueued, so the projected schedule reported
//! to clients matches what the worker executes.

use crate::metrics;
use crate::queue::SharedQueue;
use anyhow::Result;
use std::{sync::Arc, time::Duration};
use tokio::sync::{Notify, watch};
use tokio::time::Instant;

mod delivery;
use delivery::{claim_due_or_wait, deliver_entry, pace_retry};
mod posting;
mod tokens;
pub(crate) use tokens::build_octocrab;
pub use tokens::{TokenClient, token_hash};

const RETRY_OVERFLOW_FALLBACK: Duration = Duration::from_secs(100 * 365 * 24 * 60 * 60);

/// Hooks used to observe worker progress during tests.
///
/// Each hook uses [`Notify::notify_one`] which buffers a single permit for
/// one waiting task. This design supports exactly one waiter per hook; if
/// multiple tasks await the same hook, only one will be woken per notification.
#[derive(Clone, Default)]
#[cfg(any(test, feature = "test-support"))]
pub struct WorkerHooks {
    /// Signalled when the worker picks up a due entry for posting.
    ///
    /// Only one waiter is supported; additional waiters will not be notified.
    pub enqueued: Option<Arc<Notify>>,
    /// Signalled after the worker completes processing of an entry.
    ///
    /// Only one waiter is supported; additional waiters will not be notified.
    pub idle: Option<Arc<Notify>>,
    /// Signalled when the queue is empty and the worker is idle.
    ///
    /// Only one waiter is supported; additional waiters will not be notified.
    pub drained: Option<Arc<Notify>>,
    /// Signalled after the worker registers its queue-change wait for a
    /// deferred entry.
    ///
    /// Only one waiter is supported; additional waiters will not be notified.
    pub waiting: Option<Arc<Notify>>,
}

#[cfg(any(test, feature = "test-support"))]
impl WorkerHooks {
    fn notify_enqueued(&self) {
        if let Some(n) = &self.enqueued {
            n.notify_one();
        }
    }

    fn notify_idle(&self) {
        if let Some(n) = &self.idle {
            n.notify_one();
        }
    }

    fn notify_drained(&self) {
        if let Some(n) = &self.drained {
            n.notify_one();
        }
    }

    fn notify_waiting(&self) {
        if let Some(n) = &self.waiting {
            n.notify_one();
        }
    }
}

/// Wait for a failed post's retry deadline while still consuming queue changes.
///
/// Queue changes must not move this deadline earlier: otherwise an immediate
/// entry whose post failed could retry as soon as another client mutates the
/// queue. Consuming the notification here preserves the change signal without
/// allowing it to bypass the API-protection cooldown.
async fn wait_for_retry_deadline(
    deadline: Instant,
    changed: &Notify,
    shutdown: &mut watch::Receiver<()>,
) -> bool {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return false;
        }
        tokio::select! {
            biased;
            _ = shutdown.changed() => return true,
            () = changed.notified() => {}
            _ = tokio::time::sleep(remaining) => return false,
        }
    }
}

/// Waits for a scheduled entry while allowing queue changes to wake the worker.
///
/// `wait` is the time left until the head entry becomes due. A queue change
/// wakes the worker early, and the worker recomputes the due time on the next
/// iteration. Returns `true` only when shutdown was requested while waiting.
async fn wait_for_scheduled_entry(
    queue: &SharedQueue,
    wait: Duration,
    control: &mut WorkerControl,
) -> bool {
    let sleep = tokio::time::sleep(wait);
    tokio::pin!(sleep);
    let changed = queue.change_notifier().notified();
    tokio::pin!(changed);
    changed.as_mut().enable();

    #[cfg(any(test, feature = "test-support"))]
    let waiting_control = WorkerControl::new(control.shutdown.clone(), control.hooks.clone());
    #[cfg(any(test, feature = "test-support"))]
    let waiting = std::future::poll_fn(|_: &mut std::task::Context<'_>| {
        waiting_control.notify_waiting();
        std::task::Poll::<()>::Pending
    });
    #[cfg(not(any(test, feature = "test-support")))]
    let waiting = std::future::pending::<()>();
    tokio::pin!(waiting);

    tokio::select! {
        biased;
        _ = control.shutdown.changed() => true,
        () = changed.as_mut() => false,
        _ = sleep.as_mut() => false,
        () = waiting.as_mut() => unreachable!("waiting readiness future never completes"),
    }
}

/// Controls the worker task.
///
/// Bundles the shutdown signal and optional test hooks to keep the worker API
/// concise.
pub struct WorkerControl {
    /// Watch channel used to signal graceful shutdown.
    pub shutdown: watch::Receiver<()>,
    /// Optional feature-gated hooks for observing worker progress in tests.
    #[cfg(any(test, feature = "test-support"))]
    pub hooks: WorkerHooks,
}

impl WorkerControl {
    /// Create worker control without test-observation hooks.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use comenqd::daemon::WorkerControl;
    /// use tokio::sync::watch;
    ///
    /// let (_tx, rx) = watch::channel(());
    /// let control = WorkerControl::without_hooks(rx);
    /// ```
    pub fn without_hooks(shutdown: watch::Receiver<()>) -> Self {
        Self {
            shutdown,
            #[cfg(any(test, feature = "test-support"))]
            hooks: WorkerHooks::default(),
        }
    }

    /// Create worker control with hooks for test observation.
    #[cfg(any(test, feature = "test-support"))]
    pub fn new(shutdown: watch::Receiver<()>, hooks: WorkerHooks) -> Self {
        Self { shutdown, hooks }
    }

    #[cfg(any(test, feature = "test-support"))]
    fn notify_enqueued(&self) {
        self.hooks.notify_enqueued();
    }

    #[cfg(not(any(test, feature = "test-support")))]
    fn notify_enqueued(&self) {}

    #[cfg(any(test, feature = "test-support"))]
    fn notify_idle(&self) {
        self.hooks.notify_idle();
    }

    #[cfg(not(any(test, feature = "test-support")))]
    fn notify_idle(&self) {}

    #[cfg(any(test, feature = "test-support"))]
    fn notify_drained(&self) {
        self.hooks.notify_drained();
    }

    #[cfg(not(any(test, feature = "test-support")))]
    fn notify_drained(&self) {}

    #[cfg(any(test, feature = "test-support"))]
    fn notify_waiting(&self) {
        self.hooks.notify_waiting();
    }

    #[cfg(not(any(test, feature = "test-support")))]
    #[expect(dead_code, reason = "used only by the test-support readiness future")]
    fn notify_waiting(&self) {}
}

/// Posts queued comments as they fall due, enforcing the scheduled cooldowns.
///
/// The worker recomputes the head entry's due time on every iteration, so
/// queue mutations (put, bump, bust, del) interrupt empty-queue and scheduled
/// entry waits. A failed-post retry still waits for the full cooldown after a
/// queue change to avoid retrying the same failed request early.
///
/// At least one token client is required; an empty pool returns an error
/// before the worker recovers or claims queue entries. Graceful shutdown
/// returns `Ok(())` after the worker stops.
///
/// # Examples
///
/// Start the worker with one authenticated client, then signal shutdown and
/// observe its successful completion:
///
/// ```rust,no_run
/// use comenqd::daemon::{TokenClient, WorkerControl, run_worker};
/// use comenqd::queue::SharedQueue;
/// use octocrab::Octocrab;
/// use std::sync::Arc;
/// use tokio::sync::watch;
///
/// async fn stop_worker(queue: Arc<SharedQueue>, token: &str) -> anyhow::Result<()> {
///     let octocrab = Arc::new(
///         Octocrab::builder()
///             .personal_token(token.to_owned())
///             .build()?,
///     );
///     let client = TokenClient::new("primary", token, octocrab);
///     let (shutdown_tx, shutdown_rx) = watch::channel(());
///     let worker = tokio::spawn(run_worker(
///         queue,
///         Arc::new(vec![client]),
///         WorkerControl::without_hooks(shutdown_rx),
///     ));
///     shutdown_tx.send(()).expect("worker is subscribed");
///     worker.await??;
///     Ok(())
/// }
/// ```
pub async fn run_worker(
    queue: Arc<SharedQueue>,
    clients: Arc<Vec<TokenClient>>,
    mut control: WorkerControl,
) -> Result<()> {
    anyhow::ensure!(
        !clients.is_empty(),
        "the worker needs at least one GitHub token"
    );
    queue.recover_worker_state().await?;
    let config = queue.config().clone();
    loop {
        let Some(scheduled) = claim_due_or_wait(&queue, &mut control).await? else {
            break;
        };
        let wait_seconds = scheduled.eta_seconds;
        let entry = scheduled.entry;
        if wait_seconds > 0 {
            metrics::record_cooldown_wait(wait_seconds);
            let wait = Duration::from_secs(wait_seconds);
            if wait_for_scheduled_entry(&queue, wait, &mut control).await {
                break;
            }
            continue;
        }
        control.notify_enqueued();
        let posted = deliver_entry(&queue, &clients, &entry, &config).await?;
        control.notify_idle();
        if !posted && pace_retry(&queue, &mut control, config.cooldown_period_seconds).await {
            break;
        }
    }
    Ok(())
}

/// Calculate a retry deadline without panicking on an unrepresentable cooldown.
fn retry_deadline(cooldown: Duration) -> Instant {
    let now = Instant::now();
    now.checked_add(cooldown)
        .or_else(|| now.checked_add(RETRY_OVERFLOW_FALLBACK))
        .unwrap_or(now)
}

#[cfg(test)]
mod tests;
// weave: run 'weave explain crates/comenqd/src/worker.rs' for per-hunk detail, 'weave check' to verify your resolution
