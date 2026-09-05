//! Tests for the worker's shutdown waits and notification hooks.

use super::{Notify, WorkerControl, WorkerHooks, run_worker, wait_for_retry_deadline};
use crate::config::Config;
use crate::queue::SharedQueue;
use comenq_lib::CommentRequest;
use comenq_lib::protocol::Request;
use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::Duration;
use tempfile::tempdir;
use test_support::{octocrab_for, temp_config};
use tokio::sync::watch;
use tokio::time::Instant;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

mod post_span;
mod scheduling;

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

#[tokio::test]
async fn failed_post_retries_after_a_full_cooldown() {
    let dir = tempdir().expect("create temporary queue directory");
    let cfg = Arc::new(Config::from(temp_config(&dir).with_cooldown(1)));
    let queue = SharedQueue::open(cfg).expect("open queue");
    let response = queue
        .execute(Request::Put {
            request: CommentRequest {
                owner: "octocat".into(),
                repo: "hello-world".into(),
                pr_number: 7,
                body: "retry".into(),
            },
            immediate: true,
        })
        .await;
    assert!(matches!(
        response,
        comenq_lib::protocol::Response::Ok { .. }
    ));

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repos/octocat/hello-world/issues/7/comments"))
        .respond_with(ResponseTemplate::new(500))
        .expect(2..)
        .mount(&server)
        .await;
    let octocrab = octocrab_for(&server).expect("create GitHub client");
    let idle = Arc::new(Notify::new());
    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let worker = tokio::spawn(run_worker(
        Arc::clone(&queue),
        octocrab,
        WorkerControl::new(
            shutdown_rx,
            WorkerHooks {
                enqueued: None,
                idle: Some(Arc::clone(&idle)),
                drained: None,
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
    assert!(
        server
            .received_requests()
            .await
            .expect("read requests")
            .len()
            >= 2
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
    queue
        .execute(Request::Put {
            request: failed_request,
            immediate: true,
        })
        .await;

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repos/octocat/hello-world/issues/7/comments"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let octocrab = octocrab_for(&server).expect("create GitHub client");
    let idle = Arc::new(Notify::new());
    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let worker = tokio::spawn(run_worker(
        Arc::clone(&queue),
        octocrab,
        WorkerControl::new(
            shutdown_rx,
            WorkerHooks {
                enqueued: None,
                idle: Some(Arc::clone(&idle)),
                drained: None,
            },
        ),
    ));

    idle.notified().await;
    queue
        .execute(Request::Put {
            request: CommentRequest {
                owner: "octocat".into(),
                repo: "hello-world".into(),
                pr_number: 8,
                body: "queue mutation".into(),
            },
            immediate: true,
        })
        .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(100), idle.notified())
            .await
            .is_err(),
        "a queue mutation must not trigger an immediate retry"
    );

    tokio::time::timeout(Duration::from_secs(cooldown + 1), idle.notified())
        .await
        .expect("worker should retry after the cooldown");

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
