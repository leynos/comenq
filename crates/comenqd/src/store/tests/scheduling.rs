//! Queue ordering, cooldown, and ETA tests.

use super::super::PutOptions;
use super::{deferred, ids, immediate, open_store, request};
use proptest::prelude::*;
use rstest::rstest;
use tempfile::TempDir;

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

/// Verify bumping a deferred entry preserves its enqueue-time floor.

#[rstest]
fn bumping_a_deferred_entry_preserves_its_enqueue_floor() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    store
        .put(request("predecessor"), &immediate(0), 1000)
        .expect("put predecessor");
    let deferred_entry = store
        .put(request("deferred"), &deferred(), 1000)
        .expect("put deferred entry");

    store.bump(&deferred_entry.id).expect("bump deferred entry");
    let schedule = store.schedule(600, 1000).expect("schedule");

    assert_eq!(schedule[0].0.id, deferred_entry.id);
    assert_eq!(schedule[0].0.not_before, 1600);
    assert_eq!(schedule[0].1, 600);
    assert!(schedule[1].1 >= schedule[0].1);
}

#[rstest]
fn deferred_floor_includes_the_entry_flutter() {
    let dir = TempDir::new().expect("tempdir");
    let store = open_store(&dir);
    let options = PutOptions {
        cooldown: 600,
        flutter_seconds: 240,
        immediate: false,
    };
    let entry = store.put(request("a"), &options, 1000).expect("put");
    assert_eq!(
        entry.not_before,
        1600 + entry.flutter_seconds,
        "floor must be enqueue + cooldown + supplied flutter"
    );
}

proptest! {
    #[test]
    fn arbitrary_queue_operations_preserve_order_and_schedule(
        operations in prop::collection::vec((0_u8..3, 0_usize..16), 1..48),
        cooldown in any::<u64>(),
        flutter_seconds in any::<u64>(),
    ) {
        let dir = TempDir::new().expect("tempdir");
        let store = open_store(&dir);
        let options = PutOptions {
            cooldown,
            flutter_seconds,
            immediate: true,
        };
        let mut expected = Vec::new();

        for index in 0..4 {
            let entry = store
                .put(request(&format!("initial {index}")), &options, 1_000 + index)
                .expect("put initial entry");
            expected.push(entry.id);
        }

        for (step, (operation, selection)) in operations.into_iter().enumerate() {
            if expected.is_empty() {
                let entry = store
                    .put(request(&format!("replacement {step}")), &options, 2_000 + step as u64)
                    .expect("put replacement entry");
                expected.push(entry.id);
            }
            let index = selection % expected.len();
            let id = expected[index].clone();
            match operation {
                0 => {
                    store.bump(&id).expect("bump entry");
                    expected.remove(index);
                    expected.insert(0, id);
                }
                1 => {
                    store.bust(&id).expect("bust entry");
                    expected.remove(index);
                    expected.push(id);
                }
                _ => {
                    store.del(&id).expect("delete entry");
                    expected.remove(index);
                }
            }
            prop_assert_eq!(ids(&store), expected.clone());
            let etas: Vec<u64> = store
                .schedule(cooldown, 3_000)
                .expect("schedule")
                .into_iter()
                .map(|(_, eta)| eta)
                .collect();
            prop_assert!(etas.windows(2).all(|pair| pair[0] <= pair[1]));
        }
    }
}
