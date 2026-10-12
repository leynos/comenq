//! Durable posting-history log tests.

use super::{immediate, open_store, request};
use crate::store::{HistoryRecord, QueueStore};
use proptest::prelude::*;
use rstest::rstest;
use std::fs;
use tempfile::TempDir;

#[rstest]
fn history_starts_empty() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    assert!(
        store
            .history_page(None, None, 4)
            .expect("read history page")
            .records
            .is_empty()
    );
}

#[rstest]
fn history_records_round_trip_in_order_across_pages() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let entry = store.put(request("a"), &immediate(0), 1_000).expect("put");
    let success = HistoryRecord::success(&entry, 1_600);
    let failure = HistoryRecord::failure(&entry, 2_200, "timeout");
    store.append_history(&success).expect("append success");
    store.append_history(&failure).expect("append failure");

    assert_eq!(read_all(&store, None, 1), vec![success, failure.clone()]);
    assert_eq!(failure.id, entry.id);
    assert_eq!(failure.posted_at, 2_200);
    assert!(!failure.success);
    assert_eq!(failure.error.as_deref(), Some("timeout"));
    assert_eq!(failure.request.owner, "octocat");
    assert_eq!(failure.request.repo, "hello-world");
    assert_eq!(failure.request.pr_number, 7);
    assert_eq!(failure.request.body, "a");
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
    let failure = HistoryRecord::failure(&entry, 2_200, "timeout");
    store.append_history(&failure).expect("append failure");

    let records = read_all(&store, None, 1);
    assert_eq!(records.len(), 2, "the corrupt line must not hide records");
    assert!(records[0].success);
    assert!(!records[1].success);
    assert_eq!(read_all(&store, Some(1), 1), vec![failure]);
}

#[rstest]
fn appending_after_an_unterminated_partial_record_preserves_the_new_record() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let entry = store.put(request("a"), &immediate(0), 1_000).expect("put");
    fs::write(dir.path().join("history.jsonl"), b"{partial record").expect("write partial tail");
    let expected = HistoryRecord::failure(&entry, 2_200, "timeout");

    store
        .append_history(&expected)
        .expect("append after partial record");

    assert_eq!(read_all(&store, None, 1), vec![expected]);
}

#[rstest]
fn skipping_an_oversized_line_does_not_hide_the_following_record() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let entry = store.put(request("a"), &immediate(0), 1_000).expect("put");
    let log = dir.path().join("history.jsonl");
    let mut text = vec![b'x'; 2 * 1024 * 1024 + 1];
    text.push(b'\n');
    fs::write(&log, text).expect("write oversized line");
    let expected = HistoryRecord::failure(&entry, 2_200, "timeout");

    store
        .append_history(&expected)
        .expect("append after oversized line");

    assert_eq!(read_all(&store, None, 1), vec![expected]);
}

#[rstest]
fn history_cursor_keeps_the_initial_snapshot_when_new_records_are_appended() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let entry = store.put(request("a"), &immediate(0), 1_000).expect("put");
    let first = HistoryRecord::success(&entry, 1_600);
    let second = HistoryRecord::success(&entry, 2_200);
    let later = HistoryRecord::success(&entry, 2_800);
    store.append_history(&first).expect("append first");
    store.append_history(&second).expect("append second");

    let page = store.history_page(None, None, 1).expect("read first page");
    let cursor = page.next.expect("following page");
    store
        .append_history(&later)
        .expect("append outside snapshot");
    let following = store
        .history_page(Some(cursor), None, 1)
        .expect("read snapshot continuation");

    assert_eq!(
        following
            .records
            .into_iter()
            .map(|item| item.record)
            .collect::<Vec<_>>(),
        vec![second]
    );
    assert_eq!(following.next, None);
}

#[rstest]
fn recent_history_limit_is_applied_before_paginated_reads() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let entry = store.put(request("a"), &immediate(0), 1_000).expect("put");
    let expected: Vec<_> = (0..6)
        .map(|attempt| HistoryRecord::success(&entry, 1_600 + attempt))
        .collect();
    for record in &expected {
        store.append_history(record).expect("append record");
    }

    assert_eq!(read_all(&store, Some(3), 1), expected[3..]);
}

#[rstest]
fn recent_history_scan_crosses_chunk_boundaries() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let body = "recent record ".repeat(300);
    let entry = store
        .put(request(&body), &immediate(0), 1_000)
        .expect("put");
    let expected: Vec<_> = (0..12)
        .map(|attempt| HistoryRecord::success(&entry, 1_600 + attempt))
        .collect();
    for record in &expected {
        store.append_history(record).expect("append record");
    }

    assert_eq!(read_all(&store, Some(3), 1), expected[9..]);
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, .. ProptestConfig::default() })]

    #[test]
    fn history_pages_preserve_order_and_the_requested_recent_count(
        record_count in 0usize..40,
        limit in 0usize..40,
        page_size in 1usize..8,
    ) {
        let dir = TempDir::new().expect("tempdir");
        let store = open_store(&dir);
        let entry = store.put(request("generated"), &immediate(0), 1_000).expect("put");
        let expected: Vec<_> = (0..record_count)
            .map(|attempt| HistoryRecord::success(&entry, 2_000 + attempt as u64))
            .collect();
        for record in &expected {
            store.append_history(record).expect("append record");
        }
        let selected_start = record_count.saturating_sub(limit);
        prop_assert_eq!(
            read_all(&store, Some(limit), page_size),
            expected[selected_start..].to_vec(),
        );
    }
}

fn read_all(
    store: &QueueStore,
    recent_limit: Option<usize>,
    page_size: usize,
) -> Vec<HistoryRecord> {
    let mut records = Vec::new();
    let mut position = None;
    let mut is_first_page = true;
    loop {
        let page = store
            .history_page(
                position,
                is_first_page.then_some(recent_limit).flatten(),
                page_size,
            )
            .expect("read history page");
        records.extend(page.records.into_iter().map(|item| item.record));
        position = page.next;
        if position.is_none() {
            break;
        }
        is_first_page = false;
    }
    records
}
