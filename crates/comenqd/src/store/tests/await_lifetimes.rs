//! Reuse and deletion invariants for deterministic await identifiers.

use super::{immediate, open_store, request};
use crate::store::{AwaitState, HistoryRecord};
use proptest::prelude::*;
use rstest::rstest;
use std::fs;
use std::io::Write as _;
use tempfile::TempDir;

const URL: &str = "https://github.com/o/r/issues/1#issuecomment-1";

#[rstest]
fn deleted_reused_identifier_does_not_resurface_an_older_url() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let request = request("reuse");
    let original = store
        .put(request.clone(), &immediate(0), 1000)
        .expect("put original");
    store
        .complete_claim_with_token(&original, "", 1100, Some(URL.into()))
        .expect("complete original");

    let replacement = store
        .put(request, &immediate(0), 1000)
        .expect("reuse deterministic identifier");
    assert_eq!(replacement.id, original.id);
    assert_ne!(replacement.incarnation, original.incarnation);
    store.del(&replacement.id).expect("delete replacement");

    assert_eq!(
        store
            .resolve_await(&replacement.id, None, 600, 1200)
            .expect("lookup deleted reuse"),
        AwaitState::Unknown
    );
    assert_eq!(
        open_store(&dir)
            .resolve_await(&replacement.id, None, 600, 1200)
            .expect("lookup after reopen"),
        AwaitState::Unknown
    );
}

#[rstest]
fn startup_clears_a_deletion_marker_when_the_entry_was_not_removed() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let entry = store
        .put(request("interrupted delete"), &immediate(0), 1000)
        .expect("put");
    store
        .write_await_deletion(&entry)
        .expect("persist deletion intent");

    let reopened = open_store(&dir);
    assert!(matches!(
        reopened
            .resolve_await(&entry.id, None, 600, 1000)
            .expect("lookup recovered entry"),
        AwaitState::Pending { .. }
    ));
    assert!(
        !reopened
            .await_lifetime_was_deleted(&entry.id)
            .expect("check deletion marker")
    );
}

#[rstest]
fn later_success_clears_a_reuse_deletion_marker() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let request = request("reuse success");
    let original = store
        .put(request.clone(), &immediate(0), 1000)
        .expect("put original");
    store
        .complete_claim_with_token(&original, "", 1100, Some(URL.into()))
        .expect("complete original");

    let deleted = store
        .put(request.clone(), &immediate(0), 1000)
        .expect("put second incarnation");
    store.del(&deleted.id).expect("delete second incarnation");
    assert_eq!(
        store
            .resolve_await(&deleted.id, None, 600, 1200)
            .expect("lookup deleted incarnation"),
        AwaitState::Unknown
    );

    let posted = store
        .put(request, &immediate(0), 1000)
        .expect("put successful replacement");
    let new_url = "https://github.com/o/r/issues/1#issuecomment-2";
    store
        .complete_claim_with_token(&posted, "", 1300, Some(new_url.into()))
        .expect("complete replacement");
    assert_eq!(
        store
            .resolve_await(&posted.id, None, 600, 1400)
            .expect("lookup replacement"),
        AwaitState::Posted {
            comment_url: Some(new_url.into()),
            posted_at: 1300
        }
    );
}

