//! Persistent, reorderable comment queue.
//!
//! Pending comments use one JSON file beneath `<queue_path>/entries`, with an
//! ordering key for `bump`, `bust`, and `del`. `<queue_path>/last_post` and
//! `<queue_path>/last_attempt` preserve successful-post and failed-post timing
//! across daemon restarts. `<queue_path>/history.jsonl` records each posting
//! attempt for the `hist` operation; `await-deletions` records reused IDs whose
//! latest pending lifetime was deleted before it posted.
//!
//! The store holds no in-memory state; callers serialize mutations. It writes
//! temporary siblings and renames them so entries are never half-written.

use comenq_lib::CommentRequest;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write as _};
use std::num::ParseIntError;
use std::path::{Path, PathBuf};
/// Sub-directory of the queue path holding one JSON file per entry.
const ENTRIES_DIR: &str = "entries";
/// Maximum entries retained by the durable queue, independent of wire limits.
pub(crate) const MAX_QUEUE_ENTRIES: usize = 1024;
/// File recording the Unix time of the most recent successful post.
const LAST_POST_FILE: &str = "last_post";
/// File recording the Unix time of the most recent failed post attempt.
const LAST_ATTEMPT_FILE: &str = "last_attempt";
/// Append-only JSON Lines log of posting attempts.
const HISTORY_FILE: &str = "history.jsonl";
/// Recovery record for a GitHub post that succeeded before its queue cleanup.
const COMPLETION_FILE: &str = "completion";
/// A queued comment with its scheduling metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredEntry {
    /// Deterministic eight-character identifier.
    pub id: String,
    /// Unique lifetime of this entry, so a reused identifier cannot redirect a wait.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub incarnation: String,
    /// Explicit queue position; lower posts first. May go negative after
    /// repeated bumps.
    pub order: i64,
    /// Flutter sampled when the entry was enqueued, in seconds.
    pub flutter_seconds: u64,
    /// Unix time the entry was enqueued, in seconds.
    pub enqueued_at: u64,
    /// Earliest Unix time the entry may post.
    ///
    /// A default `put` waits one cooldown plus flutter after enqueue, even
    /// while idle. Immediate puts and entries persisted before this field
    /// existed use zero.
    #[serde(default)]
    pub not_before: u64,
    /// Claim token while a worker is posting this entry.
    #[serde(default)]
    pub claim_token: Option<String>,
    /// The comment to post.
    pub request: CommentRequest,
}

