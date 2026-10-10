//! Paused-time worker scheduling and queue-mutation tests.

use super::{WorkerControl, WorkerHooks, run_worker};
use crate::config::Config;
use crate::listener::dispatch_request;
use crate::queue::{FlutterSampler, SharedQueue, UnixClock};
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
const FLUTTER_SECONDS: u64 = 7;

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

#[derive(Debug)]
struct FixedFlutter(u64);

impl FlutterSampler for FixedFlutter {
    fn sample(&self, maximum: u64) -> u64 {
        self.0.min(maximum)
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
    let response = dispatch_request(queue, Request::List).await;
    let Response::Ok {
        entries: Some(entries),
        ..
    } = response
    else {
        panic!("expected queue entries, got {response:?}");
    };
    entries
}

/// Advance both clocks together so Unix ETAs and Tokio deadlines stay aligned.
///
/// For example, advancing by 66 seconds leaves one second before a 67-second
/// deadline on both clocks.
async fn advance_worker_time(clock: &AdvancingClock, seconds: u64) {
    clock.set(clock.unix_now().saturating_add(seconds));
    tokio::time::advance(Duration::from_secs(seconds)).await;
}

/// Count requests received by the mock server to detect premature posting.
async fn received_post_count(server: &MockServer) -> usize {
    server
        .received_requests()
        .await
        .expect("read received requests")
        .len()
}

async fn wait_for_hook_with_real_timeout(hook: &Notify, message: &str) {
    let (timeout_tx, timeout_rx) = tokio::sync::oneshot::channel();
    let (cancel_tx, cancel_rx) = std::sync::mpsc::channel();
    let timer = std::thread::spawn(move || {
        if cancel_rx.recv_timeout(Duration::from_secs(5)).is_err() {
            let _ = timeout_tx.send(());
        }
    });
    tokio::pin!(timeout_rx);
    let notified = hook.notified();
    tokio::pin!(notified);
    loop {
        tokio::select! {
            biased;
            () = &mut notified => {
                let _ = cancel_tx.send(());
                timer.join().expect("timer thread should exit");
                return;
            }
            _ = &mut timeout_rx => {
                timer.join().expect("timer thread should exit");
                panic!("{message}");
            }
            _ = tokio::task::yield_now() => {}
        }
    }
}

async fn worker_with_successful_posts(
    queue: Arc<SharedQueue>,
    server: &MockServer,
    expected_posts: u64,
    enqueued: Arc<Notify>,
    idle: Arc<Notify>,
    drained: Arc<Notify>,
    waiting: Arc<Notify>,
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
                enqueued: Some(enqueued),
                idle: Some(idle),
                drained: Some(drained),
                waiting: Some(waiting),
            },
        ),
    ));
    (shutdown_tx, worker)
}

