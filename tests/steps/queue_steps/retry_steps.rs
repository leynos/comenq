//! Cucumber steps for retrying queued posts after GitHub failures.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::Context as _;
use comenqd::daemon::{TokenClient, WorkerControl, WorkerHooks, run_worker};
use cucumber::{given, then, when};
use test_support::octocrab_for;
use tokio::sync::watch;
use tokio::time::timeout;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request as WiremockRequest, Respond, ResponseTemplate};

use super::QueueWorld;

fn client_pool(octocrab: Arc<octocrab::Octocrab>) -> Arc<Vec<TokenClient>> {
    Arc::new(vec![TokenClient::new("default", "t", octocrab)])
}

#[derive(Clone)]
struct FailThenSucceed {
    attempts: Arc<AtomicUsize>,
    success: serde_json::Value,
}

impl Respond for FailThenSucceed {
    fn respond(&self, _request: &WiremockRequest) -> ResponseTemplate {
        if self.attempts.fetch_add(1, Ordering::SeqCst) == 0 {
            ResponseTemplate::new(500)
        } else {
            ResponseTemplate::new(201).set_body_json(self.success.clone())
        }
    }
}

#[given("GitHub fails the first queue post and then succeeds")]
async fn github_fails_then_succeeds(world: &mut QueueWorld) -> anyhow::Result<()> {
    let server = MockServer::start().await;
    let attempts = Arc::new(AtomicUsize::new(0));
    let success: serde_json::Value = serde_json::from_str(include_str!(
        "../../../crates/comenqd/tests/fixtures/github_comment_response.json"
    ))
    .context("parse GitHub comment response fixture")?;
    Mock::given(method("POST"))
        .and(path("/repos/octocat/hello-world/issues/7/comments"))
        .respond_with(FailThenSucceed {
            attempts: Arc::clone(&attempts),
            success,
        })
        .mount(&server)
        .await;
    world.server = Some(server);
    world.attempts = Some(attempts);
    Ok(())
}

#[when("the worker retries after one cooldown")]
async fn worker_retries_after_cooldown(world: &mut QueueWorld) -> anyhow::Result<()> {
    let queue = world.queue()?.clone();
    let server = world
        .server
        .as_ref()
        .context("GitHub server not configured")?;
    let octocrab = octocrab_for(server).context("create GitHub client")?;
    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let control = WorkerControl::new(
        shutdown_rx,
        WorkerHooks {
            enqueued: None,
            idle: None,
            drained: None,
            waiting: None,
        },
    );
    let worker = tokio::spawn(async move {
        let _ = run_worker(queue, client_pool(octocrab), control).await;
    });
    world.shutdown = Some(shutdown_tx);
    world.worker = Some(worker);

    let attempts = world.attempts.as_ref().context("attempt counter missing")?;
    timeout(std::time::Duration::from_secs(5), async {
        while attempts.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .context("worker should retry after one cooldown")?;

    Ok(())
}

#[then("the queued comment has been retried and removed")]
async fn queued_comment_was_retried_and_removed(world: &mut QueueWorld) -> anyhow::Result<()> {
    let attempts = world
        .attempts
        .as_ref()
        .context("attempt counter not configured")?;
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    timeout(std::time::Duration::from_secs(5), async {
        while !world.entries().await?.is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .context("successful retry should remove the queued entry")??;
    Ok(())
}