/// Errors raised by queue store operations.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// Underlying filesystem failure.
    #[error(transparent)]
    Io(#[from] io::Error),
    /// Entry serialization failed.
    #[error("entry serialization failed: {0}")]
    Serde(#[from] serde_json::Error),
    /// No entry carries the requested identifier.
    #[error("no queued comment has id '{0}'")]
    UnknownId(String),
    /// An identifier is not safe to use as a queue-entry filename.
    #[error("queue entry id '{0}' must be eight hexadecimal characters")]
    InvalidId(String),
    /// A GitHub repository component is unsafe to persist or render.
    #[error("GitHub repository {0} contains unsafe characters")]
    InvalidRepositoryComponent(&'static str),
    /// The persisted last-post marker cannot be parsed as a Unix timestamp.
    #[error("last-post marker is invalid: {0}")]
    LastPost(#[from] ParseIntError),
    /// The persisted last-attempt marker cannot be parsed as a Unix timestamp.
    #[error("last-attempt marker is invalid: {0}")]
    LastAttempt(ParseIntError),
    /// The blocking queue operation did not complete.
    #[error("queue operation task failed: {0}")]
    BlockingTask(#[from] tokio::task::JoinError),
    /// The queue has reached its configured pending-entry limit.
    #[error("queue would exceed the maximum of {0} pending entries")]
    QueueFull(usize),
    /// An entry exceeds the maximum serialized file size.
    #[error("queue entry is {size} bytes, exceeding the per-entry limit of {limit} bytes")]
    EntryTooLarge {
        /// Serialized entry size in bytes.
        size: u64,
        /// Maximum allowed serialized size in bytes.
        limit: u64,
    },
    /// Persisted queue bytes plus the requested write exceed the durable budget.
    #[error(
        "queue byte budget exceeded: {used} bytes used, {requested} more requested, {limit} allowed"
    )]
    QueueByteBudgetExceeded {
        /// Persisted queue size before the requested write, in bytes.
        used: u64,
        /// Size of the requested write, in bytes.
        requested: u64,
        /// Configured maximum queue size, in bytes.
        limit: u64,
    },
}

impl StoreError {
    /// Return a bounded classification that does not expose stored values.
    pub(crate) fn category(&self) -> &'static str {
        match self {
            Self::Io(error) => match error.kind() {
                io::ErrorKind::NotFound => "not_found",
                io::ErrorKind::PermissionDenied => "permission_denied",
                io::ErrorKind::InvalidData => "invalid_data",
                _ => "io_error",
            },
            Self::Serde(_) => "invalid_json",
            Self::UnknownId(_) => "unknown_identifier",
            Self::InvalidId(_) => "unsafe_identifier",
            Self::InvalidRepositoryComponent(_) => "invalid_repository_component",
            Self::LastPost(_) => "invalid_last_post",
            Self::LastAttempt(_) => "invalid_last_attempt",
            Self::BlockingTask(_) => "blocking_task",
            Self::QueueFull(_) => "queue_full",
            Self::EntryTooLarge { .. } => "entry_too_large",
            Self::QueueByteBudgetExceeded { .. } => "byte_budget_exceeded",
        }
    }

    /// Whether this error represents an unexpected persistence or task failure.
    pub(crate) fn is_unexpected(&self) -> bool {
        matches!(
            self,
            Self::Io(_)
                | Self::Serde(_)
                | Self::LastPost(_)
                | Self::LastAttempt(_)
                | Self::BlockingTask(_)
        )
    }
}
/// Result alias for store operations.
pub type Result<T> = std::result::Result<T, StoreError>;
/// Scheduling inputs for [`QueueStore::put`].
#[derive(Debug, Clone, Copy)]
pub struct PutOptions {
    /// Cooldown between posts, in seconds.
    pub cooldown: u64,
    /// Flutter sampled by the daemon when it accepts the put request.
    pub flutter_seconds: u64,
    /// Post as soon as the queue allows instead of waiting a full cooldown
    /// from enqueue.
    pub immediate: bool,
}

/// Filesystem-backed queue of pending comments.
#[derive(Debug, Clone)]
pub struct QueueStore {
    entries_dir: PathBuf,
    await_deletions_dir: PathBuf,
    last_post_path: PathBuf,
    last_attempt_path: PathBuf,
    completion_path: PathBuf,
    history_path: PathBuf,
}

/// Compute the deterministic eight-character identifier for an entry.
///
/// Uses the 64-bit FNV-1a hash of the request fields and enqueue time,
/// rendered as the first eight lowercase hex digits. The hash is content
/// derived, so an entry keeps the same identifier for its whole life and
/// across daemon restarts.
fn entry_id(request: &CommentRequest, enqueued_at: u64) -> String {
    entry_id_with_salt(request, enqueued_at, None)
}

/// Derive an identifier, adding a salt only when resolving a collision.
fn entry_id_with_salt(request: &CommentRequest, enqueued_at: u64, salt: Option<u64>) -> String {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = FNV_OFFSET;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            hash ^= u64::from(*b);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
    };
    eat(request.owner.as_bytes());
    eat(&[0]);
    eat(request.repo.as_bytes());
    eat(&[0]);
    eat(&request.pr_number.to_le_bytes());
    eat(request.body.as_bytes());
    eat(&enqueued_at.to_le_bytes());
    if let Some(salt) = salt {
        eat(&salt.to_le_bytes());
    }
    let hex = format!("{hash:016x}");
    hex.chars().take(8).collect()
}

impl QueueStore {
    /// Open (creating if necessary) the store rooted at `queue_path`.
    pub fn open(queue_path: &Path) -> Result<Self> {
        let entries_dir = queue_path.join(ENTRIES_DIR);
        fs::create_dir_all(&entries_dir)?;
        let await_deletions_dir = queue_path.join(await_deletions::AWAIT_DELETIONS_DIR);
        fs::create_dir_all(&await_deletions_dir)?;
        let store = Self {
            entries_dir,
            await_deletions_dir,
            last_post_path: queue_path.join(LAST_POST_FILE),
            last_attempt_path: queue_path.join(LAST_ATTEMPT_FILE),
            completion_path: queue_path.join(COMPLETION_FILE),
            history_path: queue_path.join(HISTORY_FILE),
        };
        store.reconcile_completion()?;
        store.validate_existing_limits()?;
        store.reclaim_claims()?;
        store.reconcile_await_deletions()?;
        Ok(store)
    }

    /// Move the identified entry to the head of the queue.
    pub fn bump(&self, id: &str) -> Result<()> {
        self.reorder(id, |entry, all| {
            all.first().map_or(entry.order, |head| {
                if head.id == entry.id {
                    entry.order
                } else {
                    head.order.saturating_sub(1)
                }
            })
        })
    }

