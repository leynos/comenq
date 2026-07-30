//! Queue worker for comenqd.
//!
//! Watches the shared queue and posts the head comment once its estimated
//! posting time arrives. Cooldowns always run in full; each entry's random
//! flutter was fixed when it was enqueued, so the projected schedule reported
//! to clients matches what the worker executes.
//!
//! Posts rotate round-robin through the configured token pool: each post
//! uses the successor of the token whose hash the most recent history
//! record carries, so rotation survives restarts and redeployments.

use crate::config::Config;
use crate::queue::SharedQueue;
use anyhow::{Context as _, Result};
use comenq_lib::CommentRequest;
use octocrab::Octocrab;
use sha2::{Digest as _, Sha256};
use std::fmt::Write as _;
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

/// Hex-encoded SHA-256 hash of a token value.
///
/// History records identify tokens by this hash rather than by name or
/// value, so rotation state survives token files being renamed and the log
/// never holds a secret.
///
/// # Examples
///
/// ```rust
/// let hash = comenqd::daemon::token_hash("s3cret");
/// assert_eq!(hash.len(), 64);
/// assert_eq!(hash, comenqd::daemon::token_hash("s3cret"));
/// ```
#[must_use]
pub fn token_hash(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    let mut hex = String::with_capacity(64);
    for byte in digest {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// A GitHub client paired with its token's name and hash.
#[derive(Clone)]
pub struct TokenClient {
    /// Token file name, used only for logging.
    pub name: String,
    /// Hex SHA-256 of the token value, recorded in the history.
    pub hash: String,
    /// Client authenticated with the token.
    pub octocrab: Arc<Octocrab>,
}

impl TokenClient {
    /// Pair `octocrab` (authenticated with `token`) with the token's
    /// identifying name and hash.
    #[must_use]
    pub fn new(name: impl Into<String>, token: &str, octocrab: Arc<Octocrab>) -> Self {
        Self {
            name: name.into(),
            hash: token_hash(token),
            octocrab,
        }
    }
}

/// Index of the token to use for the next post.
///
/// The successor of the token whose hash matches `last_hash`; the first
/// token when the history is empty or names a token no longer configured.
fn next_token_index(clients: &[TokenClient], last_hash: Option<&str>) -> usize {
    last_hash
        .and_then(|hash| clients.iter().position(|client| client.hash == hash))
        .map_or(0, |index| index.saturating_add(1) % clients.len().max(1))
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
/// queue's change signal interrupts any wait. Each post rotates to the next
/// token in `clients` after the one the latest history record names.
pub async fn run_worker(
    queue: Arc<SharedQueue>,
    clients: Arc<Vec<TokenClient>>,
    mut control: WorkerControl,
) -> Result<()> {
    anyhow::ensure!(
        !clients.is_empty(),
        "the worker needs at least one GitHub token"
    );
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
        let last_hash = queue.last_token_hash().await?;
        let index = next_token_index(&clients, last_hash.as_deref());
        let client = clients.get(index).context("token index in range")?;
        match post_comment(&client.octocrab, &entry.request, &config).await {
            Ok(()) => {
                queue.complete(&entry, &client.hash).await?;
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    id = %entry.id,
                    token = %client.name,
                    owner = %entry.request.owner,
                    repo = %entry.request.repo,
                    pr = entry.request.pr_number,
                    "GitHub API call failed; will retry after cooldown",
                );
                // A history write failure must not stop the retry loop.
                if let Err(log_error) = queue
                    .record_failure(&entry, &client.hash, &e.to_string())
                    .await
                {
                    tracing::error!(
                        error = %log_error,
                        id = %entry.id,
                        "Failed to record the posting failure in the history log",
                    );
                }
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
