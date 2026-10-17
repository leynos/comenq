//! Tests for worker retry pacing, claim recovery, queue notifications, and
//! shutdown waits.

use super::{
    Notify, TokenClient, WorkerControl, WorkerHooks, retry_deadline, run_worker,
    wait_for_retry_deadline,
};
use crate::config::Config;
use crate::listener::dispatch_request;
use crate::queue::SharedQueue;
use comenq_lib::CommentRequest;
use comenq_lib::protocol::Request;
use octocrab::Octocrab;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;
use tempfile::tempdir;
use test_support::{octocrab_for, temp_config};
use tokio::sync::watch;
use tokio::time::Instant;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

mod history_unavailable;
mod post_span;
mod scheduling;
mod token_rotation;

pub(super) fn client_pool(octocrab: Arc<Octocrab>) -> Arc<Vec<TokenClient>> {
    Arc::new(vec![TokenClient::new("default", "t", octocrab)])
}

#[test]
fn overflowing_retry_cooldown_uses_a_far_future_deadline() {
    let started = Instant::now();
    let deadline = retry_deadline(Duration::from_secs(u64::MAX));

    assert!(deadline.duration_since(started) >= Duration::from_secs(99 * 365 * 24 * 60 * 60));
}

/// Tests that notify_one wakes exactly one waiter when multiple tasks are waiting.
///
/// This validates the single-waiter semantics documented on WorkerHooks.
#[tokio::test]
async fn notify_one_wakes_exactly_one_waiter() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let notify = Arc::new(Notify::new());
    let wake_count = Arc::new(AtomicUsize::new(0));

    // Spawn three waiters
    let mut handles = Vec::new();
    for _ in 0..3 {
        let n = notify.clone();
        let count = wake_count.clone();
        handles.push(tokio::spawn(async move {
            // Wait with a timeout to avoid hanging the test
            if tokio::time::timeout(Duration::from_millis(100), n.notified())
                .await
                .is_ok()
            {
                count.fetch_add(1, Ordering::SeqCst);
            }
        }));
    }

    // Give waiters time to register
    tokio::time::sleep(Duration::from_millis(10)).await;

    // Send exactly one notification
    notify.notify_one();

    // Wait for all tasks to complete (they'll timeout after 100ms)
    for h in handles {
        let _ = h.await;
    }

    // Only one waiter should have been woken
    assert_eq!(
        wake_count.load(Ordering::SeqCst),
        1,
        "notify_one should wake exactly one waiter"
    );
}

/// Tests that notify_one buffers a permit when no waiters exist.
///
/// This validates that the notification is not lost if sent before waiting.
#[tokio::test]
async fn notify_one_buffers_permit_when_no_waiters() {
    let notify = Arc::new(Notify::new());

    // Send notification before anyone is waiting
    notify.notify_one();

    // The first waiter should receive the buffered permit immediately
    let result = tokio::time::timeout(Duration::from_millis(50), notify.notified()).await;
    assert!(
        result.is_ok(),
        "buffered permit should wake first waiter immediately"
    );

    // Second waiter should NOT receive a permit (it was consumed)
    let result = tokio::time::timeout(Duration::from_millis(50), notify.notified()).await;
    assert!(
        result.is_err(),
        "second waiter should timeout with no remaining permit"
    );
}

