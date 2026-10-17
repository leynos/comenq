//! Foreground awaiting against the production listener, store, and GitHub worker.

use anyhow::Context as _;
use comenq::client::await_with_writers;
use comenq_lib::{
    CommentRequest,
    protocol::{Request, Response},
};
use comenqd::{
    config::Config,
    daemon::{
        SharedQueue, TokenClient, WorkerControl,
        listener::{dispatch_request, run_listener},
        run_worker,
    },
};
use cucumber::{World, given, then, when};
use std::{
    io::{self, Write},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tempfile::TempDir;
use test_support::{octocrab_for, temp_config, util::poll_until};
use tokio::{sync::watch, task::JoinHandle};
use wiremock::{
    Mock, MockServer, Respond, ResponseTemplate,
    matchers::{method, path},
};

const URL: &str = "https://github.com/o/r/issues/1#issuecomment-1";
const DEADLINE: Duration = Duration::from_secs(10);

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn bytes(&self) -> anyhow::Result<Vec<u8>> {
        Ok(self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("output lock poisoned"))?
            .clone())
    }
}

impl Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_| io::Error::other("output lock poisoned"))?
            .extend(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Default, World)]
pub struct AwaitWorld {
    directory: Option<TempDir>,
    queue: Option<Arc<SharedQueue>>,
    shutdown: Option<watch::Sender<()>>,
    listener: Option<JoinHandle<anyhow::Result<()>>>,
    worker: Option<JoinHandle<anyhow::Result<()>>>,
    client: Option<JoinHandle<Result<(), comenq::ClientError>>>,
    server: Option<MockServer>,
    id: String,
    fail_first: bool,
    attempts: Arc<Mutex<Vec<Instant>>>,
    stdout: Captured,
    stderr: Captured,
    result: Option<Result<(), comenq::ClientError>>,
}

impl std::fmt::Debug for AwaitWorld {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("AwaitWorld").finish()
    }
}

impl Drop for AwaitWorld {
    fn drop(&mut self) {
        if let Some(shutdown) = &self.shutdown {
            let _ = shutdown.send(());
        }
        if let Some(client) = &self.client {
            client.abort();
        }
        if let Some(worker) = &self.worker {
            worker.abort();
        }
        if let Some(listener) = &self.listener {
            listener.abort();
        }
    }
}

impl AwaitWorld {
    fn queue(&self) -> anyhow::Result<&Arc<SharedQueue>> {
        self.queue.as_ref().context("await queue not initialized")
    }

    fn start_client(&mut self, id: String) -> anyhow::Result<()> {
        let socket = self.queue()?.config().socket_path.clone();
        let (mut stdout, mut stderr) = (self.stdout.clone(), self.stderr.clone());
        self.client = Some(tokio::spawn(async move {
            await_with_writers(&[socket], &id, &mut stdout, &mut stderr).await
        }));
        Ok(())
    }

    async fn finish_client(&mut self) -> anyhow::Result<()> {
        let client = self.client.take().context("await client not started")?;
        // Retain the handle on timeout so Drop still cleans up the owned task.
        self.client = Some(client);
        let result =
            tokio::time::timeout(DEADLINE, self.client.as_mut().context("client missing")?)
                .await
                .context("await client did not finish")?
                .context("await task failed")?;
        self.client = None;
        self.result = Some(result);
        Ok(())
    }
}

#[given("a running await daemon")]
async fn running_daemon(world: &mut AwaitWorld) -> anyhow::Result<()> {
    let directory = TempDir::new().context("create await directory")?;
    let queue = SharedQueue::open(Arc::new(Config::from(temp_config(&directory))))?;
    let (shutdown, receiver) = watch::channel(());
    world.listener = Some(tokio::spawn(run_listener(queue.clone(), receiver)));
    let socket = queue.config().socket_path.clone();
    world.queue = Some(queue);
    world.shutdown = Some(shutdown);
    world.directory = Some(directory);
    anyhow::ensure!(
        poll_until(DEADLINE, Duration::from_millis(10), || async {
            socket.exists()
        })
        .await,
        "listener socket did not appear"
    );
    Ok(())
}

