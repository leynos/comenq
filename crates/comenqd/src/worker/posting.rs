//! GitHub comment posting with bounded timing and outcome metrics.
//!
//! Posting stays separate from queue state so the worker loop reads as claim,
//! post, complete or release. Every attempt records its duration and a bounded
//! outcome label; the span keeps the same name and fields across refactors.

use crate::config::Config;
use crate::metrics;
use comenq_lib::CommentRequest;
use octocrab::Octocrab;
use std::time::Duration;
use thiserror::Error;
use tokio::time::Instant;

/// Errors returned when posting a comment to GitHub.
#[derive(Debug, Error)]
pub(super) enum PostCommentError {
    /// The GitHub API request failed.
    #[error(transparent)]
    Api(#[from] octocrab::Error),
    /// The request timed out.
    #[error("timeout")]
    Timeout,
}

/// Submit a comment to GitHub with the configured API deadline.
///
/// A deadline expiry is kept distinct from an API response failure so the
/// worker can report a bounded outcome and retry the queued entry.
async fn post_comment(
    octocrab: &Octocrab,
    request: &CommentRequest,
    config: &Config,
) -> Result<String, PostCommentError> {
    let issues = octocrab.issues(&request.owner, &request.repo);
    let fut = issues.create_comment(request.pr_number, &request.body);
    match tokio::time::timeout(Duration::from_secs(config.github_api_timeout_secs), fut).await {
        Ok(res) => res
            .map(|comment| comment.html_url.to_string())
            .map_err(PostCommentError::Api),
        Err(_) => Err(PostCommentError::Timeout),
    }
}

/// Post one queued request while recording its duration and bounded outcome.
#[tracing::instrument(
    skip(octocrab, request, config),
    fields(task = "worker", outcome = tracing::field::Empty)
)]
pub(super) async fn post_comment_with_metrics(
    octocrab: &Octocrab,
    request: &CommentRequest,
    config: &Config,
) -> Result<String, PostCommentError> {
    let start = Instant::now();
    let result = post_comment(octocrab, request, config).await;
    metrics::record_github_post_duration(start.elapsed());
    let outcome = match &result {
        Ok(_) => "success",
        Err(PostCommentError::Api(_)) => "api_error",
        Err(PostCommentError::Timeout) => "timeout",
    };
    tracing::Span::current().record("outcome", outcome);
    metrics::record_github_post_outcome(outcome);
    result
}
