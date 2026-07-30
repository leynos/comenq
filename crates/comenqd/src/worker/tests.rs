//! Tests for the queue worker's cooldown, flutter, and notification hooks.

use super::{Config, Notify, WorkerHooks, cooldown_with_flutter};
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use tokio::sync::watch;

/// Build a minimal config with the given cooldown and flutter.
fn config_with_flutter(cooldown: u64, flutter: u64) -> Config {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("create tempdir: {e}"));
    let mut cfg = Config::from(test_support::temp_config(&dir).with_cooldown(cooldown));
    cfg.cooldown_flutter_seconds = flutter;
    cfg
}

#[test]
fn zero_flutter_leaves_cooldown_unchanged() {
    let cfg = config_with_flutter(960, 0);
    assert_eq!(cooldown_with_flutter(&cfg), 960);
}

#[test]
fn flutter_only_lengthens_the_cooldown() {
    let cfg = config_with_flutter(60, 240);
    for _ in 0..200 {
        let wait = cooldown_with_flutter(&cfg);
        assert!(
            (60..=300).contains(&wait),
            "wait {wait} outside [cooldown, cooldown + flutter]"
        );
    }
}

#[test]
fn flutter_saturates_instead_of_overflowing() {
    let cfg = config_with_flutter(u64::MAX, 1);
    assert_eq!(cooldown_with_flutter(&cfg), u64::MAX);
}

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
    use std::time::Duration;

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
