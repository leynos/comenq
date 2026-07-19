//! Tests for daemon components and worker behaviour.

mod util;

use comenq_lib::protocol::{PendingEntry, Request, Response};
use comenqd::config::Config;
use comenqd::daemon::{
    SharedQueue, WorkerControl, WorkerHooks,
    listener::{handle_client, prepare_listener, run_listener},
    run, run_worker,
};
use rstest::{fixture, rstest};
use std::fs as stdfs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tempfile::{TempDir, tempdir};
use test_support::{octocrab_for, temp_config};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::{Notify, watch};
use tokio::time::sleep;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use util::{TestComplexity, TimeoutConfig, join_err, timeout_with_retries};

const TEST_COOLDOWN_SECONDS: u64 = 1;

/// Convert a test configuration into the runtime `Config`.
///
/// The conversion normally relies on the crate's `test-support` feature.
/// When that feature is disabled (e.g. during coverage builds), perform the
/// field mapping manually to avoid pulling in optional code.
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

fn sample_request() -> comenq_lib::CommentRequest {
    comenq_lib::CommentRequest {
        owner: "o".into(),
        repo: "r".into(),
        pr_number: 1,
        body: "b".into(),
    }
}

/// Enqueue the sample request and return its reported entry.
async fn seed_queue(queue: &Arc<SharedQueue>) -> PendingEntry {
    match queue
        .execute(Request::Put {
            request: sample_request(),
        })
        .await
    {
        Response::Ok {
            entry: Some(entry), ..
        } => entry,
        other => panic!("expected put to succeed, got {other:?}"),
    }
}

/// Pending entries as reported by the daemon.
async fn list_entries(queue: &Arc<SharedQueue>) -> Vec<PendingEntry> {
    match queue.execute(Request::List).await {
        Response::Ok {
            entries: Some(entries),
            ..
        } => entries,
        other => panic!("expected list to succeed, got {other:?}"),
    }
}

/// Write `request` over `stream`, close the write side, and parse the reply.
async fn roundtrip(mut stream: UnixStream, request: &Request) -> Response {
    let payload = serde_json::to_vec(request).expect("serialize request");
    stream.write_all(&payload).await.expect("write request");
    stream
        .shutdown()
        .await
        .expect("close write side of connection");
    let mut reply = Vec::new();
    stream.read_to_end(&mut reply).await.expect("read reply");
    serde_json::from_slice(&reply).expect("parse reply")
}

async fn wait_for_file(path: &Path, tries: u32, delay: Duration) -> bool {
    for _ in 0..tries {
        if path.exists() {
            return true;
        }
        sleep(delay).await;
    }
    path.exists()
}

#[tokio::test]
async fn ensure_queue_dir_creates_directory() {
    let dir = tempdir().expect("Failed to create temporary directory");
    let path = dir.path().join("queue");
    comenqd::daemon::ensure_queue_dir(&path)
        .await
        .expect("Failed to ensure queue directory");
    assert!(path.is_dir());
}

#[tokio::test]
async fn run_creates_queue_directory() {
    let dir = tempdir().expect("Failed to create temporary directory");
    let cfg = cfg_from(temp_config(&dir).with_cooldown(1));
    assert!(!cfg.queue_path.exists());
    let handle = tokio::spawn(run(cfg.clone()));
    wait_for_file(&cfg.queue_path, 200, Duration::from_millis(10)).await;
    handle.abort();
    assert!(cfg.queue_path.is_dir(), "queue directory not created");
}

#[tokio::test]
async fn prepare_listener_sets_permissions() {
    let dir = tempdir().expect("tempdir");
    let sock = dir.path().join("sock");
    stdfs::write(&sock, b"stale").expect("create stale file");
    let listener = prepare_listener(&sock).expect("prepare listener");
    drop(listener);
    let meta = stdfs::metadata(&sock).expect("metadata");
    assert_eq!(meta.permissions().mode() & 0o777, 0o660);
}

