//! Tests for the worker's shutdown waits and notification hooks.

use super::{Notify, WorkerHooks};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::watch;

#[tokio::test]
async fn wait_or_shutdown_returns_false_on_timeout() {
    let (_tx, mut rx) = watch::channel(());
    let start = Instant::now();
    let result = WorkerHooks::wait_or_shutdown(0, &mut rx).await;
    assert!(!result, "should return false when timeout expires");
    assert!(
        start.elapsed().as_millis() < 500,
        "zero-second wait should return immediately"
    );
}

#[tokio::test]
async fn wait_or_shutdown_returns_true_on_shutdown() {
    let (tx, mut rx) = watch::channel(());
    // Signal shutdown before waiting
    tx.send(()).expect("send shutdown signal");
    let result = WorkerHooks::wait_or_shutdown(60, &mut rx).await;
    assert!(result, "should return true when shutdown is signalled");
}

#[tokio::test]
async fn wait_or_shutdown_prioritises_shutdown_over_timeout() {
    let (tx, mut rx) = watch::channel(());
    // Send shutdown signal
    tx.send(()).expect("send shutdown signal");
    // Even with zero timeout, shutdown should be detected due to biased select
    let result = WorkerHooks::wait_or_shutdown(0, &mut rx).await;
    assert!(result, "biased select should prioritize shutdown signal");
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

mod token_rotation {
    //! Tests for token hashing and round-robin selection.

    use super::super::{TokenClient, build_octocrab, next_token_index, token_hash};
    use rstest::rstest;
    use std::sync::Arc;

    fn clients() -> Vec<TokenClient> {
        ["alpha-token", "bravo-token"]
            .iter()
            .map(|token| {
                let octocrab = Arc::new(build_octocrab(token).expect("build octocrab"));
                TokenClient::new(format!("{token}-file"), token, octocrab)
            })
            .collect()
    }

    #[rstest]
    fn token_hash_is_deterministic_hex_sha256() {
        let hash = token_hash("s3cret");
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(hash, token_hash("s3cret"));
        assert_ne!(hash, token_hash("other"));
        // Known SHA-256 of "s3cret" pins the algorithm.
        assert_eq!(
            hash,
            "1ec1c26b50d5d3c58d9583181af8076655fe00756bf7285940ba3670f99fcba0"
        );
    }

    #[rstest]
    #[tokio::test]
    async fn empty_history_selects_the_first_token() {
        assert_eq!(next_token_index(&clients(), None), 0);
    }

    #[rstest]
    #[tokio::test]
    async fn rotation_advances_past_the_last_used_token() {
        let clients = clients();
        let first_hash = token_hash("alpha-token");
        assert_eq!(next_token_index(&clients, Some(&first_hash)), 1);
    }

    #[rstest]
    #[tokio::test]
    async fn rotation_wraps_from_the_last_token_to_the_first() {
        let clients = clients();
        let second_hash = token_hash("bravo-token");
        assert_eq!(next_token_index(&clients, Some(&second_hash)), 0);
    }

    #[rstest]
    #[tokio::test]
    async fn unknown_hashes_fall_back_to_the_first_token() {
        assert_eq!(next_token_index(&clients(), Some("not-a-hash")), 0);
    }
}
