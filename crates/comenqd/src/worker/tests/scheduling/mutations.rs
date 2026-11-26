//! Verify queue reordering and deletion interrupt a deferred worker wait.

use super::{
    AdvancingClock, COOLDOWN_SECONDS, pending_entries, received_post_count, request,
    wait_for_hook_with_real_timeout, worker_with_successful_posts,
};
use crate::config::Config;
use crate::listener::dispatch_request;
use crate::queue::{SharedQueue, UnixClock};
use comenq_lib::protocol::{PendingEntry, Request, Response};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use tempfile::tempdir;
use test_support::temp_config;
use tokio::sync::Notify;
use wiremock::MockServer;

#[tokio::test(start_paused = true)]
async fn bump_bust_and_del_wake_a_worker_waiting_for_a_deferred_entry() {
    for mutation in [Mutation::Bump, Mutation::Bust, Mutation::Del] {
        assert_queue_mutation_wakes_waiting_worker(mutation).await;
    }
}

#[derive(Clone, Copy, Debug)]
enum Mutation {
    Bump,
    Bust,
    Del,
}

/// Open a queue in `dir` driven by an advancing clock.
fn open_queue(dir: &tempfile::TempDir) -> Arc<SharedQueue> {
    let clock = Arc::new(AdvancingClock(AtomicU64::new(1_000)));
    let queue_clock: Arc<dyn UnixClock> = clock;
    SharedQueue::open_with_clock(
        Arc::new(Config::from(
            temp_config(dir).with_cooldown(COOLDOWN_SECONDS),
        )),
        queue_clock,
    )
    .expect("open queue")
}

/// Enqueue the comment for `pr_number` and return the entry the queue reports.
async fn put_entry(queue: &SharedQueue, pr_number: u64, immediate: bool) -> PendingEntry {
    let response = dispatch_request(
        queue,
        Request::Put {
            request: request(pr_number),
            immediate,
        },
    )
    .await;
    let Response::Ok {
        entry: Some(entry), ..
    } = response
    else {
        panic!("expected entry for PR {pr_number}, got {response:?}");
    };
    entry
}

/// The request that applies `mutation` to the deferred or immediate entry.
fn mutation_request(
    mutation: Mutation,
    deferred: &PendingEntry,
    immediate: &PendingEntry,
) -> Request {
    match mutation {
        Mutation::Bump => Request::Bump {
            id: immediate.id.clone(),
        },
        Mutation::Bust => Request::Bust {
            id: deferred.id.clone(),
        },
        Mutation::Del => Request::Del {
            id: deferred.id.clone(),
        },
    }
}

/// Check what is left in the queue once the exposed head has been posted.
async fn assert_remaining_entries(queue: &SharedQueue, deletes_head: bool) {
    let remaining = pending_entries(queue).await;
    if deletes_head {
        assert!(remaining.is_empty());
    } else {
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].pr_number, 7);
    }
}

async fn assert_queue_mutation_wakes_waiting_worker(mutation: Mutation) {
    let description = format!("{mutation:?}");
    let dir = tempdir().expect("create temporary queue directory");
    let queue = open_queue(&dir);
    let deferred = put_entry(&queue, 7, false).await;
    let immediate = put_entry(&queue, 8, true).await;

    let harness = start_waiting_worker(&queue, &description).await;

    let operation_request = mutation_request(mutation, &deferred, &immediate);
    let deletes_head = matches!(&operation_request, Request::Del { .. });
    apply_mutation(&queue, operation_request, &description).await;
    await_exposed_head(&harness.enqueued, &harness.idle, &description).await;

    assert_eq!(received_post_count(&harness.server).await, 1);
    assert_remaining_entries(&queue, deletes_head).await;
    stop_worker(&harness.shutdown_tx, harness.worker).await;
}

/// A running worker plus the hooks and server the scenario observes.
struct WaitingWorker {
    server: MockServer,
    enqueued: Arc<Notify>,
    idle: Arc<Notify>,
    shutdown_tx: tokio::sync::watch::Sender<()>,
    worker: tokio::task::JoinHandle<anyhow::Result<()>>,
}

/// Start a worker over `queue` and wait until it is waiting on a deferred entry.
async fn start_waiting_worker(queue: &Arc<SharedQueue>, description: &str) -> WaitingWorker {
    let server = MockServer::start().await;
    let enqueued = Arc::new(Notify::new());
    let idle = Arc::new(Notify::new());
    let drained = Arc::new(Notify::new());
    let waiting = Arc::new(Notify::new());
    let (shutdown_tx, worker) = worker_with_successful_posts(
        Arc::clone(queue),
        &server,
        1,
        Arc::clone(&enqueued),
        Arc::clone(&idle),
        Arc::clone(&drained),
        Arc::clone(&waiting),
    )
    .await;
    wait_for_hook_with_real_timeout(
        &waiting,
        &format!("{description}: worker did not register deferred wait"),
    )
    .await;
    WaitingWorker {
        server,
        enqueued,
        idle,
        shutdown_tx,
        worker,
    }
}

/// Apply `operation` to the queue and require it to succeed.
async fn apply_mutation(queue: &SharedQueue, operation: Request, description: &str) {
    assert!(
        matches!(
            dispatch_request(queue, operation).await,
            Response::Ok { .. }
        ),
        "{description}: queue mutation should succeed"
    );
}

/// Wait for the worker to claim, then finish, the head the mutation exposed.
async fn await_exposed_head(enqueued: &Notify, idle: &Notify, description: &str) {
    wait_for_hook_with_real_timeout(
        enqueued,
        &format!("{description}: worker did not claim newly exposed head"),
    )
    .await;
    wait_for_hook_with_real_timeout(
        idle,
        &format!("{description}: worker did not finish exposed head"),
    )
    .await;
}

/// Signal shutdown and require the worker task to exit cleanly.
async fn stop_worker(
    shutdown_tx: &tokio::sync::watch::Sender<()>,
    worker: tokio::task::JoinHandle<anyhow::Result<()>>,
) {
    shutdown_tx.send(()).expect("signal shutdown");
    worker
        .await
        .expect("worker task should not panic")
        .expect("worker should exit cleanly");
}