#[tokio::test]
async fn handle_client_enqueues_request() {
    let dir = tempdir().expect("tempdir");
    let cfg = Arc::new(cfg_from(temp_config(&dir)));
    let queue = SharedQueue::open(cfg).expect("open queue");

    let (client, server) = UnixStream::pair().expect("pair");
    let handle = tokio::spawn(handle_client(server, queue.clone()));
    let response = roundtrip(
        client,
        &Request::Put {
            request: sample_request(),
        },
    )
    .await;
    handle.await.expect("join").expect("client");
    let Response::Ok {
        entry: Some(entry), ..
    } = response
    else {
        panic!("expected put reply with entry, got {response:?}");
    };
    assert_eq!(entry.id.len(), 8);
    let entries = list_entries(&queue).await;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, entry.id);
}

#[tokio::test]
async fn handle_client_rejects_invalid_request() {
    let dir = tempdir().expect("tempdir");
    let cfg = Arc::new(cfg_from(temp_config(&dir)));
    let queue = SharedQueue::open(cfg).expect("open queue");

    let (mut client, server) = UnixStream::pair().expect("pair");
    let handle = tokio::spawn(handle_client(server, queue.clone()));
    client.write_all(b"not json").await.expect("write");
    client.shutdown().await.expect("shutdown");
    let mut reply = Vec::new();
    client.read_to_end(&mut reply).await.expect("read reply");
    handle.await.expect("join").expect("client");
    let response: Response = serde_json::from_slice(&reply).expect("parse reply");
    assert!(matches!(response, Response::Error { .. }));
    assert!(list_entries(&queue).await.is_empty());
}

#[tokio::test]
async fn run_listener_accepts_connections() -> Result<(), String> {
    let dir = tempdir().expect("tempdir");
    let cfg = Arc::new(cfg_from(temp_config(&dir).with_cooldown(1)));
    let queue = SharedQueue::open(cfg.clone()).expect("open queue");
    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let listener_task = tokio::spawn(run_listener(queue.clone(), shutdown_rx));
    wait_for_file(&cfg.socket_path, 10, Duration::from_millis(10)).await;
    let stream = UnixStream::connect(&cfg.socket_path)
        .await
        .expect("connect");
    let response = roundtrip(
        stream,
        &Request::Put {
            request: sample_request(),
        },
    )
    .await;
    assert!(matches!(response, Response::Ok { .. }));
    assert_eq!(list_entries(&queue).await.len(), 1);
    let _ = shutdown_tx.send(());
    let timeout = TimeoutConfig::new(10, TestComplexity::Moderate)
        .with_ci(false)
        .with_coverage(false)
        .calculate_timeout();
    let mut listener_handle = Some(listener_task);
    let listener_res = match tokio::time::timeout(timeout, async {
        listener_handle
            .as_mut()
            .expect("listener handle already taken")
            .await
    })
    .await
    {
        Ok(join_res) => join_res,
        Err(_elapsed) => {
            if let Some(h) = listener_handle.take() {
                h.abort();
            }
            return Err("listener join timeout".to_string());
        }
    };
    match listener_res {
        Ok(res) => {
            if let Err(e) = res {
                return Err(format!("listener task failed: {e}"));
            }
        }
        Err(e) => return Err(join_err("listener", e)),
    }
    Ok(())
}

/// Worker behaviour tests.
mod worker_tests {
    use super::*;
    const DRAINED_NOTIFICATION: TimeoutConfig = TimeoutConfig::new(15, TestComplexity::Moderate);
    const WORKER_SUCCESS: TimeoutConfig = TimeoutConfig::new(10, TestComplexity::Moderate);
    const WORKER_ERROR: TimeoutConfig = TimeoutConfig::new(15, TestComplexity::Complex);

    struct WorkerTestContext {
        server: MockServer,
        queue: Arc<SharedQueue>,
        octo: Arc<octocrab::Octocrab>,
        _dir: TempDir,
    }

