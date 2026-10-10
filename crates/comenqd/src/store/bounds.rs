//! Bounded queue-entry reads, admission, and list materialization.

use super::{QueueStore, Result, StoreError, StoredEntry, is_valid_id};
use comenq_lib::protocol::MAX_PENDING_ENTRIES;
use std::fs;
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};

/// Maximum size of one serialized queue-entry file.
pub(super) const MAX_ENTRY_BYTES: u64 = 2 * 1024 * 1024;
/// Maximum aggregate budget for entry files and their remaining update headroom.
pub(super) const MAX_QUEUE_BYTES: u64 = 32 * 1024 * 1024;
/// Space reserved per entry for durable claim and ordering updates.
pub(super) const ENTRY_MUTATION_HEADROOM_BYTES: u64 = 80;

/// Metadata needed to enforce the durable limits before retaining entry data.
#[derive(Debug, Clone, Copy)]
pub(super) struct QueueUsage {
    pub(super) count: usize,
    pub(super) bytes: u64,
    headroom_bytes: u64,
}

impl QueueUsage {
    /// Return persisted bytes plus reserved mutation headroom.
    fn accounted_bytes(self) -> u64 {
        self.bytes.saturating_add(self.headroom_bytes)
    }
}

impl QueueStore {
    /// Validate startup data after bounded entry reads account for reservations.
    pub(super) fn validate_existing_limits(&self) -> Result<()> {
        let (_, usage) = self.entries_with_usage()?;
        ensure_queue_bytes(usage.bytes, usage.headroom_bytes)
    }

    /// Return pending entries in posting order, skipping unreadable records.
    pub fn entries(&self) -> Result<Vec<StoredEntry>> {
        self.entries_with_usage().map(|(entries, _)| entries)
    }

    /// Return the valid entry count and accounted bytes for queue metrics.
    pub(crate) fn queue_metrics_snapshot(&self) -> Result<(usize, u64)> {
        let (entries, usage) = self.entries_with_usage()?;
        Ok((entries.len(), usage.accounted_bytes()))
    }

    /// Read a bounded directory snapshot and retain its metadata for admission.
    pub(super) fn entries_with_usage(&self) -> Result<(Vec<StoredEntry>, QueueUsage)> {
        let (files, mut usage) = self.scan_entry_files()?;
        let mut entries = Vec::with_capacity(files.len());
        for path in files {
            match read_entry(&path) {
                Ok(entry) if is_valid_id(&entry.id) => {
                    let file_bytes = match fs::metadata(&path) {
                        Ok(metadata) => metadata.len(),
                        Err(error) => {
                            tracing::error!(
                                error_kind = io_error_kind(&error),
                                "Skipping unreadable queue entry"
                            );
                            continue;
                        }
                    };
                    let required_bytes = file_bytes.saturating_add(maximum_mutation_growth(&entry));
                    if required_bytes > MAX_ENTRY_BYTES {
                        return Err(StoreError::EntryTooLarge {
                            size: required_bytes,
                            limit: MAX_ENTRY_BYTES,
                        });
                    }
                    usage.headroom_bytes =
                        usage.headroom_bytes.saturating_add(entry_headroom(&entry));
                    entries.push(entry);
                }
                Ok(_) => {
                    tracing::error!(
                        error_kind = "unsafe_identifier",
                        "Skipping queue entry with an unsafe identifier"
                    );
                }
                Err(error) => {
                    tracing::error!(
                        error_kind = store_error_kind(&error),
                        "Skipping unreadable queue entry"
                    );
                }
            }
        }
        entries
            .sort_by(|a, b| (a.order, a.enqueued_at, &a.id).cmp(&(b.order, b.enqueued_at, &b.id)));
        Ok((entries, usage))
    }

    /// Persist an updated entry after rechecking the durable capacity.
    pub(super) fn write_entry(&self, entry: &StoredEntry) -> Result<()> {
        let (_, usage) = self.entries_with_usage()?;
        self.write_entry_with_usage(entry, usage)
    }

    /// Persist an entry using a previously scanned store metadata snapshot.
    pub(super) fn write_entry_with_usage(
        &self,
        entry: &StoredEntry,
        usage: QueueUsage,
    ) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(entry)?;
        let new_bytes = u64::try_from(bytes.len()).unwrap_or(u64::MAX);

