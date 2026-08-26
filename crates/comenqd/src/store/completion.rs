//! Durable recovery for successful GitHub posts.

use super::{QueueStore, Result, StoreError};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;

/// A post that must be reflected in the queue before another entry is sent.
#[derive(Debug, Deserialize, Serialize)]
struct CompletionRecord {
    id: String,
    posted_at: u64,
}

impl QueueStore {
    /// Persist post completion before removing its entry and updating the marker.
    pub fn complete(&self, id: &str, now: u64) -> Result<()> {
        self.find(id)?;
        let record = serde_json::to_vec(&CompletionRecord {
            id: id.to_owned(),
            posted_at: now,
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
        match fs::remove_file(&entry_path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(StoreError::Io(error)),
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
