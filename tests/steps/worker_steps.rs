//! Behavioural test steps for the worker task.
//!
//! These steps drive the Cucumber scenarios that verify the worker posts
//! queued comments and handles failures gracefully.
//!
//! Uses Wiremock to stub the GitHub Issues API and yaque for the on-disk queue.
//! See also: `test-support::octocrab_for()` and `yaque::channel()`.

use std::sync::Arc;
use std::time::Duration;

use comenq_lib::CommentRequest;
use comenqd::config::Config;
use comenqd::daemon::Worker;
use cucumber::{World, given, then, when};
use tempfile::TempDir;
use test_support::{octocrab_for, temp_config};
use tokio::time::timeout;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use yaque::{self, channel};

#[derive(World, Default)]
pub struct WorkerWorld {
    dir: Option<TempDir>,
    cfg: Option<Arc<Config>>,
    receiver: Option<yaque::Receiver>,
    server: Option<MockServer>,
}

impl std::fmt::Debug for WorkerWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerWorld").finish()
    }
}

#[given("a queued comment request")]
#[expect(
    clippy::expect_used,
    reason = "test harness: fail fast on setup/IO errors"
)]
async fn queued_request(world: &mut WorkerWorld) {
    let dir = TempDir::new().expect("tempdir");
    let cfg = Arc::new(Config::from(temp_config(&dir).with_cooldown(0)));
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
#[expect(
    clippy::expect_used,
    reason = "test harness: fail fast on world state errors"
)]
async fn worker_runs(world: &mut WorkerWorld) {
    let cfg = world
        .cfg
        .as_ref()
        .expect("configuration should be initialised")
        .clone();
    let rx = world
        .receiver
        .take()
        .expect("receiver should be initialised");
    let server = world.server.as_ref().expect("server should be initialised");
    let octocrab = octocrab_for(server);
    let (worker, signals) = Worker::spawn_with_signals(cfg, rx, octocrab);
    timeout(Duration::from_secs(30), signals.on_enqueued())
        .await
        .expect("worker did not start processing");
    timeout(Duration::from_secs(30), worker.shutdown())
        .await
        .expect("worker shutdown timed out")
        .expect("shutdown");
}

#[then("the comment is posted")]
#[expect(
    clippy::expect_used,
    reason = "test harness: expect wiremock state in assertion"
)]
async fn comment_posted(world: &mut WorkerWorld) {
    let server = world.server.as_ref().expect("server should be initialised");
    assert!(
        !server
            .received_requests()
            .await
            .expect("inbound requests should be recorded")
            .is_empty()
    );
}

#[then("the queue retains the job")]
#[expect(
    clippy::expect_used,
    reason = "test harness: expect fs state in assertion"
)]
fn queue_retains(world: &mut WorkerWorld) {
    let cfg = world
        .cfg
        .as_ref()
        .expect("configuration should be initialised");
    assert!(
        std::fs::read_dir(&cfg.queue_path)
            .expect("queue directory should be readable")
            .count()
            > 0
    );
}