    /// Move the identified entry to the tail of the queue.
    pub fn bust(&self, id: &str) -> Result<()> {
        self.reorder(id, |entry, all| {
            all.last().map_or(entry.order, |tail| {
                if tail.id == entry.id {
                    entry.order
                } else {
                    tail.order.saturating_add(1)
                }
            })
        })
    }

    /// Remove the identified entry from the queue.
    pub fn del(&self, id: &str) -> Result<()> {
        let entry = self.find(id)?;
        self.write_await_deletion(&entry)?;
        fs::remove_file(self.entry_path(id)?)?;
        Ok(())
    }

    /// Unix time of the most recent successful post, when any.
    pub fn last_post(&self) -> Result<Option<u64>> {
        match fs::read_to_string(&self.last_post_path) {
            Ok(text) => Ok(Some(text.trim().parse()?)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Unix time of the most recent failed post attempt, when any.
    pub fn last_attempt(&self) -> Result<Option<u64>> {
        match fs::read_to_string(&self.last_attempt_path) {
            Ok(text) => Ok(Some(text.trim().parse().map_err(StoreError::LastAttempt)?)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Append a posting attempt to the history log.
    pub(crate) fn append_history(&self, record: &HistoryRecord) -> Result<()> {
        history::append(&self.history_path, record)
    }

    /// Append a recovered completion unless it is already durable in history.
    fn append_completion_history(&self, record: &HistoryRecord, search_from: u64) -> Result<()> {
        history::append_once(&self.history_path, record, search_from)
    }

    /// Read one bounded posting-history page.
    pub(crate) fn history_page(
        &self,
        position: Option<history::HistoryPosition>,
        recent_limit: Option<usize>,
        page_size: usize,
    ) -> Result<history::HistoryPage> {
        history::read_page(&self.history_path, position, recent_limit, page_size)
    }

    /// Resolve a validated identifier to its persisted entry path.
    fn entry_path(&self, id: &str) -> Result<PathBuf> {
        if is_valid_id(id) {
            Ok(self.entries_dir.join(format!("{id}.json")))
        } else {
            Err(StoreError::InvalidId(id.to_owned()))
        }
    }

    /// Read one entry by identifier, preserving typed missing-entry errors.
    fn find(&self, id: &str) -> Result<StoredEntry> {
        let path = self.entry_path(id)?;
        let entry = match bounds::read_entry(&path) {
            Ok(entry) => entry,
            Err(StoreError::Io(e)) if e.kind() == io::ErrorKind::NotFound => {
                return Err(StoreError::UnknownId(id.to_owned()));
            }
            Err(e) => return Err(e),
        };
        if entry.id != id {
            return Err(StoreError::InvalidId(entry.id));
        }
        Ok(entry)
    }

    /// Apply an order-key change and atomically replace each affected file.
    fn reorder(
        &self,
        id: &str,
        new_order: impl Fn(&StoredEntry, &[StoredEntry]) -> i64,
    ) -> Result<()> {
        let all = self.entries()?;
        let mut entry = self.find(id)?;
        entry.order = new_order(&entry, &all);
        self.write_entry(&entry)
    }

    /// Replace a file through a temporary sibling so partial writes are unseen.
    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        let tmp = path.with_extension("tmp");
        let parent = path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "entry path has no parent")
        })?;
        let mut file = fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
        let directory = fs::File::open(parent)?;
        directory.sync_all()?;
        fs::rename(&tmp, path)?;
        directory.sync_all()?;
        Ok(())
    }

    /// Find a request's stable identifier, salting only on content collisions.
    fn resolve_entry_id(
        &self,
        request: &CommentRequest,
        now: u64,
    ) -> Result<(String, Option<StoredEntry>)> {
        let mut salt = None;
        loop {
            let id = salt.map_or_else(
                || entry_id(request, now),
                |value| entry_id_with_salt(request, now, Some(value)),
            );
            match self.find(&id) {
                Ok(existing) if existing.request == *request => return Ok((id, Some(existing))),
                Ok(_) => salt = Some(salt.map_or(1, |value| value.saturating_add(1))),
                Err(StoreError::UnknownId(_)) => return Ok((id, None)),
                Err(e) => return Err(e),
            }
        }
    }
}

/// Check whether an identifier is safe as a queue-entry filename.
fn is_valid_id(id: &str) -> bool {
    id.len() == 8 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

mod await_lookup;
pub(crate) use await_lookup::{AwaitCursor, AwaitState};
mod await_deletions;
mod bounds;
mod completion;
mod history;
mod scheduling;

#[cfg(test)]
pub(crate) use history::HistoryRecordAt;
pub(crate) use history::{HistoryPage, HistoryPosition, HistoryRecord};

#[cfg(test)]
mod tests;
