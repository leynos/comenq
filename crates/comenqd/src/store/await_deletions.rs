//! Durable tombstones that keep deleted identifier lifetimes from exposing old URLs.

use super::{QueueStore, Result, StoreError, StoredEntry, is_valid_id};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Sub-directory holding deletion markers for deterministic identifiers.
pub(super) const AWAIT_DELETIONS_DIR: &str = "await-deletions";

/// Remove the deletion marker at `path` if it exists.
///
/// Returns `Ok(true)` when a marker was found and removed, `Ok(false)` when
/// none existed, and the I/O error for any other failure. Scope: used by
/// reconciliation only; it does not sync the parent directory, so the caller
/// does that once after the last removal.
fn remove_marker_if_present(path: &std::path::Path) -> Result<bool> {
    match fs::read(path) {
        Ok(_) => {
            fs::remove_file(path)?;
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// Delete a marker and sync its parent directory.
///
/// The parent sync makes the removal durable: a marker resurrected by a crash
/// would keep hiding an already-posted URL.
fn remove_marker_durably(path: &Path) -> Result<()> {
    fs::remove_file(path)?;
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "deletion marker has no parent")
    })?;
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

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
            Ok(bytes) => self.clear_marker(&path, id, incarnation, &bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    /// Remove a marker that still describes `id`, then sync its parent.
    ///
    /// A marker for another incarnation is left in place: it still hides an
    /// earlier incarnation's URL.
    fn clear_marker(&self, path: &Path, id: &str, incarnation: &str, bytes: &[u8]) -> Result<()> {
        let marker: AwaitDeletionRecord = serde_json::from_slice(bytes)?;
        if !self.marker_applies(id, incarnation, &marker)? {
            return Ok(());
        }
        remove_marker_durably(path)
    }

    /// Report whether `incarnation` owns either the marker or the live entry.
    ///
    /// The entry lookup runs unconditionally, so a storage failure surfaces
    /// even when the marker alone would have justified the removal.
    fn marker_applies(
        &self,
        id: &str,
        incarnation: &str,
        marker: &AwaitDeletionRecord,
    ) -> Result<bool> {
        let current_matches = match self.find(id) {
            Ok(entry) => entry.incarnation == incarnation,
            Err(StoreError::UnknownId(_)) => false,
            Err(error) => return Err(error),
        };
        Ok(marker.incarnation == incarnation || current_matches)
    }

    /// Clear a deletion marker left before a crash that did not remove its entry.
    pub(super) fn reconcile_await_deletions(&self) -> Result<()> {
        if self.remove_markers_of_queued_entries()? {
            // Persist reconciliation before a later completion can outlive the
            // entry while a resurrected tombstone still hides its URL.
            fs::File::open(&self.await_deletions_dir)?.sync_all()?;
        }
        Ok(())
    }

    /// Remove the deletion marker of every queued entry that still has one.
    ///
    /// Returns whether any marker was removed, so the caller knows whether the
    /// directory needs syncing. Scope: used by `reconcile_await_deletions` only.
    fn remove_markers_of_queued_entries(&self) -> Result<bool> {
        let mut removed_marker = false;
        for entry in self.entries()? {
            let path = self.await_deletion_path(&entry.id)?;
            removed_marker |= remove_marker_if_present(&path)?;
        }
        Ok(removed_marker)
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
