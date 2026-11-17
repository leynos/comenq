//! Query tests: the projections read the repository and never write to it.
//!
//! A query that quietly mutated would let a caller's read-only path rotate a
//! claim or move the posting anchor, so each test snapshots the in-memory
//! repository's write count and stored entries around the call.

use super::support::{Fixture, comment, seeded_entry, unwrap_ok};
use crate::{ClaimToken, PutComment, QueuePolicy, QueueQuery};
use rstest::rstest;

/// A fixture holding one deferred entry, so projections have work to do.
fn fixture_with_one_entry() -> Fixture {
    let fixture = Fixture::with_policy(1_000, 37, QueuePolicy::new(600, 0));
    unwrap_ok(fixture.service().put(PutComment {
        comment: comment(),
        immediate: false,
    }));
    fixture
}

#[test]
fn schedule_and_next_due_leave_the_repository_untouched() {
    let fixture = fixture_with_one_entry();
    let writes = fixture.repository.write_count();
    let entries = fixture.repository.stored_entries();
    let service = fixture.service();

    unwrap_ok(service.schedule());
    unwrap_ok(service.next_due());

    assert_eq!(fixture.repository.write_count(), writes);
    assert_eq!(fixture.repository.stored_entries(), entries);
}

#[rstest]
#[case::list(QueueQuery::List)]
#[case::next_due(QueueQuery::NextDue)]
fn query_leaves_the_repository_untouched(#[case] query: QueueQuery) {
    let fixture = fixture_with_one_entry();
    let writes = fixture.repository.write_count();
    let entries = fixture.repository.stored_entries();

    unwrap_ok(fixture.service().query(query));

    assert_eq!(fixture.repository.write_count(), writes);
    assert_eq!(fixture.repository.stored_entries(), entries);
}

/// Claim exclusion is a read-time decision, so a claimed queue suppresses the
/// head without touching the claim it found.
#[test]
fn next_due_over_a_claimed_queue_is_read_only() {
    let fixture = Fixture::with_policy(1_000, 0, QueuePolicy::new(0, 0));
    let mut entry = seeded_entry("aaaaaaaa", 0, 1_000, 0, 0);
    entry.claim_token = Some(ClaimToken::new("held"));
    fixture.repository.seed(entry);
    let writes = fixture.repository.write_count();

    let head = unwrap_ok(fixture.service().next_due());

    assert_eq!(head, None);
    assert_eq!(fixture.repository.write_count(), writes);
}