#[rstest]
fn a_replacement_cannot_redirect_an_existing_wait() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let entry = store
        .put(request("reuse"), &immediate(0), 1000)
        .expect("put");
    let AwaitState::Pending { cursor, .. } = store
        .resolve_await(&entry.id, None, 600, 1000)
        .expect("lookup")
    else {
        panic!("pending")
    };
    store.del(&entry.id).expect("delete");
    let replacement = store
        .put(request("reuse"), &immediate(0), 1000)
        .expect("replace");
    assert_ne!(entry.incarnation, replacement.incarnation);
    assert_eq!(
        store
            .resolve_await(&entry.id, Some(&cursor), 600, 1000)
            .expect("lookup"),
        AwaitState::Unknown
    );
    store
        .complete_claim_with_token(&replacement, "", 1100, Some(URL.into()))
        .expect("complete replacement");
    assert_eq!(
        store
            .resolve_await(&entry.id, Some(&cursor), 600, 1100)
            .expect("lookup"),
        AwaitState::Unknown
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn generated_queue_lifetimes_keep_each_await_bound_to_its_entry(
        operations in prop::collection::vec(any::<u8>(), 0..48)
    ) {
        let dir = TempDir::new().expect("tempdir");
        let store = open_store(&dir);
        let request = request("generated lifetime");
        let initial = store
            .put(request.clone(), &immediate(0), 1000)
            .expect("initial put");
        let id = initial.id.clone();
        let first_incarnation = initial.incarnation.clone();
        let AwaitState::Pending { cursor, .. } = store
            .resolve_await(&id, None, 600, 1000)
            .expect("select initial lifetime")
        else {
            panic!("initial entry must be pending");
        };

        let mut current = Some(initial);
        let mut first_success = None;
        let mut latest_success = None;
        let mut latest_lifetime_deleted = false;

        for (step, operation) in operations.into_iter().enumerate() {
            let now = 1200 + step as u64;
            match operation % 6 {
                0 => {
                    if let Some(entry) = current.as_ref() {
                        store
                            .append_history(&HistoryRecord::failure(
                                entry,
                                "",
                                now,
                                "generated retry",
                            ))
                            .expect("append generated failure");
                    }
                }
                1 => {
                    if let Some(entry) = current.take() {
                        let url = format!(
                            "https://github.com/o/r/issues/1#issuecomment-{step}"
                        );
                        store
                            .complete_claim_with_token(&entry, "", now, Some(url.clone()))
                            .expect("complete current lifetime");
                        let success = (url, now);
                        if entry.incarnation == first_incarnation {
                            first_success = Some(success.clone());
                        }
                        latest_success = Some(success);
                        latest_lifetime_deleted = false;
                    }
                }
                2 => {
                    if let Some(entry) = current.take() {
                        store.del(&id).expect("delete current lifetime");
                        latest_lifetime_deleted = true;
                        prop_assert_eq!(entry.id, id.clone());
                    }
                }
                3 => {
                    if current.is_none() {
                        current = Some(
                            store
                                .put(request.clone(), &immediate(0), 1000)
                                .expect("put replacement lifetime"),
                        );
                        latest_lifetime_deleted = false;
                    }
                }
                4 => {
                    let path = dir.path().join("history.jsonl");
                    let mut history = fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                        .expect("open history tail");
                    history.write_all(b"corrupt generated tail").expect("write tail");
                }
                _ => {}
            }

            let selected = store
                .resolve_await(&id, Some(&cursor), 600, now)
                .expect("resolve selected lifetime");
            if current
                .as_ref()
                .is_some_and(|entry| entry.incarnation == first_incarnation)
            {
                prop_assert!(
                    matches!(selected, AwaitState::Pending { .. }),
                    "selected lifetime remains pending"
                );
            } else if let Some((url, posted_at)) = first_success.as_ref() {
                prop_assert_eq!(
                    selected,
                    AwaitState::Posted {
                        comment_url: Some(url.clone()),
                        posted_at: *posted_at,
                    }
                );
            } else {
                prop_assert_eq!(selected, AwaitState::Unknown);
            }

            let latest = store
                .resolve_await(&id, None, 600, now)
                .expect("resolve latest identifier lifetime");
            if current.is_some() {
                prop_assert!(
                    matches!(latest, AwaitState::Pending { .. }),
                    "latest identifier remains pending"
                );
            } else if latest_lifetime_deleted {
                prop_assert_eq!(latest, AwaitState::Unknown);
            } else if let Some((url, posted_at)) = latest_success.as_ref() {
                prop_assert_eq!(
                    latest,
                    AwaitState::Posted {
                        comment_url: Some(url.clone()),
                        posted_at: *posted_at,
                    }
                );
            } else {
                prop_assert_eq!(latest, AwaitState::Unknown);
            }
        }
    }
}
