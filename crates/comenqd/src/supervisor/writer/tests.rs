//! Tests queue-writer recovery without dropping accepted client payloads.

use super::{
    MAX_WRITER_RESTARTS, QueueWriterExit, QueueWriterState, queue_writer, run_queue_writer,
    run_queue_writer_with_after_enqueue, spawn_queue_writer, supervise_writer,
};
use crate::supervisor::ensure_queue_dir;
use ::metrics::set_default_local_recorder;
use metrics_util::debugging::{DebugValue, DebuggingRecorder};
use rstest::fixture;
use std::fs;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::{Notify, mpsc, watch};
use yaque::{Receiver, Sender, SenderBuilder};

struct WriterTestSetup {
    _dir: TempDir,
    cfg: Arc<crate::config::Config>,
    sender: Sender,
}

#[fixture]
async fn writer_test_setup() -> WriterTestSetup {
    let dir = tempfile::tempdir().expect("create tempdir");
    let cfg: Arc<crate::config::Config> = Arc::new(test_support::temp_config(&dir).into());
    ensure_queue_dir(&cfg.queue_path)
        .await
        .expect("create queue directory");
    let sender = Sender::open(&cfg.queue_path).expect("open queue sender");
    WriterTestSetup {
        _dir: dir,
        cfg,
        sender,
    }
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
            .any(|(key, _, _, _)| key.key().name() == "comenqd_client_channel_depth")
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
async fn queue_writer_retries_failed_payload() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let queue_path = dir.path().join("queue");
    let sender = SenderBuilder::new()
        .segment_size(1)
        .open(&queue_path)
        .expect("open initial queue sender");
    let payload = b"preserve this request".to_vec();
    let (input_tx, input_rx) = mpsc::channel(2);
    input_tx
        .send(vec![0])
        .await
        .expect("advance the initial queue segment");
    input_tx
        .send(payload.clone())
        .await
        .expect("queue payload for writer");
    drop(input_tx);
    fs::remove_dir_all(&queue_path).expect("force enqueue failure");

    let QueueWriterExit::EnqueueFailed(state) =
        run_queue_writer(sender, QueueWriterState::new(input_rx)).await
    else {
        panic!("removed queue sender must fail enqueue");
    };
    assert_eq!(
        state.pending.lock().await.as_deref(),
        Some(payload.as_slice())
    );
    let sender = Sender::open(&queue_path).expect("reopen queue sender");
    assert!(matches!(
        run_queue_writer(sender, state).await,
        QueueWriterExit::ClientChannelClosed
    ));
    let mut receiver = Receiver::open(&queue_path).expect("open queue receiver");
    let queued = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
        .await
        .expect("retry should enqueue payload")
        .expect("receive retried payload");
    assert_eq!(&*queued, payload.as_slice());
    queued.commit().expect("commit retried payload");
    assert!(
        tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .is_err(),
        "the completed retry does not enqueue a second payload"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn writer_exits_when_all_client_senders_are_dropped(
    #[future(awt)] writer_test_setup: WriterTestSetup,
) {
    let WriterTestSetup { cfg, sender, .. } = writer_test_setup;
    let (client_tx, client_rx) = mpsc::channel(1);
    drop(client_tx);
    let writer = spawn_queue_writer(sender, QueueWriterState::new(client_rx), &cfg.queue_path, 0);
    let (shutdown_tx, shutdown_rx) = watch::channel(());

    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _recorder_guard = set_default_local_recorder(&recorder);
    tokio::time::timeout(
        Duration::from_secs(1),
        supervise_writer(
            writer,
            QueueWriterState::new(mpsc::channel(1).1),
            std::iter::empty(),
            cfg,
            |path| Sender::open(path).map_err(anyhow::Error::from),
            shutdown_tx,
            shutdown_rx,
        ),
    )
    .await
    .expect("supervisor exits after normal client channel closure");

    assert!(
        snapshotter
            .snapshot()
            .into_vec()
            .iter()
            .all(|(key, _, _, _)| { key.key().name() != "comenqd_task_restarts_total" })
    );
}

#[rstest::rstest]
#[tokio::test]
async fn shutdown_after_enqueue_preserves_the_persisted_payload(
    #[future(awt)] writer_test_setup: WriterTestSetup,
) {
    let WriterTestSetup { cfg, sender, .. } = writer_test_setup;
    let (client_tx, client_rx) = mpsc::channel(1);
    let payload = b"persist before shutdown".to_vec();
    client_tx
        .send(payload.clone())
        .await
        .expect("queue payload");
    let state = QueueWriterState::new(client_rx);
    let recovery_state = state.clone();
    let enqueued = Arc::new(Notify::new());
    let continue_after_enqueue = Arc::new(Notify::new());
    let notify_enqueued = Arc::clone(&enqueued);
    let wait_for_shutdown = Arc::clone(&continue_after_enqueue);
    let writer = tokio::spawn(run_queue_writer_with_after_enqueue(
        sender,
        state,
        move || {
            let notify_enqueued = Arc::clone(&notify_enqueued);
            let wait_for_shutdown = Arc::clone(&wait_for_shutdown);
            async move {
                notify_enqueued.notify_one();
                wait_for_shutdown.notified().await;
            }
        },
    ));
    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let supervisor = tokio::spawn(supervise_writer(
        writer,
        recovery_state,
        std::iter::empty(),
        cfg.clone(),
        |path| Sender::open(path).map_err(anyhow::Error::from),
        shutdown_tx.clone(),
        shutdown_rx,
    ));

    enqueued.notified().await;
    shutdown_tx.send(()).expect("signal shutdown");
    supervisor.await.expect("join writer supervisor");

    let mut receiver = Receiver::open(&cfg.queue_path).expect("open queue receiver");
    let queued = receiver.recv().await.expect("receive persisted payload");
    assert_eq!(&*queued, payload.as_slice());
    queued.commit().expect("commit persisted payload");
    assert!(
        tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .is_err(),
        "shutdown must not duplicate the payload that was already persisted"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn cancelled_writer_releases_the_receiver_for_recovery(
    #[future(awt)] writer_test_setup: WriterTestSetup,
) {
    let WriterTestSetup { cfg, sender, .. } = writer_test_setup;
    let (client_tx, client_rx) = mpsc::channel(1);
    let state = QueueWriterState::new(client_rx);
    let recovery_state = state.clone();
    let writer = tokio::spawn(run_queue_writer(sender, state));

    tokio::task::yield_now().await;
    writer.abort();
    let join_error = match writer.await {
        Ok(_) => panic!("cancelled writer must not complete"),
        Err(error) => error,
    };
    assert!(join_error.is_cancelled());

    let payload = b"recover after cancellation".to_vec();
    client_tx
        .send(payload.clone())
        .await
        .expect("queue payload after cancellation");
    drop(client_tx);
    let replacement_sender = Sender::open(&cfg.queue_path).expect("open replacement sender");
    assert!(matches!(
        run_queue_writer(replacement_sender, recovery_state).await,
        QueueWriterExit::ClientChannelClosed
    ));

    let mut receiver = Receiver::open(&cfg.queue_path).expect("open queue receiver");
    let queued = receiver.recv().await.expect("receive recovered payload");
    assert_eq!(&*queued, payload.as_slice());
    queued.commit().expect("commit recovered payload");
}

#[tokio::test]
async fn writer_panic_preserves_buffered_client_request() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let mut config: crate::config::Config = test_support::temp_config(&dir).into();
    config.restart_min_delay_ms = 1;
    let cfg = Arc::new(config);
    ensure_queue_dir(&cfg.queue_path)
        .await
        .expect("create queue directory");

    let initial_queue_sender = Sender::open(&cfg.queue_path).expect("open initial queue sender");
    let (client_tx, client_rx) = mpsc::channel(cfg.client_channel_capacity);
    let writer_state = QueueWriterState::new(client_rx);
    let request = comenq_lib::CommentRequest {
        owner: "owner".into(),
        repo: "repo".into(),
        pr_number: 1,
        body: "body".into(),
    };
    client_tx
        .send(serde_json::to_vec(&request).expect("serialize request"))
        .await
        .expect("queue client request before writer panic");
    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let writer = tokio::spawn(async move {
        drop(initial_queue_sender);
        panic!("writer panic for recovery test");
    });
    let writer_supervisor = tokio::spawn(supervise_writer(
        writer,
        writer_state,
        std::iter::once(Duration::from_millis(1)),
        cfg.clone(),
        |path| Sender::open(path).map_err(anyhow::Error::from),
        shutdown_tx.clone(),
        shutdown_rx.clone(),
    ));

    let mut receiver = Receiver::open(&cfg.queue_path).expect("open queue receiver");
    let queued_guard = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
        .await
        .expect("request should reach recovered writer")
        .expect("receive queued request");
    let queued: comenq_lib::CommentRequest =
        serde_json::from_slice(&queued_guard).expect("deserialize queued request");
    assert_eq!(queued, request);
    queued_guard.commit().expect("commit recovered queue entry");
    assert!(
        tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .is_err(),
        "recovered writer must not duplicate the buffered request"
    );

    shutdown_tx.send(()).expect("signal shutdown");
    writer_supervisor.await.expect("join writer supervisor");
}

#[tokio::test]
async fn writer_retries_retained_payload_after_sender_open_failure() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let cfg: Arc<crate::config::Config> = Arc::new(test_support::temp_config(&dir).into());
    ensure_queue_dir(&cfg.queue_path)
        .await
        .expect("create queue directory");
    let initial_sender = Sender::open(&cfg.queue_path).expect("open initial queue sender");
    let (client_tx, client_rx) = mpsc::channel(1);
    let payload = b"retry after sender open failure".to_vec();
    client_tx
        .send(payload.clone())
        .await
        .expect("queue client request");
    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let writer = tokio::spawn(async move {
        drop(initial_sender);
        panic!("writer panic before queue reopen");
    });
    let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let open_attempts = Arc::clone(&attempts);
    let supervisor = tokio::spawn(supervise_writer(
        writer,
        QueueWriterState::new(client_rx),
        std::iter::repeat(Duration::from_millis(1)),
        cfg.clone(),
        move |path| {
            if open_attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                anyhow::bail!("forced sender-open failure");
            }
            Sender::open(path).map_err(anyhow::Error::from)
        },
        shutdown_tx.clone(),
        shutdown_rx,
    ));

    let mut receiver = Receiver::open(&cfg.queue_path).expect("open queue receiver");
    let queued = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
        .await
        .expect("recovered writer should enqueue payload")
        .expect("receive queued payload");
    assert_eq!(&*queued, payload.as_slice());
    queued.commit().expect("commit queued payload");
    assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);

    shutdown_tx.send(()).expect("signal shutdown");
    supervisor.await.expect("join writer supervisor");
}

#[tokio::test]
async fn writer_restart_limit_signals_shutdown() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let cfg: Arc<crate::config::Config> = Arc::new(test_support::temp_config(&dir).into());
    let (_client_tx, client_rx) = mpsc::channel(1);
    let (shutdown_tx, mut shutdown_rx) = watch::channel(());
    let writer = tokio::spawn(async { panic!("writer panic for retry-limit test") });
    let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let open_attempts = Arc::clone(&attempts);
    let supervisor = tokio::spawn(supervise_writer(
        writer,
        QueueWriterState::new(client_rx),
        std::iter::repeat_n(Duration::ZERO, MAX_WRITER_RESTARTS),
        cfg,
        move |_| {
            open_attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            anyhow::bail!("forced sender-open failure");
        },
        shutdown_tx,
        shutdown_rx.clone(),
    ));

    shutdown_rx
        .changed()
        .await
        .expect("supervisor signals shutdown");
    supervisor.await.expect("join writer supervisor");
    assert_eq!(
        attempts.load(std::sync::atomic::Ordering::SeqCst),
        MAX_WRITER_RESTARTS
    );
}
