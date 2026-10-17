//! Durable tombstones that keep deleted identifier lifetimes from exposing old URLs.

use super::{QueueStore, Result, StoreError, StoredEntry, is_valid_id};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::PathBuf;

/// Sub-directory holding deletion markers for deterministic identifiers.
pub(super) const AWAIT_DELETIONS_DIR: &str = "await-deletions";

/// Incarnation hidden by the latest successful deletion of an entry.
#[derive(Debug, Deserialize, Serialize)]
struct AwaitDeletionRecord {
    incarnation: String,
}

impl QueueStore {
    /// Check whether the latest queue lifetime was deleted without posting.
    pub(crate) fn await_lifetime_was_deleted(&self, id: &str) -> Result<bool> {
        match fs::read(self.await_deletion_path(id)?) {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    /// Record a deletion before removing its entry so stale successes stay hidden.
    pub(super) fn write_await_deletion(&self, entry: &StoredEntry) -> Result<()> {
        let marker = serde_json::to_vec(&AwaitDeletionRecord {
            incarnation: entry.incarnation.clone(),
        })?;
        self.write_atomic(&self.await_deletion_path(&entry.id)?, &marker)
    }

    /// Remove a deletion marker when the posting belongs to it or its successor.
    pub(crate) fn clear_await_deletion(&self, id: &str, incarnation: &str) -> Result<()> {
        let path = self.await_deletion_path(id)?;
        match fs::read(&path) {
            Ok(bytes) => {
                let marker: AwaitDeletionRecord = serde_json::from_slice(&bytes)?;
                let current_matches = match self.find(id) {
                    Ok(entry) => entry.incarnation == incarnation,
                    Err(StoreError::UnknownId(_)) => false,
                    Err(error) => return Err(error),
                };
                if marker.incarnation != incarnation && !current_matches {
                    return Ok(());
                }
                fs::remove_file(&path)?;
                let parent = path.parent().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "deletion marker has no parent")
                })?;
                fs::File::open(parent)?.sync_all()?;
                Ok(())
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    /// Clear a deletion marker left before a crash that did not remove its entry.
    pub(super) fn reconcile_await_deletions(&self) -> Result<()> {
        let mut removed_marker = false;
        for entry in self.entries()? {
            let path = self.await_deletion_path(&entry.id)?;
            match fs::read(&path) {
                Ok(_) => {
                    fs::remove_file(path)?;
                    removed_marker = true;
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        if removed_marker {
            // Persist reconciliation before a later completion can outlive the
            // entry while a resurrected tombstone still hides its URL.
            fs::File::open(&self.await_deletions_dir)?.sync_all()?;
        }
        Ok(())
    }

    /// Resolve an identifier's durable deletion marker path.
    fn await_deletion_path(&self, id: &str) -> Result<PathBuf> {
        if is_valid_id(id) {
            Ok(self.await_deletions_dir.join(id))
        } else {
            Err(StoreError::InvalidId(id.to_owned()))
        }
    }
}
