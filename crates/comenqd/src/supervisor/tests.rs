//! Tests for task supervision and failure logging.

use super::observability::log_task_restart;
use super::{STABLE_TASK_RUN, backoff, log_task_failure, reset_backoff_after_stable_run};
use crate::config::Config;
use anyhow::anyhow;
use rstest::rstest;
use serde_json::Value;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::task::JoinError;

/// Convert a test configuration into the runtime configuration in all builds.
#[cfg(feature = "test-support")]
fn cfg_from(cfg: test_support::daemon::TestConfig) -> Config {
    Config::from(cfg)
}

#[cfg(not(feature = "test-support"))]
fn cfg_from(cfg: test_support::daemon::TestConfig) -> Config {
    Config {
        github_token: cfg.github_token,
        github_token_file: None,
        socket_path: cfg.socket_path,
        queue_path: cfg.queue_path,
        cooldown_period_seconds: cfg.cooldown_period_seconds,
        cooldown_flutter_seconds: 0,
        restart_min_delay_ms: cfg.restart_min_delay_ms,
        github_api_timeout_secs: cfg.github_api_timeout_secs,
    }
}

/// In-memory writer used to capture JSON-formatted tracing events.
#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Write for Buffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("lock buffer").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Create a [`JoinError`] representing a cancelled task.
///
/// The task awaits a future that can never complete, so `abort` always
/// cancels it. A current-thread runtime guarantees the task is not even
/// polled before the abort, because only `block_on` drives the executor.
/// A multi-threaded runtime would race the task against `abort`, letting
/// it finish first under load and yield `Ok(())` instead of a
/// [`JoinError`] (see issue #139).
fn create_cancelled_join_error() -> JoinError {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("create runtime")
        .block_on(async {
            let handle = tokio::spawn(std::future::pending::<()>());
            handle.abort();
            handle.await.expect_err("aborted task must be cancelled")
        })
}

#[rstest]
#[case(Ok(Ok(())), None)]
#[case(Ok(Err(anyhow!("boom"))), Some(("inner_error", "boom")))]
#[case(Err(create_cancelled_join_error()), Some(("join_error", "cancel")))]
fn logs_failures(
    #[case] res: std::result::Result<anyhow::Result<()>, JoinError>,
    #[case] expected: Option<(&str, &str)>,
) {
    use tracing_subscriber::prelude::*;

    let buf = Buffer::default();
    let writer = buf.clone();
    let subscriber = tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer()
            .json()
            .with_writer(move || writer.clone())
            .with_filter(tracing_subscriber::filter::LevelFilter::ERROR),
    );
    tracing::subscriber::with_default(subscriber, || {
        log_task_failure("task", &res);
    });

    let output = String::from_utf8(buf.0.lock().expect("read buffer").clone()).expect("utf8");
    match expected {
        None => assert!(output.is_empty()),
        Some((kind, err)) => {
            let line = output.lines().next().expect("log entry");
            let v: Value = serde_json::from_str(line).expect("json");
            let fields = &v["fields"];
            assert_eq!(fields["task"], "task");
            assert_eq!(fields["kind"], kind);
            assert!(fields["error"].as_str().expect("error str").contains(err));
            assert_eq!(fields["message"], "Task failed");
        }
    }
}

/// Verify restart events expose bounded timing and backoff decision fields.
#[rstest]
#[case::transient_failure(1, 250, 3_000)]
#[case::stable_run_reset(2, 100, 60_000)]
fn logs_restart_decisions(
    #[case] attempt: u64,
    #[case] selected_delay_ms: u64,
    #[case] stable_run_duration_ms: u64,
) {
    use tracing_subscriber::prelude::*;

    let buffer = Buffer::default();
    let writer = buffer.clone();
    let subscriber = tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer()
            .json()
            .with_writer(move || writer.clone())
            .with_filter(tracing_subscriber::filter::LevelFilter::WARN),
    );
    let stable_run_duration = Duration::from_millis(stable_run_duration_ms);
    let mut restart_backoff = backoff(Duration::from_millis(100));
    let backoff_reset = reset_backoff_after_stable_run(
        &mut restart_backoff,
        Duration::from_millis(100),
        stable_run_duration,
    );
    tracing::subscriber::with_default(subscriber, || {
        log_task_restart(
            "worker",
            attempt,
            Duration::from_millis(selected_delay_ms),
            stable_run_duration,
            backoff_reset,
        );
    });

    let output = String::from_utf8(buffer.0.lock().expect("read buffer").clone())
        .expect("log output is UTF-8");
    let event: Value = serde_json::from_str(output.lines().next().expect("restart event"))
        .expect("decode structured restart event");
    let fields = &event["fields"];
    assert_eq!(fields["task"], "worker");
    assert_eq!(fields["restart_attempt"].as_u64(), Some(attempt));
    assert_eq!(
        fields["selected_delay_ms"].as_u64(),
        Some(selected_delay_ms)
    );
    assert_eq!(
        fields["stable_run_duration_ms"].as_u64(),
        Some(stable_run_duration_ms)
    );
    assert_eq!(fields["backoff_reset"].as_bool(), Some(backoff_reset));
    assert_eq!(fields["message"], "Restarting task after failure");
}

/// Reset the delay sequence exactly when a task reaches the stable-run floor.
#[rstest]
#[case::transient(59, false)]
#[case::stable(STABLE_TASK_RUN.as_secs(), true)]
fn stable_run_controls_backoff_reset(#[case] seconds: u64, #[case] expected_reset: bool) {
    let mut restart_backoff = backoff(Duration::from_secs(1));
    let was_reset = reset_backoff_after_stable_run(
        &mut restart_backoff,
        Duration::from_secs(1),
        Duration::from_secs(seconds),
    );

    assert_eq!(was_reset, expected_reset);
}

/// The worker starts against the shared queue and shuts down cleanly.
#[rstest]
#[tokio::test]
async fn worker_starts_and_stops_cleanly() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let cfg = std::sync::Arc::new(cfg_from(test_support::temp_config(&dir)));
    super::ensure_queue_dir(&cfg.queue_path)
        .await
        .expect("create queue dir");
    let queue = crate::queue::SharedQueue::open(cfg).expect("open shared queue");
    let octocrab =
        std::sync::Arc::new(crate::worker::build_octocrab("token").expect("build octocrab"));
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(());
    let handle = super::spawn_worker(queue, octocrab, shutdown_rx);
    shutdown_tx.send(()).expect("signal shutdown");

    let res = tokio::time::timeout(std::time::Duration::from_secs(5), handle)
        .await
        .expect("worker should exit promptly")
        .expect("worker task should not panic");
    assert!(res.is_ok(), "worker must exit cleanly on shutdown: {res:?}");
}

mod restart;
