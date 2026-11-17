//! Property tests for the scheduling, ordering, identifier and claim rules.
//!
//! The example-based tests pin the readable contract; these properties cover
//! the same rules over generated input, so a change that breaks only an
//! untested corner of the range still fails. `proptest` is a dev-dependency,
//! leaving `thiserror` the crate's only runtime dependency.

use super::support::{FixedClock, FixedFlutter, Fixture, MemoryRepository};
use crate::policy::entry_order;
use crate::{
    Comment, EntryId, EntryLifetime, PutComment, QueuePolicy, QueueService, QueuedEntry,
    derive_entry_id, project_schedule,
};
use proptest::prelude::*;
use std::cmp::Ordering;

/// An eight-digit hexadecimal identifier, the only form the domain accepts.
fn entry_id_strategy() -> impl Strategy<Value = EntryId> {
    "[0-9a-f]{8}".prop_filter_map("a parseable identifier", |text| EntryId::parse(&text).ok())
}

/// An arbitrary queued entry with every schedule field drawn from its range.
fn entry_strategy() -> impl Strategy<Value = QueuedEntry> {
    (
        entry_id_strategy(),
        any::<i64>(),
        any::<u64>(),
        any::<u64>(),
        any::<u64>(),
    )
        .prop_map(
            |(id, order, enqueued_at, not_before, flutter_seconds)| QueuedEntry {
                lifetime: EntryLifetime::new(format!("lifetime-{}", id.as_str())),
                comment: Comment::new("octocat", "hello-world", 7, id.as_str()),
                id,
                order,
                flutter_seconds,
                enqueued_at,
                not_before,
                claim_token: None,
            },
        )
}

/// The posting fields a derived identifier is hashed from.
fn comment_strategy() -> impl Strategy<Value = (String, String, u64, String, u64)> {
    (".*", ".*", any::<u64>(), ".*", any::<u64>())
}

proptest! {
    /// Projected waits never decrease, and no entry projects before its floor.
    #[test]
    fn projection_is_monotone_and_respects_each_floor(
        entries in prop::collection::vec(entry_strategy(), 0..12),
        previous_post in prop::option::of(any::<u64>()),
        cooldown in any::<u64>(),
        now in any::<u64>(),
    ) {
        let schedule = project_schedule(entries, previous_post, cooldown, now);

        for pair in schedule.windows(2) {
            prop_assert!(pair[0].eta_seconds <= pair[1].eta_seconds);
        }
        for scheduled in &schedule {
            prop_assert!(scheduled.eta_seconds >= scheduled.entry.not_before.saturating_sub(now));
        }
    }

    /// The ordering relation is total and matches its documented tuple.
    #[test]
    fn entry_order_is_a_total_order(
        first in entry_strategy(),
        second in entry_strategy(),
        third in entry_strategy(),
    ) {
        prop_assert_eq!(
            entry_order(&first, &second),
            entry_order(&second, &first).reverse()
        );

        let ascending = |left: Ordering, right: Ordering| {
            left != Ordering::Greater && right != Ordering::Greater
        };
        if ascending(entry_order(&first, &second), entry_order(&second, &third)) {
            prop_assert_ne!(entry_order(&first, &third), Ordering::Greater);
        }

        let key = |entry: &QueuedEntry| {
            (entry.order, entry.enqueued_at, entry.id.as_str().to_owned())
        };
        prop_assert_eq!(entry_order(&first, &second), key(&first).cmp(&key(&second)));
    }

    /// Derived identifiers are stable and always eight lowercase hex digits.
    #[test]
    fn derived_identifiers_are_stable_lowercase_hex(
        (owner, repo, pr_number, body, enqueued_at) in comment_strategy(),
    ) {
        let comment = Comment::new(owner, repo, pr_number, body);
        let derived = derive_entry_id(&comment, enqueued_at);

        prop_assert_eq!(derived.clone(), derive_entry_id(&comment, enqueued_at));
        let text = derived.as_str();
        prop_assert_eq!(text.len(), 8);
        prop_assert!(
            text.chars()
                .all(|digit| digit.is_ascii_digit() || ('a'..='f').contains(&digit))
        );
    }
}

