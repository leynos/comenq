//! Paused-time worker scheduling and queue-mutation tests.

use super::{WorkerControl, WorkerHooks, run_worker};
use crate::config::Config;
use crate::queue::{SharedQueue, UnixClock};
use comenq_lib::CommentRequest;
use comenq_lib::protocol::{PendingEntry, Request, Response};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tempfile::tempdir;
use test_support::{octocrab_for, temp_config};
use tokio::sync::{Notify, watch};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

const COOLDOWN_SECONDS: u64 = 60;

#[derive(Debug)]
struct AdvancingClock(AtomicU64);

impl AdvancingClock {
    fn set(&self, now: u64) {
        self.0.store(now, Ordering::SeqCst);
    }
}

impl UnixClock for AdvancingClock {
    fn unix_now(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

fn request(pr_number: u64) -> CommentRequest {
    CommentRequest {
        owner: "octocat".into(),
        repo: "hello-world".into(),
        pr_number,
        body: "comment".into(),
    }
}

fn response_template() -> ResponseTemplate {
    let body: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/github_comment_response.json"
    ))
    .expect("parse GitHub comment fixture");
    ResponseTemplate::new(201).set_body_json(body)
}

async fn pending_entries(queue: &SharedQueue) -> Vec<PendingEntry> {
    let response = queue.execute(Request::List).await;
    let Response::Ok {
        entries: Some(entries),
        ..
    } = response
    else {
        panic!("expected queue entries, got {response:?}");
    };
    entries
}

async fn wait_for_idle_without_advancing_time(idle: &Notify) {
    tokio::select! {
        () = idle.notified() => {}
        () = async {
            for _ in 0..1_000 {
                tokio::task::yield_now().await;
            }
        } => panic!("worker did not complete the scheduled post"),
    }
}

async fn yield_to_worker() {
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
}

async fn worker_with_successful_posts(
    queue: Arc<SharedQueue>,
    server: &MockServer,
    expected_posts: u64,
    idle: Arc<Notify>,
) -> (
    watch::Sender<()>,
    tokio::task::JoinHandle<anyhow::Result<()>>,
) {
    Mock::given(method("POST"))
        .respond_with(response_template())
        .expect(expected_posts)
        .mount(server)
        .await;
    let octocrab = octocrab_for(server).expect("create GitHub client");
    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let worker = tokio::spawn(run_worker(
        queue,
        octocrab,
        WorkerControl::new(
            shutdown_rx,
            WorkerHooks {
                enqueued: None,
                idle: Some(idle),
                drained: None,
            },
        ),
    ));
    (shutdown_tx, worker)
}

#[tokio::test(start_paused = true)]
async fn worker_respects_deferred_and_successive_entry_etas() {
    let dir = tempdir().expect("create temporary queue directory");
    let clock = Arc::new(AdvancingClock(AtomicU64::new(1_000)));
    let queue_clock: Arc<dyn UnixClock> = clock.clone();
    let queue = SharedQueue::open_with_clock(
        Arc::new(Config::from(
            temp_config(&dir).with_cooldown(COOLDOWN_SECONDS),
        )),
        queue_clock,
    )
    .expect("open queue");
    for pr_number in [7, 8] {
        assert!(matches!(
            queue
                .execute(Request::Put {
                    request: request(pr_number),
                    immediate: false,
                })
                .await,
            Response::Ok { .. }
        ));
    }

    let server = MockServer::start().await;
    let idle = Arc::new(Notify::new());
    let (shutdown_tx, worker) =
        worker_with_successful_posts(Arc::clone(&queue), &server, 2, Arc::clone(&idle)).await;
    yield_to_worker().await;

    tokio::time::advance(Duration::from_secs(COOLDOWN_SECONDS - 1)).await;
    clock.set(1_000 + COOLDOWN_SECONDS - 1);
    yield_to_worker().await;
    assert_eq!(pending_entries(&queue).await.len(), 2);

    clock.set(1_000 + COOLDOWN_SECONDS);
    tokio::time::advance(Duration::from_secs(1)).await;
    wait_for_idle_without_advancing_time(&idle).await;
    assert_eq!(pending_entries(&queue).await.len(), 1);

    yield_to_worker().await;
    tokio::time::advance(Duration::from_secs(COOLDOWN_SECONDS - 1)).await;
    clock.set(1_000 + (2 * COOLDOWN_SECONDS) - 1);
    yield_to_worker().await;
    assert_eq!(pending_entries(&queue).await.len(), 1);

    clock.set(1_000 + (2 * COOLDOWN_SECONDS));
    tokio::time::advance(Duration::from_secs(1)).await;
    wait_for_idle_without_advancing_time(&idle).await;
    assert!(pending_entries(&queue).await.is_empty());

    shutdown_tx.send(()).expect("signal shutdown");
    worker
        .await
        .expect("worker task should not panic")
        .expect("worker should exit cleanly");
}

#[tokio::test(start_paused = true)]
async fn queue_mutation_wakes_a_worker_waiting_for_a_deferred_entry() {
    let dir = tempdir().expect("create temporary queue directory");
    let clock = Arc::new(AdvancingClock(AtomicU64::new(1_000)));
    let queue_clock: Arc<dyn UnixClock> = clock.clone();
    let queue = SharedQueue::open_with_clock(
        Arc::new(Config::from(
            temp_config(&dir).with_cooldown(COOLDOWN_SECONDS),
        )),
        queue_clock,
    )
    .expect("open queue");
    queue
        .execute(Request::Put {
            request: request(7),
            immediate: false,
        })
        .await;

    let server = MockServer::start().await;
    let idle = Arc::new(Notify::new());
    let (shutdown_tx, worker) =
        worker_with_successful_posts(Arc::clone(&queue), &server, 1, Arc::clone(&idle)).await;
    tokio::task::yield_now().await;

    let response = queue
        .execute(Request::Put {
            request: request(8),
            immediate: true,
        })
        .await;
    let Response::Ok {
        entry: Some(entry), ..
    } = response
    else {
        panic!("expected immediate entry, got {response:?}");
    };
    assert!(matches!(
        queue.execute(Request::Bump { id: entry.id }).await,
        Response::Ok { .. }
    ));

    wait_for_idle_without_advancing_time(&idle).await;
    assert_eq!(pending_entries(&queue).await.len(), 1);

    shutdown_tx.send(()).expect("signal shutdown");
    worker
        .await
        .expect("worker task should not panic")
        .expect("worker should exit cleanly");
}
