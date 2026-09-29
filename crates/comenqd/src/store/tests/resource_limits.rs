//! Durable capacity and bounded persisted-entry read tests.

use super::{StoreError, immediate, open_store, request};
use crate::store::bounds::{
    ENTRY_MUTATION_HEADROOM_BYTES, MAX_ENTRY_BYTES, MAX_QUEUE_BYTES, entry_headroom,
};
use crate::store::{QueueStore, StoredEntry};
use comenq_lib::CommentRequest;
use comenq_lib::protocol::MAX_PENDING_ENTRIES;
use std::fs::{self, OpenOptions};
use std::path::Path;
use tempfile::TempDir;

fn entry(request: CommentRequest, order: i64, enqueued_at: u64) -> StoredEntry {
    StoredEntry {
        id: super::entry_id(&request, enqueued_at),
        order,
        flutter_seconds: 0,
        enqueued_at,
        not_before: 0,
        claim_token: None,
        request,
    }
}

fn write_sparse_file(path: &Path, bytes: u64) {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .expect("create sparse queue file")
        .set_len(bytes)
        .expect("size sparse queue file");
}

fn write_entry(entries_dir: &Path, entry: &StoredEntry) -> u64 {
    let bytes = serde_json::to_vec_pretty(entry).expect("serialize stored entry");
    fs::write(entries_dir.join(format!("{}.json", entry.id)), &bytes).expect("write stored entry");
    u64::try_from(bytes.len()).expect("entry size fits u64")
}

fn write_padding(entries_dir: &Path, used: u64, requested: u64, final_count: usize, extra: u64) {
    // The sparse padding record exceeds MAX_ENTRY_BYTES and has no mutation
    // headroom; reserve for the other existing records and the candidate put.
    let headroom = u64::try_from(final_count.saturating_sub(1)).expect("entry count fits u64")
        * ENTRY_MUTATION_HEADROOM_BYTES;
    let padding = MAX_QUEUE_BYTES - used - requested - headroom + extra;
    write_sparse_file(&entries_dir.join("padding.json"), padding);
}

#[test]
fn put_is_admitted_when_existing_data_leaves_exactly_enough_budget() {
    let dir = TempDir::new().expect("create queue directory");
    open_store(&dir)
        .put(request("existing"), &immediate(0), 1_000)
        .expect("put existing entry");

    let entries_dir = dir.path().join("entries");
    let existing_bytes = fs::metadata(entries_dir.join(format!(
        "{}.json",
        super::entry_id(&request("existing"), 1_000)
    )))
    .expect("read existing entry metadata")
    .len();
    let candidate = entry(request("at the limit"), 1, 1_001);
    let candidate_bytes = u64::try_from(
        serde_json::to_vec_pretty(&candidate)
            .expect("serialize candidate")
            .len(),
    )
    .expect("candidate size fits u64");
    write_padding(&entries_dir, existing_bytes, candidate_bytes, 3, 0);

    let store = QueueStore::open(dir.path()).expect("open within queue byte budget");
    let inserted = store
        .put(request("at the limit"), &immediate(0), 1_001)
        .expect("admit put at exact budget");

    assert_eq!(inserted.id, candidate.id);
    assert!(entries_dir.join(format!("{}.json", inserted.id)).exists());
    store
        .bump(&inserted.id)
        .expect("reorder within reserved headroom");
    let (claimed, _) = store
        .claim_next_due(0, 1_001)
        .expect("claim within reserved headroom")
        .expect("head entry is due");
    assert_eq!(claimed.id, inserted.id);
    let stored_bytes = fs::read_dir(&entries_dir)
        .expect("read entries after claim")
        .map(|item| {
            item.expect("read entry metadata")
                .metadata()
                .expect("entry metadata")
                .len()
        })
        .sum::<u64>();
    let headroom = store
        .entries()
        .expect("read valid entries after claim")
        .iter()
        .map(entry_headroom)
        .sum::<u64>();
    assert!(
        stored_bytes + headroom <= MAX_QUEUE_BYTES,
        "claim stays inside the durable budget"
    );
}

#[test]
fn put_is_rejected_before_persistence_when_existing_data_exhausts_budget() {
    let dir = TempDir::new().expect("create queue directory");
    open_store(&dir)
        .put(request("existing"), &immediate(0), 1_000)
        .expect("put existing entry");

    let entries_dir = dir.path().join("entries");
    let existing_bytes = fs::read_dir(&entries_dir)
        .expect("read entries directory")
        .map(|item| {
            item.expect("read entry metadata")
                .metadata()
                .expect("entry metadata")
                .len()
        })
        .sum::<u64>();
    let candidate = entry(request("over the limit"), 1, 1_001);
    let candidate_bytes = u64::try_from(
        serde_json::to_vec_pretty(&candidate)
            .expect("serialize candidate")
            .len(),
    )
    .expect("candidate size fits u64");
    write_padding(&entries_dir, existing_bytes, candidate_bytes, 3, 1);

    let store = QueueStore::open(dir.path()).expect("open within existing byte budget");
    let error = store
        .put(request("over the limit"), &immediate(0), 1_001)
        .expect_err("reject put before persistence");

    assert!(matches!(error, StoreError::QueueByteBudgetExceeded { .. }));
    assert!(!entries_dir.join(format!("{}.json", candidate.id)).exists());
}

