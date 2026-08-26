//! Queue worker for comenqd.
//!
//! Watches the shared queue and posts the head comment once its estimated
//! posting time arrives. Cooldowns always run in full; each entry's random
//! flutter was fixed when it was enqueued, so the projected schedule reported
//! to clients matches what the worker executes.

use crate::config::Config;
use crate::metrics;
use crate::queue::SharedQueue;
use anyhow::Result;
use comenq_lib::CommentRequest;
use octocrab::Octocrab;
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
#[cfg(any(test, feature = "test-support"))]
use tokio::sync::Notify;
use tokio::sync::watch;

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

/// Post one queued request while recording its duration and bounded outcome.
#[tracing::instrument(
    skip(octocrab, request, config),
    fields(task = "worker", outcome = tracing::field::Empty)
)]
async fn post_comment_with_metrics(
    octocrab: &Octocrab,
    request: &CommentRequest,
    config: &Config,
) -> Result<(), PostCommentError> {
    let start = Instant::now();
    let result = post_comment(octocrab, request, config).await;
    metrics::record_github_post_duration(start.elapsed());
    let outcome = match &result {
        Ok(()) => "success",
        Err(PostCommentError::Api(_)) => "api_error",
        Err(PostCommentError::Timeout) => "timeout",
    };
    tracing::Span::current().record("outcome", outcome);
    metrics::record_github_post_outcome(outcome);
    result
}

/// Hooks used to observe worker progress during tests.
///
/// Each hook uses [`Notify::notify_one`] which buffers a single permit for
/// one waiting task. This design supports exactly one waiter per hook; if
/// multiple tasks await the same hook, only one will be woken per notification.
#[derive(Default)]
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
}

/// Wait for a retry cooldown unless the daemon is shutting down.
async fn wait_or_shutdown(secs: u64, shutdown: &mut watch::Receiver<()>) -> bool {
    tokio::select! {
        biased;
        _ = shutdown.changed() => true,
        _ = tokio::time::sleep(Duration::from_secs(secs)) => false,
    }
}

/// Controls the worker task.
///
/// Bundles the shutdown signal and optional test hooks to keep the worker API
/// concise.
pub struct WorkerControl {
    /// Watch channel used to signal graceful shutdown.
    pub shutdown: watch::Receiver<()>,
    /// Optional test-observation hooks.
    #[cfg(any(test, feature = "test-support"))]
    /// Hooks for observing worker progress during tests.
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
    let config = queue.config().clone();
    loop {
        let due = queue.next_due().await?;
        let Some((entry, wait_seconds)) = due else {
            control.notify_drained();
            tokio::select! {
                biased;
                _ = control.shutdown.changed() => break,
                () = queue.changed() => continue,
            }
        };
        if wait_seconds > 0 {
            metrics::record_cooldown_wait(wait_seconds);
            tokio::select! {
                biased;
                _ = control.shutdown.changed() => break,
                () = queue.changed() => {}
                _ = tokio::time::sleep(Duration::from_secs(wait_seconds)) => {}
            }
            continue;
        }
        control.notify_enqueued();
        match post_comment_with_metrics(&octocrab, &entry.request, &config).await {
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
                control.notify_idle();
                // Pace retries so a persistently failing API is not hammered.
                metrics::record_cooldown_wait(config.cooldown_period_seconds);
                tokio::select! {
                    should_shutdown = wait_or_shutdown(
                        config.cooldown_period_seconds,
                        &mut control.shutdown,
                    ) => {
                        if should_shutdown {
                            break;
                        }
                    }
                    () = queue.changed() => {}
                }
                continue;
            }
        }
        control.notify_idle();
    }
    Ok(())
}

#[cfg(test)]
mod tests;
