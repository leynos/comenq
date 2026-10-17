//! Request and response types exchanged between the `comenq` client and the
//! `comenqd` daemon over the Unix domain socket.
//!
//! Every connection carries exactly one JSON-encoded [`Request`]; the daemon
//! replies with one JSON-encoded [`Response`] and closes the connection.
//! [`Request::Await`] instead streams newline-delimited [`AwaitEvent`] frames,
//! ending with exactly one posted or error frame before closure. Frames include
//! their newline in the 64 KiB [`MAX_AWAIT_FRAME_BYTES`] limit. Idle waits send a
//! heartbeat at least every [`AWAIT_HEARTBEAT_SECS`] seconds; clients allow
//! [`AWAIT_IDLE_TIMEOUT_SECS`] seconds between complete frames. A disconnected
//! wait never cancels the entry or changes its posting schedule.

use serde::{Deserialize, Serialize};

use crate::CommentRequest;

/// Maximum JSON response size accepted by the client and emitted by the daemon.
pub const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
/// Maximum JSON request size emitted by the client and accepted by the daemon.
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;
/// Maximum number of pending entries retained and returned by `list`.
pub const MAX_PENDING_ENTRIES: usize = 1024;

/// Maximum size of one newline-delimited await frame, including its delimiter.
pub const MAX_AWAIT_FRAME_BYTES: usize = 64 * 1024;
/// Maximum interval between await heartbeats when the store is idle.
pub const AWAIT_HEARTBEAT_SECS: u64 = 15;
/// Client deadline between complete await frames.
pub const AWAIT_IDLE_TIMEOUT_SECS: u64 = 45;

/// One JSON line in an await stream; posted and error frames are terminal.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AwaitEvent {
    /// The entry is still queued, including after a failed posting attempt.
    Waiting {
        /// Estimated seconds until the next posting attempt.
        eta_seconds: u64,
    },
    /// GitHub accepted the comment and its URL is durably recorded.
    Posted {
        /// Full GitHub comment URL, including its comment fragment.
        comment_url: String,
    },
    /// The entry is unrecoverable or the daemon is shutting down.
    Error {
        /// Human-readable failure description.
        message: String,
    },
}

/// Operation requested by the client.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// Enqueue a new comment.
    Put {
        /// The comment to enqueue.
        request: CommentRequest,
        /// Post as soon as the queue allows instead of waiting a full
        /// cooldown from enqueue.
        #[serde(default)]
        immediate: bool,
    },
    /// List pending comments in posting order.
    List,
    /// Move the identified entry to the head of the queue.
    Bump {
        /// Identifier printed by `list` and `put`.
        id: String,
    },
    /// Move the identified entry to the tail of the queue.
    Bust {
        /// Identifier printed by `list` and `put`.
        id: String,
    },
    /// Remove the identified entry from the queue.
    Del {
        /// Identifier printed by `list` and `put`.
        id: String,
    },
    /// Wait for this entry's successful post without mutating the queue.
    Await {
        /// Eight hexadecimal characters printed by `put` and `list`.
        id: String,
    },
    /// Report past posting attempts in chronological order.
    Hist {
        /// Most recent records to return; `None` returns them all.
        #[serde(default)]
        limit: Option<usize>,
        /// Position from which to continue a paged history response.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cursor: Option<HistoryCursor>,
    },
}

/// Snapshot position for continuing a paged history response.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryCursor {
    /// Byte offset of the next JSON line in the history log.
    pub offset: u64,
    /// End of the log snapshot selected by the first request.
    pub end_offset: u64,
}

/// A pending queue entry as reported by the daemon.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingEntry {
    /// Deterministic eight-character identifier.
    pub id: String,
    /// Approximate seconds until the comment is posted.
    pub eta_seconds: u64,
    /// Repository owner.
    pub owner: String,
    /// Repository name.
    pub repo: String,
    /// Pull request number.
    pub pr_number: u64,
    /// Full comment body; consumers truncate for display.
    pub body: String,
}

/// A past posting attempt as reported by the daemon.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryEntry {
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
    #[serde(default)]
    pub token_hash: String,
    /// Repository owner.
    pub owner: String,
    /// Repository name.
    pub repo: String,
    /// Pull request number.
    pub pr_number: u64,
    /// Full comment body; consumers truncate for display.
    pub body: String,
}

/// Daemon reply to a [`Request`].
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Response {
    /// The request succeeded.
    Ok {
        /// Entry affected by `put`, when applicable.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        entry: Option<PendingEntry>,
        /// Pending entries, returned by `list`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        entries: Option<Vec<PendingEntry>>,
        /// Past posting attempts, returned by `hist`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        history: Option<Vec<HistoryEntry>>,
        /// Cursor for a following `hist` page, when more records remain.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next_cursor: Option<HistoryCursor>,
    },
    /// The request failed; `message` explains why.
    Error {
        /// Human-readable failure description.
        message: String,
    },
}

impl Response {
    /// Successful reply carrying no payload.
    #[must_use]
    pub fn ok() -> Self {
        Self::Ok {
            entry: None,
            entries: None,
            history: None,
            next_cursor: None,
        }
    }

    /// Successful reply for `put`, echoing the enqueued entry.
    #[must_use]
    pub fn entry(entry: PendingEntry) -> Self {
        Self::Ok {
            entry: Some(entry),
            entries: None,
            history: None,
            next_cursor: None,
        }
    }

    /// Successful reply for `list`.
    #[must_use]
    pub fn entries(entries: Vec<PendingEntry>) -> Self {
        Self::Ok {
            entry: None,
            entries: Some(entries),
            history: None,
            next_cursor: None,
        }
    }

    /// Successful reply for `hist`.
    #[must_use]
    pub fn history(history: Vec<HistoryEntry>, next_cursor: Option<HistoryCursor>) -> Self {
        Self::Ok {
            entry: None,
            entries: None,
            history: Some(history),
            next_cursor,
        }
    }

    /// Failed reply with a description.
    #[must_use]
    pub fn error(message: impl Into<String>) -> Self {
        Self::Error {
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests;
