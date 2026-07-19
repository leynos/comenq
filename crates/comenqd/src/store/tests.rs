//! Tests for the reorderable persistent queue store.

use super::{PutOptions, QueueStore, StoreError, entry_id};
use comenq_lib::CommentRequest;
use rstest::rstest;
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

/// Options for an immediate put with the given flutter ceiling.
fn immediate(flutter_max: u64) -> PutOptions {
    PutOptions {
        cooldown: 600,
        flutter_max,
        immediate: true,
    }
}

/// Options for a default (deferred) put with no flutter.
fn deferred() -> PutOptions {
    PutOptions {
        cooldown: 600,
        flutter_max: 0,
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
fn put_samples_flutter_within_bounds() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    for i in 0..50 {
        let entry = store
            .put(request(&format!("body {i}")), &immediate(240), 1000 + i)
            .expect("put entry");
        assert!(entry.flutter_seconds <= 240);
    }
    let zero = store
        .put(request("plain"), &immediate(0), 2000)
        .expect("put plain");
    assert_eq!(zero.flutter_seconds, 0);
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
fn schedule_starts_immediately_when_never_posted() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    store.put(request("a"), &immediate(0), 1000).expect("put a");
    store.put(request("b"), &immediate(0), 1001).expect("put b");
    let schedule = store.schedule(600, 2000).expect("schedule");
    let etas: Vec<u64> = schedule.iter().map(|(_, eta)| *eta).collect();
    // Head posts immediately; the next entry follows one full cooldown
    // (flutter is zero here).
    assert_eq!(etas, vec![0, 600]);
}

#[rstest]
fn schedule_respects_last_post_and_flutter() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let a = store.put(request("a"), &immediate(0), 1000).expect("put a");
    store.put(request("b"), &immediate(0), 1001).expect("put b");
    // Pretend `a` was posted at t=1000 by another entry's completion.
    store.complete(&a.id, 1000).expect("complete a");
    let schedule = store.schedule(600, 1100).expect("schedule");
    assert_eq!(schedule.len(), 1);
    let (_, eta) = &schedule[0];
    // Due at 1000 + 600 = 1600; now is 1100, so 500 seconds remain.
    assert_eq!(*eta, 500);
}

#[rstest]
fn complete_records_last_post_and_removes_entry() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let a = store.put(request("a"), &immediate(0), 1000).expect("put a");
    assert_eq!(store.last_post(), None);
    store.complete(&a.id, 4242).expect("complete a");
    assert_eq!(store.last_post(), Some(4242));
    assert!(ids(&store).is_empty());
}

#[rstest]
fn next_due_returns_the_head() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let a = store.put(request("a"), &immediate(0), 1000).expect("put a");
    store.put(request("b"), &immediate(0), 1001).expect("put b");
    let (head, _) = store
        .next_due(600, 2000)
        .expect("next_due")
        .expect("head entry");
    assert_eq!(head.id, a.id);
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
fn deferred_put_waits_a_full_cooldown_when_idle() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let entry = store
        .put(request("a"), &deferred(), 1000)
        .expect("put deferred");
    assert_eq!(entry.not_before, 1600);
    let schedule = store.schedule(600, 1000).expect("schedule");
    let etas: Vec<u64> = schedule.iter().map(|(_, eta)| *eta).collect();
    assert_eq!(etas, vec![600], "idle queue must still wait one cooldown");
}

#[rstest]
fn immediate_put_bypasses_the_enqueue_floor() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let entry = store
        .put(request("a"), &immediate(0), 1000)
        .expect("put immediate");
    assert_eq!(entry.not_before, 0);
    let schedule = store.schedule(600, 1000).expect("schedule");
    let etas: Vec<u64> = schedule.iter().map(|(_, eta)| *eta).collect();
    assert_eq!(etas, vec![0]);
}

#[rstest]
fn deferred_floor_never_shortens_the_chain_schedule() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let head = store
        .put(request("head"), &immediate(0), 1000)
        .expect("put head");
    store
        .put(request("tail"), &deferred(), 1001)
        .expect("put tail");
    // Head posted at t=1000; the tail's chain due time (1000 + 600) is
    // later than its own floor (1001 + 600 = 1601)... choose the maximum.
    store.complete(&head.id, 1000).expect("complete head");
    let schedule = store.schedule(600, 1002).expect("schedule");
    assert_eq!(schedule.len(), 1);
    let (_, eta) = &schedule[0];
    // max(chain 1600, floor 1601) = 1601; now is 1002 → 599 seconds.
    assert_eq!(*eta, 599);
}

#[rstest]
fn deferred_floor_includes_the_entry_flutter() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let options = PutOptions {
        cooldown: 600,
        flutter_max: 240,
        immediate: false,
    };
    let entry = store.put(request("a"), &options, 1000).expect("put");
    assert_eq!(
        entry.not_before,
        1600 + entry.flutter_seconds,
        "floor must be enqueue + cooldown + sampled flutter"
    );
}
