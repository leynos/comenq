//! Worker shutdown controls and test-observation hooks.
//!
//! Keeps the public worker control surface separate from queue processing so
//! production code and tests share the same shutdown contract.

use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::{Notify, watch};

#[cfg(any(test, feature = "test-support"))]
use crate::util::is_metadata_file;
#[cfg(any(test, feature = "test-support"))]
use std::fs as stdfs;
#[cfg(any(test, feature = "test-support"))]
use std::path::Path;

/// Hooks used to observe worker progress during tests.
///
/// Each hook uses [`Notify::notify_one`] which buffers a single permit for
/// one waiting task. This design supports exactly one waiter per hook; if
/// multiple tasks await the same hook, only one will be woken per notification.
#[derive(Default)]
pub struct WorkerHooks {
    /// Signalled when a request is retrieved from the queue.
    ///
    /// Only one waiter is supported; additional waiters will not be notified.
    pub enqueued: Option<Arc<Notify>>,
    /// Signalled after the worker completes processing of a request.
    ///
    /// Only one waiter is supported; additional waiters will not be notified.
    pub idle: Option<Arc<Notify>>,
    /// Signalled when the queue is empty and the worker is idle.
    ///
    /// Only one waiter is supported; additional waiters will not be notified.
    pub drained: Option<Arc<Notify>>,
    /// Records completion of a cooldown wait in unit tests.
    #[cfg(test)]
    pub cooldown_complete: Option<Arc<AtomicBool>>,
    /// Signals that a unit-test cooldown wait has started.
    #[cfg(test)]
    pub cooldown_started: Option<Arc<Notify>>,
}

impl WorkerHooks {
    pub(super) fn notify_enqueued(&self) {
        if let Some(n) = &self.enqueued {
            n.notify_one();
        }
    }

    pub(super) fn notify_idle(&self) {
        if let Some(n) = &self.idle {
            n.notify_one();
        }
    }

    #[cfg(test)]
    pub(super) fn notify_cooldown_started(&self) {
        if let Some(started) = &self.cooldown_started {
            started.notify_one();
        }
    }

    #[cfg(test)]
    pub(super) fn notify_cooldown_complete(&self) {
        if let Some(completed) = &self.cooldown_complete {
            completed.store(true, Ordering::SeqCst);
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn notify_drained_if_empty(&self, queue_path: &Path) -> std::io::Result<()> {
        if let Some(n) = &self.drained {
            // Ignore sentinel files left by the queue implementation and
            // consider the directory empty when no other files remain.
            let empty = !stdfs::read_dir(queue_path)?
                .filter_map(Result::ok)
                .any(|e| !is_metadata_file(e.file_name()));
            if empty {
                n.notify_one();
            }
        }
        Ok(())
    }

    /// Waits for the specified number of seconds or until a shutdown is signalled.
    ///
    /// Returns `true` if shutdown was signalled, `false` if the timeout expired.
    ///
    /// # Arguments
    ///
    /// - `secs` - Number of seconds to wait before continuing.
    /// - `shutdown` - Watch channel signalled when the worker should cease waiting.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tokio::sync::watch;
    /// use comenqd::daemon::WorkerHooks;
    ///
    /// # tokio::runtime::Runtime::new().expect("runtime").block_on(async {
    /// let (tx, mut rx) = watch::channel(());
    ///
    /// // Wait for the full second when no shutdown signal is sent.
    /// assert!(!WorkerHooks::wait_or_shutdown(1, &mut rx).await);
    ///
    /// // Sending a shutdown signal returns immediately.
    /// let mut rx = tx.subscribe();
    /// tx.send(()).expect("notify shutdown");
    /// assert!(WorkerHooks::wait_or_shutdown(60, &mut rx).await);
    /// # });
    /// ```
    ///
    /// Passing `secs = 0` returns immediately with `false` unless shutdown was
    /// already signalled.
    pub async fn wait_or_shutdown(secs: u64, shutdown: &mut watch::Receiver<()>) -> bool {
        tokio::select! {
            biased;
            _ = shutdown.changed() => true,
            _ = tokio::time::sleep(Duration::from_secs(secs)) => false,
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
    #[cfg(test)]
    pub(super) test_flutter: Option<u64>,
}

impl WorkerControl {
    /// Create a new [`WorkerControl`].
    ///
    /// # Examples
    ///
    /// ```rust
    /// use comenqd::daemon::{WorkerControl, WorkerHooks};
    /// use tokio::sync::watch;
    ///
    /// let (_tx, rx) = watch::channel(());
    /// let hooks = WorkerHooks::default();
    /// let control = WorkerControl::new(rx, hooks);
    /// ```
    pub fn new(shutdown: watch::Receiver<()>, hooks: WorkerHooks) -> Self {
        Self {
            shutdown,
            hooks,
            #[cfg(test)]
            test_flutter: None,
        }
    }

    #[cfg(test)]
    pub(super) fn with_test_flutter(mut self, flutter: u64) -> Self {
        self.test_flutter = Some(flutter);
        self
    }
}
