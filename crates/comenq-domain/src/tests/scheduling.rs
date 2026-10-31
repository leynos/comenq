//! Scheduling, ordering, identifier and admission behaviour.

use super::support::{Fixture, TestError, comment, seeded_entry, unwrap_err, unwrap_ok};
use crate::{
    ClaimToken, PutComment, QueueError, QueuePolicy, activity_anchor, derive_entry_id,
    project_schedule,
};

#[test]
fn put_projects_cooldown_plus_flutter() {
    let fixture = Fixture::new(1_000, 37);
    let scheduled = unwrap_ok(fixture.service().put(PutComment {
        comment: comment(),
        immediate: false,
    }));

    assert_eq!(scheduled.eta_seconds, 637);
    assert_eq!(scheduled.entry.not_before, 1_637);
    assert_eq!(scheduled.entry.enqueued_at, 1_000);
    assert_eq!(scheduled.entry.flutter_seconds, 37);
    assert_eq!(fixture.sampler.last_maximum(), Some(240));
    assert_eq!(fixture.clock.reads(), 1);
}

#[test]
fn immediate_put_bypasses_only_the_enqueue_floor() {
    let fixture = Fixture::new(2_000, 0);
    fixture.repository.set_last_post(Some(1_000));
    let scheduled = unwrap_ok(fixture.service().put(PutComment {
        comment: comment(),
        immediate: true,
    }));

    assert_eq!(scheduled.entry.not_before, 0);
    assert_eq!(scheduled.eta_seconds, 0);
}

#[test]
fn deferred_put_respects_the_previous_activity_anchor() {
    let fixture = Fixture::new(4_000, 0);
    fixture.repository.set_last_post(Some(1_000));
    fixture.repository.set_last_attempt(Some(4_000));
    let scheduled = unwrap_ok(fixture.service().put(PutComment {
        comment: comment(),
        immediate: false,
    }));

    assert_eq!(scheduled.eta_seconds, 600);
    assert_eq!(scheduled.entry.not_before, 4_600);
}

#[test]
fn activity_anchor_chooses_the_later_marker() {
    assert_eq!(activity_anchor(Some(100), Some(130)), Some(130));
    assert_eq!(activity_anchor(Some(100), None), Some(100));
    assert_eq!(activity_anchor(None, Some(130)), Some(130));
    assert_eq!(activity_anchor(None, None), None);
}

#[test]
fn schedule_orders_by_position_then_enqueue_time_then_identifier() {
    let fixture = Fixture::with_policy(1_000, 0, QueuePolicy::new(600, 0));
    fixture
        .repository
        .seed(seeded_entry("cccccccc", 2, 1_000, 0, 0));
    fixture
        .repository
        .seed(seeded_entry("bbbbbbbb", 1, 1_000, 0, 0));
    fixture
        .repository
        .seed(seeded_entry("aaaaaaaa", 1, 1_000, 0, 0));

    let schedule = unwrap_ok(fixture.service().schedule());
    let ids: Vec<&str> = schedule
        .iter()
        .map(|scheduled| scheduled.entry.id.as_str())
        .collect();
    let etas: Vec<u64> = schedule
        .iter()
        .map(|scheduled| scheduled.eta_seconds)
        .collect();

    assert_eq!(ids, ["aaaaaaaa", "bbbbbbbb", "cccccccc"]);
    assert_eq!(etas, [0, 600, 1_200]);
}

#[test]
fn queue_wide_claim_exclusion_suppresses_the_head() {
    let fixture = Fixture::with_policy(1_000, 0, QueuePolicy::new(0, 0));
    fixture
        .repository
        .seed(seeded_entry("aaaaaaaa", 0, 1_000, 0, 0));
    fixture
        .repository
        .seed(seeded_entry("bbbbbbbb", 1, 1_000, 0, 0));
    let service = fixture.service();

    let claimed = unwrap_ok(service.claim_next_due());
    assert_eq!(
        claimed.and_then(|scheduled| scheduled.entry.claim_token),
        Some(ClaimToken::new("token-1"))
    );
    assert_eq!(unwrap_ok(service.next_due()), None);
}

