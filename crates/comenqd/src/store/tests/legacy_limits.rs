//! Legacy incarnation migration consumes reserved space without surprising writes.

use super::{immediate, open_store, request};
use crate::store::bounds::{MAX_ENTRY_BYTES, MAX_QUEUE_BYTES, entry_headroom};
use crate::store::{QueueStore, StoredEntry};
use rstest::rstest;
use std::fs;
use tempfile::TempDir;

fn legacy(entry: &StoredEntry) -> StoredEntry {
    let mut entry = entry.clone();
    entry.incarnation.clear();
    entry
}

#[rstest]
fn legacy_migration_preserves_the_accounted_queue_budget_at_its_limit() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let entry = store
        .put(request("legacy"), &immediate(0), 1000)
        .expect("put");
    let legacy = legacy(&entry);
    let bytes = serde_json::to_vec_pretty(&legacy).expect("encode legacy");
    let path = dir
        .path()
        .join("entries")
        .join(format!("{}.json", entry.id));
    fs::write(&path, &bytes).expect("seed legacy");
    let reserved = entry_headroom(&legacy);
    let padding = MAX_QUEUE_BYTES - bytes.len() as u64 - reserved;
    let file = fs::File::create(dir.path().join("entries/padding.json")).expect("padding");
    file.set_len(padding).expect("size padding");
    let reopened = QueueStore::open(dir.path()).expect("admit and migrate at queue limit");
    let migrated = reopened
        .entries()
        .expect("entries")
        .into_iter()
        .next()
        .expect("entry");
    assert!(!migrated.incarnation.is_empty());
    let actual_growth = fs::metadata(&path).expect("metadata").len() - bytes.len() as u64;
    assert_eq!(actual_growth, 57);
    assert_eq!(actual_growth + entry_headroom(&migrated), reserved);
    reopened
        .claim_next_due(0, 1000)
        .expect("claim inside reservation")
        .expect("due");
    let (_, used) = reopened.queue_metrics_snapshot().expect("usage");
    assert_eq!(used, MAX_QUEUE_BYTES);
}

#[rstest]
fn legacy_entry_near_its_size_limit_can_migrate_and_then_be_claimed() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let entry = store.put(request(""), &immediate(0), 1000).expect("put");
    let mut legacy = legacy(&entry);
    let empty_bytes = serde_json::to_vec_pretty(&legacy).expect("encode").len();
    // Reserve the worst-case order and claim growth, plus the new UUID field.
    let body_bytes = MAX_ENTRY_BYTES as usize - empty_bytes - 19 - 34 - 57;
    legacy.request.body = "x".repeat(body_bytes);
    let path = dir
        .path()
        .join("entries")
        .join(format!("{}.json", entry.id));
    fs::write(
        path,
        serde_json::to_vec_pretty(&legacy).expect("encode legacy"),
    )
    .expect("write legacy");
    let reopened = QueueStore::open(dir.path()).expect("migrate near entry limit");
    let (claimed, _) = reopened
        .claim_next_due(0, 1000)
        .expect("claim")
        .expect("due");
    assert!(!claimed.incarnation.is_empty());
}
