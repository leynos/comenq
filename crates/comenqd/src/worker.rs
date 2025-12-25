//! Queue worker for comenqd.
//!
//! Dequeues requests from the persistent queue and posts comments to GitHub
//! while enforcing a fixed cooldown between attempts.

use crate::config::Config;
use anyhow::Result;
use comenq_lib::CommentRequest;
use octocrab::Octocrab;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::{Notify, watch};
use yaque::Receiver;

#[cfg(any(test, feature = "test-support"))]
use crate::util::is_metadata_file;
#[cfg(any(test, feature = "test-support"))]
use std::fs as stdfs;
#[cfg(any(test, feature = "test-support"))]
use std::path::Path;

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
    #[cfg_attr(
        not(any(test, feature = "test-support")),
        expect(dead_code, reason = "test hook only used in test/test-support builds")
    )]
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

    #[cfg(any(test, feature = "test-support"))]
    fn notify_drained_if_empty(&self, queue_path: &Path) -> std::io::Result<()> {
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

/// Processes queued comment requests and posts them to GitHub, enforcing a cooldown between attempts.
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
            biased;
            _ = shutdown.changed() => {
                break;
            }
            res = rx.recv() => {
                res?
            }
        };
        hooks.notify_enqueued();
        let request: CommentRequest = match serde_json::from_slice::<CommentRequest>(&guard) {
            Ok(req) => req,
            Err(e) => {
                tracing::error!(error = %e, "Failed to deserialise queued request; dropping");
                if let Err(commit_err) = guard.commit() {
                    tracing::error!(error = %commit_err, "Failed to commit malformed queue entry");
                }
                hooks.notify_idle();
                #[cfg(any(test, feature = "test-support"))]
                if let Err(check_err) = hooks.notify_drained_if_empty(&config.queue_path) {
                    tracing::warn!(error = %check_err, "Queue emptiness check failed after drop");
                }
                if WorkerHooks::wait_or_shutdown(config.cooldown_period_seconds, shutdown).await {
                    break;
                }
                continue;
            }
        };

        match post_comment(&octocrab, &request, &config).await {
            Ok(_) => {
                guard.commit()?;
            }
            Err(PostCommentError::Api(e)) => {
                tracing::error!(
                    error = %e,
                    owner = %request.owner,
                    repo = %request.repo,
                    pr = request.pr_number,
                    "GitHub API call failed",
                );
            }
            Err(PostCommentError::Timeout) => {
                tracing::error!(
                    owner = %request.owner,
                    repo = %request.repo,
                    pr = request.pr_number,
                    "GitHub API call timed out",
                );
            }
        }

        hooks.notify_idle();
        #[cfg(any(test, feature = "test-support"))]
        hooks.notify_drained_if_empty(&config.queue_path)?;
        if WorkerHooks::wait_or_shutdown(config.cooldown_period_seconds, shutdown).await {
            break;
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    hooks.notify_drained_if_empty(&config.queue_path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use tokio::sync::watch;

    #[tokio::test]
    async fn wait_or_shutdown_returns_false_on_timeout() {
        let (_tx, mut rx) = watch::channel(());
        let start = Instant::now();
        let result = WorkerHooks::wait_or_shutdown(0, &mut rx).await;
        assert!(!result, "should return false when timeout expires");
        assert!(
            start.elapsed().as_millis() < 100,
            "zero-second wait should return immediately"
        );
    }

    #[tokio::test]
    async fn wait_or_shutdown_returns_true_on_shutdown() {
        let (tx, mut rx) = watch::channel(());
        // Signal shutdown before waiting
        tx.send(()).expect("send shutdown signal");
        let result = WorkerHooks::wait_or_shutdown(60, &mut rx).await;
        assert!(result, "should return true when shutdown is signalled");
    }

    #[tokio::test]
    async fn wait_or_shutdown_prioritises_shutdown_over_timeout() {
        let (tx, mut rx) = watch::channel(());
        // Send shutdown signal
        tx.send(()).expect("send shutdown signal");
        // Even with zero timeout, shutdown should be detected due to biased select
        let result = WorkerHooks::wait_or_shutdown(0, &mut rx).await;
        assert!(result, "biased select should prioritise shutdown signal");
    }
}
