//! Queue worker for comenqd.
//!
//! Watches the shared queue and posts the head comment once its estimated
//! posting time arrives. Cooldowns always run in full; each entry's random
//! flutter was fixed when it was enqueued, so the projected schedule reported
//! to clients matches what the worker executes.

use crate::config::Config;
use crate::queue::SharedQueue;
use anyhow::Result;
use comenq_lib::CommentRequest;
use octocrab::Octocrab;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::{Notify, watch};

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
#[expect(clippy::result_large_err, reason = "propagate Octocrab errors")]
pub(crate) fn build_octocrab(token: &str) -> octocrab::Result<Octocrab> {
    Octocrab::builder()
        .personal_token(token.to_string())
        .build()
}

async fn post_comment(
    octocrab: &Octocrab,
    request: &CommentRequest,
    config: &Config,
) -> Result<(), PostCommentError> {
    let issues = octocrab.issues(&request.owner, &request.repo);
    let fut = issues.create_comment(request.pr_number, &request.body);
    match tokio::time::timeout(Duration::from_secs(config.github_api_timeout_secs), fut).await {
        Ok(res) => res.map(|_| ()).map_err(PostCommentError::Api),
        Err(_) => Err(PostCommentError::Timeout),
    }
}

/// Hooks used to observe worker progress during tests.
///
/// Each hook uses [`Notify::notify_one`] which buffers a single permit for
/// one waiting task. This design supports exactly one waiter per hook; if
/// multiple tasks await the same hook, only one will be woken per notification.
#[derive(Default)]
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
}

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
        Self { shutdown, hooks }
    }
}

/// Posts queued comments as they fall due, enforcing the scheduled cooldowns.
///
/// The worker recomputes the head entry's due time on every iteration, so
/// queue mutations (put, bump, bust, del) take effect immediately: the shared
/// queue's change signal interrupts any wait.
pub async fn run_worker(
    queue: Arc<SharedQueue>,
    octocrab: Arc<Octocrab>,
    mut control: WorkerControl,
) -> Result<()> {
    let hooks = &control.hooks;
    let shutdown = &mut control.shutdown;
    let config = queue.config().clone();
    loop {
        let due = queue.next_due().await?;
        let Some((entry, wait_seconds)) = due else {
            hooks.notify_drained();
            tokio::select! {
                biased;
                _ = shutdown.changed() => break,
                () = queue.changed() => continue,
            }
        };
        if wait_seconds > 0 {
            tokio::select! {
                biased;
                _ = shutdown.changed() => break,
                () = queue.changed() => {}
                _ = tokio::time::sleep(Duration::from_secs(wait_seconds)) => {}
            }
            continue;
        }
        hooks.notify_enqueued();
        match post_comment(&octocrab, &entry.request, &config).await {
            Ok(()) => {
                queue.complete(&entry.id).await?;
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    id = %entry.id,
                    owner = %entry.request.owner,
                    repo = %entry.request.repo,
                    pr = entry.request.pr_number,
                    "GitHub API call failed; will retry after cooldown",
                );
                hooks.notify_idle();
                // Pace retries so a persistently failing API is not hammered.
                if WorkerHooks::wait_or_shutdown(config.cooldown_period_seconds, shutdown).await {
                    break;
                }
                continue;
            }
        }
        hooks.notify_idle();
    }
    Ok(())
}

#[cfg(test)]
mod tests;