#[tokio::test(start_paused = true)]
async fn timer_expiry_posts_first_and_successive_entries_at_flutter_etas() {
    let dir = tempdir().expect("create temporary queue directory");
    let clock = Arc::new(AdvancingClock(AtomicU64::new(1_000)));
    let queue_clock: Arc<dyn UnixClock> = clock.clone();
    let mut config = Config::from(temp_config(&dir).with_cooldown(COOLDOWN_SECONDS));
    config.cooldown_flutter_seconds = FLUTTER_SECONDS;
    let queue = SharedQueue::open_with_clock_and_flutter(
        Arc::new(config),
        queue_clock,
        Arc::new(FixedFlutter(FLUTTER_SECONDS)),
    )
    .expect("open queue");

    let mut ids = Vec::new();
    let mut etas = Vec::new();
    for pr_number in [7, 8] {
        let response = dispatch_request(
            &queue,
            Request::Put {
                request: request(pr_number),
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
        ids.push(entry.id);
        etas.push(entry.eta_seconds);
    }
    let projected_interval = COOLDOWN_SECONDS + FLUTTER_SECONDS;
    assert_eq!(etas, vec![projected_interval, 2 * projected_interval]);

    let server = MockServer::start().await;
    let enqueued = Arc::new(Notify::new());
    let idle = Arc::new(Notify::new());
    let drained = Arc::new(Notify::new());
    let waiting = Arc::new(Notify::new());
    let (shutdown_tx, worker) = worker_with_successful_posts(
        Arc::clone(&queue),
        &server,
        2,
        Arc::clone(&enqueued),
        Arc::clone(&idle),
        Arc::clone(&drained),
        Arc::clone(&waiting),
    )
    .await;
    wait_for_hook_with_real_timeout(&waiting, "worker did not register deferred wait").await;
    assert_eq!(received_post_count(&server).await, 0);

    advance_worker_time(&clock, projected_interval - 1).await;
    assert_eq!(received_post_count(&server).await, 0);
    let first = pending_entries(&queue).await;
    assert_eq!(first[0].id, ids[0]);
    assert_eq!(first[0].eta_seconds, 1);

    advance_worker_time(&clock, 1).await;
    wait_for_hook_with_real_timeout(&enqueued, "worker did not claim the first entry").await;
    wait_for_hook_with_real_timeout(&idle, "worker did not complete the first timed post").await;
    assert_eq!(received_post_count(&server).await, 1);
    let second = pending_entries(&queue).await;
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].id, ids[1]);
    assert_eq!(second[0].eta_seconds, projected_interval);

    wait_for_hook_with_real_timeout(&waiting, "worker did not register the second deferred wait")
        .await;
    assert_eq!(received_post_count(&server).await, 1);
    advance_worker_time(&clock, projected_interval - 1).await;
    assert_eq!(received_post_count(&server).await, 1);
    let second = pending_entries(&queue).await;
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].id, ids[1]);
    assert_eq!(second[0].eta_seconds, 1);

    advance_worker_time(&clock, 1).await;
    wait_for_hook_with_real_timeout(&enqueued, "worker did not claim the second entry").await;
    wait_for_hook_with_real_timeout(&idle, "worker did not complete the second timed post").await;
    assert_eq!(received_post_count(&server).await, 2);
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
    let server = MockServer::start().await;
    let enqueued = Arc::new(Notify::new());
    let idle = Arc::new(Notify::new());
    let drained = Arc::new(Notify::new());
    let waiting = Arc::new(Notify::new());
    let (shutdown_tx, worker) = worker_with_successful_posts(
        Arc::clone(&queue),
        &server,
        1,
        Arc::clone(&enqueued),
        Arc::clone(&idle),
        Arc::clone(&drained),
        Arc::clone(&waiting),
    )
    .await;
    wait_for_hook_with_real_timeout(&drained, "worker did not become idle").await;

    let deferred = dispatch_request(
        &queue,
        Request::Put {
            request: request(7),
            immediate: false,
        },
    )
    .await;
    assert!(matches!(deferred, Response::Ok { .. }));
    wait_for_hook_with_real_timeout(&waiting, "worker did not register deferred wait").await;

    let response = dispatch_request(
        &queue,
        Request::Put {
            request: request(8),
            immediate: true,
        },
    )
    .await;
    let Response::Ok {
        entry: Some(entry), ..
    } = response
    else {
        panic!("expected immediate entry, got {response:?}");
    };
    assert!(matches!(
        dispatch_request(&queue, Request::Bump { id: entry.id }).await,
        Response::Ok { .. }
    ));

    wait_for_hook_with_real_timeout(&enqueued, "worker did not claim the immediate entry").await;
    wait_for_hook_with_real_timeout(&idle, "worker did not post the immediate entry").await;
    assert_eq!(pending_entries(&queue).await.len(), 1);
    let remaining = pending_entries(&queue).await;
    assert_eq!(
        remaining.first().expect("deferred entry remains").pr_number,
        7
    );
    assert_eq!(
        server
            .received_requests()
            .await
            .expect("read requests")
            .len(),
        1
    );

    shutdown_tx.send(()).expect("signal shutdown");
    worker
        .await
        .expect("worker task should not panic")
        .expect("worker should exit cleanly");
}

#[tokio::test]
async fn deleting_an_in_flight_entry_makes_completion_idempotent() {
    let dir = tempdir().expect("create temporary queue directory");
    let queue = SharedQueue::open(Arc::new(Config::from(temp_config(&dir).with_cooldown(0))))
        .expect("open queue");
    let response = dispatch_request(
        &queue,
        Request::Put {
            request: request(9),
            immediate: true,
        },
    )
    .await;
    let Response::Ok {
        entry: Some(entry), ..
    } = response
    else {
        panic!("expected in-flight entry, got {response:?}");
    };

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(response_template().set_delay(Duration::from_millis(100)))
        .expect(1)
        .mount(&server)
        .await;
    let octocrab = octocrab_for(&server).expect("create GitHub client");
    let enqueued = Arc::new(Notify::new());
    let idle = Arc::new(Notify::new());
    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let worker = tokio::spawn(run_worker(
        Arc::clone(&queue),
        octocrab,
        WorkerControl::new(
            shutdown_rx,
            WorkerHooks {
                enqueued: Some(Arc::clone(&enqueued)),
                idle: Some(Arc::clone(&idle)),
                drained: None,
                waiting: None,
            },
        ),
    ));

    tokio::time::timeout(Duration::from_secs(5), enqueued.notified())
        .await
        .expect("worker should claim the entry");
    assert!(matches!(
        dispatch_request(&queue, Request::Del { id: entry.id }).await,
        Response::Ok { .. }
    ));
    tokio::time::timeout(Duration::from_secs(5), idle.notified())
        .await
        .expect("worker should finish the in-flight post");
    assert!(pending_entries(&queue).await.is_empty());

    shutdown_tx.send(()).expect("signal shutdown");
    worker
        .await
        .expect("worker task should not panic")
        .expect("worker should exit cleanly");
}
