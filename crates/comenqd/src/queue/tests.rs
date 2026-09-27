//! Deterministic tests for shared queue scheduling and request handling.

use super::{SharedQueue, StoredEntry, UnixClock};
use crate::config::Config;
use comenq_lib::CommentRequest;
use comenq_lib::protocol::{MAX_PENDING_ENTRIES, Request, Response};
use std::fs;
use std::sync::Arc;
use tempfile::tempdir;

#[derive(Debug)]
struct FixedClock(u64);

impl UnixClock for FixedClock {
    fn unix_now(&self) -> u64 {
        self.0
    }
}

#[tokio::test]
async fn fixed_clock_controls_deferred_put_eta() {
    let dir = tempdir().expect("create temporary queue directory");
    let queue = SharedQueue::open_with_clock(
        Arc::new(Config {
            github_token: "token".into(),
            github_token_file: None,
            socket_path: dir.path().join("comenq.sock"),
            queue_path: dir.path().join("queue"),
            cooldown_period_seconds: 600,
            cooldown_flutter_seconds: 0,
            restart_min_delay_ms: 1,
            github_api_timeout_secs: 1,
        }),
        Arc::new(FixedClock(1_000)),
    )
    .expect("open queue");

    let response = queue
        .execute(Request::Put {
            request: CommentRequest {
                owner: "octocat".into(),
                repo: "hello-world".into(),
                pr_number: 7,
                body: "comment".into(),
            },
            immediate: false,
        })
        .await;
    let Response::Ok {
        entry: Some(entry), ..
    } = response
    else {
        panic!("expected queued entry, got {response:?}");
    };
    assert_eq!(entry.eta_seconds, 600);
}

#[tokio::test]
async fn put_rejects_unsafe_repository_components() {
    let dir = tempdir().expect("create temporary queue directory");
    let queue = SharedQueue::open(Arc::new(Config {
        github_token: "token".into(),
        github_token_file: None,
        socket_path: dir.path().join("comenq.sock"),
        queue_path: dir.path().join("queue"),
        cooldown_period_seconds: 600,
        cooldown_flutter_seconds: 0,
        restart_min_delay_ms: 1,
        github_api_timeout_secs: 1,
    }))
    .expect("open queue");

    let response = queue
        .execute(Request::Put {
            request: CommentRequest {
                owner: "octocat\u{1b}[2J".into(),
                repo: "hello-world".into(),
                pr_number: 7,
                body: "comment".into(),
            },
            immediate: true,
        })
        .await;
    assert!(matches!(response, Response::Error { .. }));
    assert!(
        matches!(queue.execute(Request::List).await, Response::Ok { entries: Some(entries), .. } if entries.is_empty())
    );
}

#[tokio::test]
async fn put_does_not_persist_when_eta_projection_fails() {
    let dir = tempdir().expect("create temporary queue directory");
    let queue_path = dir.path().join("queue");
    let queue = SharedQueue::open(Arc::new(Config {
        github_token: "token".into(),
        github_token_file: None,
        socket_path: dir.path().join("comenq.sock"),
        queue_path: queue_path.clone(),
        cooldown_period_seconds: 600,
        cooldown_flutter_seconds: 0,
        restart_min_delay_ms: 1,
        github_api_timeout_secs: 1,
    }))
    .expect("open queue");
    fs::write(queue_path.join("last_post"), "not a timestamp").expect("write malformed marker");

    let response = queue
        .execute(Request::Put {
            request: CommentRequest {
                owner: "octocat".into(),
                repo: "hello-world".into(),
                pr_number: 7,
                body: "comment".into(),
            },
            immediate: true,
        })
        .await;
    assert!(matches!(response, Response::Error { .. }));

    fs::remove_file(queue_path.join("last_post")).expect("remove malformed marker");
    assert!(
        matches!(queue.execute(Request::List).await, Response::Ok { entries: Some(entries), .. } if entries.is_empty())
    );
}

#[tokio::test]
async fn list_caps_the_response_at_the_pending_entry_limit() {
    let dir = tempdir().expect("create temporary queue directory");
    let entries_dir = dir.path().join("queue").join("entries");
    fs::create_dir_all(&entries_dir).expect("create entries directory");
    for index in 0..=MAX_PENDING_ENTRIES {
        let entry = StoredEntry {
            id: format!("{index:08x}"),
            order: index as i64,
            flutter_seconds: 0,
            enqueued_at: index as u64,
            not_before: 0,
            claim_token: None,
            request: CommentRequest {
                owner: "octocat".into(),
                repo: "hello-world".into(),
                pr_number: index as u64,
                body: "comment".into(),
            },
        };
        fs::write(
            entries_dir.join(format!("{index:08x}.json")),
            serde_json::to_vec(&entry).expect("serialize entry"),
        )
        .expect("write entry");
    }
    let queue = SharedQueue::open(Arc::new(Config {
        github_token: "token".into(),
        github_token_file: None,
        socket_path: dir.path().join("comenq.sock"),
        queue_path: dir.path().join("queue"),
        cooldown_period_seconds: 0,
        cooldown_flutter_seconds: 0,
        restart_min_delay_ms: 1,
        github_api_timeout_secs: 1,
    }))
    .expect("open queue");

    let response = queue.execute(Request::List).await;
    let Response::Ok {
        entries: Some(entries),
        ..
    } = response
    else {
        panic!("expected bounded list response, got {response:?}");
    };
    assert_eq!(entries.len(), MAX_PENDING_ENTRIES);
}