#[given("a comment queued for awaiting")]
async fn queued_comment(world: &mut AwaitWorld) -> anyhow::Result<()> {
    let response = dispatch_request(
        world.queue()?,
        Request::Put {
            request: CommentRequest {
                owner: "octocat".into(),
                repo: "hello-world".into(),
                pr_number: 7,
                body: "await me".into(),
            },
            immediate: false,
        },
    )
    .await;
    let Response::Ok {
        entry: Some(entry), ..
    } = response
    else {
        anyhow::bail!("put failed: {response:?}");
    };
    world.id = entry.id;
    Ok(())
}

#[given("GitHub fails the first await post")]
fn fail_first(world: &mut AwaitWorld) {
    world.fail_first = true;
}

#[when("I await the queued comment")]
async fn await_queued(world: &mut AwaitWorld) -> anyhow::Result<()> {
    world.start_client(world.id.clone())?;
    anyhow::ensure!(
        poll_until(DEADLINE, Duration::from_millis(10), || async {
            world.stderr.bytes().is_ok_and(|bytes| !bytes.is_empty())
        })
        .await,
        "await did not report its first countdown"
    );
    Ok(())
}

#[then("the await client is still waiting with a countdown")]
fn still_waiting(world: &mut AwaitWorld) -> anyhow::Result<()> {
    anyhow::ensure!(
        !world
            .client
            .as_ref()
            .context("client missing")?
            .is_finished(),
        "client finished before posting"
    );
    anyhow::ensure!(
        world.stdout.bytes()?.is_empty(),
        "stdout must be empty while queued"
    );
    let countdown = String::from_utf8(world.stderr.bytes()?)?;
    anyhow::ensure!(
        countdown.ends_with(" remaining\n") || countdown.ends_with("due now\n"),
        "unexpected countdown {countdown}"
    );
    Ok(())
}

struct FailThenSucceed {
    fail_first: bool,
    attempts: Arc<Mutex<Vec<Instant>>>,
    success: serde_json::Value,
}

impl Respond for FailThenSucceed {
    fn respond(&self, _: &wiremock::Request) -> ResponseTemplate {
        let Ok(mut attempts) = self.attempts.lock() else {
            return ResponseTemplate::new(500);
        };
        let first = attempts.is_empty();
        attempts.push(Instant::now());
        // A non-retryable HTTP response exercises the worker's cooldown,
        // rather than Octocrab's internal HTTP retry policy.
        if self.fail_first && first {
            ResponseTemplate::new(400)
        } else {
            ResponseTemplate::new(201).set_body_json(&self.success)
        }
    }
}

#[when("the production worker posts the comment")]
async fn worker_posts(world: &mut AwaitWorld) -> anyhow::Result<()> {
    let server = MockServer::start().await;
    let success = serde_json::from_str(include_str!(
        "../../crates/comenqd/tests/fixtures/github_comment_response.json"
    ))?;
    Mock::given(method("POST"))
        .and(path("/repos/octocat/hello-world/issues/7/comments"))
        .respond_with(FailThenSucceed {
            fail_first: world.fail_first,
            attempts: world.attempts.clone(),
            success,
        })
        .mount(&server)
        .await;
    let clients = Arc::new(vec![TokenClient::new(
        "default",
        "t",
        octocrab_for(&server)?,
    )]);
    let control = WorkerControl::without_hooks(
        world
            .shutdown
            .as_ref()
            .context("shutdown missing")?
            .subscribe(),
    );
    world.worker = Some(tokio::spawn(run_worker(
        world.queue()?.clone(),
        clients,
        control,
    )));
    world.server = Some(server);
    let queue = world.queue()?.clone();
    anyhow::ensure!(poll_until(DEADLINE, Duration::from_millis(10), || async {
        matches!(dispatch_request(&queue, Request::List).await, Response::Ok { entries: Some(entries), .. } if entries.is_empty())
    }).await, "worker did not complete the post");
    Ok(())
}

