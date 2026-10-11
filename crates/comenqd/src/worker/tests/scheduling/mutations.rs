//! Verify queue reordering and deletion interrupt a deferred worker wait.

use super::{
    AdvancingClock, COOLDOWN_SECONDS, pending_entries, received_post_count, request,
    wait_for_hook_with_real_timeout, worker_with_successful_posts,
};
use crate::config::Config;
use crate::listener::dispatch_request;
use crate::queue::{SharedQueue, UnixClock};
use comenq_lib::protocol::{Request, Response};
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

async fn assert_queue_mutation_wakes_waiting_worker(mutation: Mutation) {
    let description = format!("{mutation:?}");
    let dir = tempdir().expect("create temporary queue directory");
    let clock = Arc::new(AdvancingClock(AtomicU64::new(1_000)));
    let queue_clock: Arc<dyn UnixClock> = clock;
    let queue = SharedQueue::open_with_clock(
        Arc::new(Config::from(
            temp_config(&dir).with_cooldown(COOLDOWN_SECONDS),
        )),
        queue_clock,
    )
    .expect("open queue");
    let deferred = dispatch_request(
        &queue,
        Request::Put {
            request: request(7),
            immediate: false,
        },
    )
    .await;
    let Response::Ok {
        entry: Some(deferred),
        ..
    } = deferred
    else {
        panic!("expected deferred entry, got {deferred:?}");
    };
    let immediate = dispatch_request(
        &queue,
        Request::Put {
            request: request(8),
            immediate: true,
        },
    )
    .await;
    let Response::Ok {
        entry: Some(immediate),
        ..
    } = immediate
    else {
        panic!("expected immediate entry, got {immediate:?}");
    };

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
    wait_for_hook_with_real_timeout(
        &waiting,
        &format!("{description}: worker did not register deferred wait"),
    )
    .await;

    let operation_request = match mutation {
        Mutation::Bump => Request::Bump { id: immediate.id },
        Mutation::Bust => Request::Bust { id: deferred.id },
        Mutation::Del => Request::Del { id: deferred.id },
    };
    let deletes_head = matches!(&operation_request, Request::Del { .. });
    assert!(
        matches!(
            dispatch_request(&queue, operation_request).await,
            Response::Ok { .. }
        ),
        "{description}: queue mutation should succeed"
    );
    wait_for_hook_with_real_timeout(
        &enqueued,
        &format!("{description}: worker did not claim newly exposed head"),
    )
    .await;
    wait_for_hook_with_real_timeout(
        &idle,
        &format!("{description}: worker did not finish exposed head"),
    )
    .await;

    assert_eq!(received_post_count(&server).await, 1);
    let remaining = pending_entries(&queue).await;
    if deletes_head {
        assert!(remaining.is_empty());
    } else {
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].pr_number, 7);
    }

    shutdown_tx.send(()).expect("signal shutdown");
    worker
        .await
        .expect("worker task should not panic")
        .expect("worker should exit cleanly");
}
