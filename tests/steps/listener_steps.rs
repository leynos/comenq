//! Behavioural test steps for the listener task.
#![expect(clippy::expect_used, reason = "simplify test failure output")]
#![expect(clippy::unwrap_used, reason = "simplify test failure output")]

use std::sync::Arc;
use std::time::Duration;

use cucumber::World;
use cucumber::{given, then, when};
use tempfile::TempDir;
use test_support::{SOCKET_RETRY_COUNT, SOCKET_RETRY_DELAY, temp_config, wait_for_file};
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio::sync::{mpsc, watch};

use comenq_lib::CommentRequest;
use comenqd::config::Config;
use comenqd::daemon::{queue_writer, run_listener};
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
async fn running_listener(world: &mut ListenerWorld) {
    let dir = TempDir::new().expect("tempdir");
    let cfg = Arc::new(temp_config(&dir));
    let (sender, receiver) = channel(&cfg.queue_path).expect("channel");
    let (client_tx, writer_rx) = mpsc::unbounded_channel();
    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let cfg_clone = cfg.clone();
    let writer = tokio::spawn(async move {
        queue_writer(sender, writer_rx).await.unwrap();
    });
    let handle = tokio::spawn(async move {
        run_listener(cfg_clone, client_tx, shutdown_rx)
            .await
            .unwrap();
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
        .expect("config not initialised in ListenerWorld")
        .socket_path;
    assert!(
        wait_for_file(socket_path, SOCKET_RETRY_COUNT, SOCKET_RETRY_DELAY).await,
        "socket file {} not created within timeout",
        socket_path.display()
    );
}

#[when("a client sends a valid request")]
async fn client_sends_valid(world: &mut ListenerWorld) {
    let cfg = world.cfg.as_ref().unwrap();
    let mut stream = UnixStream::connect(&cfg.socket_path)
        .await
        .expect("connect");
    let req = CommentRequest {
        owner: "o".into(),
        repo: "r".into(),
        pr_number: 1,
        body: "b".into(),
    };
    let data = serde_json::to_vec(&req).unwrap();
    stream.write_all(&data).await.unwrap();
    stream.shutdown().await.expect("shutdown");
}

#[when("a client sends invalid JSON")]
async fn client_sends_invalid(world: &mut ListenerWorld) {
    let cfg = world.cfg.as_ref().unwrap();
    let mut stream = UnixStream::connect(&cfg.socket_path)
        .await
        .expect("connect");
    stream.write_all(b"not json").await.unwrap();
    stream.shutdown().await.expect("shutdown");
}

#[then("the request is enqueued")]
async fn request_enqueued(world: &mut ListenerWorld) {
    let receiver = world.receiver.as_mut().unwrap();
    let guard = receiver.recv().await.expect("recv");
    let req: CommentRequest = serde_json::from_slice(&guard).unwrap();
    assert_eq!(req.owner, "o");
}

#[then("the request is rejected")]
async fn request_rejected(world: &mut ListenerWorld) {
    let receiver = world.receiver.as_mut().unwrap();
    let res = tokio::time::timeout(Duration::from_millis(100), receiver.recv()).await;
    assert!(res.is_err());
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
