//! Worker behavior when posting history cannot be read.

use super::super::{Notify, TokenClient, WorkerControl, WorkerHooks, run_worker};
use super::assert_retry_respects_cooldown;
use crate::config::Config;
use crate::listener::dispatch_request;
use crate::queue::SharedQueue;
use comenq_lib::CommentRequest;
use comenq_lib::protocol::Request;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::tempdir;
use test_support::{octocrab_with_token, temp_config};
use tokio::sync::watch;
use tokio::time::Instant;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn failed_post_retries_after_a_full_cooldown_when_history_is_unavailable() {
    let dir = tempdir().expect("create temporary queue directory");
    let cooldown = Duration::from_secs(1);
    let test_config = temp_config(&dir).with_cooldown(cooldown.as_secs());
    std::fs::create_dir_all(test_config.queue_path.join("history.jsonl"))
        .expect("make history log path unavailable");
    let cfg = Arc::new(Config::from(test_config));
    let queue = SharedQueue::open(cfg).expect("open queue");
    let response = dispatch_request(
        &queue,
        Request::Put {
            request: CommentRequest {
                owner: "octocat".into(),
                repo: "hello-world".into(),
                pr_number: 7,
                body: "retry".into(),
            },
            immediate: true,
        },
    )
    .await;
    assert!(matches!(
        response,
        comenq_lib::protocol::Response::Ok { .. }
    ));

    let server = MockServer::start().await;
    let post_times = Arc::new(Mutex::new(Vec::new()));
    let recorded_post_times = Arc::clone(&post_times);
    // Use a client-non-retryable failure so timestamps measure worker retries.
    Mock::given(method("POST"))
        .and(path("/repos/octocat/hello-world/issues/7/comments"))
        .respond_with(move |_: &wiremock::Request| {
            recorded_post_times
                .lock()
                .expect("POST time lock should not be poisoned")
                .push(Instant::now());
            ResponseTemplate::new(400)
        })
        .expect(2)
        .mount(&server)
        .await;
    let first_token = "alpha-token";
    let second_token = "bravo-token";
    let clients = Arc::new(vec![
        TokenClient::new(
            "first",
            first_token,
            octocrab_with_token(&server, first_token).expect("first GitHub client"),
        ),
        TokenClient::new(
            "second",
            second_token,
            octocrab_with_token(&server, second_token).expect("second GitHub client"),
        ),
    ]);
    let idle = Arc::new(Notify::new());
    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let worker = tokio::spawn(run_worker(
        Arc::clone(&queue),
        clients,
        WorkerControl::new(
            shutdown_rx,
            WorkerHooks {
                enqueued: None,
                idle: Some(Arc::clone(&idle)),
                drained: None,
                waiting: None,
            },
        ),
    ));

    for _ in 0..2 {
        tokio::time::timeout(Duration::from_secs(5), idle.notified())
            .await
            .expect("worker should complete a failed post attempt");
    }
    shutdown_tx.send(()).expect("signal shutdown");
    worker
        .await
        .expect("worker task should not panic")
        .expect("worker should exit cleanly");
    assert_retry_respects_cooldown(&post_times, cooldown);
    let requests = server
        .received_requests()
        .await
        .expect("requests should be recorded");
    assert_eq!(requests.len(), 2);
    let authorization_headers: Vec<_> = requests
        .iter()
        .map(|request| {
            request
                .headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
        })
        .collect();
    assert_eq!(
        authorization_headers,
        vec![Some("Bearer alpha-token"); 2],
        "unreadable history should select the first token on every attempt",
    );
}
