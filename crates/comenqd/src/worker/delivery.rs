//! Per-entry steps of the worker loop: claiming, delivering and retry pacing.

use super::posting::{PostCommentError, post_comment_with_metrics};
use super::tokens::next_token_index;
use super::{TokenClient, WorkerControl, retry_deadline, wait_for_retry_deadline};
use crate::config::Config;
use crate::metrics;
use crate::queue::SharedQueue;
use crate::queue::convert::request_from_comment;
use anyhow::{Context, Result};
use comenq_domain::{QueuedEntry, ScheduledEntry};
use std::time::Duration;

/// Post a due entry with the next token in rotation.
///
/// Returns `true` once the entry is posted and completed, or `false` after a
/// failed post has been recorded and its claim released for a retry.
pub(super) async fn deliver_entry(
    queue: &SharedQueue,
    clients: &[TokenClient],
    entry: &QueuedEntry,
    config: &Config,
) -> Result<bool> {
    let last_hash = last_token_hash_or_none(queue).await;
    let index = next_token_index(
        clients.iter().map(|client| client.hash.as_str()),
        last_hash.as_deref(),
    );
    let client = clients.get(index).context("token index in range")?;
    let request = request_from_comment(&entry.comment);
    match post_comment_with_metrics(&client.octocrab, &request, config).await {
        Ok(comment_url) => {
            queue
                .complete_entry(entry, &client.hash, Some(comment_url))
                .await?;
            Ok(true)
        }
        Err(failure) => {
            record_post_failure(queue, entry, client, &failure).await?;
            Ok(false)
        }
    }
}

/// Claim the next due entry, idling until the queue changes when none is due.
///
/// Returns `None` once shutdown is signalled while the queue is drained.
pub(super) async fn claim_due_or_wait(
    queue: &SharedQueue,
    control: &mut WorkerControl,
) -> Result<Option<ScheduledEntry>> {
    loop {
        if let Some(scheduled) = queue.claim_next_due_domain().await? {
            return Ok(Some(scheduled));
        }
        control.notify_drained();
        tokio::select! {
            biased;
            _ = control.shutdown.changed() => return Ok(None),
            () = queue.changed() => {}
        }
    }
}

/// Read the token hash of the latest posting, falling back to `None` (the
/// first configured token) when the history cannot be read.
async fn last_token_hash_or_none(queue: &SharedQueue) -> Option<String> {
    match queue.last_token_hash().await {
        Ok(last_hash) => last_hash,
        Err(error) => {
            tracing::warn!(
                error = %error,
                "Unable to read posting history; selecting the first configured GitHub token",
            );
            None
        }
    }
}

/// Log a failed GitHub post, record it in the history log and release the
/// entry's claim so it can be retried.
async fn record_post_failure(
    queue: &SharedQueue,
    entry: &QueuedEntry,
    client: &TokenClient,
    failure: &PostCommentError,
) -> Result<()> {
    tracing::error!(
        error = %failure,
        id = %entry.id,
        token = %client.name,
        owner = %entry.comment.owner,
        repo = %entry.comment.repo,
        pr = entry.comment.pr_number,
        "GitHub API call failed; will retry after cooldown",
    );
    if let Err(error) = queue
        .record_failure_for(entry, &client.hash, &failure.to_string())
        .await
    {
        tracing::error!(
            error = %error,
            id = %entry.id,
            "Failed to record the posting failure in the history log",
        );
    }
    if let Some(claim_token) = entry.claim_token.as_ref() {
        queue
            .release_claim(entry.id.as_str(), claim_token.as_str())
            .await?;
    }
    Ok(())
}

/// Pace retries so a persistently failing API is not hammered.
///
/// Returns `true` when shutdown was signalled during the cooldown.
pub(super) async fn pace_retry(
    queue: &SharedQueue,
    control: &mut WorkerControl,
    cooldown_seconds: u64,
) -> bool {
    metrics::record_cooldown_wait(cooldown_seconds);
    let deadline = retry_deadline(Duration::from_secs(cooldown_seconds));
    wait_for_retry_deadline(deadline, queue.change_notifier(), &mut control.shutdown).await
}