        let path = self.entry_path(&entry.id)?;
        let previous_bytes = match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_file() => Some(metadata.len()),
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "queue entry path is not a regular file",
                )
                .into());
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let count = usage.count + usize::from(previous_bytes.is_none());
        if count > MAX_PENDING_ENTRIES {
            return Err(StoreError::QueueFull(MAX_PENDING_ENTRIES));
        }
        let entry_limit = if previous_bytes.is_none() {
            MAX_ENTRY_BYTES.saturating_sub(ENTRY_MUTATION_HEADROOM_BYTES)
        } else {
            MAX_ENTRY_BYTES
        };
        ensure_entry_size(new_bytes, entry_limit)?;
        let used_bytes = usage.bytes.saturating_sub(previous_bytes.unwrap_or(0));
        let previous_headroom = if previous_bytes.is_some() {
            entry_headroom(&read_entry(&path)?)
        } else {
            0
        };
        let other_headroom = usage.headroom_bytes.saturating_sub(previous_headroom);
        let requested_bytes = new_bytes
            .saturating_add(other_headroom)
            .saturating_add(entry_headroom(entry));
        ensure_queue_bytes(used_bytes, requested_bytes)?;
        self.write_atomic(&path, &bytes)
    }

    /// Collect entry-file metadata while enforcing count and aggregate limits.
    fn scan_entry_files(&self) -> Result<(Vec<PathBuf>, QueueUsage)> {
        let mut paths = Vec::new();
        let mut bytes = 0_u64;
        for dirent in fs::read_dir(&self.entries_dir)? {
            let dirent = dirent?;
            let path = dirent.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            if !dirent.file_type()?.is_file() {
                tracing::error!(
                    error_kind = "non_regular_file",
                    "Skipping non-regular queue entry"
                );
                continue;
            }
            if paths.len() == MAX_PENDING_ENTRIES {
                return Err(StoreError::QueueFull(MAX_PENDING_ENTRIES));
            }
            let file_bytes = dirent.metadata()?.len();
            ensure_queue_bytes(bytes, file_bytes)?;
            bytes = bytes.saturating_add(file_bytes);
            paths.push(path);
        }
        let count = paths.len();
        Ok((
            paths,
            QueueUsage {
                count,
                bytes,
                headroom_bytes: 0,
            },
        ))
    }
}

/// Read at most one entry's limit plus a sentinel byte before deserializing.
pub(super) fn read_entry(path: &Path) -> Result<StoredEntry> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "queue entry path is not a regular file",
        )
        .into());
    }
    let entry_limit = MAX_ENTRY_BYTES;
    ensure_entry_size(metadata.len(), entry_limit)?;

    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    fs::File::open(path)?
        .take(entry_limit + 1)
        .read_to_end(&mut bytes)?;
    ensure_entry_size(u64::try_from(bytes.len()).unwrap_or(u64::MAX), entry_limit)?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Reject an entry size above its durable per-file limit.
fn ensure_entry_size(size: u64, limit: u64) -> Result<()> {
    if size > limit {
        Err(StoreError::EntryTooLarge { size, limit })
    } else {
        Ok(())
    }
}

/// Classify a filesystem failure without including its path or nested message.
fn io_error_kind(error: &io::Error) -> &'static str {
    match error.kind() {
        io::ErrorKind::NotFound => "not_found",
        io::ErrorKind::PermissionDenied => "permission_denied",
        io::ErrorKind::InvalidData => "invalid_data",
        _ => "io_error",
    }
}

/// Classify persisted-entry failures without formatting untrusted error data.
fn store_error_kind(error: &StoreError) -> &'static str {
    error.category()
}

/// Remaining reservation for the only fields changed after enqueue.
///
/// The reservation starts with room for worst-case `order` and `claim_token`
/// updates. As their serialized widths grow, the unused part shrinks by the
/// same amount, so a later mutation cannot exceed the bytes already budgeted.
pub(super) fn entry_headroom(entry: &StoredEntry) -> u64 {
    let order_growth =
        u64::try_from(entry.order.to_string().len().saturating_sub(1)).unwrap_or(u64::MAX);
    let claim_growth = entry.claim_token.as_deref().map_or(0, |token| {
        serde_json::to_vec(token).map_or(u64::MAX, |encoded| {
            u64::try_from(encoded.len())
                .unwrap_or(u64::MAX)
                .saturating_sub(4)
        })
    });
    ENTRY_MUTATION_HEADROOM_BYTES
        .saturating_sub(order_growth)
        .saturating_sub(claim_growth)
}

/// Calculate worst-case growth needed for the entry's order and claim fields.
fn maximum_mutation_growth(entry: &StoredEntry) -> u64 {
    const MAX_ORDER_BYTES: u64 = 20;
    const UUID_JSON_BYTES: u64 = 38;

    let order_growth = MAX_ORDER_BYTES
        .saturating_sub(u64::try_from(entry.order.to_string().len()).unwrap_or(u64::MAX));
    let claim_bytes = entry.claim_token.as_deref().map_or(4, |token| {
        serde_json::to_vec(token).map_or(usize::MAX, |encoded| encoded.len())
    });
    let claim_growth =
        UUID_JSON_BYTES.saturating_sub(u64::try_from(claim_bytes).unwrap_or(u64::MAX));
    order_growth.saturating_add(claim_growth)
}

/// Reject a write whose persisted bytes and remaining reservations exceed budget.
fn ensure_queue_bytes(used: u64, requested: u64) -> Result<()> {
    if used.saturating_add(requested) > MAX_QUEUE_BYTES {
        Err(StoreError::QueueByteBudgetExceeded {
            used,
            requested,
            limit: MAX_QUEUE_BYTES,
        })
    } else {
        Ok(())
    }
}
