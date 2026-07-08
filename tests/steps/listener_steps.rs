//! Behavioural test steps for the listener task.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use cucumber::World;
use cucumber::{given, then, when};
use tempfile::TempDir;
use test_support::{SOCKET_RETRY_COUNT, SOCKET_RETRY_DELAY, temp_config, wait_for_file};
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio::sync::{mpsc, watch};

use comenq_lib::CommentRequest;
use comenqd::config::Config;
use comenqd::daemon::{listener::run_listener, queue_writer};
use yaque::channel;

#[derive(Default, World)]
pub struct ListenerWorld {
    dir: Option<TempDir>,
    cfg: Option<Arc<Config>>,
    receiver: Option<yaque::Receiver>,
    shutdown: Option<watch::Sender<()>>,
    writer: Option<tokio::task::JoinHandle<()>>,
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl std::fmt::Debug for ListenerWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListenerWorld").finish()
    }
}

#[given("a running listener task")]
async fn running_listener(world: &mut ListenerWorld) -> anyhow::Result<()> {
    let dir = TempDir::new().context("tempdir")?;
    let cfg = Arc::new(Config::from(temp_config(&dir)));
    let (sender, receiver) = channel(&cfg.queue_path).context("channel")?;
    let (client_tx, writer_rx) = mpsc::channel(4);
    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let cfg_clone = cfg.clone();
    let writer = tokio::spawn(async move {
        // Intentionally ignore the returned receiver: the writer remains
        // alive for the test's duration and the channel is not reused.
        let _ = queue_writer(sender, writer_rx).await;
    });
    let handle = tokio::spawn(async move {
        if let Err(error) = run_listener(cfg_clone, client_tx, shutdown_rx).await {
            panic!("listener task failed: {error}");
        }
    });
    world.dir = Some(dir);
    world.cfg = Some(cfg);
    world.shutdown = Some(shutdown_tx);
    world.writer = Some(writer);
    world.receiver = Some(receiver);
    world.handle = Some(handle);

    let socket_path = &world
        .cfg
        .as_ref()
        .context("config not initialised in ListenerWorld")?
        .socket_path;
    assert!(
        wait_for_file(socket_path, SOCKET_RETRY_COUNT, SOCKET_RETRY_DELAY).await,
        "socket file {} not created within timeout",
        socket_path.display()
    );
    Ok(())
}

#[when("a client sends a valid request")]
async fn client_sends_valid(world: &mut ListenerWorld) -> anyhow::Result<()> {
    let cfg = world.cfg.as_ref().context("config initialised")?;
    let mut stream = UnixStream::connect(&cfg.socket_path)
        .await
        .context("connect")?;
    let req = CommentRequest {
        owner: "o".into(),
        repo: "r".into(),
        pr_number: 1,
        body: "b".into(),
    };
    let data = serde_json::to_vec(&req).context("serialise request")?;
    stream.write_all(&data).await.context("write request")?;
    stream.shutdown().await.context("shutdown")?;
    Ok(())
}

#[when("a client sends invalid JSON")]
async fn client_sends_invalid(world: &mut ListenerWorld) -> anyhow::Result<()> {
    let cfg = world.cfg.as_ref().context("config initialised")?;
    let mut stream = UnixStream::connect(&cfg.socket_path)
        .await
        .context("connect")?;
    stream
        .write_all(b"not json")
        .await
        .context("write request")?;
    stream.shutdown().await.context("shutdown")?;
    Ok(())
}

#[then("the request is enqueued")]
async fn request_enqueued(world: &mut ListenerWorld) -> anyhow::Result<()> {
    let receiver = world.receiver.as_mut().context("receiver initialised")?;
    let guard = receiver.recv().await.context("recv")?;
    let req: CommentRequest = serde_json::from_slice(&guard).context("parse request")?;
    assert_eq!(req.owner, "o");
    Ok(())
}

#[then("the request is rejected")]
async fn request_rejected(world: &mut ListenerWorld) -> anyhow::Result<()> {
    let receiver = world.receiver.as_mut().context("receiver initialised")?;
    let res = tokio::time::timeout(Duration::from_millis(100), receiver.recv()).await;
    assert!(res.is_err());
    Ok(())
}

impl Drop for ListenerWorld {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(writer) = self.writer.take() {
            writer.abort();
        }
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}