#[test]
fn startup_counts_existing_entry_data_towards_the_admission_budget() {
    let dir = TempDir::new().expect("create queue directory");
    let existing = entry(request("already on disk"), 0, 1_000);
    let entries_dir = dir.path().join("entries");
    fs::create_dir_all(&entries_dir).expect("create entries directory");
    let existing_bytes = write_entry(&entries_dir, &existing);
    let candidate = entry(request("after restart"), 1, 1_001);
    let candidate_bytes = u64::try_from(
        serde_json::to_vec_pretty(&candidate)
            .expect("serialize candidate")
            .len(),
    )
    .expect("candidate size fits u64");
    write_padding(&entries_dir, existing_bytes, candidate_bytes, 3, 1);

    let store = QueueStore::open(dir.path()).expect("recover existing queue within budget");
    let error = store
        .put(request("after restart"), &immediate(0), 1_001)
        .expect_err("startup data must count against future puts");

    assert!(matches!(error, StoreError::QueueByteBudgetExceeded { .. }));
    assert!(entries_dir.join(format!("{}.json", existing.id)).exists());
    assert!(!entries_dir.join(format!("{}.json", candidate.id)).exists());
}

#[test]
fn startup_fails_closed_without_removing_data_over_the_byte_budget() {
    let dir = TempDir::new().expect("create queue directory");
    let entries_dir = dir.path().join("entries");
    fs::create_dir_all(&entries_dir).expect("create entries directory");
    let oversized_path = entries_dir.join("oversized.json");
    write_sparse_file(&oversized_path, MAX_QUEUE_BYTES + 1);

    let error = QueueStore::open(dir.path()).expect_err("reject an over-budget store");

    assert!(matches!(error, StoreError::QueueByteBudgetExceeded { .. }));
    assert_eq!(
        fs::metadata(oversized_path).expect("file remains").len(),
        MAX_QUEUE_BYTES + 1
    );
}

#[test]
fn startup_fails_closed_when_an_entry_has_no_mutation_headroom() {
    let dir = TempDir::new().expect("create queue directory");
    let entries_dir = dir.path().join("entries");
    fs::create_dir_all(&entries_dir).expect("create entries directory");

    let empty = entry(request(""), 0, 1_000);
    let empty_size = serde_json::to_vec_pretty(&empty)
        .expect("serialize empty entry")
        .len();
    let target_size = usize::try_from(MAX_ENTRY_BYTES - 1).expect("entry limit fits usize");
    let body = "x".repeat(target_size - empty_size);
    let near_limit = entry(request(&body), 0, 1_000);
    let persisted_size = write_entry(&entries_dir, &near_limit);
    assert_eq!(persisted_size, MAX_ENTRY_BYTES - 1);

    let error = QueueStore::open(dir.path()).expect_err("reject an unmodifiable entry");

    assert!(matches!(
        error,
        StoreError::EntryTooLarge {
            limit: MAX_ENTRY_BYTES,
            ..
        }
    ));
    assert!(entries_dir.join(format!("{}.json", near_limit.id)).exists());
}

#[test]
fn startup_fails_closed_when_existing_entry_count_exceeds_the_limit() {
    let dir = TempDir::new().expect("create queue directory");
    let entries_dir = dir.path().join("entries");
    fs::create_dir_all(&entries_dir).expect("create entries directory");
    for index in 0..=MAX_PENDING_ENTRIES {
        fs::write(entries_dir.join(format!("{index:08x}.json")), b"{}").expect("write entry");
    }

    let error = QueueStore::open(dir.path()).expect_err("reject too many existing entries");

    assert!(matches!(error, StoreError::QueueFull(MAX_PENDING_ENTRIES)));
    assert_eq!(
        fs::read_dir(entries_dir).expect("entries remain").count(),
        MAX_PENDING_ENTRIES + 1
    );
}

#[test]
fn entries_skip_a_valid_but_oversized_record_and_keep_other_entries() {
    let dir = TempDir::new().expect("create queue directory");
    let entries_dir = dir.path().join("entries");
    fs::create_dir_all(&entries_dir).expect("create entries directory");

    let valid = entry(request("valid"), 0, 1_000);
    write_entry(&entries_dir, &valid);
    let oversized = entry(
        CommentRequest {
            body: "x".repeat(MAX_ENTRY_BYTES as usize),
            ..request("unused")
        },
        1,
        1_001,
    );
    let oversized_bytes = write_entry(&entries_dir, &oversized);
    assert!(oversized_bytes > MAX_ENTRY_BYTES);

    let store = QueueStore::open(dir.path()).expect("open under aggregate byte budget");
    let entries = store.entries().expect("read bounded entries");

    assert_eq!(entries, vec![valid]);
    assert!(entries_dir.join(format!("{}.json", oversized.id)).exists());
}
