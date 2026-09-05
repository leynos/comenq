//! Behavioural test steps for the listener task.

use std::sync::Arc;

use anyhow::Context as _;
use cucumber::World;
use cucumber::{given, then, when};
use tempfile::TempDir;
use test_support::{SOCKET_RETRY_COUNT, SOCKET_RETRY_DELAY, temp_config, wait_for_file};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::watch;

use comenq_lib::CommentRequest;
use comenq_lib::protocol::{PendingEntry, Request, Response};
use comenqd::config::Config;
use comenqd::daemon::{SharedQueue, listener::run_listener};

#[derive(Default, World)]
pub struct ListenerWorld {
    dir: Option<TempDir>,
    cfg: Option<Arc<Config>>,
    queue: Option<Arc<SharedQueue>>,
    shutdown: Option<watch::Sender<()>>,
    handle: Option<tokio::task::JoinHandle<()>>,
    response: Option<Response>,
}

impl std::fmt::Debug for ListenerWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListenerWorld").finish()
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

#[given("a running listener task")]
async fn running_listener(world: &mut ListenerWorld) -> anyhow::Result<()> {
    let dir = TempDir::new().context("tempdir")?;
    let cfg = Arc::new(Config::from(temp_config(&dir)));
    let queue = SharedQueue::open(cfg.clone()).context("open queue")?;
    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let listener_queue = queue.clone();
    let handle = tokio::spawn(async move {
        if let Err(error) = run_listener(listener_queue, shutdown_rx).await {
            panic!("listener task failed: {error}");
        }
    });
    world.dir = Some(dir);
    world.cfg = Some(cfg);
    world.queue = Some(queue);
    world.shutdown = Some(shutdown_tx);
    world.handle = Some(handle);

    let socket_path = &world
        .cfg
        .as_ref()
        .context("config not initialized in ListenerWorld")?
        .socket_path;
    assert!(
        wait_for_file(socket_path, SOCKET_RETRY_COUNT, SOCKET_RETRY_DELAY).await,
        "socket file {} not created within timeout",
        socket_path.display()
    );
    Ok(())
}

/// Send raw `bytes` to the daemon socket and parse the JSON reply.
async fn send_raw(world: &mut ListenerWorld, bytes: &[u8]) -> anyhow::Result<Response> {
    let cfg = world.cfg.as_ref().context("config initialized")?;
    let mut stream = UnixStream::connect(&cfg.socket_path)
        .await
        .context("connect")?;
    stream.write_all(bytes).await.context("write request")?;
    stream.shutdown().await.context("shutdown")?;
    let mut reply = Vec::new();
    stream.read_to_end(&mut reply).await.context("read reply")?;
    serde_json::from_slice(&reply).context("parse reply")
}

#[when("a client sends a valid request")]
async fn client_sends_valid(world: &mut ListenerWorld) -> anyhow::Result<()> {
    let request = Request::Put {
        request: CommentRequest {
            owner: "o".into(),
            repo: "r".into(),
            pr_number: 1,
            body: "b".into(),
        },
        immediate: false,
    };
    let data = serde_json::to_vec(&request).context("serialize request")?;
    let response = send_raw(world, &data).await?;
    world.response = Some(response);
    Ok(())
}

#[when("a client sends invalid JSON")]
async fn client_sends_invalid(world: &mut ListenerWorld) -> anyhow::Result<()> {
    let response = send_raw(world, b"not json").await?;
    world.response = Some(response);
    Ok(())
}

#[then("the request is enqueued")]
async fn request_enqueued(world: &mut ListenerWorld) -> anyhow::Result<()> {
    match world.response.take() {
        Some(Response::Ok {
            entry: Some(entry), ..
        }) => {
            assert_eq!(entry.owner, "o");
            assert_eq!(entry.id.len(), 8);
        }
        other => anyhow::bail!("expected put reply with entry, got {other:?}"),
    }
    let queue = world.queue.as_ref().context("queue initialized")?;
    let entries = listed_entries(queue).await?;
    assert_eq!(entries.len(), 1);
    Ok(())
}

#[then("the request is rejected")]
async fn request_rejected(world: &mut ListenerWorld) -> anyhow::Result<()> {
    match world.response.take() {
        Some(Response::Error { .. }) => {}
        other => anyhow::bail!("expected error reply, got {other:?}"),
    }
    let queue = world.queue.as_ref().context("queue initialized")?;
    assert!(listed_entries(queue).await?.is_empty());
    Ok(())
}

impl Drop for ListenerWorld {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}
