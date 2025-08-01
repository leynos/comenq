#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "simplify test output"
)]

use std::sync::Arc;
use std::time::Duration;

use crate::util::{octocrab_for, temp_config_with};
use comenq_lib::CommentRequest;
use comenqd::config::Config;
use comenqd::daemon::run_worker;
use cucumber::{World, given, then, when};
use tempfile::TempDir;
use tokio::time::sleep;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use yaque::{self, channel};

#[derive(World, Default)]
pub struct WorkerWorld {
    dir: Option<TempDir>,
    cfg: Option<Arc<Config>>,
    receiver: Option<yaque::Receiver>,
    server: Option<MockServer>,
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl std::fmt::Debug for WorkerWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerWorld").finish()
    }
}

#[given("a queued comment request")]
async fn queued_request(world: &mut WorkerWorld) {
    let dir = TempDir::new().expect("tempdir");
    let cfg = Arc::new(temp_config_with(&dir, 0)); // Immediate execution, no cooldown
    let (mut sender, receiver) = channel(&cfg.queue_path).expect("channel");
    let req = CommentRequest {
        owner: "o".into(),
        repo: "r".into(),
        pr_number: 1,
        body: "b".into(),
    };
    let data = serde_json::to_vec(&req).expect("serialize");
    sender.send(data).await.expect("send");
    world.dir = Some(dir);
    world.cfg = Some(cfg);
    world.receiver = Some(receiver);
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
async fn worker_runs(world: &mut WorkerWorld) {
    let cfg = world.cfg.as_ref().unwrap().clone();
    let rx = world.receiver.take().unwrap();
    let server = world.server.as_ref().unwrap();
    let octocrab = octocrab_for(server);
    let handle = tokio::spawn(async move {
        let _ = run_worker(cfg, rx, octocrab).await;
    });
    sleep(Duration::from_millis(100)).await;
    handle.abort();
    world.handle = Some(handle);
}

#[then("the comment is posted")]
async fn comment_posted(world: &mut WorkerWorld) {
    let server = world.server.as_ref().unwrap();
    assert!(!server.received_requests().await.unwrap().is_empty());
}

#[then("the queue retains the job")]
fn queue_retains(world: &mut WorkerWorld) {
    let cfg = world.cfg.as_ref().unwrap();
    assert!(std::fs::read_dir(&cfg.queue_path).unwrap().count() > 0);
}

impl Drop for WorkerWorld {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}