#[then("awaiting succeeds with only the full comment URL on stdout")]
async fn successful_url(world: &mut AwaitWorld) -> anyhow::Result<()> {
    world.finish_client().await?;
    world.result.take().context("await result missing")??;
    anyhow::ensure!(
        world.stdout.bytes()? == format!("{URL}\n").as_bytes(),
        "await did not print the full comment URL"
    );
    Ok(())
}

#[then("the worker retried after a full cooldown")]
fn retried(world: &mut AwaitWorld) -> anyhow::Result<()> {
    let attempts = world
        .attempts
        .lock()
        .map_err(|_| anyhow::anyhow!("attempt lock poisoned"))?;
    let [first, second] = attempts.as_slice() else {
        anyhow::bail!("expected one failed and one successful attempt");
    };
    anyhow::ensure!(
        second.duration_since(*first) >= Duration::from_secs(1),
        "retry bypassed full cooldown"
    );
    Ok(())
}

#[when("I await an unknown comment")]
async fn await_unknown(world: &mut AwaitWorld) -> anyhow::Result<()> {
    world.start_client("00000000".into())?;
    world.finish_client().await
}

#[then("awaiting fails before any countdown")]
fn unknown_failed(world: &mut AwaitWorld) -> anyhow::Result<()> {
    anyhow::ensure!(
        matches!(world.result.as_ref(), Some(Err(comenq::ClientError::Daemon(message))) if message.contains("unknown comment identifier")),
        "unknown identifier did not fail"
    );
    anyhow::ensure!(
        world.stdout.bytes()?.is_empty() && world.stderr.bytes()?.is_empty(),
        "unknown identifier produced output"
    );
    Ok(())
}

#[when("the queued comment is deleted")]
async fn delete_comment(world: &mut AwaitWorld) -> anyhow::Result<()> {
    let response = dispatch_request(
        world.queue()?,
        Request::Del {
            id: world.id.clone(),
        },
    )
    .await;
    anyhow::ensure!(matches!(response, Response::Ok { .. }), "deletion failed");
    Ok(())
}

#[then("awaiting fails because the entry was removed")]
async fn removed(world: &mut AwaitWorld) -> anyhow::Result<()> {
    world.finish_client().await?;
    anyhow::ensure!(
        matches!(world.result.as_ref(), Some(Err(comenq::ClientError::Daemon(message))) if message.contains("was removed")),
        "removed entry did not fail awaiting"
    );
    anyhow::ensure!(
        world.stdout.bytes()?.is_empty(),
        "removed entry produced a URL"
    );
    Ok(())
}

#[when("I interrupt the await client")]
async fn interrupt(world: &mut AwaitWorld) -> anyhow::Result<()> {
    let client = world.client.take().context("client missing")?;
    client.abort();
    anyhow::ensure!(
        client.await.is_err_and(|error| error.is_cancelled()),
        "client was not interrupted"
    );
    world.stdout = Captured::default();
    world.stderr = Captured::default();
    Ok(())
}

#[then("the comment remains queued")]
async fn preserved(world: &mut AwaitWorld) -> anyhow::Result<()> {
    let response = dispatch_request(world.queue()?, Request::List).await;
    anyhow::ensure!(
        matches!(response, Response::Ok { entries: Some(entries), .. } if entries.len() == 1 && entries.first().is_some_and(|entry| entry.id == world.id)),
        "interruption changed the queue"
    );
    Ok(())
}

#[when("I await the same identifier again")]
fn await_again(world: &mut AwaitWorld) -> anyhow::Result<()> {
    world.start_client(world.id.clone())
}