/// One command in a generated queue-operation sequence.
#[derive(Debug, Clone, Copy)]
enum Op {
    /// Enqueue a fresh comment.
    Put { immediate: bool },
    /// Claim the due head, if one is claimable.
    Claim,
    /// Complete the most recently claimed entry.
    Complete,
    /// Release with a token no repository ever minted.
    ReleaseStale,
    /// Move a known entry to the head.
    Bump,
    /// Move a known entry to the tail.
    Bust,
    /// Remove a known entry.
    Del,
}

/// Any command, with both enqueue modes represented.
fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        2 => any::<bool>().prop_map(|immediate| Op::Put { immediate }),
        2 => Just(Op::Claim),
        1 => Just(Op::Complete),
        2 => Just(Op::ReleaseStale),
        1 => Just(Op::Bump),
        1 => Just(Op::Bust),
        1 => Just(Op::Del),
    ]
}

/// The service composed over the fixture's in-memory ports.
type TestService<'a> = QueueService<&'a MemoryRepository, &'a FixedClock, &'a FixedFlutter>;

/// Apply one command, ignoring the errors a generated sequence may provoke.
fn apply_op(op: Op, service: &TestService<'_>, claimed: &mut Vec<QueuedEntry>, puts: &mut u64) {
    match op {
        Op::Put { immediate } => {
            let body = format!("comment-{puts}");
            *puts = puts.saturating_add(1);
            let comment = Comment::new("octocat", "hello-world", 7, body);
            let _ = service.put(PutComment { comment, immediate });
        }
        Op::Claim => {
            if let Ok(Some(scheduled)) = service.claim_next_due() {
                claimed.push(scheduled.entry);
            }
        }
        Op::Complete => {
            if let Some(entry) = claimed.pop() {
                let _ = service.complete(entry, "token-hash".to_owned(), None);
            }
        }
        Op::ReleaseStale => {
            if let Some(entry) = claimed.last() {
                let _ = service.release_claim(entry.id.as_str(), "unminted-token");
            }
        }
        Op::Bump => {
            let _ = service.bump(&known_id(claimed));
        }
        Op::Bust => {
            let _ = service.bust(&known_id(claimed));
        }
        Op::Del => {
            let _ = service.del(&known_id(claimed));
        }
    }
}

/// An identifier the sequence has already seen, or an unknown placeholder.
fn known_id(claimed: &[QueuedEntry]) -> String {
    claimed.first().map_or_else(
        || "aaaaaaaa".to_owned(),
        |entry| entry.id.as_str().to_owned(),
    )
}

proptest! {
    /// A generated sequence never leaves two claims, and a stale release keeps
    /// the claim it must not clear.
    #[test]
    fn claim_exclusion_holds_across_operation_sequences(
        ops in prop::collection::vec(op_strategy(), 1..12),
    ) {
        let fixture = Fixture::with_policy(1_000, 0, QueuePolicy::new(0, 0));
        let service = fixture.service();
        let mut claimed = Vec::new();
        let mut puts = 0_u64;

        for op in ops {
            let stale_target = match op {
                Op::ReleaseStale => claimed.first().cloned(),
                _ => None,
            };
            apply_op(op, &service, &mut claimed, &mut puts);

            let entries = fixture.repository.stored_entries();
            let claims = entries.iter().filter(|entry| entry.is_claimed()).count();
            prop_assert!(claims <= 1);

            if let Some(target) = stale_target {
                let stored = entries.iter().find(|entry| entry.id == target.id);
                if let Some(entry) = stored {
                    prop_assert_eq!(&entry.claim_token, &target.claim_token);
                }
            }
        }
    }
}
