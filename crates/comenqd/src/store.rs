//! Persistent, reorderable comment queue.
//!
//! Each pending comment lives in its own JSON file under
//! `<queue_path>/entries`, carrying an explicit ordering key so entries can
//! be moved to the head (`bump`) or tail (`bust`) of the queue, or removed
//! (`del`). The Unix timestamp of the most recent successful post is kept in
//! `<queue_path>/last_post` so estimated posting times survive restarts.
//!
//! The store itself holds no in-memory state; callers serialize mutations
//! (the daemon wraps it in a `tokio::sync::Mutex`). Files are written to a
//! temporary sibling and renamed into place so entries are never observed
//! half-written.

use comenq_lib::CommentRequest;
use comenq_lib::protocol::PendingEntry;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Sub-directory of the queue path holding one JSON file per entry.
const ENTRIES_DIR: &str = "entries";
/// File recording the Unix time of the most recent successful post.
const LAST_POST_FILE: &str = "last_post";

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
}

/// Result alias for store operations.
pub type Result<T> = std::result::Result<T, StoreError>;

/// Filesystem-backed queue of pending comments.
#[derive(Debug, Clone)]
pub struct QueueStore {
    entries_dir: PathBuf,
    last_post_path: PathBuf,
}

/// Compute the deterministic eight-character identifier for an entry.
///
/// Uses the 64-bit FNV-1a hash of the request fields and enqueue time,
/// rendered as the first eight lowercase hex digits. The hash is content
/// derived, so an entry keeps the same identifier for its whole life and
/// across daemon restarts.
fn entry_id(request: &CommentRequest, enqueued_at: u64) -> String {
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
    let hex = format!("{hash:016x}");
    hex.chars().take(8).collect()
}

impl QueueStore {
    /// Open (creating if necessary) the store rooted at `queue_path`.
    pub fn open(queue_path: &Path) -> Result<Self> {
        let entries_dir = queue_path.join(ENTRIES_DIR);
        fs::create_dir_all(&entries_dir)?;
        Ok(Self {
            entries_dir,
            last_post_path: queue_path.join(LAST_POST_FILE),
        })
    }

    /// Enqueue `request` at the tail, sampling its flutter now.
    ///
    /// The flutter is fixed at enqueue time so the entry's estimated posting
    /// time is stable from the moment it is reported to the client.
    pub fn put(&self, request: CommentRequest, flutter_max: u64, now: u64) -> Result<StoredEntry> {
        let flutter_seconds = if flutter_max == 0 {
            0
        } else {
            rand::rng().random_range(0..=flutter_max)
        };
        let order = self
            .entries()?
            .last()
            .map_or(0, |e| e.order.saturating_add(1));
        let entry = StoredEntry {
            id: entry_id(&request, now),
            order,
            flutter_seconds,
            enqueued_at: now,
            request,
        };
        self.write_entry(&entry)?;
        Ok(entry)
    }

    /// All pending entries in posting order.
    pub fn entries(&self) -> Result<Vec<StoredEntry>> {
        let mut entries = Vec::new();
        for dirent in fs::read_dir(&self.entries_dir)? {
            let path = dirent?.path();
            if path.extension().is_some_and(|e| e == "json") {
                let text = fs::read_to_string(&path)?;
                match serde_json::from_str::<StoredEntry>(&text) {
                    Ok(entry) => entries.push(entry),
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
        let entry = self.find(id)?;
        fs::remove_file(self.entry_path(&entry.id))?;
        Ok(())
    }

    /// Unix time of the most recent successful post, when any.
    pub fn last_post(&self) -> Option<u64> {
        let text = fs::read_to_string(&self.last_post_path).ok()?;
        text.trim().parse().ok()
    }

    /// Remove the posted head entry and record the posting time.
    pub fn complete(&self, id: &str, now: u64) -> Result<()> {
        self.del(id)?;
        self.write_atomic(&self.last_post_path, now.to_string().as_bytes())?;
        Ok(())
    }

    /// Pending entries paired with their estimated seconds-until-post.
    ///
    /// The head is due one full cooldown plus its own flutter after the most
    /// recent post (immediately when nothing has been posted yet); each
    /// subsequent entry follows a further cooldown plus its own flutter after
    /// the projected posting time of its predecessor.
    pub fn schedule(&self, cooldown: u64, now: u64) -> Result<Vec<(StoredEntry, u64)>> {
        let mut previous_post = self.last_post();
        let mut scheduled = Vec::new();
        for entry in self.entries()? {
            let due = previous_post.map_or(now, |prev| {
                prev.saturating_add(cooldown)
                    .saturating_add(entry.flutter_seconds)
            });
            let post_at = due.max(now);
            previous_post = Some(post_at);
            scheduled.push((entry, post_at.saturating_sub(now)));
        }
        Ok(scheduled)
    }

    /// The head entry and its estimated seconds-until-post, when any.
    pub fn next_due(&self, cooldown: u64, now: u64) -> Result<Option<(StoredEntry, u64)>> {
        Ok(self.schedule(cooldown, now)?.into_iter().next())
    }

    fn entry_path(&self, id: &str) -> PathBuf {
        self.entries_dir.join(format!("{id}.json"))
    }

    fn find(&self, id: &str) -> Result<StoredEntry> {
        self.entries()?
            .into_iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| StoreError::UnknownId(id.to_owned()))
    }

    fn reorder(
        &self,
        id: &str,
        new_order: impl Fn(&StoredEntry, &[StoredEntry]) -> i64,
    ) -> Result<()> {
        let all = self.entries()?;
        let mut entry = all
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
            .ok_or_else(|| StoreError::UnknownId(id.to_owned()))?;
        entry.order = new_order(&entry, &all);
        self.write_entry(&entry)
    }

    fn write_entry(&self, entry: &StoredEntry) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(entry)?;
        self.write_atomic(&self.entry_path(&entry.id), &bytes)
    }

    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, bytes)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
