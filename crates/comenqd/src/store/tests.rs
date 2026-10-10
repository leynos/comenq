//! Tests for the reorderable persistent queue store.

use super::{PutOptions, QueueStore, StoreError, StoredEntry, entry_id};
use crate::store::MAX_QUEUE_ENTRIES;
use comenq_lib::CommentRequest;
use rstest::rstest;
use std::fs;
use tempfile::TempDir;

fn request(body: &str) -> CommentRequest {
    CommentRequest {
        owner: "octocat".into(),
        repo: "hello-world".into(),
        pr_number: 7,
        body: body.into(),
    }
}

fn open_store(dir: &TempDir) -> QueueStore {
    QueueStore::open(dir.path()).expect("open store")
}

/// Options for an immediate put with the given sampled flutter.
fn immediate(flutter_seconds: u64) -> PutOptions {
    PutOptions {
        cooldown: 600,
        flutter_seconds,
        immediate: true,
    }
}

/// Options for a default (deferred) put with no flutter.
fn deferred() -> PutOptions {
    PutOptions {
        cooldown: 600,
        flutter_seconds: 0,
        immediate: false,
    }
}

fn ids(store: &QueueStore) -> Vec<String> {
    store
        .entries()
        .expect("list entries")
        .into_iter()
        .map(|entry| entry.id)
        .collect()
}

#[rstest]
fn identifiers_are_deterministic_and_eight_characters() {
    let a = entry_id(&request("Hi"), 1000);
    let b = entry_id(&request("Hi"), 1000);
    assert_eq!(a, b);
    assert_eq!(a.len(), 8);
    assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(a, entry_id(&request("Hi"), 1001), "time must vary the id");
    assert_ne!(a, entry_id(&request("Yo"), 1000), "body must vary the id");
}

#[rstest]
fn put_preserves_arrival_order() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let first = store
        .put(request("first"), &immediate(0), 1000)
        .expect("put first");
    let second = store
        .put(request("second"), &immediate(0), 1001)
        .expect("put second");
    let third = store
        .put(request("third"), &immediate(0), 1002)
        .expect("put third");
    assert_eq!(ids(&store), vec![first.id, second.id, third.id]);
}

#[rstest]
fn put_persists_the_supplied_flutter() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    for i in 0..50 {
        let entry = store
            .put(request(&format!("body {i}")), &immediate(240), 1000 + i)
            .expect("put entry");
        assert_eq!(entry.flutter_seconds, 240);
    }
    let zero = store
        .put(request("plain"), &immediate(0), 2000)
        .expect("put plain");
    assert_eq!(zero.flutter_seconds, 0);
}

#[rstest]
fn put_rejects_entries_beyond_the_pending_limit() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    for index in 0..MAX_QUEUE_ENTRIES {
        store
            .put(
                request(&format!("body {index}")),
                &immediate(0),
                1_000 + index as u64,
            )
            .expect("put within capacity");
    }
    let error = store
        .put(request("over capacity"), &immediate(0), 9_999)
        .expect_err("capacity must be enforced before persistence");
    assert!(matches!(error, StoreError::QueueFull(MAX_QUEUE_ENTRIES)));
    assert_eq!(ids(&store).len(), MAX_QUEUE_ENTRIES);
}

#[rstest]
fn bump_moves_entry_to_head() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let a = store.put(request("a"), &immediate(0), 1000).expect("put a");
    let b = store.put(request("b"), &immediate(0), 1001).expect("put b");
    let c = store.put(request("c"), &immediate(0), 1002).expect("put c");
    store.bump(&c.id).expect("bump c");
    assert_eq!(ids(&store), vec![c.id, a.id, b.id]);
}

#[rstest]
fn bump_of_head_is_a_no_op() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let a = store.put(request("a"), &immediate(0), 1000).expect("put a");
    let b = store.put(request("b"), &immediate(0), 1001).expect("put b");
    store.bump(&a.id).expect("bump head");
    assert_eq!(ids(&store), vec![a.id, b.id]);
}

