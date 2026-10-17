//! Serialized pending-first lookup of an entry's posting outcome.

use super::{QueueStore, Result, StoreError, StoredEntry, history};
use std::fs;
use std::io;

/// The history boundary and lifetime selected by the first pending lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwaitCursor {
    /// Byte boundary before attempts belonging to the selected wait.
    pub offset: u64,
    /// Unique lifetime of the selected queue entry.
    pub incarnation: String,
}

/// Current durable outcome of a read-only foreground wait.
#[derive(Debug, PartialEq, Eq)]
pub enum AwaitState {
    /// The selected entry is still queued.
    Pending {
        /// Projected seconds until posting.
        eta_seconds: u64,
        /// Boundary and lifetime to retain for later queries.
        cursor: AwaitCursor,
    },
    /// A matching successful completion was recorded.
    Posted {
        /// Full GitHub comment URL, absent in legacy history.
        comment_url: Option<String>,
        /// Unix time of the successful posting attempt.
        posted_at: u64,
    },
    /// Neither the selected pending entry nor its completion exists.
    Unknown,
}

impl QueueStore {
    /// Resolve pending entries before history, binding repeat lookups to one lifetime.
    #[cfg(test)]
    pub(crate) fn resolve_await(
        &self,
        id: &str,
        cursor: Option<&AwaitCursor>,
        cooldown: u64,
        now: u64,
    ) -> Result<AwaitState> {
        self.resolve_await_with_eta(id, cursor, |store, entry| {
            Ok(store
                .schedule(cooldown, now)?
                .into_iter()
                .find(|(scheduled, _)| scheduled.id == entry.id)
                .map_or(0, |(_, eta)| eta))
        })
    }

    /// Resolve one entry, invoking the projection only for a matching pending lifetime.
    #[cfg(test)]
    pub(crate) fn resolve_await_with_eta(
        &self,
        id: &str,
        cursor: Option<&AwaitCursor>,
        eta: impl FnOnce(&Self, &StoredEntry) -> Result<u64>,
    ) -> Result<AwaitState> {
        if let Some(pending) = self.resolve_await_pending_with_eta(id, cursor, eta)? {
            return Ok(pending);
        }
        self.resolve_await_completion(id, cursor)
    }

    /// Resolve a matching queued lifetime without consulting the history log.
    pub(crate) fn resolve_await_pending_with_eta(
        &self,
        id: &str,
        cursor: Option<&AwaitCursor>,
        eta: impl FnOnce(&Self, &StoredEntry) -> Result<u64>,
    ) -> Result<Option<AwaitState>> {
        match self.find(id) {
            Ok(entry) if cursor.is_none_or(|cursor| cursor.incarnation == entry.incarnation) => {
                let eta_seconds = eta(self, &entry)?;
                return self
                    .pending_await_state(entry, cursor, eta_seconds)
                    .map(Some);
            }
            Ok(_) | Err(StoreError::UnknownId(_)) => {}
            Err(error) => return Err(error),
        }
        Ok(None)
    }

    /// Resolve durable completion without requiring the queue-state mutex.
    pub(crate) fn resolve_await_completion(
        &self,
        id: &str,
        cursor: Option<&AwaitCursor>,
    ) -> Result<AwaitState> {
        // A successful post clears this marker in the same journal recovery
        // transaction. Without a matching pending entry, it means a reused
        // identifier was deleted and older URLs must stay hidden.
        if cursor.is_none() && self.await_lifetime_was_deleted(id)? {
            return Ok(AwaitState::Unknown);
        }
        let record = history::latest_completion(&self.history_path, id, cursor)?;
        Ok(
            record.map_or(AwaitState::Unknown, |record| AwaitState::Posted {
                comment_url: record.comment_url,
                posted_at: record.posted_at,
            }),
        )
    }

    /// Project a selected entry using the same scheduling rules as `list`.
    fn pending_await_state(
        &self,
        entry: StoredEntry,
        cursor: Option<&AwaitCursor>,
        eta_seconds: u64,
    ) -> Result<AwaitState> {
        let cursor = match cursor {
            Some(cursor) => cursor.clone(),
            None => AwaitCursor {
                offset: self.history_end()?,
                incarnation: entry.incarnation,
            },
        };
        Ok(AwaitState::Pending {
            eta_seconds,
            cursor,
        })
    }

    /// Capture the current history length as the start boundary for a new wait.
    fn history_end(&self) -> Result<u64> {
        match fs::metadata(&self.history_path) {
            Ok(metadata) => Ok(metadata.len()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(0),
            Err(error) => Err(error.into()),
        }
    }
}
