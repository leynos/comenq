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
use comenq_lib::protocol::{HistoryEntry, PendingEntry, Request, Response};
use comenqd::config::Config;
use comenqd::daemon::{SharedQueue, TokenClient, WorkerControl, WorkerHooks, run_worker};
use cucumber::{World, given, then, when};
use tempfile::TempDir;
use test_support::{octocrab_with_token, temp_config};
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

/// Token pool used when no rotation is configured: one anonymous token.
const DEFAULT_TOKENS: [(&str, &str); 1] = [("test-token", "t")];
#[derive(World, Default)]
pub struct WorkerWorld {
    dir: Option<TempDir>,
    pub(crate) queue: Option<Arc<SharedQueue>>,
    pub(crate) server: Option<MockServer>,
    /// Rotation pool (name, token value); empty means the single default.
    pub(crate) tokens: Vec<(String, String)>,
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
    pub(crate) async fn shutdown_and_join(&mut self) {
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

/// Posting history as reported through the protocol.
pub(crate) async fn listed_history(queue: &Arc<SharedQueue>) -> anyhow::Result<Vec<HistoryEntry>> {
    match queue.execute(Request::Hist { limit: None }).await {
        Response::Ok {
            history: Some(history),
            ..
        } => Ok(history),
        other => anyhow::bail!("expected hist reply, got {other:?}"),
    }
}

async fn queue_with_entries(world: &mut WorkerWorld, bodies: &[&str]) -> anyhow::Result<()> {
    let dir = TempDir::new().context("tempdir")?;
    let cfg = Arc::new(Config::from(temp_config(&dir).with_cooldown(0)));
    let queue = SharedQueue::open(cfg).context("open queue")?;
    for body in bodies {
        let response = queue
            .execute(Request::Put {
                request: CommentRequest {
                    owner: "o".into(),
                    repo: "r".into(),
                    pr_number: 1,
                    body: (*body).to_owned(),
                },
                immediate: true,
            })
            .await;
        anyhow::ensure!(
            matches!(response, Response::Ok { .. }),
            "put should succeed, got {response:?}"
        );
    }
    world.dir = Some(dir);
    world.queue = Some(queue);
    Ok(())
}

#[given("a queued comment request")]
async fn queued_request(world: &mut WorkerWorld) -> anyhow::Result<()> {
    queue_with_entries(world, &["b"]).await
}

#[given("two queued comment requests")]
async fn two_queued_requests(world: &mut WorkerWorld) -> anyhow::Result<()> {
    queue_with_entries(world, &["first", "second"]).await
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

/// Which worker milestone a scenario waits for before asserting.
enum WaitFor {
    /// One processing pass has completed.
    Idle,
    /// The queue is empty and the worker is parked.
    Drained,
}

fn token_clients(world: &WorkerWorld, server: &MockServer) -> anyhow::Result<Vec<TokenClient>> {
    let pool: Vec<(String, String)> = if world.tokens.is_empty() {
        DEFAULT_TOKENS
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    } else {
        world.tokens.clone()
    };
    pool.into_iter()
        .map(|(name, value)| {
            let octocrab = octocrab_with_token(server, &value)
                .context("octocrab client should build for the mock server")?;
            Ok(TokenClient::new(name, &value, octocrab))
        })
        .collect()
}

async fn start_worker(world: &mut WorkerWorld, wait_for: WaitFor) -> anyhow::Result<()> {
    let queue = world
        .queue
        .as_ref()
        .context("queue should be initialized")?
        .clone();
    let server = world
        .server
        .as_ref()
        .context("server should be initialized")?;
    let clients = Arc::new(token_clients(world, server)?);
    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let milestone = Arc::new(Notify::new());
    let milestone_for_wait = Arc::clone(&milestone);
    let milestone_notified = milestone_for_wait.notified();
    let hooks = match wait_for {
        WaitFor::Idle => WorkerHooks {
            enqueued: None,
            idle: Some(milestone),
            drained: None,
        },
        WaitFor::Drained => WorkerHooks {
            enqueued: None,
            idle: None,
            drained: Some(milestone),
        },
    };
    let control = WorkerControl::new(shutdown_rx, hooks);
    let handle = tokio::spawn(async move {
        let _ = run_worker(queue, clients, control).await;
    });
    timeout(
        Duration::from_secs(30 * u64::from(coverage_timeout_multiplier())),
        milestone_notified,
    )
    .await
    .context("worker reached the awaited state within timeout")?;

    // Store handles in world for proper cleanup
    world.shutdown = Some(shutdown_tx);
    world.handle = Some(handle);
    Ok(())
}

#[when("the worker runs briefly")]
async fn worker_runs(world: &mut WorkerWorld) -> anyhow::Result<()> {
    start_worker(world, WaitFor::Idle).await
}

#[when("the worker drains the queue")]
async fn worker_drains(world: &mut WorkerWorld) -> anyhow::Result<()> {
    start_worker(world, WaitFor::Drained).await
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

#[then("the posting history records the success")]
async fn history_records_success(world: &mut WorkerWorld) -> anyhow::Result<()> {
    let queue = world.queue.as_ref().context("queue initialized")?.clone();
    let history = listed_history(&queue).await?;
    assert_eq!(history.len(), 1, "one post should yield one record");
    let record = history.first().context("history should hold a record")?;
    assert!(record.success, "the record should mark a success");
    assert_eq!(record.error, None);
    assert_eq!(record.owner, "o");
    assert_eq!(record.repo, "r");
    assert_eq!(record.pr_number, 1);
    Ok(())
}

#[then("the posting history records the failure")]
async fn history_records_failure(world: &mut WorkerWorld) -> anyhow::Result<()> {
    let queue = world.queue.as_ref().context("queue initialized")?.clone();
    let history = listed_history(&queue).await?;
    anyhow::ensure!(
        !history.is_empty(),
        "a failed attempt should be recorded in the history"
    );
    for record in &history {
        assert!(!record.success, "every record should mark a failure");
        let error = record
            .error
            .as_deref()
            .context("failure carries an error")?;
        anyhow::ensure!(
            !error.is_empty(),
            "the error description should not be empty"
        );
    }
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
