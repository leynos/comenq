//! Durable posting attempt data, including completion URLs and entry lifetimes.

use super::super::StoredEntry;
use comenq_lib::CommentRequest;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Bound error text so one failed attempt always fits within a history page.
const MAX_HISTORY_ERROR_BYTES: usize = 64 * 1024;

/// One posting attempt, as recorded in the history log.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct HistoryRecord {
    /// Stable identifier used to make completion recovery idempotent.
    #[serde(default)]
    pub attempt_id: String,
    /// Identifier the entry carried while queued.
    pub id: String,
    /// Unix time the posting attempt finished, in seconds.
    pub posted_at: u64,
    /// Whether GitHub accepted the comment.
    pub success: bool,
    /// Failure description when `success` is false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Hex SHA-256 of the token that made the attempt.
    ///
    /// Missing values identify records written before token rotation existed.
    #[serde(default)]
    pub token_hash: String,
    /// GitHub comment URL; older records and failures have no URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment_url: Option<String>,
    /// Entry incarnation, retained across claims and reorderings.
    #[serde(default)]
    pub incarnation: String,
    /// The comment that was posted (or attempted).
    pub request: CommentRequest,
}

impl HistoryRecord {
    /// Record a successful post of `entry` using `token_hash`.
    #[must_use]
    pub(crate) fn success(
        entry: &StoredEntry,
        token_hash: &str,
        posted_at: u64,
        comment_url: Option<String>,
    ) -> Self {
        Self {
            attempt_id: Uuid::new_v4().to_string(),
            id: entry.id.clone(),
            posted_at,
            success: true,
            error: None,
            comment_url,
            incarnation: entry.incarnation.clone(),
            token_hash: token_hash.to_owned(),
            request: entry.request.clone(),
        }
    }

    /// Record a failed posting attempt of `entry` using `token_hash`.
    #[must_use]
    pub(crate) fn failure(
        entry: &StoredEntry,
        token_hash: &str,
        posted_at: u64,
        error: impl Into<String>,
    ) -> Self {
        Self {
            attempt_id: Uuid::new_v4().to_string(),
            id: entry.id.clone(),
            posted_at,
            success: false,
            error: Some(bounded_error(error.into())),
            comment_url: None,
            incarnation: entry.incarnation.clone(),
            token_hash: token_hash.to_owned(),
            request: entry.request.clone(),
        }
    }
}

fn bounded_error(mut error: String) -> String {
    if error.len() <= MAX_HISTORY_ERROR_BYTES {
        return error;
    }
    let mut end = MAX_HISTORY_ERROR_BYTES - 3;
    while !error.is_char_boundary(end) {
        end -= 1;
    }
    error.truncate(end);
    error.push_str("...");
    error
}
