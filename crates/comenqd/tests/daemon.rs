//! Tests for daemon components and worker behaviour.

mod util;

use comenqd::config::Config;
use comenqd::daemon::{
    WorkerControl, WorkerHooks,
    listener::{handle_client, prepare_listener, run_listener},
    queue_writer, run, run_worker,
};
use octocrab::Octocrab;
use rstest::{fixture, rstest};
use std::fs as stdfs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tempfile::{TempDir, tempdir};
use test_support::{octocrab_for, temp_config};
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio::sync::{Notify, mpsc, watch};
use tokio::time::sleep;
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};
use yaque::{Receiver, channel};

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
        socket_path: cfg.socket_path,
        queue_path: cfg.queue_path,
        cooldown_period_seconds: cfg.cooldown_period_seconds,
        restart_min_delay_ms: cfg.restart_min_delay_ms,
        github_api_timeout_secs: cfg.github_api_timeout_secs,
        client_channel_capacity: cfg.client_channel_capacity,
    }
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
    let queue_path = dir.path().join("q");
    let (sender, mut receiver) = channel(&queue_path).expect("channel");
    let (client_tx, writer_rx) = mpsc::channel(4);
    let writer = tokio::spawn(queue_writer(sender, writer_rx));

    let (mut client, server) = UnixStream::pair().expect("pair");
    let handle = tokio::spawn(handle_client(server, client_tx));
    let req = comenq_lib::CommentRequest {
        owner: "o".into(),
        repo: "r".into(),
        pr_number: 1,
        body: "b".into(),
    };
    let payload = serde_json::to_vec(&req).expect("serialize");
    client.write_all(&payload).await.expect("write");
    client.shutdown().await.expect("shutdown");
    handle.await.expect("join").expect("client");
    drop(writer); // stop queue writer
    let guard = receiver.recv().await.expect("recv");
    let stored: comenq_lib::CommentRequest = serde_json::from_slice(&guard).expect("parse");
    assert_eq!(stored, req);
}

#[tokio::test]
async fn run_listener_accepts_connections() -> Result<(), String> {
    let dir = tempdir().expect("tempdir");
    let cfg = Arc::new(cfg_from(temp_config(&dir).with_cooldown(1)));
    let (sender, mut receiver) = channel(&cfg.queue_path).expect("channel");
    let (client_tx, writer_rx) = mpsc::channel(4);
    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let writer_handle = tokio::spawn(queue_writer(sender, writer_rx));
    let listener_task = tokio::spawn(run_listener(cfg.clone(), client_tx, shutdown_rx));
    wait_for_file(&cfg.socket_path, 10, Duration::from_millis(10)).await;
    let mut stream = UnixStream::connect(&cfg.socket_path)
        .await
        .expect("connect");
    let req = comenq_lib::CommentRequest {
        owner: "o".into(),
        repo: "r".into(),
        pr_number: 1,
        body: "b".into(),
    };
    let payload = serde_json::to_vec(&req).expect("serialize");
    stream.write_all(&payload).await.expect("write");
    stream.shutdown().await.expect("shutdown");
    let guard = receiver.recv().await.expect("recv");
    let stored: comenq_lib::CommentRequest = serde_json::from_slice(&guard).expect("parse");
    assert_eq!(stored, req);
    let _ = shutdown_tx.send(());
    let timeout = TimeoutConfig::new(10, TestComplexity::Moderate).calculate_timeout();
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
    match writer_handle.await {
        Ok(_) => {}
        Err(e) => return Err(join_err("writer", e)),
    }
    Ok(())
}

/// Worker behaviour tests.
mod worker_tests {
    use super::*;
    const DRAINED_NOTIFICATION: TimeoutConfig = TimeoutConfig::new(15, TestComplexity::Moderate);
    const WORKER_SUCCESS: TimeoutConfig = TimeoutConfig::new(10, TestComplexity::Moderate);
    const WORKER_ERROR: TimeoutConfig = TimeoutConfig::new(15, TestComplexity::Complex);
    use wiremock::MockServer;

    struct WorkerTestContext {
        server: MockServer,
        cfg: Arc<Config>,
        rx: Receiver,
        octo: Arc<Octocrab>,
        _dir: TempDir,
    }

    #[fixture]
    async fn worker_test_context(#[default(201)] status: u16) -> WorkerTestContext {
        let dir = tempdir().expect("tempdir");
        let cfg = Arc::new(cfg_from(
            temp_config(&dir).with_cooldown(TEST_COOLDOWN_SECONDS),
        ));
        let (mut sender, rx) = channel(&cfg.queue_path).expect("channel");
        let req = comenq_lib::CommentRequest {
            owner: "o".into(),
            repo: "r".into(),
            pr_number: 1,
            body: "b".into(),
        };
        sender
            .send(serde_json::to_vec(&req).expect("serialize"))
            .await
            .expect("send");
        let server = MockServer::start().await;
        // Build response body based on status - success returns Comment, error returns GitHub error
        let response_body = if (200..300).contains(&status) {
            serde_json::json!({
                "id": 1,
                "node_id": "IC_test",
                "url": "https://api.github.com/repos/o/r/issues/comments/1",
                "html_url": "https://github.com/o/r/issues/1#issuecomment-1",
                "body": "b",
                "user": {
                    "login": "test-user",
                    "id": 1,
                    "node_id": "U_test",
                    "avatar_url": "https://example.com/avatar",
                    "gravatar_id": "",
                    "url": "https://api.github.com/users/test-user",
                    "html_url": "https://github.com/test-user",
                    "followers_url": "https://api.github.com/users/test-user/followers",
                    "following_url": "https://api.github.com/users/test-user/following{/other_user}",
                    "gists_url": "https://api.github.com/users/test-user/gists{/gist_id}",
                    "starred_url": "https://api.github.com/users/test-user/starred{/owner}{/repo}",
                    "subscriptions_url": "https://api.github.com/users/test-user/subscriptions",
                    "organizations_url": "https://api.github.com/users/test-user/orgs",
                    "repos_url": "https://api.github.com/users/test-user/repos",
                    "events_url": "https://api.github.com/users/test-user/events{/privacy}",
                    "received_events_url": "https://api.github.com/users/test-user/received_events",
                    "type": "User",
                    "site_admin": false
                },
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "author_association": "NONE"
            })
        } else {
            // GitHub API error format for non-2xx responses
            serde_json::json!({
                "message": "Internal Server Error",
                "documentation_url": "https://docs.github.com/rest"
            })
        };
        Mock::given(method("POST"))
            .and(path("/repos/o/r/issues/1/comments"))
            .respond_with(ResponseTemplate::new(status).set_body_json(response_body))
            .expect(1..)  // Expect at least one request
            .mount(&server)
            .await;
        let octo = octocrab_for(&server);
        WorkerTestContext {
            server,
            cfg,
            rx,
            octo,
            _dir: dir,
        }
    }

