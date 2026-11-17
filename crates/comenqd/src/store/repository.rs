//! Domain repository adapter over the filesystem queue store.
//!
//! The adapter maps stored entries to domain values and delegates every
//! persistence rule that already existed: atomic writes, byte and entry
//! budgets, tombstones, completion journalling and claim-token minting. The
//! domain owns ordering, scheduling and claim semantics.

use std::fs;

use comenq_domain::{ClaimToken, Completion, EntryId, NewEntry, QueueRepository, QueuedEntry};
use uuid::Uuid;

use super::{QueueStore, Result, StoreError, StoredEntry};
use crate::queue::convert::{queued_from_stored, request_from_comment, stored_from_queued};

impl QueueRepository for QueueStore {
    type Error = StoreError;

    fn entries(&self) -> Result<Vec<QueuedEntry>> {
        QueueStore::entries(self)?
            .into_iter()
            .map(queued_from_stored)
            .collect()
    }

    fn find(&self, id: &EntryId) -> Result<Option<QueuedEntry>> {
        match QueueStore::find(self, id.as_str()) {
            Ok(entry) => queued_from_stored(entry).map(Some),
            Err(StoreError::UnknownId(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn insert(&self, entry: NewEntry) -> Result<QueuedEntry> {
        let stored = StoredEntry {
            id: entry.id.into_string(),
            incarnation: Uuid::new_v4().to_string(),
            order: entry.order,
            flutter_seconds: entry.flutter_seconds,
            enqueued_at: entry.enqueued_at,
            not_before: entry.not_before,
            claim_token: None,
            request: request_from_comment(&entry.comment),
        };
        self.write_entry(&stored)?;
        queued_from_stored(stored)
    }

    fn save(&self, entry: &QueuedEntry) -> Result<()> {
        self.write_entry(&stored_from_queued(entry))
    }

    fn remove(&self, id: &EntryId) -> Result<()> {
        let entry = QueueStore::find(self, id.as_str())?;
        self.write_await_deletion(&entry)?;
        fs::remove_file(self.entry_path(id.as_str())?)?;
        Ok(())
    }

    fn last_post(&self) -> Result<Option<u64>> {
        QueueStore::last_post(self)
    }

    fn last_attempt(&self) -> Result<Option<u64>> {
        QueueStore::last_attempt(self)
    }

    fn record_attempt(&self, attempted_at: u64) -> Result<()> {
        let latest = QueueStore::last_attempt(self)?
            .unwrap_or(0)
            .max(attempted_at);
        self.write_atomic(&self.last_attempt_path, latest.to_string().as_bytes())
    }

    fn mint_claim_token(&self) -> ClaimToken {
        ClaimToken::new(Uuid::new_v4().to_string())
    }

    fn complete(&self, completion: &Completion) -> Result<()> {
        let entry = stored_from_queued(&completion.entry);
        self.complete_claim_with_token(
            &entry,
            &completion.token_hash,
            completion.posted_at,
            completion.comment_url.clone(),
        )
    }
}
