//! Claim, release, completion and removal transitions.

use super::support::{Fixture, comment, seeded_entry, unwrap_ok};
use crate::{ClaimToken, PutComment, QueuePolicy, QueueRepository, ReleaseOutcome};
use rstest::{fixture, rstest};

/// A queue holding one entry claimed with the token `"held"`.
#[fixture]
fn claimed() -> Fixture {
    let fixture = Fixture::with_policy(1_000, 0, QueuePolicy::new(0, 0));
    let mut entry = seeded_entry("aaaaaaaa", 0, 1_000, 0, 0);
    entry.claim_token = Some(ClaimToken::new("held"));
    fixture.repository.seed(entry);
    fixture
}

#[test]
fn claim_takes_only_a_due_head() {
    let fixture = Fixture::with_policy(1_000, 0, QueuePolicy::new(0, 0));
    unwrap_ok(fixture.service().put(PutComment {
        comment: comment(),
        immediate: false,
    }));
    let service = fixture.service();

    let first = match unwrap_ok(service.claim_next_due()) {
        Some(scheduled) => scheduled,
        None => panic!("expected a claimable head"),
    };
    assert_eq!(first.entry.claim_token, Some(ClaimToken::new("token-1")));
    assert_eq!(unwrap_ok(service.claim_next_due()), None);
    assert_eq!(fixture.clock.reads(), 3);
}

#[test]
fn claim_returns_a_waiting_head_unclaimed() {
    let fixture = Fixture::new(1_000, 37);
    unwrap_ok(fixture.service().put(PutComment {
        comment: comment(),
        immediate: false,
    }));

    let head = match unwrap_ok(fixture.service().claim_next_due()) {
        Some(scheduled) => scheduled,
        None => panic!("expected a head entry"),
    };
    assert_eq!(head.eta_seconds, 637);
    assert_eq!(head.entry.claim_token, None);
    assert_eq!(stored_claim(&fixture), None);
}

#[test]
fn del_removes_the_entry() {
    let fixture = Fixture::with_policy(1_000, 0, QueuePolicy::new(0, 0));
    fixture
        .repository
        .seed(seeded_entry("aaaaaaaa", 0, 1_000, 0, 0));

    unwrap_ok(fixture.service().del("aaaaaaaa"));

    assert!(fixture.repository.stored_entries().is_empty());
}

#[rstest]
#[case::matching("held", Some(900), ReleaseOutcome::Released, None, Some(1_000))]
#[case::later_marker_kept("held", Some(2_000), ReleaseOutcome::Released, None, Some(2_000))]
#[case::stale_token("other", None, ReleaseOutcome::Stale, Some("held"), None)]
fn release_outcomes(
    claimed: Fixture,
    #[case] token: &str,
    #[case] seeded_attempt: Option<u64>,
    #[case] expected: ReleaseOutcome,
    #[case] expected_claim: Option<&str>,
    #[case] expected_attempt: Option<u64>,
) {
    claimed.repository.set_last_attempt(seeded_attempt);

    let outcome = unwrap_ok(claimed.service().release_claim("aaaaaaaa", token));

    assert_eq!(outcome, expected);
    assert_eq!(stored_claim(&claimed), expected_claim.map(ClaimToken::new));
    let stored = claimed.repository.stored_entries();
    let stored = match stored.first() {
        Some(entry) => entry,
        None => panic!("expected the released entry"),
    };
    assert_eq!(stored.lifetime.as_str(), "lifetime-aaaaaaaa");
    assert_eq!(
        claimed.repository.last_attempt().expect("read attempt"),
        expected_attempt
    );
    assert_eq!(claimed.clock.reads(), 1);
}

#[test]
fn missing_entry_release_is_stale() {
    let fixture = Fixture::with_policy(1_000, 0, QueuePolicy::new(0, 0));

    let outcome = unwrap_ok(fixture.service().release_claim("aaaaaaaa", "held"));

    assert_eq!(outcome, ReleaseOutcome::Stale);
    match fixture.repository.last_attempt() {
        Ok(attempt) => assert_eq!(attempt, None),
        Err(error) => panic!("last attempt read failed: {error}"),
    }
}

#[test]
fn release_without_a_claim_is_stale() {
    let fixture = Fixture::with_policy(1_000, 0, QueuePolicy::new(0, 0));
    fixture
        .repository
        .seed(seeded_entry("aaaaaaaa", 0, 1_000, 0, 0));

    let outcome = unwrap_ok(fixture.service().release_claim("aaaaaaaa", "held"));

    assert_eq!(outcome, ReleaseOutcome::Stale);
}

fn stored_claim(fixture: &Fixture) -> Option<ClaimToken> {
    let entries = fixture.repository.stored_entries();
    match entries.first() {
        Some(entry) => entry.claim_token.clone(),
        None => panic!("expected a stored entry"),
    }
}

#[test]
fn completion_records_data_and_retires_the_entry() {
    let fixture = Fixture::with_policy(1_000, 0, QueuePolicy::new(0, 0));
    let mut entry = seeded_entry("aaaaaaaa", 0, 1_000, 0, 0);
    entry.claim_token = Some(ClaimToken::new("held"));
    fixture.repository.seed(entry.clone());

    unwrap_ok(fixture.service().complete(
        entry.clone(),
        "token-hash".to_owned(),
        Some("https://github.com/octocat/hello-world/pull/7#issuecomment-1".to_owned()),
    ));

    assert!(fixture.repository.stored_entries().is_empty());
    let completions = fixture.repository.completions();
    assert_eq!(completions.len(), 1);
    let completion = &completions[0];
    assert_eq!(completion.entry, entry);
    assert_eq!(completion.token_hash, "token-hash");
    assert_eq!(
        completion.comment_url.as_deref(),
        Some("https://github.com/octocat/hello-world/pull/7#issuecomment-1")
    );
    assert_eq!(completion.posted_at, 1_000);
    assert_eq!(fixture.clock.reads(), 1);
}