    async fn diagnose_queue_state(
        cfg: &Config,
        server: &MockServer,
        expected_files: usize,
    ) -> String {
        let queue_files = stdfs::read_dir(&cfg.queue_path)
            .map(|entries| entries.count())
            .unwrap_or(0);
        let server_requests = server.received_requests().await.unwrap_or_default().len();
        let mut output =
            format!("Queue directory contains {queue_files} files (expected {expected_files})\n");
        output.push_str(&format!(
            "Mock server received {server_requests} requests\n"
        ));
        if let Ok(entries) = stdfs::read_dir(&cfg.queue_path) {
            output.push_str("Remaining queue files:\n");
            for (i, entry) in entries.enumerate() {
                if let Ok(entry) = entry {
                    let name = entry.file_name();
                    let file_num = i + 1;
                    output.push_str(&format!("  {file_num}. {}\n", name.to_string_lossy()));
                    if let Ok(metadata) = entry.metadata() {
                        let size = metadata.len();
                        output.push_str(&format!("     Size: {size} bytes\n"));
                        if let Ok(modified) = metadata.modified()
                            && let Ok(elapsed) = modified.elapsed()
                        {
                            let age = elapsed.as_secs_f32();
                            output.push_str(&format!("     Age: {age:.1}s ago\n"));
                        }
                    }
                }
            }
        }
        output
    }

    #[rstest]
    #[tokio::test]
    async fn run_worker_commits_on_success(
        #[future]
        #[from(worker_test_context)]
        ctx: WorkerTestContext,
    ) {
        let ctx = ctx.await;
        let server = Arc::new(ctx.server);
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
        let h = tokio::spawn(run_worker(ctx.cfg.clone(), ctx.rx, ctx.octo, control));

        // Wait for idle notification which fires after processing completes
        if let Err(e) =
            timeout_with_retries(DRAINED_NOTIFICATION, "worker idle notification", || {
                let idle = idle.clone();
                async move {
                    idle.notified().await;
                    Ok(())
                }
            })
            .await
        {
            let diagnostics = diagnose_queue_state(&ctx.cfg, &server, 0).await;
            tracing::error!("Timeout waiting for worker idle notification: {e}");
            tracing::error!("{diagnostics}");
            panic!("worker idle: PROCESSING FAILURE");
        }
        shutdown_tx.send(()).expect("send shutdown");
        let mut join_handle = Some(h);
        if let Err(e) = timeout_with_retries(WORKER_SUCCESS, "worker join", || {
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
        {
            let diagnostics = diagnose_queue_state(&ctx.cfg, &server, 0).await;
            tracing::error!("\u{274C} Worker join timeout: {e}");
            tracing::error!("{diagnostics}");
            panic!("join worker: timeout in success test");
        }
        // Verify exactly one request was made - this proves the item was processed
        // and committed (otherwise the worker would retry and make multiple requests)
        assert_eq!(server.received_requests().await.expect("requests").len(), 1);
        // Note: We don't assert on queue data files because yaque cleans up segment
        // files lazily during the next recv() call. Since the worker exited after
        // shutdown, the segment file may still exist on disk even though the item
        // was logically committed.
    }

    #[rstest]
    #[tokio::test]
    async fn run_worker_requeues_on_error(
        #[future]
        #[with(500)]
        #[from(worker_test_context)]
        ctx: WorkerTestContext,
    ) {
        let ctx = ctx.await;
        let server = Arc::new(ctx.server);
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
        let h = tokio::spawn(run_worker(ctx.cfg.clone(), ctx.rx, ctx.octo, control));

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
        if let Err(e) = timeout_with_retries(WORKER_ERROR, "worker join", || {
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
        {
            let diagnostics = diagnose_queue_state(&ctx.cfg, &server, 1).await;
            tracing::error!("\u{274C} Worker join timeout: {e}");
            tracing::error!("{diagnostics}");
            panic!("join worker: timeout in error test");
        }
        // At least one request should have been made (proving worker attempted processing)
        assert!(
            !server
                .received_requests()
                .await
                .expect("requests")
                .is_empty(),
            "worker should attempt the request at least once",
        );
        // Queue should still have data files (job was NOT committed due to error)
        assert!(
            stdfs::read_dir(&ctx.cfg.queue_path)
                .expect("read queue directory")
                .count()
                > 0,
            "Queue should retain job after API failure",
        );
    }
}
