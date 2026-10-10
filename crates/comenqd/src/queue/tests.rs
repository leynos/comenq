//! Deterministic tests for shared queue scheduling and request handling.

use super::{FlutterSampler, SharedQueue, StoredEntry, UnixClock};
use crate::config::Config;
use crate::listener::dispatch_request;
use comenq_lib::CommentRequest;
use comenq_lib::protocol::{MAX_PENDING_ENTRIES, MAX_RESPONSE_BYTES, Request, Response};
use std::fs;
use std::sync::Arc;
use tempfile::tempdir;

fn open_queue_with_bodies(body_sizes: &[usize]) -> (tempfile::TempDir, Arc<SharedQueue>) {
    let dir = tempdir().expect("create temporary queue directory");
    let entries_dir = dir.path().join("queue").join("entries");
    fs::create_dir_all(&entries_dir).expect("create entries directory");
    for (index, body_size) in body_sizes.iter().copied().enumerate() {
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
                body: "x".repeat(body_size),
            },
        };
        let bytes = serde_json::to_vec(&entry).expect("serialize entry");
        assert!(
            bytes.len() <= 2 * 1024 * 1024,
            "test entry exceeds store limit"
        );
        fs::write(entries_dir.join(format!("{index:08x}.json")), bytes).expect("write entry");
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
    (dir, queue)
}

fn body_sizes_at_response_limit(extra_bytes: usize) -> [usize; 2] {
    let entries = (0..2)
        .map(|index| StoredEntry {
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
                body: String::new(),
            },
        })
        .map(|entry| crate::listener::protocol::pending_entry(entry, 0))
        .collect();
    let base_size = serde_json::to_vec(&Response::entries(entries))
        .expect("serialize empty-body response")
        .len();
    let body_bytes = MAX_RESPONSE_BYTES - base_size + extra_bytes;
    [body_bytes / 2, body_bytes - body_bytes / 2]
}

#[derive(Debug)]
struct FixedClock(u64);

impl UnixClock for FixedClock {
    fn unix_now(&self) -> u64 {
        self.0
    }
}

#[derive(Debug)]
struct FixedFlutter(u64);

impl FlutterSampler for FixedFlutter {
    fn sample(&self, maximum: u64) -> u64 {
        self.0.min(maximum)
    }
}

/// Verify enqueue uses the injected flutter sample.
#[tokio::test]
async fn put_uses_the_injected_flutter_sample() {
    let dir = tempdir().expect("create temporary queue directory");
    let queue = SharedQueue::open_with_clock_and_flutter(
        Arc::new(Config {
            github_token: "token".into(),
            github_token_file: None,
            socket_path: dir.path().join("comenq.sock"),
            queue_path: dir.path().join("queue"),
            cooldown_period_seconds: 600,
            cooldown_flutter_seconds: 240,
            restart_min_delay_ms: 1,
            github_api_timeout_secs: 1,
        }),
        Arc::new(FixedClock(1_000)),
        Arc::new(FixedFlutter(37)),
    )
    .expect("open queue");

    let response = dispatch_request(
        &queue,
        Request::Put {
            request: CommentRequest {
                owner: "octocat".into(),
                repo: "hello-world".into(),
                pr_number: 7,
                body: "comment".into(),
            },
            immediate: false,
        },
    )
    .await;

    assert!(matches!(
        response,
        Response::Ok {
            entry: Some(entry),
            ..
        } if entry.eta_seconds == 637
    ));
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

    let response = dispatch_request(
        &queue,
        Request::Put {
            request: CommentRequest {
                owner: "octocat".into(),
                repo: "hello-world".into(),
                pr_number: 7,
                body: "comment".into(),
            },
            immediate: false,
        },
    )
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

    let response = dispatch_request(
        &queue,
        Request::Put {
            request: CommentRequest {
                owner: "octocat\u{1b}[2J".into(),
                repo: "hello-world".into(),
                pr_number: 7,
                body: "comment".into(),
            },
            immediate: true,
        },
    )
    .await;
    assert!(matches!(response, Response::Error { .. }));
    assert!(
        matches!(dispatch_request(&queue, Request::List).await, Response::Ok { entries: Some(entries), .. } if entries.is_empty())
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

    let response = dispatch_request(
        &queue,
        Request::Put {
            request: CommentRequest {
                owner: "octocat".into(),
                repo: "hello-world".into(),
                pr_number: 7,
                body: "comment".into(),
            },
            immediate: true,
        },
    )
    .await;
    assert!(matches!(response, Response::Error { .. }));

    fs::remove_file(queue_path.join("last_post")).expect("remove malformed marker");
    assert!(
        matches!(dispatch_request(&queue, Request::List).await, Response::Ok { entries: Some(entries), .. } if entries.is_empty())
    );
}

#[tokio::test]
async fn list_returns_at_most_the_pending_entry_limit() {
    let body_sizes = ["comment".len(); MAX_PENDING_ENTRIES];
    let (_dir, queue) = open_queue_with_bodies(&body_sizes);
    let response = dispatch_request(&queue, Request::List).await;
    let Response::Ok {
        entries: Some(entries),
        ..
    } = response
    else {
        panic!("expected bounded list response, got {response:?}");
    };
    assert_eq!(entries.len(), MAX_PENDING_ENTRIES);
    assert!(
        serde_json::to_vec(&Response::entries(entries))
            .expect("serialize bounded list")
            .len()
            <= MAX_RESPONSE_BYTES
    );
}

#[tokio::test]
async fn list_returns_error_instead_of_exceeding_the_response_byte_limit() {
    let body_sizes = body_sizes_at_response_limit(1);
    let (_dir, queue) = open_queue_with_bodies(&body_sizes);

    let response = dispatch_request(&queue, Request::List).await;

    assert!(matches!(response, Response::Error { .. }));
    assert!(
        serde_json::to_vec(&response)
            .expect("serialize bounded error response")
            .len()
            <= MAX_RESPONSE_BYTES
    );
}

#[tokio::test]
async fn list_allows_a_response_exactly_at_the_byte_limit() {
    let body_sizes = body_sizes_at_response_limit(0);
    let (_dir, queue) = open_queue_with_bodies(&body_sizes);

    let response = dispatch_request(&queue, Request::List).await;
    let serialized = serde_json::to_vec(&response).expect("serialize list response");

    assert!(matches!(response, Response::Ok { .. }));
    assert_eq!(serialized.len(), MAX_RESPONSE_BYTES);
}
