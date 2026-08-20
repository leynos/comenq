//! Tests for task supervision and failure logging.

use super::{
    backoff, ensure_queue_dir, log_task_failure, queue_writer, spawn_listener, supervise_task,
    supervise_writer,
};
use ::metrics::set_default_local_recorder;
use anyhow::anyhow;
use metrics_util::debugging::{DebugValue, DebuggingRecorder};
use rstest::rstest;
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio::sync::{Notify, mpsc, watch};
use tokio::task::JoinError;
use yaque::{Receiver, Sender, SenderBuilder};

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
#[tokio::test(flavor = "current_thread")]
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
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _recorder_guard = set_default_local_recorder(&recorder);
    let supervisor = tokio::spawn(supervise_task(
        "worker",
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
    let metrics = snapshotter.snapshot().into_vec();
    assert!(metrics.iter().any(|(key, _, _, value)| {
        key.key().name() == "comenqd_task_restarts_total"
            && key
                .key()
                .labels()
                .any(|label| label.key() == "task" && label.value() == "worker")
            && matches!(value, DebugValue::Counter(2))
    }));
}

#[tokio::test(flavor = "current_thread")]
async fn queue_writer_records_depth_and_enqueue_failure() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let queue_path = dir.path().join("queue");
    let sender = SenderBuilder::new()
        .segment_size(1)
        .open(&queue_path)
        .expect("open queue sender");
    let (tx, rx) = mpsc::channel(2);
    tx.send(vec![1]).await.expect("queue first request");
    tx.send(vec![2]).await.expect("queue second request");
    drop(tx);
    fs::remove_dir_all(&queue_path).expect("remove queue directory");

    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _recorder_guard = set_default_local_recorder(&recorder);
    let _ = queue_writer(sender, rx).await;

    let metrics = snapshotter.snapshot().into_vec();
    assert!(
        metrics
            .iter()
            .any(|(key, _, _, _)| { key.key().name() == "comenqd_client_channel_depth" })
    );
    assert!(metrics.iter().any(|(key, _, _, value)| {
        key.key().name() == "comenqd_queue_writer_failures_total"
            && key
                .key()
                .labels()
                .any(|label| label.key() == "queue_side" && label.value() == "sender")
            && matches!(value, DebugValue::Counter(1))
    }));
}

#[tokio::test]
async fn listener_uses_replacement_sender_after_writer_panics() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let mut config: crate::config::Config = test_support::temp_config(&dir).into();
    config.restart_min_delay_ms = 1;
    let cfg = Arc::new(config);
    ensure_queue_dir(&cfg.queue_path)
        .await
        .expect("create queue directory");

    let initial_queue_sender = Sender::open(&cfg.queue_path).expect("open initial queue sender");
    let (initial_tx, initial_rx) = mpsc::channel(cfg.client_channel_capacity);
    let initial_sender = initial_tx.clone();
    let client_tx = Arc::new(tokio::sync::Mutex::new(initial_tx));
    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let writer: tokio::task::JoinHandle<mpsc::Receiver<Vec<u8>>> = tokio::spawn(async move {
        drop(initial_queue_sender);
        drop(initial_rx);
        panic!("writer panic for recovery test");
    });
    let writer_supervisor = tokio::spawn(supervise_writer(
        writer,
        backoff(Duration::from_millis(1)),
        cfg.clone(),
        client_tx.clone(),
        shutdown_tx.clone(),
        shutdown_rx.clone(),
    ));
    let listener = spawn_listener(cfg.clone(), client_tx.clone(), shutdown_rx.clone());

    tokio::time::timeout(Duration::from_secs(1), async {
        while !initial_sender.is_closed() {
            tokio::task::yield_now().await;
        }
        loop {
            if !client_tx.lock().await.is_closed() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("writer replacement should be ready");
    tokio::time::timeout(Duration::from_secs(1), async {
        while !cfg.socket_path.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("listener socket should be ready");

    let mut client = UnixStream::connect(&cfg.socket_path)
        .await
        .expect("connect to listener");
    let request = comenq_lib::CommentRequest {
        owner: "owner".into(),
        repo: "repo".into(),
        pr_number: 1,
        body: "body".into(),
    };
    client
        .write_all(&serde_json::to_vec(&request).expect("serialize request"))
        .await
        .expect("write request");
    client.shutdown().await.expect("close request");

    let mut receiver = Receiver::open(&cfg.queue_path).expect("open queue receiver");
    let queued = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
        .await
        .expect("request should reach replacement writer")
        .expect("receive queued request");
    let queued: comenq_lib::CommentRequest =
        serde_json::from_slice(&queued).expect("deserialize queued request");
    assert_eq!(queued, request);

    shutdown_tx.send(()).expect("signal shutdown");
    listener
        .await
        .expect("join listener")
        .expect("listener result");
    writer_supervisor.await.expect("join writer supervisor");
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