#[test]
fn next_due_returns_an_unclaimed_head_while_waiting() {
    let fixture = Fixture::new(1_000, 37);
    unwrap_ok(fixture.service().put(PutComment {
        comment: comment(),
        immediate: false,
    }));

    let head = unwrap_ok(fixture.service().next_due());
    let head = head.unwrap_or_else(|| panic!("expected a head entry"));
    assert_eq!(head.eta_seconds, 637);
    assert_eq!(head.entry.claim_token, None);
}

#[test]
fn bump_moves_the_entry_and_preserves_floor_and_claim() {
    let fixture = Fixture::with_policy(1_000, 0, QueuePolicy::new(0, 0));
    let mut tail = seeded_entry("bbbbbbbb", 1, 1_000, 5_000, 0);
    tail.claim_token = Some(ClaimToken::new("held"));
    fixture
        .repository
        .seed(seeded_entry("aaaaaaaa", 0, 1_000, 0, 0));
    fixture.repository.seed(tail);
    let service = fixture.service();

    unwrap_ok(service.bump("bbbbbbbb"));
    let schedule = unwrap_ok(service.schedule());

    let head = schedule
        .first()
        .unwrap_or_else(|| panic!("expected a head entry"));
    assert_eq!(head.entry.id.as_str(), "bbbbbbbb");
    assert_eq!(head.entry.not_before, 5_000);
    assert_eq!(head.entry.claim_token, Some(ClaimToken::new("held")));
}

#[test]
fn bust_moves_the_entry_to_the_tail() {
    let fixture = Fixture::with_policy(1_000, 0, QueuePolicy::new(0, 0));
    fixture
        .repository
        .seed(seeded_entry("aaaaaaaa", 0, 1_000, 0, 0));
    fixture
        .repository
        .seed(seeded_entry("bbbbbbbb", 1, 1_000, 0, 0));
    let service = fixture.service();

    unwrap_ok(service.bust("aaaaaaaa"));
    let schedule = unwrap_ok(service.schedule());

    let last = schedule
        .last()
        .unwrap_or_else(|| panic!("expected a tail entry"));
    assert_eq!(last.entry.id.as_str(), "aaaaaaaa");
}

#[test]
fn repeated_put_returns_the_existing_entry_unchanged() {
    let fixture = Fixture::new(1_000, 0);
    let first = unwrap_ok(fixture.service().put(PutComment {
        comment: comment(),
        immediate: false,
    }));
    let second = unwrap_ok(fixture.service().put(PutComment {
        comment: comment(),
        immediate: false,
    }));

    assert_eq!(second.entry, first.entry);
    assert_eq!(second.eta_seconds, first.eta_seconds);
    assert_eq!(fixture.repository.stored_entries().len(), 1);
}

#[test]
fn colliding_identifier_is_salted_for_a_different_comment() {
    let fixture = Fixture::new(1_000, 0);
    let collision = derive_entry_id(&comment(), 1_000);
    fixture
        .repository
        .seed(seeded_entry(collision.as_str(), 0, 1_000, 0, 0));

    let scheduled = unwrap_ok(fixture.service().put(PutComment {
        comment: comment(),
        immediate: false,
    }));

    assert_ne!(scheduled.entry.id, collision);
    assert_eq!(fixture.repository.stored_entries().len(), 2);
}

#[test]
fn failed_read_prevents_persistence() {
    let fixture = Fixture::new(1_000, 0);
    fixture.repository.fail_on("last_post");

    let error = unwrap_err(fixture.service().put(PutComment {
        comment: comment(),
        immediate: false,
    }));

    assert!(matches!(
        error,
        QueueError::Repository(TestError {
            operation: "last_post"
        })
    ));
    assert!(fixture.repository.stored_entries().is_empty());
}

#[test]
fn projection_saturates_extreme_timestamps() {
    let schedule = project_schedule(
        vec![seeded_entry("aaaaaaaa", 0, u64::MAX, 0, 0)],
        Some(u64::MAX),
        600,
        0,
    );

    let first = schedule
        .first()
        .unwrap_or_else(|| panic!("expected a projected entry"));
    assert_eq!(first.eta_seconds, u64::MAX);
}

#[test]
fn golden_identifier_is_stable() {
    assert_eq!(derive_entry_id(&comment(), 1_000).as_str(), "088fc020");
}
