//! Tests listener socket setup and request handling.
use super::*;
use ::metrics::set_default_local_recorder;
use metrics_util::debugging::{DebugValue, DebuggingRecorder};
use std::fs::OpenOptions;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use tempfile::tempdir;
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, Notify, mpsc};

fn request_outcomes(
    metrics: &[(
        metrics_util::CompositeKey,
        Option<::metrics::Unit>,
        Option<::metrics::SharedString>,
        DebugValue,
    )],
) -> Vec<&str> {
    metrics
        .iter()
        .filter(|(key, _, _, _)| key.key().name() == "comenqd_requests_total")
        .flat_map(|(key, _, _, _)| key.key().labels())
        .filter_map(|label| (label.key() == "outcome").then_some(label.value()))
        .collect()
}

#[tokio::test]
async fn prepare_listener_creates_missing_parent_directory() {
    let dir = tempdir().expect("create tempdir");
    let sock = dir.path().join("missing/nested/comenq.sock");
    let listener = prepare_listener(&sock).expect("prepare listener");
    let meta = std::fs::symlink_metadata(&sock).expect("metadata");
    assert!(meta.file_type().is_socket());
    assert_eq!(meta.permissions().mode() & 0o777, 0o660);
    let parent = sock.parent().expect("socket parent");
    let parent_meta = std::fs::symlink_metadata(parent).expect("parent metadata");
    assert_eq!(parent_meta.permissions().mode() & 0o777, 0o700);
    let nested_parent = parent.parent().expect("nested socket parent");
    let nested_parent_meta =
        std::fs::symlink_metadata(nested_parent).expect("nested parent metadata");
    assert_eq!(nested_parent_meta.permissions().mode() & 0o777, 0o700);
    drop(listener);
}

#[tokio::test]
async fn prepare_listener_preserves_existing_parent_permissions() {
    let dir = tempdir().expect("create tempdir");
    let parent = dir.path().join("existing");
    std::fs::create_dir(&parent).expect("create parent");
    std::fs::set_permissions(&parent, stdfs::Permissions::from_mode(0o755))
        .expect("set parent permissions");

    let listener = prepare_listener(&parent.join("comenq.sock")).expect("prepare listener");
    let metadata = std::fs::symlink_metadata(&parent).expect("parent metadata");
    assert_eq!(metadata.permissions().mode() & 0o777, 0o755);
    drop(listener);
}

#[tokio::test]
async fn prepare_listener_prevents_pre_bind_race() {
    let dir = tempdir().expect("create tempdir");
    let sock = dir.path().join("sock");
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = Arc::clone(&stop);
    let sock_clone = sock.clone();
    let attacker = thread::spawn(move || {
        while !stop_clone.load(Ordering::SeqCst) {
            let _ = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&sock_clone);
            std::thread::yield_now();
        }
    });
    let listener = prepare_listener(&sock).expect("prepare listener");
    stop.store(true, Ordering::SeqCst);
    attacker.join().expect("attacker thread");
    // Avoid following symlinks when asserting the final on-disk type.
    let meta = std::fs::symlink_metadata(&sock).expect("metadata");
    assert!(meta.file_type().is_socket());
    drop(listener);
}

#[tokio::test(flavor = "current_thread")]
async fn handle_client_records_accepted_and_rejected_request_metrics() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _recorder_guard = set_default_local_recorder(&recorder);
    let (tx, mut rx) = mpsc::channel(2);

    let (mut valid_client, valid_server) = UnixStream::pair().expect("create valid pair");
    let request = CommentRequest {
        owner: "owner".into(),
        repo: "repo".into(),
        pr_number: 1,
        body: "body".into(),
    };
    valid_client
        .write_all(&serde_json::to_vec(&request).expect("serialize request"))
        .await
        .expect("write valid request");
    valid_client.shutdown().await.expect("close valid request");
    handle_client(valid_server, tx.clone())
        .await
        .expect("accept valid request");
    let _ = rx.recv().await.expect("receive valid request");

    let (mut invalid_client, invalid_server) = UnixStream::pair().expect("create invalid pair");
    invalid_client
        .write_all(b"not json")
        .await
        .expect("write invalid request");
    invalid_client
        .shutdown()
        .await
        .expect("close invalid request");
    assert!(handle_client(invalid_server, tx).await.is_err());

    let metrics = snapshotter.snapshot().into_vec();
    let outcomes = request_outcomes(&metrics);
    assert!(outcomes.contains(&"accepted"));
    assert!(outcomes.contains(&"rejected"));
}

#[tokio::test]
async fn in_flight_handler_uses_the_replacement_sender() {
    let (original_tx, original_rx) = mpsc::channel(1);
    let current_tx: ClientSender = Arc::new(Mutex::new(original_tx));
    let (mut client, server) = UnixStream::pair().expect("create Unix stream pair");
    let read_started = Arc::new(Notify::new());
    let started = Arc::clone(&read_started);
    let shared_tx = Arc::clone(&current_tx);
    let handler = tokio::spawn(async move {
        let result = handle_client_inner_with_before_read(server, shared_tx, move || {
            started.notify_one();
        })
        .await;
        record_request_outcome(result)
    });

    read_started.notified().await;
    let (replacement_tx, mut replacement_rx) = mpsc::channel(1);
    {
        let mut sender = current_tx.lock().await;
        *sender = replacement_tx;
    }
    drop(original_rx);

    let request = CommentRequest {
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

    handler
        .await
        .expect("join handler")
        .expect("handler uses replacement sender");
    let queued = replacement_rx
        .recv()
        .await
        .expect("receive replacement request");
    assert_eq!(
        serde_json::from_slice::<CommentRequest>(&queued).expect("deserialize request"),
        request
    );
}
