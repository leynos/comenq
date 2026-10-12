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
            history: Some(HistoryRecord::success(entry, now)),
            history_offset: Some(history_offset),
        })?;
        self.write_atomic(&self.completion_path, &record)?;
        self.reconcile_completion()
    }

    /// Finish a persisted completion record, if a previous cleanup stopped early.
    pub(super) fn reconcile_completion(&self) -> Result<()> {
        let text = match fs::read_to_string(&self.completion_path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        let record: CompletionRecord = serde_json::from_str(&text)?;
        let entry_path = self.entry_path(&record.id)?;
        if let Some(history) = &record.history {
            self.append_completion_history(history, record.history_offset.unwrap_or(0))?;
        }
        let remove_entry = match self.find(&record.id) {
            Ok(entry) => record
                .claim_token
                .as_deref()
                .is_none_or(|token| entry.claim_token.as_deref() == Some(token)),
            Err(StoreError::UnknownId(_)) => false,
            Err(error) => return Err(error),
        };
        if remove_entry {
            match fs::remove_file(&entry_path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(StoreError::Io(error)),
            }
        }
        self.write_atomic(
            &self.last_post_path,
            record.posted_at.to_string().as_bytes(),
        )?;
        fs::remove_file(&self.completion_path)?;
        let parent = self.completion_path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "completion path has no parent")
        })?;
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    }
}
