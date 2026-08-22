//! Queue worker for comenqd.
//!
//! Dequeues requests from the persistent queue and posts comments to GitHub
//! while enforcing a cooldown between attempts. The cooldown always runs in
//! full; an optional random flutter lengthens each wait to avoid a perfectly
//! regular posting cadence.

use crate::config::Config;
use crate::metrics;
use anyhow::Result;
use comenq_lib::CommentRequest;
use octocrab::Octocrab;
use rand::Rng;
use std::ops::RangeInclusive;
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::sync::watch;
use yaque::Receiver;

mod control;
pub use control::{WorkerControl, WorkerHooks};

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

/// Seconds to wait after processing a request: the configured cooldown plus
/// a fresh random flutter of up to `cooldown_flutter_seconds`.
///
/// Flutter only ever lengthens the wait — the full cooldown always elapses —
/// so the posting cadence stays below GitHub's secondary rate limits while
/// avoiding a perfectly regular interval.
fn cooldown_with_flutter(config: &Config) -> u64 {
    cooldown_with_flutter_using(config, |range| rand::rng().random_range(range))
}

/// Calculate the cooldown using a supplied flutter selection.
fn cooldown_with_flutter_using<F>(config: &Config, select_flutter: F) -> u64
where
    F: FnOnce(RangeInclusive<u64>) -> u64,
{
    let flutter = config.cooldown_flutter_seconds;
    if flutter == 0 {
        return config.cooldown_period_seconds;
    }
    let jitter = select_flutter(0..=flutter);
    cooldown_with_selected_flutter(config, jitter)
}

/// Apply a selected flutter duration without overflowing the cooldown.
fn cooldown_with_selected_flutter(config: &Config, jitter: u64) -> u64 {
    config.cooldown_period_seconds.saturating_add(jitter)
}

#[tracing::instrument(
    skip(config, shutdown),
    fields(
        task = "worker",
        base_seconds = config.cooldown_period_seconds,
        flutter_seconds = config.cooldown_flutter_seconds,
    )
)]
#[cfg(not(test))]
async fn wait_for_cooldown(config: &Config, shutdown: &mut watch::Receiver<()>) -> bool {
    let wait_seconds = cooldown_with_flutter(config);
    tracing::debug!(
        task = "worker",
        base_seconds = config.cooldown_period_seconds,
        flutter_seconds = config.cooldown_flutter_seconds,
        wait_seconds,
        "Waiting before the next queue attempt",
    );
    metrics::record_cooldown_wait(wait_seconds);
    WorkerHooks::wait_or_shutdown(wait_seconds, shutdown).await
}

#[cfg(test)]
async fn wait_for_cooldown(
    config: &Config,
    shutdown: &mut watch::Receiver<()>,
    hooks: &WorkerHooks,
    test_flutter: Option<u64>,
) -> bool {
    let wait_seconds = test_flutter.map_or_else(
        || cooldown_with_flutter(config),
        |jitter| cooldown_with_flutter_using(config, |_| jitter),
    );
    tracing::debug!(
        task = "worker",
        base_seconds = config.cooldown_period_seconds,
        flutter_seconds = config.cooldown_flutter_seconds,
        wait_seconds,
        "Waiting before the next queue attempt",
    );
    metrics::record_cooldown_wait(wait_seconds);
    let interrupted = wait_or_shutdown_after_cooldown_starts(wait_seconds, shutdown, hooks).await;
    if !interrupted {
        hooks.notify_cooldown_complete();
    }
    interrupted
}

#[cfg(test)]
async fn wait_or_shutdown_after_cooldown_starts(
    wait_seconds: u64,
    shutdown: &mut watch::Receiver<()>,
    hooks: &WorkerHooks,
) -> bool {
    let sleep = tokio::time::sleep(Duration::from_secs(wait_seconds));
    tokio::pin!(sleep);
    let mut started = false;
    tokio::select! {
        biased;
        _ = shutdown.changed() => true,
        _ = std::future::poll_fn(|cx| {
            let poll = sleep.as_mut().poll(cx);
            if !started {
                started = true;
                hooks.notify_cooldown_started();
            }
            poll
        }) => false,
    }
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
async fn post_comment_with_metrics(
    octocrab: &Octocrab,
    request: &CommentRequest,
    config: &Config,
) -> Result<(), PostCommentError> {
    let start = Instant::now();
    let result = post_comment(octocrab, request, config).await;
    metrics::record_github_post_duration(start.elapsed());
    metrics::record_github_post_outcome(match &result {
        Ok(()) => "success",
        Err(PostCommentError::Api(_)) => "api_error",
        Err(PostCommentError::Timeout) => "timeout",
    });
    result
}

/// Processes queued comment requests and posts them to GitHub, enforcing a cooldown between attempts.
pub async fn run_worker(
    config: Arc<Config>,
    mut rx: Receiver,
    octocrab: Arc<Octocrab>,
    mut control: WorkerControl,
) -> Result<()> {
    #[cfg(test)]
    let test_flutter = control.test_flutter;
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
                tracing::error!(error = %e, "Failed to deserialize queued request; dropping");
                if let Err(commit_err) = guard.commit() {
                    tracing::error!(error = %commit_err, "Failed to commit malformed queue entry");
                }
                hooks.notify_idle();
                #[cfg(any(test, feature = "test-support"))]
                if let Err(check_err) = hooks.notify_drained_if_empty(&config.queue_path) {
                    tracing::warn!(error = %check_err, "Queue emptiness check failed after drop");
                }
                #[cfg(test)]
                let interrupted = wait_for_cooldown(&config, shutdown, hooks, test_flutter).await;
                #[cfg(not(test))]
                let interrupted = wait_for_cooldown(&config, shutdown).await;
                if interrupted {
                    break;
                }
                continue;
            }
        };

        match post_comment_with_metrics(&octocrab, &request, &config).await {
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
        #[cfg(test)]
        let interrupted = wait_for_cooldown(&config, shutdown, hooks, test_flutter).await;
        #[cfg(not(test))]
        let interrupted = wait_for_cooldown(&config, shutdown).await;
        if interrupted {
            break;
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    hooks.notify_drained_if_empty(&config.queue_path)?;
    Ok(())
}

#[cfg(test)]
mod tests;
