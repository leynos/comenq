//! Completion URL and identifier-lifetime lookup regressions.

use super::{deferred, immediate, open_store, request};
use crate::store::{AwaitState, HistoryRecord, StoreError};
use rstest::rstest;
use std::fs;
use tempfile::TempDir;

const URL: &str = "https://github.com/o/r/issues/1#issuecomment-1";

#[rstest]
fn completion_url_survives_journal_recovery() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let entry = store
        .put(request("recover"), &immediate(0), 1000)
        .expect("put");
    let history = HistoryRecord::success(&entry, "hash", 1200, Some(URL.into()));
    let marker = serde_json::json!({
        "id": entry.id, "posted_at": 1200, "history": history, "history_offset": 0,
    });
    fs::write(
        dir.path().join("completion"),
        serde_json::to_vec(&marker).expect("encode"),
    )
    .expect("persist marker");
    store.recover_worker_state().expect("recover");
    let reopened = open_store(&dir);
    assert_eq!(
        reopened
            .resolve_await(&entry.id, None, 600, 1200)
            .expect("lookup"),
        AwaitState::Posted {
            comment_url: Some(URL.into()),
            posted_at: 1200
        }
    );
    assert_eq!(
        reopened
            .history_page(None, None, 1)
            .expect("history")
            .records[0]
            .record
            .comment_url
            .as_deref(),
        Some(URL)
    );
}

#[rstest]
fn pending_entry_takes_precedence_over_an_older_success() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let entry = store.put(request("reuse"), &deferred(), 1000).expect("put");
    store
        .complete_claim_with_token(&entry, "", 1100, Some(URL.into()))
        .expect("complete");
    let replacement = store
        .put(request("reuse"), &deferred(), 1000)
        .expect("replace");
    assert_eq!(replacement.id, entry.id);
    let state = store
        .resolve_await(&entry.id, None, 600, 1100)
        .expect("lookup");
    let AwaitState::Pending {
        eta_seconds,
        cursor,
    } = state
    else {
        panic!("pending wins")
    };
    assert_eq!(eta_seconds, 600);
    assert!(cursor.offset > 0);
    store.del(&entry.id).expect("delete");
    assert_eq!(
        store
            .resolve_await(&entry.id, Some(&cursor), 600, 1100)
            .expect("lookup"),
        AwaitState::Unknown
    );
    assert_eq!(
        store
            .resolve_await(&entry.id, None, 600, 1100)
            .expect("later lookup"),
        AwaitState::Unknown
    );
}

#[rstest]
#[case::legacy_success(true, None)]
#[case::posted_url(true, Some(URL))]
#[case::failed_only(false, None)]
fn completion_lookup_skips_corrupt_lines_and_failures(
    #[case] success: bool,
    #[case] url: Option<&str>,
) {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let entry = store
        .put(request("history"), &immediate(0), 1000)
        .expect("put");
    if success {
        store
            .complete_claim_with_token(&entry, "", 1100, url.map(str::to_owned))
            .expect("record successful completion");
    } else {
        let record = HistoryRecord::failure(&entry, "", 1100, "failed");
        store.append_history(&record).expect("append failure");
        store.del(&entry.id).expect("remove failed entry");
    }
    let path = dir.path().join("history.jsonl");
    let mut bytes = fs::read(&path).expect("read");
    bytes.extend_from_slice(b"bad json\n");
    fs::write(path, bytes).expect("corrupt tail");
    let expected = if success {
        AwaitState::Posted {
            comment_url: url.map(str::to_owned),
            posted_at: 1100,
        }
    } else {
        AwaitState::Unknown
    };
    assert_eq!(
        store
            .resolve_await(&entry.id, None, 600, 1200)
            .expect("lookup"),
        expected
    );
}

#[rstest]
#[case("")]
#[case("1234567")]
#[case("123456789")]
#[case("../badid")]
#[case("zzzzzzzz")]
fn await_lookup_rejects_invalid_identifiers(#[case] id: &str) {
    let dir = TempDir::new().expect("tempdir");
    assert!(matches!(
        open_store(&dir).resolve_await(id, None, 600, 1000),
        Err(StoreError::InvalidId(_))
    ));
}

#[rstest]
fn legacy_history_without_url_or_incarnation_remains_readable() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let entry = store
        .put(request("legacy"), &immediate(0), 1000)
        .expect("put");
    let mut value =
        serde_json::to_value(HistoryRecord::success(&entry, "", 1100, None)).expect("encode");
    value.as_object_mut().expect("object").remove("incarnation");
    let record: HistoryRecord = serde_json::from_value(value).expect("legacy decode");
    assert_eq!(record.comment_url, None);
    assert!(record.incarnation.is_empty());
}

#[rstest]
fn legacy_entries_gain_persisted_distinct_lifetimes_before_waiting() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let entry = store
        .put(request("legacy reuse"), &immediate(0), 1000)
        .expect("put");
    let path = dir
        .path()
        .join("entries")
        .join(format!("{}.json", entry.id));
    let mut legacy = serde_json::to_value(&entry).expect("encode entry");
    legacy
        .as_object_mut()
        .expect("object")
        .remove("incarnation");
    let legacy_bytes = serde_json::to_vec(&legacy).expect("encode legacy");
    fs::write(&path, &legacy_bytes).expect("seed legacy entry");
    let reopened = open_store(&dir);
    let AwaitState::Pending { cursor, .. } = reopened
        .resolve_await(&entry.id, None, 600, 1000)
        .expect("lookup")
    else {
        panic!("pending")
    };
    assert!(!cursor.incarnation.is_empty());
    let stable = open_store(&dir)
        .resolve_await(&entry.id, None, 600, 1000)
        .expect("repeat lookup");
    assert!(matches!(stable, AwaitState::Pending { cursor: ref again, .. } if *again == cursor));
    reopened.del(&entry.id).expect("delete legacy lifetime");
    fs::write(&path, legacy_bytes).expect("seed another legacy lifetime");
    reopened
        .recover_worker_state()
        .expect("migrate second lifetime");
    let replacement = reopened
        .entries()
        .expect("entries")
        .into_iter()
        .next()
        .expect("replacement");
    assert_ne!(replacement.incarnation, cursor.incarnation);
    reopened
        .complete_claim_with_token(&replacement, "", 1200, Some(URL.into()))
        .expect("complete replacement");
    let original = reopened
        .resolve_await(&entry.id, Some(&cursor), 600, 1200)
        .expect("original lookup");
    assert_eq!(original, AwaitState::Unknown);
}

#[rstest]
fn completion_after_a_torn_snapshot_tail_resolves_the_wait() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let entry = store
        .put(request("torn tail"), &immediate(0), 1000)
        .expect("put");
    let partial = b"not json, without a final newline";
    fs::write(dir.path().join("history.jsonl"), partial).expect("seed torn tail");
    let state = store
        .resolve_await(&entry.id, None, 600, 1000)
        .expect("initial lookup");
    let AwaitState::Pending { cursor, .. } = state else {
        panic!("pending")
    };
    assert_eq!(cursor.offset, partial.len() as u64);
    store
        .complete_claim_with_token(&entry, "hash", 1200, Some(URL.into()))
        .expect("complete");
    let outcome = store
        .resolve_await(&entry.id, Some(&cursor), 600, 1200)
        .expect("resolve after append");
    assert_eq!(
        outcome,
        AwaitState::Posted {
            comment_url: Some(URL.into()),
            posted_at: 1200
        }
    );
}
