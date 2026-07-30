//! Posting-history records and their append-only JSON Lines log.
//!
//! Every posting attempt, successful or not, becomes one [`HistoryRecord`]
//! appended as a single JSON line to `<queue_path>/history.jsonl`.

use comenq_lib::CommentRequest;
use comenq_lib::protocol::HistoryEntry;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write as _};
use std::path::Path;

use super::{Result, StoredEntry};

/// One posting attempt, as recorded in the history log.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryRecord {
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
    /// The worker reads the latest record's hash to pick the next token in
    /// the round-robin rotation.
    pub token_hash: String,
    /// The comment that was posted (or attempted).
    pub request: CommentRequest,
}

impl HistoryRecord {
    /// Record a successful post of `entry` by the token hashing to
    /// `token_hash` at `posted_at`.
    #[must_use]
    pub fn success(entry: &StoredEntry, token_hash: &str, posted_at: u64) -> Self {
        Self {
            id: entry.id.clone(),
            posted_at,
            success: true,
            error: None,
            token_hash: token_hash.to_owned(),
            request: entry.request.clone(),
        }
    }

    /// Record a failed posting attempt of `entry` by the token hashing to
    /// `token_hash` at `posted_at`.
    #[must_use]
    pub fn failure(
        entry: &StoredEntry,
        token_hash: &str,
        posted_at: u64,
        error: impl Into<String>,
    ) -> Self {
        Self {
            id: entry.id.clone(),
            posted_at,
            success: false,
            error: Some(error.into()),
            token_hash: token_hash.to_owned(),
            request: entry.request.clone(),
        }
    }

    /// Convert to the wire representation.
    #[must_use]
    pub fn to_entry(&self) -> HistoryEntry {
        HistoryEntry {
            id: self.id.clone(),
            posted_at: self.posted_at,
            success: self.success,
            error: self.error.clone(),
            token_hash: self.token_hash.clone(),
            owner: self.request.owner.clone(),
            repo: self.request.repo.clone(),
            pr_number: self.request.pr_number,
            body: self.request.body.clone(),
        }
    }
}

/// Append `record` as one line to the log at `path`.
///
/// The log is JSON Lines: one record per line, appended atomically enough
/// for a single writer (the daemon serializes store access).
pub(super) fn append(path: &Path, record: &HistoryRecord) -> Result<()> {
    let mut line = serde_json::to_vec(record)?;
    line.push(b'\n');
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(&line)?;
    Ok(())
}

/// All recorded posting attempts in chronological order.
///
/// A missing log means no attempts have been recorded. Malformed lines are
/// skipped with an error log so one corrupt record cannot hide the rest of
/// the history.
pub(super) fn read(path: &Path) -> Result<Vec<HistoryRecord>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut records = Vec::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        match serde_json::from_str::<HistoryRecord>(line) {
            Ok(record) => records.push(record),
            Err(e) => {
                tracing::error!(
                    path = %path.display(),
                    error = %e,
                    "Skipping unreadable history record"
                );
            }
        }
    }
    Ok(records)
}
