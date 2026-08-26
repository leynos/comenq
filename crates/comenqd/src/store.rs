//! Persistent, reorderable comment queue.
//!
//! Pending comments use one JSON file beneath `<queue_path>/entries`, with an
//! ordering key for `bump`, `bust`, and `del`. `<queue_path>/last_post`
//! persists the last successful post for restart-stable ETAs.
//!
//! The store holds no in-memory state; callers serialize mutations. It writes
//! temporary siblings and renames them so entries are never half-written.

use comenq_lib::CommentRequest;
use comenq_lib::protocol::PendingEntry;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write as _};
use std::num::ParseIntError;
use std::path::{Path, PathBuf};
/// Sub-directory of the queue path holding one JSON file per entry.
const ENTRIES_DIR: &str = "entries";
/// File recording the Unix time of the most recent successful post.
const LAST_POST_FILE: &str = "last_post";
/// Recovery record for a GitHub post that succeeded before its queue cleanup.
const COMPLETION_FILE: &str = "completion";
/// A queued comment with its scheduling metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredEntry {
    /// Deterministic eight-character identifier.
    pub id: String,
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
    /// The comment to post.
    pub request: CommentRequest,
}

impl StoredEntry {
    /// Convert to the wire representation with the given ETA.
    #[must_use]
    pub fn to_pending(&self, eta_seconds: u64) -> PendingEntry {
        PendingEntry {
            id: self.id.clone(),
            eta_seconds,
            owner: self.request.owner.clone(),
            repo: self.request.repo.clone(),
            pr_number: self.request.pr_number,
            body: self.request.body.clone(),
        }
    }
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
    /// The blocking queue operation did not complete.
    #[error("queue operation task failed: {0}")]
    BlockingTask(#[from] tokio::task::JoinError),
}
/// Result alias for store operations.
pub type Result<T> = std::result::Result<T, StoreError>;
/// Scheduling inputs for [`QueueStore::put`].
#[derive(Debug, Clone, Copy)]
pub struct PutOptions {
    /// Cooldown between posts, in seconds.
    pub cooldown: u64,
    /// Flutter ceiling to sample from, in seconds.
    pub flutter_max: u64,
    /// Post as soon as the queue allows instead of waiting a full cooldown
    /// from enqueue.
    pub immediate: bool,
}

/// Filesystem-backed queue of pending comments.
#[derive(Debug, Clone)]
pub struct QueueStore {
    entries_dir: PathBuf,
    last_post_path: PathBuf,
    completion_path: PathBuf,
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
        let store = Self {
            entries_dir,
            last_post_path: queue_path.join(LAST_POST_FILE),
            completion_path: queue_path.join(COMPLETION_FILE),
        };
        store.reconcile_completion()?;
        Ok(store)
    }

    /// Enqueue `request` at the tail, sampling its flutter now.
    ///
    /// The flutter is fixed at enqueue time so the entry's estimated posting
    /// time is stable from the moment it is reported to the client. By
    /// default the entry may not post until one full cooldown plus its
    /// flutter after enqueue; `options.immediate` lifts that floor so the
    /// entry posts as soon as the queue allows.
    ///
    /// Identifiers derive from the request content and enqueue second, so an
    /// identical request repeated within the same second maps to the same
    /// identifier; the operation is idempotent and returns the existing
    /// entry unchanged.
    pub fn put(
        &self,
        request: CommentRequest,
        options: &PutOptions,
        now: u64,
    ) -> Result<StoredEntry> {
        let (id, existing) = self.resolve_entry_id(&request, now)?;
        if let Some(existing) = existing {
            return Ok(existing);
        }
        let flutter_seconds = if options.flutter_max == 0 {
            0
        } else {
            rand::rng().random_range(0..=options.flutter_max)
        };
        let not_before = if options.immediate {
            0
        } else {
            now.saturating_add(options.cooldown)
                .saturating_add(flutter_seconds)
        };
        let order = self
            .entries()?
            .last()
            .map_or(0, |e| e.order.saturating_add(1));
        let entry = StoredEntry {
            id,
            order,
            flutter_seconds,
            enqueued_at: now,
            not_before,
            request,
        };
        self.write_entry(&entry)?;
        Ok(entry)
    }

    /// All pending entries in posting order.
    pub fn entries(&self) -> Result<Vec<StoredEntry>> {
        self.reconcile_completion()?;
        let mut entries = Vec::new();
        for dirent in fs::read_dir(&self.entries_dir)? {
            let path = dirent?.path();
            if path.extension().is_some_and(|e| e == "json") {
                let text = match fs::read_to_string(&path) {
                    Ok(text) => text,
                    Err(e) => {
                        tracing::error!(
                            path = %path.display(),
                            error = %e,
                            "Skipping unreadable queue entry"
                        );
                        continue;
                    }
                };
                match serde_json::from_str::<StoredEntry>(&text) {
                    Ok(entry) if is_valid_id(&entry.id) => entries.push(entry),
                    Ok(entry) => {
                        tracing::error!(
                            path = %path.display(),
                            id = %entry.id,
                            "Skipping queue entry with an unsafe identifier"
                        );
                    }
                    Err(e) => {
                        tracing::error!(
                            path = %path.display(),
                            error = %e,
                            "Skipping unreadable queue entry"
                        );
                    }
                }
            }
        }
        entries
            .sort_by(|a, b| (a.order, a.enqueued_at, &a.id).cmp(&(b.order, b.enqueued_at, &b.id)));
        Ok(entries)
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
        self.find(id)?;
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

    /// Pending entries paired with their estimated seconds-until-post.
    ///
    /// The head is due one full cooldown plus its own flutter after the most
    /// recent post (immediately when nothing has been posted yet); each
    /// subsequent entry follows a further cooldown plus its own flutter after
    /// the projected posting time of its predecessor. An entry additionally
    /// never posts before its own `not_before` floor.
    pub fn schedule(&self, cooldown: u64, now: u64) -> Result<Vec<(StoredEntry, u64)>> {
        let mut previous_post = self.last_post()?;
        let mut scheduled = Vec::new();
        for entry in self.entries()? {
            let due = previous_post.map_or(now, |prev| {
                prev.saturating_add(cooldown)
                    .saturating_add(entry.flutter_seconds)
            });
            let post_at = due.max(entry.not_before).max(now);
            previous_post = Some(post_at);
            scheduled.push((entry, post_at.saturating_sub(now)));
        }
        Ok(scheduled)
    }

    /// The head entry and its estimated seconds-until-post, when any.
    pub fn next_due(&self, cooldown: u64, now: u64) -> Result<Option<(StoredEntry, u64)>> {
        Ok(self.schedule(cooldown, now)?.into_iter().next())
    }

    fn entry_path(&self, id: &str) -> Result<PathBuf> {
        if is_valid_id(id) {
            Ok(self.entries_dir.join(format!("{id}.json")))
        } else {
            Err(StoreError::InvalidId(id.to_owned()))
        }
    }

    fn find(&self, id: &str) -> Result<StoredEntry> {
        let path = self.entry_path(id)?;
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(StoreError::UnknownId(id.to_owned()));
            }
            Err(e) => return Err(e.into()),
        };
        let entry: StoredEntry = serde_json::from_str(&text)?;
        if entry.id != id {
            return Err(StoreError::InvalidId(entry.id));
        }
        Ok(entry)
    }

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

    fn write_entry(&self, entry: &StoredEntry) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(entry)?;
        self.write_atomic(&self.entry_path(&entry.id)?, &bytes)
    }

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

fn is_valid_id(id: &str) -> bool {
    id.len() == 8 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

mod completion;

#[cfg(test)]
mod tests;
