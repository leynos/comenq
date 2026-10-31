//! Identifier, repository-component and facade error contracts.

use super::support::{Fixture, TestError, comment, unwrap_err, unwrap_ok};
use crate::{
    CommandOutcome, Comment, EntryId, PutComment, QueryOutcome, QueueCommand, QueueError,
    QueuePolicy, QueueQuery,
};

#[test]
fn entry_id_requires_exactly_eight_hex_digits() {
    assert!(EntryId::parse("0123abcd").is_ok());
    assert!(EntryId::parse("ABCDEF01").is_ok());
    for rejected in ["", "0123abc", "0123abcde", "zzzzzzzz", "0123abcd "] {
        assert!(EntryId::parse(rejected).is_err(), "{rejected:?} must fail");
    }
}

#[test]
fn invalid_identifier_message_matches_the_wire_contract() {
    let error = EntryId::parse("bad")
        .err()
        .unwrap_or_else(|| panic!("identifier must be rejected"));

    assert_eq!(
        error.to_string(),
        "queue entry id 'bad' must be eight hexadecimal characters"
    );
    assert_eq!(error.value(), "bad");
}

#[test]
fn unknown_identifier_is_reported_for_each_mutation() {
    let fixture = Fixture::with_policy(1_000, 0, QueuePolicy::new(0, 0));
    let service = fixture.service();

    for error in [
        unwrap_err(service.bump("aaaaaaaa")),
        unwrap_err(service.bust("aaaaaaaa")),
        unwrap_err(service.del("aaaaaaaa")),
    ] {
        assert_eq!(error.to_string(), "no queued comment has id 'aaaaaaaa'");
    }
}

#[test]
fn malformed_identifier_is_rejected_before_storage_access() {
    let fixture = Fixture::with_policy(1_000, 0, QueuePolicy::new(0, 0));
    fixture.repository.fail_on("find");
    let service = fixture.service();

    for error in [
        unwrap_err(service.bump("bad")),
        unwrap_err(service.bust("bad")),
        unwrap_err(service.del("bad")),
        unwrap_err(service.release_claim("bad", "token")),
    ] {
        assert_eq!(
            error.to_string(),
            "queue entry id 'bad' must be eight hexadecimal characters"
        );
    }
}

#[test]
fn unsafe_owner_and_repository_are_rejected() {
    let fixture = Fixture::new(1_000, 0);
    let service = fixture.service();

    let owner = unwrap_err(service.put(PutComment {
        comment: Comment::new("../owner", "hello-world", 7, "body"),
        immediate: false,
    }));
    let repository = unwrap_err(service.put(PutComment {
        comment: Comment::new("octocat", "..", 7, "body"),
        immediate: false,
    }));

    assert_eq!(
        owner.to_string(),
        "GitHub repository owner contains unsafe characters"
    );
    assert_eq!(
        repository.to_string(),
        "GitHub repository name contains unsafe characters"
    );
}

#[test]
fn repository_failure_is_transparent() {
    let fixture = Fixture::new(1_000, 0);
    fixture.repository.fail_on("entries");
    let service = fixture.service();

    let error = unwrap_err(service.put(PutComment {
        comment: comment(),
        immediate: false,
    }));

    assert!(matches!(
        error,
        QueueError::Repository(TestError {
            operation: "entries"
        })
    ));
    assert_eq!(error.to_string(), "test repository failure: entries");
    assert!(error.repository().is_some());
}

#[test]
fn commands_and_queries_drive_the_service() {
    let fixture = Fixture::with_policy(1_000, 0, QueuePolicy::new(0, 0));
    let service = fixture.service();

    let put = unwrap_ok(service.execute(QueueCommand::Put(PutComment {
        comment: comment(),
        immediate: false,
    })));
    let CommandOutcome::Put(scheduled) = put else {
        panic!("expected a put outcome, got {put:?}");
    };
    let id = scheduled.entry.id;
    assert_eq!(
        unwrap_ok(service.execute(QueueCommand::Bump(id.clone()))),
        CommandOutcome::Changed
    );
    let listed = unwrap_ok(service.query(QueueQuery::List));
    let QueryOutcome::List(entries) = listed else {
        panic!("expected a list outcome, got {listed:?}");
    };
    assert_eq!(entries.len(), 1);
    assert_eq!(
        unwrap_ok(service.execute(QueueCommand::Del(id))),
        CommandOutcome::Changed
    );
    let next_due = unwrap_ok(service.query(QueueQuery::NextDue));
    let QueryOutcome::NextDue(entry) = next_due else {
        panic!("expected a next-due outcome, got {next_due:?}");
    };
    assert!(entry.is_none());
}