/// Verify a restarted worker recovers and posts a claim on the shared queue.
#[tokio::test]
async fn restarted_worker_recovers_a_claim_on_the_shared_queue() {
    let dir = tempdir().expect("create temporary queue directory");
    let queue = SharedQueue::open(Arc::new(Config::from(temp_config(&dir).with_cooldown(0))))
        .expect("open queue");
    let put = dispatch_request(
        &queue,
        Request::Put {
            request: CommentRequest {
                owner: "octocat".into(),
                repo: "hello-world".into(),
                pr_number: 7,
                body: "recover claim".into(),
            },
            immediate: true,
        },
    )
    .await;
    let comenq_lib::protocol::Response::Ok {
        entry: Some(entry), ..
    } = put
    else {
        panic!("expected queued entry, got {put:?}");
    };
    let (claimed, wait_seconds) = queue
        .claim_next_due()
        .await
        .expect("claim queued entry")
        .expect("entry is due");
    assert_eq!(claimed.id, entry.id);
    assert_eq!(wait_seconds, 0);
    assert!(claimed.claim_token.is_some());

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repos/octocat/hello-world/issues/7/comments"))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(
                serde_json::from_str::<serde_json::Value>(include_str!(
                    "../../tests/fixtures/github_comment_response.json"
                ))
                .expect("parse GitHub comment fixture"),
            ),
        )
        .expect(1)
        .mount(&server)
        .await;
    let octocrab = octocrab_for(&server).expect("create GitHub client");
    let idle = Arc::new(Notify::new());
    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let worker = tokio::spawn(run_worker(
        Arc::clone(&queue),
        client_pool(octocrab),
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

    tokio::time::timeout(Duration::from_secs(5), idle.notified())
        .await
        .expect("restarted worker should post the recovered entry");
    assert!(matches!(
        dispatch_request(&queue, Request::List).await,
        comenq_lib::protocol::Response::Ok {
            entries: Some(entries),
            ..
        } if entries.is_empty()
    ));

    shutdown_tx.send(()).expect("signal shutdown");
    worker
        .await
        .expect("worker task should not panic")
        .expect("worker should exit cleanly");
}

fn assert_retry_respects_cooldown(post_times: &Mutex<Vec<Instant>>, cooldown: Duration) {
    let post_times = post_times
        .lock()
        .expect("POST time lock should not be poisoned");
    assert_eq!(post_times.len(), 2, "mock should record both POST attempts");
    let retry_delay = post_times[1].duration_since(post_times[0]);
    assert!(
        retry_delay >= cooldown,
        "retry must wait at least {cooldown:?}, but elapsed {retry_delay:?}",
    );
}

#[tokio::test]
async fn queue_changes_do_not_shorten_a_failed_post_retry_cooldown() {
    let dir = tempdir().expect("create temporary queue directory");
    let cooldown = 1;
    let cfg = Arc::new(Config::from(temp_config(&dir).with_cooldown(cooldown)));
    let queue = SharedQueue::open(cfg).expect("open queue");
    let failed_request = CommentRequest {
        owner: "octocat".into(),
        repo: "hello-world".into(),
        pr_number: 7,
        body: "retry".into(),
    };
    dispatch_request(
        &queue,
        Request::Put {
            request: failed_request,
            immediate: true,
        },
    )
    .await;

    let server = MockServer::start().await;
    let post_times = Arc::new(Mutex::new(Vec::new()));
    let recorded_post_times = Arc::clone(&post_times);
    Mock::given(method("POST"))
        .and(path("/repos/octocat/hello-world/issues/7/comments"))
        .respond_with(move |_: &wiremock::Request| {
            recorded_post_times
                .lock()
                .expect("POST time lock should not be poisoned")
                .push(Instant::now());
            // Measure worker retries without the HTTP client's own 5xx retries.
            ResponseTemplate::new(400)
        })
        .mount(&server)
        .await;
    let octocrab = octocrab_for(&server).expect("create GitHub client");
    let idle = Arc::new(Notify::new());
    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let worker = tokio::spawn(run_worker(
        Arc::clone(&queue),
        client_pool(octocrab),
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

    scheduling::wait_for_hook_with_real_timeout(&idle, "worker should report the failed post")
        .await;
    assert_eq!(post_times.lock().expect("POST time lock").len(), 1);
    dispatch_request(
        &queue,
        Request::Put {
            request: CommentRequest {
                owner: "octocat".into(),
                repo: "hello-world".into(),
                pr_number: 8,
                body: "queue mutation".into(),
            },
            immediate: true,
        },
    )
    .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(100), idle.notified())
            .await
            .is_err(),
        "a queue mutation must not trigger an immediate retry"
    );
    assert_eq!(post_times.lock().expect("POST time lock").len(), 1);

    // Keep the real-time retry assertion reliable when parallel tests delay
    // the worker's timer wake-up under load.
    let retry_wait = Duration::from_secs(cooldown).saturating_add(Duration::from_secs(5));
    tokio::time::timeout(retry_wait, idle.notified())
        .await
        .expect("worker should retry after the cooldown");
    assert_retry_respects_cooldown(&post_times, Duration::from_secs(cooldown));

    shutdown_tx.send(()).expect("signal shutdown");
    worker
        .await
        .expect("worker task should not panic")
        .expect("worker should exit cleanly");
}

#[tokio::test(start_paused = true)]
async fn retry_deadline_ignores_queue_notifications_until_it_expires() {
    let changed = Notify::new();
    let (_shutdown_tx, mut shutdown_rx) = watch::channel(());
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut wait = Box::pin(wait_for_retry_deadline(
        deadline,
        &changed,
        &mut shutdown_rx,
    ));
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);

    changed.notify_one();
    assert!(matches!(wait.as_mut().poll(&mut context), Poll::Pending));
    tokio::time::advance(Duration::from_secs(59)).await;
    assert!(matches!(wait.as_mut().poll(&mut context), Poll::Pending));
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(matches!(
        wait.as_mut().poll(&mut context),
        Poll::Ready(false)
    ));
}
