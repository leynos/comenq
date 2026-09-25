//! Behavioural test steps for the worker task.
//!
//! These steps drive the Cucumber scenarios that verify the worker posts
//! queued comments and handles failures gracefully.
//!
//! Uses Wiremock to stub the GitHub Issues API and the daemon's shared queue
//! for on-disk persistence.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use comenq_lib::CommentRequest;
use comenq_lib::protocol::{PendingEntry, Request, Response};
use comenqd::config::Config;
use comenqd::daemon::{SharedQueue, WorkerControl, WorkerHooks, run_worker};
use cucumber::{World, given, then, when};
use tempfile::TempDir;
use test_support::{octocrab_for, temp_config};
use tokio::sync::{Notify, watch};
use tokio::time::timeout;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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
    queue: Option<Arc<SharedQueue>>,
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

/// Pending entries as reported through the protocol.
async fn listed_entries(queue: &Arc<SharedQueue>) -> anyhow::Result<Vec<PendingEntry>> {
    match queue.execute(Request::List).await {
        Response::Ok {
            entries: Some(entries),
            ..
        } => Ok(entries),
        other => anyhow::bail!("expected list reply, got {other:?}"),
    }
}

#[given("a queued comment request")]
async fn queued_request(world: &mut WorkerWorld) -> anyhow::Result<()> {
    let dir = TempDir::new().context("tempdir")?;
    let cfg = Arc::new(Config::from(temp_config(&dir).with_cooldown(0)));
    let queue = SharedQueue::open(cfg).context("open queue")?;
    let response = queue
        .execute(Request::Put {
            request: CommentRequest {
                owner: "o".into(),
                repo: "r".into(),
                pr_number: 1,
                body: "b".into(),
            },
            immediate: true,
        })
        .await;
    anyhow::ensure!(
        matches!(response, Response::Ok { .. }),
        "put should succeed, got {response:?}"
    );
    world.dir = Some(dir);
    world.queue = Some(queue);
    Ok(())
}

#[given("GitHub returns success")]
async fn github_success(world: &mut WorkerWorld) -> anyhow::Result<()> {
    let server = MockServer::start().await;
    // A structurally valid comment body: an empty object fails octocrab's
    // response parsing and would masquerade as an API error.
    let body: serde_json::Value = serde_json::from_str(include_str!(
        "../../crates/comenqd/tests/fixtures/github_comment_response.json"
    ))
    .context("parse comment fixture")?;
    Mock::given(method("POST"))
        .and(path("/repos/o/r/issues/1/comments"))
        .respond_with(ResponseTemplate::new(201).set_body_json(body))
        .mount(&server)
        .await;
    world.server = Some(server);
    Ok(())
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
    let queue = world
        .queue
        .as_ref()
        .context("queue should be initialized")?
        .clone();
    let server = world
        .server
        .as_ref()
        .context("server should be initialized")?;
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
        let _ = run_worker(queue, octocrab, control).await;
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
        .context("server should be initialized")?;
    assert!(
        !server
            .received_requests()
            .await
            .context("inbound requests should be recorded")?
            .is_empty(),
    );
    let queue = world.queue.as_ref().context("queue initialized")?.clone();
    assert!(
        listed_entries(&queue).await?.is_empty(),
        "posted entry should leave the queue"
    );
    world.shutdown_and_join().await;
    Ok(())
}

#[then("the queue retains the job")]
async fn queue_retains(world: &mut WorkerWorld) -> anyhow::Result<()> {
    let queue = world.queue.as_ref().context("queue initialized")?.clone();
    assert_eq!(
        listed_entries(&queue).await?.len(),
        1,
        "queue should retain the entry after an API failure"
    );
    world.shutdown_and_join().await;
    Ok(())
}
