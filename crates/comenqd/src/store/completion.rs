//! Durable recovery for successful GitHub posts.

use super::{HistoryRecord, QueueStore, Result, StoreError, StoredEntry};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;

/// A post that must be reflected in the queue before another entry is sent.
#[derive(Debug, Deserialize, Serialize)]
struct CompletionRecord {
    id: String,
    posted_at: u64,
    #[serde(default)]
    claim_token: Option<String>,
    #[serde(default)]
    history: Option<HistoryRecord>,
    #[serde(default)]
    history_offset: Option<u64>,
}

impl QueueStore {
    /// Persist post completion before removing its entry and updating the marker.
    pub fn complete(&self, id: &str, now: u64) -> Result<()> {
        let entry = self.find(id)?;
        self.complete_claim(&entry, now)
    }

    /// Complete a claimed post without deleting a replacement entry.
    pub fn complete_claim(&self, entry: &StoredEntry, now: u64) -> Result<()> {
        self.complete_claim_with_token(entry, "", now, None)
    }

    /// Complete a claimed post and persist its token identity for rotation.
    pub(crate) fn complete_claim_with_token(
        &self,
        entry: &StoredEntry,
        token_hash: &str,
        now: u64,
        comment_url: Option<String>,
    ) -> Result<()> {
        if entry.claim_token.is_none() {
            self.find(&entry.id)?;
        }
        let history_offset = match fs::metadata(&self.history_path) {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
            Err(error) => return Err(error.into()),
        };
        let record = serde_json::to_vec(&CompletionRecord {
            id: entry.id.clone(),
            posted_at: now,
            claim_token: entry.claim_token.clone(),
            history: Some(HistoryRecord::success(entry, token_hash, now, comment_url)),
            history_offset: Some(history_offset),
        })?;
        self.write_atomic(&self.completion_path, &record)?;
        self.reconcile_completion()
    }

    /// Finish a persisted completion record, if a previous cleanup stopped early.
    pub(super) fn reconcile_completion(&self) -> Result<()> {
        let Some(record) = self.read_completion_record()? else {
            return Ok(());
        };
        let entry_path = self.entry_path(&record.id)?;
        self.apply_completion_history(&record)?;
        if self.completion_owns_entry(&record)? {
            remove_entry_file(&entry_path)?;
        }
        self.write_atomic(
            &self.last_post_path,
            record.posted_at.to_string().as_bytes(),
        )?;
        self.remove_completion_record()
    }

    /// Replay the history entry of `record`, if it has one, and clear the
    /// matching deletion marker.
    ///
    /// Scope: used by `reconcile_completion` only.
    fn apply_completion_history(&self, record: &CompletionRecord) -> Result<()> {
        let Some(history) = &record.history else {
            return Ok(());
        };
        self.append_completion_history(history, record.history_offset.unwrap_or(0))?;
        self.clear_await_deletion(&record.id, &history.incarnation)
    }

    /// Remove the completion record and persist the removal by syncing its
    /// parent directory.
    ///
    /// Scope: the last step of `reconcile_completion`.
    fn remove_completion_record(&self) -> Result<()> {
        fs::remove_file(&self.completion_path)?;
        let parent = self.completion_path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "completion path has no parent")
        })?;
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    }

    /// Read the persisted completion record.
    ///
    /// Returns `Ok(None)` when no record exists, so there is nothing to
    /// reconcile; a read or parse failure is returned as an error. Scope: used
    /// by `reconcile_completion` only.
    fn read_completion_record(&self) -> Result<Option<CompletionRecord>> {
        let text = match fs::read_to_string(&self.completion_path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        Ok(Some(serde_json::from_str(&text)?))
    }

    /// Decide whether completing `record` should remove the queued entry.
    ///
    /// True when the entry still exists and either the record carries no claim
    /// token or the entry's token matches it; false when the entry is gone or
    /// was claimed afresh. Lookup errors other than an unknown identifier are
    /// returned. Scope: used by `reconcile_completion` only.
    fn completion_owns_entry(&self, record: &CompletionRecord) -> Result<bool> {
        match self.find(&record.id) {
            Ok(entry) => Ok(record
                .claim_token
                .as_deref()
                .is_none_or(|token| entry.claim_token.as_deref() == Some(token))),
            Err(StoreError::UnknownId(_)) => Ok(false),
            Err(error) => Err(error),
        }
    }
}

/// Remove the entry file at `path`, treating a missing file as already removed.
fn remove_entry_file(path: &std::path::Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(StoreError::Io(error)),
    }
}