#[rstest]
fn bust_moves_entry_to_tail() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let a = store.put(request("a"), &immediate(0), 1000).expect("put a");
    let b = store.put(request("b"), &immediate(0), 1001).expect("put b");
    let c = store.put(request("c"), &immediate(0), 1002).expect("put c");
    store.bust(&a.id).expect("bust a");
    assert_eq!(ids(&store), vec![b.id, c.id, a.id]);
}

#[rstest]
fn del_removes_entry() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let a = store.put(request("a"), &immediate(0), 1000).expect("put a");
    let b = store.put(request("b"), &immediate(0), 1001).expect("put b");
    store.del(&a.id).expect("del a");
    assert_eq!(ids(&store), vec![b.id]);
}

#[rstest]
#[case::bump(|store: &QueueStore| store.bump("deadbeef"))]
#[case::bust(|store: &QueueStore| store.bust("deadbeef"))]
#[case::del(|store: &QueueStore| store.del("deadbeef"))]
fn unknown_ids_are_rejected(#[case] op: fn(&QueueStore) -> super::Result<()>) {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    store.put(request("a"), &immediate(0), 1000).expect("put a");
    let err = op(&store).expect_err("unknown id must fail");
    assert!(matches!(err, StoreError::UnknownId(id) if id == "deadbeef"));
}

#[rstest]
fn entries_survive_reopen() {
    let dir = TempDir::new().expect("tempdir");
    let first = open_store(&dir);
    let a = first.put(request("a"), &immediate(0), 1000).expect("put a");
    drop(first);
    let reopened = open_store(&dir);
    assert_eq!(ids(&reopened), vec![a.id]);
}

#[rstest]
fn identical_put_within_the_same_second_is_idempotent() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let first = store
        .put(request("dup"), &immediate(240), 1000)
        .expect("first put");
    let second = store
        .put(request("dup"), &immediate(240), 1000)
        .expect("second put");
    assert_eq!(first, second, "repeat put must return the existing entry");
    assert_eq!(ids(&store).len(), 1);
}

#[rstest]
fn malformed_last_post_marker_is_an_error() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    fs::write(dir.path().join("last_post"), "not a timestamp").expect("write marker");
    assert!(matches!(store.last_post(), Err(StoreError::LastPost(_))));
}

#[rstest]
fn id_collision_with_a_different_request_uses_a_salted_id() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let original_request = request("original");
    let colliding_id = entry_id(&original_request, 1000);
    store
        .write_entry(&StoredEntry {
            id: colliding_id.clone(),
            order: 0,
            flutter_seconds: 0,
            enqueued_at: 1000,
            not_before: 0,
            claim_token: None,
            request: request("different"),
        })
        .expect("seed colliding entry");

    let inserted = store
        .put(original_request, &immediate(0), 1000)
        .expect("put after collision");
    let repeated = store
        .put(request("original"), &immediate(0), 1000)
        .expect("repeat put after collision");

    assert_ne!(inserted.id, colliding_id);
    assert_eq!(repeated, inserted);
    assert_eq!(ids(&store).len(), 2);
}

#[rstest]
fn unsafe_stored_identifiers_are_skipped_without_path_traversal() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let unsafe_entry = StoredEntry {
        id: "../../outside".into(),
        order: 0,
        flutter_seconds: 0,
        enqueued_at: 1000,
        not_before: 0,
        claim_token: None,
        request: request("unsafe"),
    };
    let bytes = serde_json::to_vec(&unsafe_entry).expect("serialize unsafe entry");
    fs::write(dir.path().join("entries/deadbeef.json"), bytes).expect("write unsafe entry");

    assert!(store.entries().expect("list entries").is_empty());
    assert!(matches!(
        store.del("../../outside"),
        Err(StoreError::InvalidId(id)) if id == "../../outside"
    ));
}

mod claims;
mod resource_limits;
mod scheduling;
