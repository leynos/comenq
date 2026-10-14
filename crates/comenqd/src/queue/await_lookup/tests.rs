//! One schedule build per version, even when many waiters query concurrently.

use super::*;
use crate::config::Config;
use crate::queue::UnixClock;
use comenq_lib::CommentRequest;
use std::sync::Arc;
use tempfile::tempdir;
use test_support::temp_config;

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
            AwaitState::Pending {
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
        AwaitState::Pending {
            eta_seconds: 60,
            ..
        }
    ));
    let next_builds = queue.store.lock().expect("lock").projection_builds;
    assert_eq!(next_builds, 2);
}
