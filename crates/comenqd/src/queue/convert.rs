//! Conversions between protocol requests, storage entries, and domain values.
//!
//! The domain knows neither the wire format nor the stored JSON layout, and the
//! protocol types know nothing of either. These free functions keep every
//! translation in the adapter layer so no representation leaks across a port.

use comenq_domain::{ClaimToken, Comment, EntryId, EntryLifetime, QueuedEntry, ScheduledEntry};
use comenq_lib::CommentRequest;

use crate::store::{StoreError, StoredEntry};

/// Translate a wire request into a domain comment without validating it.
pub(crate) fn comment_from_request(request: &CommentRequest) -> Comment {
    Comment::new(
        request.owner.clone(),
        request.repo.clone(),
        request.pr_number,
        request.body.clone(),
    )
}

/// Translate a domain comment back into the wire request shape.
pub(crate) fn request_from_comment(comment: &Comment) -> CommentRequest {
    CommentRequest {
        owner: comment.owner.clone(),
        repo: comment.repo.clone(),
        pr_number: comment.pr_number,
        body: comment.body.clone(),
    }
}

/// Translate a stored entry into the domain's queued-entry view.
///
/// Entries read from disk are already validated, but the identifier is parsed
/// again so an invalid value cannot enter the domain through this path.
///
/// # Errors
///
/// Returns [`StoreError::InvalidId`] when the stored identifier is not eight
/// hexadecimal characters.
pub(crate) fn queued_from_stored(entry: StoredEntry) -> Result<QueuedEntry, StoreError> {
    let id = EntryId::parse(&entry.id)
        .map_err(|error| StoreError::InvalidId(error.value().to_owned()))?;
    Ok(QueuedEntry {
        id,
        lifetime: EntryLifetime::new(entry.incarnation),
        order: entry.order,
        flutter_seconds: entry.flutter_seconds,
        enqueued_at: entry.enqueued_at,
        not_before: entry.not_before,
        claim_token: entry.claim_token.map(ClaimToken::new),
        comment: comment_from_request(&entry.request),
    })
}

/// Translate the domain's queued-entry view back into the stored layout.
///
/// The conversion preserves the repository-minted lifetime as the stored
/// incarnation, and writes the comment as the legacy request shape so the
/// on-disk JSON is unchanged.
pub(crate) fn stored_from_queued(entry: &QueuedEntry) -> StoredEntry {
    StoredEntry {
        id: entry.id.as_str().to_owned(),
        incarnation: entry.lifetime.as_str().to_owned(),
        order: entry.order,
        flutter_seconds: entry.flutter_seconds,
        enqueued_at: entry.enqueued_at,
        not_before: entry.not_before,
        claim_token: entry
            .claim_token
            .as_ref()
            .map(|token| token.as_str().to_owned()),
        request: request_from_comment(&entry.comment),
    }
}

/// Convert a scheduled domain entry back to the stored tuple shape.
pub(crate) fn stored_schedule(scheduled: ScheduledEntry) -> (StoredEntry, u64) {
    (stored_from_queued(&scheduled.entry), scheduled.eta_seconds)
}
