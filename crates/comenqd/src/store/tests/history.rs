//! Durable posting-history log tests.

use super::{immediate, open_store, request};
use crate::store::HistoryRecord;
use rstest::rstest;
use std::fs;
use tempfile::TempDir;

#[rstest]
fn history_starts_empty() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    assert!(store.history().expect("read history").is_empty());
}

#[rstest]
fn history_records_round_trip_in_order() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let entry = store.put(request("a"), &immediate(0), 1_000).expect("put");
    let success = HistoryRecord::success(&entry, 1_600);
    let failure = HistoryRecord::failure(&entry, 2_200, "timeout");
    store.append_history(&success).expect("append success");
    store.append_history(&failure).expect("append failure");

    let records = store.history().expect("read history");
    assert_eq!(records, vec![success, failure.clone()]);
    let wire = failure.to_entry();
    assert_eq!(wire.id, entry.id);
    assert_eq!(wire.posted_at, 2_200);
    assert!(!wire.success);
    assert_eq!(wire.error.as_deref(), Some("timeout"));
    assert_eq!(wire.owner, "octocat");
    assert_eq!(wire.repo, "hello-world");
    assert_eq!(wire.pr_number, 7);
    assert_eq!(wire.body, "a");
}

#[rstest]
fn history_skips_malformed_lines() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let entry = store.put(request("a"), &immediate(0), 1_000).expect("put");
    store
        .append_history(&HistoryRecord::success(&entry, 1_600))
        .expect("append success");
    let log = dir.path().join("history.jsonl");
    let mut text = fs::read(&log).expect("read log");
    text.extend_from_slice(b"not json\ninvalid\xFFjson\n");
    fs::write(&log, text).expect("corrupt log");
    store
        .append_history(&HistoryRecord::failure(&entry, 2_200, "timeout"))
        .expect("append failure");

    let records = store.history().expect("read history");
    assert_eq!(records.len(), 2, "the corrupt line must not hide records");
    assert!(records[0].success);
    assert!(!records[1].success);
}
