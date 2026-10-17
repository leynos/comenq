//! One schedule build per version, even when many waiters query concurrently.

use super::*;
use crate::config::Config;
use crate::queue::UnixClock;
use comenq_lib::CommentRequest;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::Duration;
use tempfile::tempdir;
use test_support::temp_config;
use tokio::sync::oneshot;

#[derive(Debug)]
struct FixedClock;
impl UnixClock for FixedClock {
    fn unix_now(&self) -> u64 {
        1000
    }
}

#[tokio::test]
async fn waiters_share_a_projection_and_mutations_invalidate_it() {
    let dir = tempdir().expect("tempdir");
    let queue = SharedQueue::open_with_clock(
        Arc::new(Config::from(temp_config(&dir).with_cooldown(60))),
        Arc::new(FixedClock),
    )
    .expect("queue");
    let (entry, _) = queue
        .put(
            CommentRequest {
                owner: "o".into(),
                repo: "r".into(),
                pr_number: 1,
                body: "cache".into(),
            },
            false,
        )
        .await
        .expect("put");
    let mut waiters = Vec::new();
    for _ in 0..64 {
        let queue = queue.clone();
        let id = entry.id.clone();
        waiters.push(tokio::spawn(
            async move { queue.resolve_await(&id, None).await },
        ));
    }
    for waiter in waiters {
        let state = waiter.await.expect("join").expect("lookup");
        assert!(matches!(
            state,
            AwaitOutcome::Pending {
                eta_seconds: 60,
                ..
            }
        ));
    }
    let first_builds = queue.store.lock().expect("lock").projection_builds;
    assert_eq!(first_builds, 1);
    queue.bump(&entry.id).await.expect("mutation");
    let updated = queue
        .resolve_await(&entry.id, None)
        .await
        .expect("lookup after mutation");
    assert!(matches!(
        updated,
        AwaitOutcome::Pending {
            eta_seconds: 60,
            ..
        }
    ));
    let next_builds = queue.store.lock().expect("lock").projection_builds;
    assert_eq!(next_builds, 2);
}

#[tokio::test]
async fn history_scan_does_not_hold_the_queue_lock_and_rechecks_mutations() {
    let dir = tempdir().expect("tempdir");
    let queue = SharedQueue::open_with_clock(
        Arc::new(Config::from(temp_config(&dir).with_cooldown(60))),
        Arc::new(FixedClock),
    )
    .expect("queue");
    let request = CommentRequest {
        owner: "o".into(),
        repo: "r".into(),
        pr_number: 1,
        body: "unlocked history scan".into(),
    };
    let (first, _) = queue.put(request.clone(), true).await.expect("initial put");
    queue.del(&first.id).await.expect("delete initial entry");

    let (started_tx, started_rx) = oneshot::channel();
    let started_tx = Arc::new(Mutex::new(Some(started_tx)));
    let (release_tx, release_rx) = mpsc::channel();
    let release_rx = Arc::new(Mutex::new(release_rx));
    let first_scan = Arc::new(AtomicBool::new(true));
    let history_lookup = {
        let started_tx = Arc::clone(&started_tx);
        let release_rx = Arc::clone(&release_rx);
        let first_scan = Arc::clone(&first_scan);
        move |store: crate::store::QueueStore,
              id: String,
              cursor: Option<crate::store::AwaitCursor>| {
            let outcome = store.resolve_await_completion(&id, cursor.as_ref());
            if first_scan.swap(false, Ordering::AcqRel) {
                if let Some(started) = started_tx.lock().expect("start lock").take() {
                    let _ = started.send(());
                }
                release_rx
                    .lock()
                    .expect("release lock")
                    .recv()
                    .expect("release history lookup");
            }
            outcome
        }
    };
    let wait_queue = Arc::clone(&queue);
    let id = first.id.clone();
    let waiter = tokio::spawn(async move {
        wait_queue
            .resolve_await_store_with_history(&id, None, history_lookup)
            .await
    });
    started_rx.await.expect("history scan started");

    let posted_url = "https://github.com/o/r/issues/1#issuecomment-42";
    let enqueue_and_complete = tokio::time::timeout(Duration::from_secs(2), async {
        let (replacement, _) = queue
            .put(request, true)
            .await
            .expect("enqueue replacement during scan");
        assert_eq!(replacement.id, first.id);
        queue
            .complete(&replacement, "hash", posted_url)
            .await
            .expect("complete replacement during scan");
    })
    .await;
    let _ = release_tx.send(());
    enqueue_and_complete.expect("queue mutations proceed during history scan");

    assert_eq!(
        waiter.await.expect("waiter task").expect("lookup"),
        AwaitState::Posted {
            comment_url: Some(posted_url.into()),
            posted_at: 1000,
        }
    );
}
