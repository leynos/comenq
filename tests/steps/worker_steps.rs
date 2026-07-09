//! Behavioural test steps for the worker task.
//!
//! These steps drive the Cucumber scenarios that verify the worker posts
//! queued comments and handles failures gracefully.
//!
//! Uses Wiremock to stub the GitHub Issues API and yaque for the on-disk queue.
//! See also: `test-support::octocrab_for()` and `yaque::channel()`.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use comenq_lib::CommentRequest;
use comenqd::config::Config;
use comenqd::daemon::{WorkerControl, WorkerHooks, is_metadata_file, run_worker};
use cucumber::{World, given, then, when};
use tempfile::TempDir;
use test_support::{octocrab_for, temp_config};
use tokio::sync::{Notify, watch};
use tokio::time::timeout;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use yaque::{self, channel};

fn coverage_timeout_multiplier() -> u32 {
    if std::env::var("CARGO_LLVM_COV_TARGET_DIR").is_ok()
        || std::env::var("RUSTFLAGS").is_ok_and(|f| f.contains("coverage"))
    {
        10
    } else {
        1
    }
}

#[derive(World, Default)]
pub struct WorkerWorld {
    dir: Option<TempDir>,
    cfg: Option<Arc<Config>>,
    receiver: Option<yaque::Receiver>,
    server: Option<MockServer>,
    shutdown: Option<watch::Sender<()>>,
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl std::fmt::Debug for WorkerWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerWorld").finish()
    }
}

impl Drop for WorkerWorld {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

impl WorkerWorld {
    async fn shutdown_and_join(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            let _ = timeout(Duration::from_secs(5), handle).await;
        }
    }
}

#[given("a queued comment request")]
async fn queued_request(world: &mut WorkerWorld) -> anyhow::Result<()> {
    let dir = TempDir::new().context("tempdir")?;
    let cfg = Arc::new(Config::from(temp_config(&dir).with_cooldown(0)));
    let (mut sender, receiver) = channel(&cfg.queue_path).context("channel")?;
    let req = CommentRequest {
        owner: "o".into(),
        repo: "r".into(),
        pr_number: 1,
        body: "b".into(),
    };
    let data = serde_json::to_vec(&req).context("serialize")?;
    sender.send(data).await.context("send")?;
    world.dir = Some(dir);
    world.cfg = Some(cfg);
    world.receiver = Some(receiver);
    Ok(())
}

#[given("GitHub returns success")]
async fn github_success(world: &mut WorkerWorld) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repos/o/r/issues/1/comments"))
        .respond_with(ResponseTemplate::new(201).set_body_raw("{}", "application/json"))
        .mount(&server)
        .await;
    world.server = Some(server);
}

#[given("GitHub returns an error")]
async fn github_error(world: &mut WorkerWorld) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repos/o/r/issues/1/comments"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    world.server = Some(server);
}

#[when("the worker runs briefly")]
async fn worker_runs(world: &mut WorkerWorld) -> anyhow::Result<()> {
    let cfg = world
        .cfg
        .as_ref()
        .context("configuration should be initialised")?
        .clone();
    let rx = world
        .receiver
        .take()
        .context("receiver should be initialised")?;
    let server = world
        .server
        .as_ref()
        .context("server should be initialised")?;
    let octocrab =
        octocrab_for(server).context("octocrab client should build for the mock server")?;
    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let idle = Arc::new(Notify::new());
    let idle_for_wait = Arc::clone(&idle);
    let idle_notified = idle_for_wait.notified();
    let control = WorkerControl::new(
        shutdown_rx,
        WorkerHooks {
            enqueued: None,
            idle: Some(idle),
            drained: None,
        },
    );
    let handle = tokio::spawn(async move {
        let _ = run_worker(cfg, rx, octocrab, control).await;
    });
    timeout(
        Duration::from_secs(30 * u64::from(coverage_timeout_multiplier())),
        idle_notified,
    )
    .await
    .context("worker reached idle state within timeout")?;

    // Store handles in world for proper cleanup
    world.shutdown = Some(shutdown_tx);
    world.handle = Some(handle);
    Ok(())
}

#[then("the comment is posted")]
async fn comment_posted(world: &mut WorkerWorld) -> anyhow::Result<()> {
    let server = world
        .server
        .as_ref()
        .context("server should be initialised")?;
    assert!(
        !server
            .received_requests()
            .await
            .context("inbound requests should be recorded")?
            .is_empty(),
    );
    world.shutdown_and_join().await;
    Ok(())
}

#[then("the queue retains the job")]
async fn queue_retains(world: &mut WorkerWorld) -> anyhow::Result<()> {
    let cfg = world
        .cfg
        .as_ref()
        .context("configuration should be initialised")?;
    let job_count = std::fs::read_dir(&cfg.queue_path)
        .context("queue directory should be readable")?
        .filter_map(Result::ok)
        .filter(|e| !is_metadata_file(e.file_name()))
        .count();
    assert!(job_count > 0, "queue should retain at least one job file");
    world.shutdown_and_join().await;
    Ok(())
}
