//! Tests for the queue worker's cooldown, flutter, and notification hooks.

use super::{
    Config, Notify, WorkerControl, WorkerHooks, build_octocrab, cooldown_with_flutter,
    cooldown_with_flutter_using, cooldown_with_selected_flutter, run_worker, wait_for_cooldown,
};
use ::metrics::set_default_local_recorder;
use metrics_util::debugging::{DebugValue, DebuggingRecorder};
use proptest::prelude::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use std::time::Instant;
use tokio::sync::watch;
use yaque::{Receiver, Sender};

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
fn non_zero_flutter_can_exceed_the_base_cooldown() {
    let cfg = config_with_flutter(60, 240);
    assert_eq!(cooldown_with_flutter_using(&cfg, |_| 240), 300);
}

proptest! {
    #[test]
    fn flutter_wait_stays_within_saturating_bounds(
        (cooldown, flutter, jitter) in (any::<u64>(), any::<u64>())
            .prop_flat_map(|(cooldown, flutter)| {
                (Just(cooldown), Just(flutter), 0..=flutter)
            })
    ) {
        let config = config_with_flutter(cooldown, flutter);
        let wait = cooldown_with_selected_flutter(&config, jitter);
        prop_assert_eq!(wait, cooldown.saturating_add(jitter));
        prop_assert!(wait >= cooldown);
        prop_assert!(wait <= cooldown.saturating_add(flutter));
    }
}

#[tokio::test(start_paused = true)]
async fn run_worker_waits_for_the_selected_flutter() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let mut config = config_with_flutter(60, 240);
    config.queue_path = dir.path().join("queue");
    crate::supervisor::ensure_queue_dir(&config.queue_path)
        .await
        .expect("create queue directory");
    let mut sender = Sender::open(&config.queue_path).expect("open queue sender");
    sender
        .send(b"not valid JSON".to_vec())
        .await
        .expect("enqueue malformed request");
    let receiver = Receiver::open(&config.queue_path).expect("open queue receiver");
    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let cooldown_complete = Arc::new(AtomicBool::new(false));
    let idle = Arc::new(Notify::new());
    let hooks = WorkerHooks {
        idle: Some(Arc::clone(&idle)),
        cooldown_complete: Some(Arc::clone(&cooldown_complete)),
        ..WorkerHooks::default()
    };
    let control = WorkerControl::new(shutdown_rx, hooks).with_test_flutter(240);
    let octocrab = Arc::new(build_octocrab("token").expect("build Octocrab"));
    let worker = tokio::spawn(run_worker(Arc::new(config), receiver, octocrab, control));

    idle.notified().await;
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(299)).await;
    assert!(!cooldown_complete.load(Ordering::SeqCst));
    tokio::time::advance(Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    assert!(cooldown_complete.load(Ordering::SeqCst));

    shutdown_tx.send(()).expect("signal shutdown");
    worker
        .await
        .expect("join worker")
        .expect("worker should exit cleanly");
}

#[tokio::test(flavor = "current_thread")]
async fn cooldown_wait_records_the_effective_flutter_duration() {
    let config = config_with_flutter(60, 240);
    let (shutdown_tx, mut shutdown) = watch::channel(());
    shutdown_tx.send(()).expect("signal shutdown");
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _recorder_guard = set_default_local_recorder(&recorder);

    assert!(wait_for_cooldown(&config, &mut shutdown, &WorkerHooks::default(), Some(240),).await);

    let metrics = snapshotter.snapshot().into_vec();
    assert!(metrics.iter().any(|(key, _, _, value)| {
        key.key().name() == "comenqd_cooldown_wait_duration_seconds"
            && matches!(value, DebugValue::Histogram(values) if values
                .iter()
                .any(|value| value.0.to_bits() == 300.0_f64.to_bits()))
    }));
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