    #[fixture]
    async fn worker_test_context(#[default(201)] status: u16) -> WorkerTestContext {
        let dir = tempdir().expect("tempdir");
        let cfg = Arc::new(cfg_from(
            temp_config(&dir).with_cooldown(TEST_COOLDOWN_SECONDS),
        ));
        let queue = SharedQueue::open(cfg).expect("open queue");
        seed_queue(&queue).await;
        let server = MockServer::start().await;
        // Build response body based on status - success returns Comment, error returns GitHub error
        let response_body: serde_json::Value = if (200..300).contains(&status) {
            serde_json::from_str(include_str!("fixtures/github_comment_response.json"))
                .expect("parse comment fixture")
        } else {
            serde_json::from_str(include_str!("fixtures/github_error_response.json"))
                .expect("parse error fixture")
        };
        Mock::given(method("POST"))
            .and(path("/repos/o/r/issues/1/comments"))
            .respond_with(ResponseTemplate::new(status).set_body_json(response_body))
            .expect(1..) // Expect at least one request
            .mount(&server)
            .await;
        let octo = octocrab_for(&server).expect("octocrab client should build for the mock server");
        WorkerTestContext {
            server,
            queue,
            octo,
            _dir: dir,
        }
    }

    #[rstest]
    #[tokio::test]
    async fn run_worker_completes_entry_on_success(
        #[future]
        #[from(worker_test_context)]
        ctx: WorkerTestContext,
    ) {
        let ctx = ctx.await;
        let idle = Arc::new(Notify::new());
        let (shutdown_tx, shutdown_rx) = watch::channel(());
        let control = WorkerControl {
            shutdown: shutdown_rx,
            hooks: WorkerHooks {
                enqueued: None,
                idle: Some(idle.clone()),
                drained: None,
            },
        };
        let h = tokio::spawn(run_worker(ctx.queue.clone(), ctx.octo, control));

        timeout_with_retries(DRAINED_NOTIFICATION, "worker idle notification", || {
            let idle = idle.clone();
            async move {
                idle.notified().await;
                Ok(())
            }
        })
        .await
        .expect("worker processed the entry");
        shutdown_tx.send(()).expect("send shutdown");
        let mut join_handle = Some(h);
        timeout_with_retries(WORKER_SUCCESS, "worker join", || {
            let handle = join_handle.take();
            async move {
                let handle = handle.ok_or_else(|| "join handle consumed".to_string())?;
                match handle.await {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(e)) => Err(e.to_string()),
                    Err(e) => Err(join_err("worker", e)),
                }
            }
        })
        .await
        .expect("worker exited cleanly");
        // Exactly one request proves the entry was posted once and removed.
        assert_eq!(
            ctx.server
                .received_requests()
                .await
                .expect("requests")
                .len(),
            1
        );
        assert!(
            list_entries(&ctx.queue).await.is_empty(),
            "posted entry should leave the queue"
        );
    }

    #[rstest]
    #[tokio::test]
    async fn run_worker_retains_entry_on_error(
        #[future]
        #[with(500)]
        #[from(worker_test_context)]
        ctx: WorkerTestContext,
    ) {
        let ctx = ctx.await;
        let (shutdown_tx, shutdown_rx) = watch::channel(());
        let enqueued = Arc::new(Notify::new());
        let control = WorkerControl {
            shutdown: shutdown_rx,
            hooks: WorkerHooks {
                enqueued: Some(enqueued.clone()),
                idle: None,
                drained: None,
            },
        };
        let h = tokio::spawn(run_worker(ctx.queue.clone(), ctx.octo, control));

        timeout_with_retries(WORKER_SUCCESS, "worker enqueued", || {
            let enqueued = enqueued.clone();
            async move {
                enqueued.notified().await;
                Ok(())
            }
        })
        .await
        .expect("worker picked up job");
        shutdown_tx.send(()).expect("send shutdown");
        let mut join_handle = Some(h);
        timeout_with_retries(WORKER_ERROR, "worker join", || {
            let handle = join_handle.take();
            async move {
                let handle = handle.ok_or_else(|| "join handle consumed".to_string())?;
                match handle.await {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(e)) => Err(e.to_string()),
                    Err(e) => Err(join_err("worker", e)),
                }
            }
        })
        .await
        .expect("worker exited cleanly");
        // At least one request should have been made (proving worker attempted processing)
        assert!(
            !ctx.server
                .received_requests()
                .await
                .expect("requests")
                .is_empty(),
            "worker should attempt the request at least once",
        );
        assert_eq!(
            list_entries(&ctx.queue).await.len(),
            1,
            "queue should retain the entry after an API failure",
        );
    }

    /// Tests that the worker loop terminates promptly when shutdown is signalled
    /// while the worker is idle (waiting on an empty queue).
    #[tokio::test]
    async fn worker_terminates_on_shutdown_while_idle() {
        let dir = tempdir().expect("tempdir");
        let cfg = Arc::new(cfg_from(temp_config(&dir).with_cooldown(60)));
        let queue = SharedQueue::open(cfg).expect("open queue");
        let server = MockServer::start().await;
        let octo = octocrab_for(&server).expect("octocrab client should build for the mock server");

        let (shutdown_tx, shutdown_rx) = watch::channel(());
        let control = WorkerControl {
            shutdown: shutdown_rx,
            hooks: WorkerHooks::default(),
        };

        let h = tokio::spawn(run_worker(queue, octo, control));

        // Give worker time to park on the change signal
        sleep(Duration::from_millis(50)).await;

        // Signal shutdown
        shutdown_tx.send(()).expect("send shutdown");

        // Worker should terminate promptly (not wait for cooldown)
        let timeout = Duration::from_secs(2);
        match tokio::time::timeout(timeout, h).await {
            Ok(Ok(Ok(()))) => {} // Success
            Ok(Ok(Err(e))) => panic!("worker returned error: {e}"),
            Ok(Err(e)) => panic!("worker task panicked: {e}"),
            Err(_) => panic!("worker did not terminate within {timeout:?} after shutdown signal"),
        }
    }

    /// Tests that shutdown during a cooldown wait (between entries) causes
    /// immediate termination.
    #[tokio::test]
    async fn worker_terminates_on_shutdown_during_cooldown() {
        let dir = tempdir().expect("tempdir");
        // Long cooldown to ensure we're testing shutdown during the wait
        let cfg = Arc::new(cfg_from(temp_config(&dir).with_cooldown(60)));
        let queue = SharedQueue::open(cfg).expect("open queue");
        // Two entries: the second is due one full cooldown after the first.
        seed_queue(&queue).await;
        queue
            .execute(Request::Put {
                request: comenq_lib::CommentRequest {
                    owner: "o".into(),
                    repo: "r".into(),
                    pr_number: 1,
                    body: "second".into(),
                },
            })
            .await;

        let server = MockServer::start().await;
        let response_body: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/github_comment_response.json"))
                .expect("parse fixture");
        Mock::given(method("POST"))
            .and(path("/repos/o/r/issues/1/comments"))
            .respond_with(ResponseTemplate::new(201).set_body_json(response_body))
            .expect(1)
            .mount(&server)
            .await;
        let octo = octocrab_for(&server).expect("octocrab client should build for the mock server");

        let (shutdown_tx, shutdown_rx) = watch::channel(());
        let idle = Arc::new(Notify::new());
        let control = WorkerControl {
            shutdown: shutdown_rx,
            hooks: WorkerHooks {
                enqueued: None,
                idle: Some(idle.clone()),
                drained: None,
            },
        };

        let h = tokio::spawn(run_worker(queue, octo, control));

        // Wait for idle notification (first entry posted; worker now waits a
        // full cooldown before the second).
        let wait_timeout = Duration::from_secs(10);
        if tokio::time::timeout(wait_timeout, idle.notified())
            .await
            .is_err()
        {
            panic!("worker did not reach idle state within {wait_timeout:?}");
        }

        // Worker is now waiting out the 60 second cooldown.
        // Signal shutdown - it should terminate immediately
        shutdown_tx.send(()).expect("send shutdown");

        let shutdown_timeout = Duration::from_secs(2);
        match tokio::time::timeout(shutdown_timeout, h).await {
            Ok(Ok(Ok(()))) => {} // Success
            Ok(Ok(Err(e))) => panic!("worker returned error: {e}"),
            Ok(Err(e)) => panic!("worker task panicked: {e}"),
            Err(_) => {
                panic!("worker did not terminate within {shutdown_timeout:?} during cooldown")
            }
        }
    }
}
