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

#[cfg(test)]
use std::fs as stdfs;
#[cfg(test)]
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
#[derive(Default)]
pub struct WorkerHooks {
    /// Signalled when a request is retrieved from the queue.
    pub enqueued: Option<Arc<Notify>>,
    /// Signalled after the worker completes processing of a request.
    pub idle: Option<Arc<Notify>>,
    /// Signalled when the queue is empty and the worker is idle.
    #[cfg_attr(not(test), allow(dead_code, reason = "test hook"))]
    pub drained: Option<Arc<Notify>>,
}

impl WorkerHooks {
    fn notify_enqueued(&self) {
        if let Some(n) = &self.enqueued {
            n.notify_waiters();
        }
    }

    fn notify_idle(&self) {
        if let Some(n) = &self.idle {
            n.notify_waiters();
        }
    }

    #[cfg(test)]
    fn notify_drained_if_empty(&self, queue_path: &Path) -> std::io::Result<()> {
        if let Some(n) = &self.drained {
            // Ignore sentinel files left by the queue implementation and
            // consider the directory empty when no other files remain.
            let empty = !stdfs::read_dir(queue_path)?
                .filter_map(Result::ok)
                .any(|e| {
                    let name = e.file_name();
                    let name = name.to_string_lossy();
                    name != "version" && name != "recv.lock"
                });
            if empty {
                n.notify_waiters();
            }
        }
        Ok(())
    }

    /// Waits for the specified number of seconds or until a shutdown is signalled.
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
    /// use comenqd::worker::WorkerHooks;
    ///
    /// # tokio::runtime::Runtime::new().expect("runtime").block_on(async {
    /// let (tx, mut rx) = watch::channel(());
    ///
    /// // Wait for the full second when no shutdown signal is sent.
    /// WorkerHooks::wait_or_shutdown(1, &mut rx).await;
    ///
    /// // Sending a shutdown signal returns immediately.
    /// let mut rx = tx.subscribe();
    /// tx.send(()).expect("notify shutdown");
    /// WorkerHooks::wait_or_shutdown(60, &mut rx).await;
    /// # });
    /// ```
    ///
    /// The function returns after either the timeout or a shutdown signal,
    /// without indicating which occurred. Passing `secs = 0` returns immediately.
    pub async fn wait_or_shutdown(secs: u64, shutdown: &mut watch::Receiver<()>) {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(secs)) => {},
            _ = shutdown.changed() => {},
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
    /// use comenqd::worker::{WorkerControl, WorkerHooks};
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
            res = rx.recv() => res?,
            _ = shutdown.changed() => break,
        };
        hooks.notify_enqueued();
        let request: CommentRequest = match serde_json::from_slice(&guard) {
            Ok(req) => req,
            Err(e) => {
                tracing::error!(error = %e, "Failed to deserialise queued request; dropping");
                if let Err(commit_err) = guard.commit() {
                    tracing::error!(error = %commit_err, "Failed to commit malformed queue entry");
                }
                hooks.notify_idle();
                #[cfg(test)]
                if let Err(check_err) = hooks.notify_drained_if_empty(&config.queue_path) {
                    tracing::warn!(error = %check_err, "Queue emptiness check failed after drop");
                }
                WorkerHooks::wait_or_shutdown(config.cooldown_period_seconds, shutdown).await;
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
        #[cfg(test)]
        hooks.notify_drained_if_empty(&config.queue_path)?;
        WorkerHooks::wait_or_shutdown(config.cooldown_period_seconds, shutdown).await;
    }
    #[cfg(test)]
    hooks.notify_drained_if_empty(&config.queue_path)?;
    Ok(())
}
