//! Tests for task supervision and failure logging.

use super::{log_task_failure, supervise_task};
use anyhow::anyhow;
use rstest::rstest;
use serde_json::Value;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{Notify, watch};
use tokio::task::JoinError;

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

/// Consecutive failures must advance the same backoff iterator.
#[tokio::test]
async fn consecutive_failures_use_increasing_restart_delays() {
    struct RecordingBackoff {
        delays: Vec<Duration>,
        observed: Arc<Mutex<Vec<Duration>>>,
    }

    impl Iterator for RecordingBackoff {
        type Item = Duration;

        fn next(&mut self) -> Option<Self::Item> {
            let delay = self.delays.pop()?;
            self.observed.lock().expect("record delay").push(delay);
            Some(delay)
        }
    }

    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let spawned_twice = Arc::new(Notify::new());
    let respawn_shutdown = shutdown_rx.clone();
    let respawn_signal = Arc::clone(&spawned_twice);
    let observed = Arc::new(Mutex::new(Vec::new()));
    let supervisor = tokio::spawn(supervise_task(
        "test",
        tokio::spawn(async { Err(anyhow!("first failure")) }),
        RecordingBackoff {
            delays: vec![Duration::from_millis(2), Duration::from_millis(1)],
            observed: Arc::clone(&observed),
        },
        move |attempt| {
            let mut shutdown = respawn_shutdown.clone();
            let signal = Arc::clone(&respawn_signal);
            tokio::spawn(async move {
                if attempt == 1 {
                    Err(anyhow!("second failure"))
                } else {
                    signal.notify_one();
                    let _ = shutdown.changed().await;
                    Ok(())
                }
            })
        },
        shutdown_rx,
    ));

    tokio::time::timeout(Duration::from_secs(1), spawned_twice.notified())
        .await
        .expect("task should restart twice");
    shutdown_tx.send(()).expect("signal shutdown");
    supervisor.await.expect("join supervisor");

    let delays = observed.lock().expect("read delays");
    assert_eq!(
        delays.as_slice(),
        &[Duration::from_millis(1), Duration::from_millis(2)]
    );
}

/// The worker must start while the queue writer holds the sender.
///
/// Regression test for the daemon's startup topology: the writer owns the
/// queue's `yaque::Sender` and the worker must open only the `Receiver`.
/// Opening a full `channel()` on both sides contends for yaque's per-side
/// lock files and left the worker in a permanent restart loop.
#[rstest]
#[tokio::test]
async fn worker_starts_while_writer_holds_the_sender() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let cfg: std::sync::Arc<crate::config::Config> =
        std::sync::Arc::new(test_support::temp_config(&dir).into());
    super::ensure_queue_dir(&cfg.queue_path)
        .await
        .expect("create queue dir");
    let _sender = yaque::Sender::open(&cfg.queue_path).expect("open queue sender");

    let octocrab =
        std::sync::Arc::new(crate::worker::build_octocrab("token").expect("build octocrab"));
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(());
    let handle = super::spawn_worker(cfg, octocrab, shutdown_rx, 0);
    shutdown_tx.send(()).expect("signal shutdown");

    let res = tokio::time::timeout(std::time::Duration::from_secs(5), handle)
        .await
        .expect("worker should exit promptly")
        .expect("worker task should not panic");
    assert!(
        res.is_ok(),
        "worker must open the queue receiver while the sender is held: {res:?}"
    );
}
