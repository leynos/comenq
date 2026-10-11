//! Claim, completion, and recovery tests for the persistent queue.

use super::{ids, immediate, open_store, request};
use rstest::rstest;
use std::fs;
use tempfile::TempDir;

#[rstest]
fn complete_records_last_post_and_removes_entry() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let a = store.put(request("a"), &immediate(0), 1000).expect("put a");
    assert_eq!(store.last_post().expect("read last post"), None);
    store.complete(&a.id, 4242).expect("complete a");
    assert_eq!(store.last_post().expect("read last post"), Some(4242));
    assert!(ids(&store).is_empty());
}

#[rstest]
fn reopen_reconciles_a_persisted_completion_before_scheduling() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let entry = store.put(request("a"), &immediate(0), 1000).expect("put a");
    fs::write(
        dir.path().join("completion"),
        format!(r#"{{"id":"{}","posted_at":4242}}"#, entry.id),
    )
    .expect("persist completion record");
    drop(store);

    let reopened = open_store(&dir);
    assert!(ids(&reopened).is_empty());
    assert_eq!(reopened.last_post().expect("read last post"), Some(4242));
    assert!(!dir.path().join("completion").exists());
}

#[rstest]
fn entries_does_not_apply_completion_recovery() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let entry = store.put(request("a"), &immediate(0), 1000).expect("put a");
    fs::write(
        dir.path().join("completion"),
        format!(r#"{{"id":"{}","posted_at":4242}}"#, entry.id),
    )
    .expect("persist completion record");

    assert_eq!(ids(&store), vec![entry.id]);
    assert!(dir.path().join("completion").exists());
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
fn claim_is_released_after_a_failed_post() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let entry = store
        .put(request("claim"), &immediate(0), 1000)
        .expect("put");
    let (claimed, eta) = store
        .claim_next_due(600, 1000)
        .expect("claim")
        .expect("due entry");
    assert_eq!(eta, 0);
    let token = claimed.claim_token.clone().expect("claim token");
    assert!(store.next_due(600, 1000).expect("next due").is_none());
    store
        .release_claim(&entry.id, &token, 1000)
        .expect("release claim");
    assert!(store.next_due(600, 1000).expect("next due").is_some());
    assert_eq!(store.last_attempt().expect("last attempt"), Some(1000));
}

#[rstest]
fn failed_post_retry_floor_survives_store_reopen() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let entry = store
        .put(request("retry after restart"), &immediate(0), 1000)
        .expect("put entry");
    let (claimed, _) = store
        .claim_next_due(600, 1000)
        .expect("claim entry")
        .expect("entry is due");
    let token = claimed.claim_token.expect("claim token");
    store
        .release_claim(&entry.id, &token, 1000)
        .expect("release failed claim");
    drop(store);

    let reopened = open_store(&dir);
    let schedule = reopened.schedule(600, 1030).expect("schedule after reopen");
    assert_eq!(schedule.len(), 1);
    assert_eq!(schedule[0].0.id, entry.id);
    assert_eq!(schedule[0].1, 570);

    let inserted = reopened
        .put_with_eta(request("new entry during cooldown"), &immediate(0), 1030)
        .expect("put during cooldown");
    assert_eq!(inserted.1, 1170);
}

#[rstest]
fn completion_is_idempotent_after_delete_and_records_the_post_without_removing_a_replacement() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let entry = store
        .put(request("claim"), &immediate(0), 1000)
        .expect("put");
    let (claimed, _) = store
        .claim_next_due(600, 1000)
        .expect("claim")
        .expect("due entry");
    let token = claimed.claim_token.clone().expect("claim token");
    store.del(&entry.id).expect("delete in-flight entry");
    let replacement = store
        .put(request("claim"), &immediate(0), 1000)
        .expect("replace");
    store
        .complete_claim(&entry.id, Some(&token), 4242)
        .expect("stale completion is harmless");
    assert_eq!(ids(&store), vec![replacement.id]);
    assert_eq!(store.last_post().expect("last post"), Some(4242));
}

#[rstest]
fn bump_and_bust_preserve_an_in_flight_claim() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let entry = store
        .put(request("claim"), &immediate(0), 1000)
        .expect("put");
    let other = store
        .put(request("other"), &immediate(0), 1001)
        .expect("put");
    let (claimed, _) = store
        .claim_next_due(600, 1000)
        .expect("claim")
        .expect("due entry");
    let token = claimed.claim_token.clone().expect("claim token");

    store.bust(&entry.id).expect("bust in-flight entry");
    store.bump(&entry.id).expect("bump in-flight entry");
    store
        .complete_claim(&entry.id, Some(&token), 4242)
        .expect("complete in-flight entry");
    assert_eq!(ids(&store), vec![other.id]);
}

/// Verify reopening a store reclaims an entry left claimed by a prior worker.
#[rstest]
fn reopening_store_reclaims_a_claimed_entry() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let entry = store
        .put(request("interrupted"), &immediate(0), 1000)
        .expect("put entry");
    let (claimed, wait_seconds) = store
        .claim_next_due(600, 1000)
        .expect("claim entry")
        .expect("entry is due");
    assert_eq!(wait_seconds, 0);
    assert!(claimed.claim_token.is_some());
    drop(store);

    let reopened = open_store(&dir);
    let (reclaimed, wait_seconds) = reopened
        .claim_next_due(600, 1000)
        .expect("claim recovered entry")
        .expect("entry is due after reopen");
    assert_eq!(reclaimed.id, entry.id);
    assert_eq!(wait_seconds, 0);
    assert!(reclaimed.claim_token.is_some());
}
