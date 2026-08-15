//! Behavioural test steps for the client binary's interaction with the daemon.

use anyhow::Context as _;
use comenq::{Args, ClientError, run};
use comenq_lib::CommentRequest;
use cucumber::{World, given, then, when};
use std::fs;
use tempfile::TempDir;
use test_support::EnvVarGuard;
use tokio::io::AsyncReadExt;
use tokio::net::UnixListener;

#[derive(Debug, Default, World)]
pub struct ClientWorld {
    args: Option<Args>,
    tempdir: Option<TempDir>,
    runtime_dir_guard: Option<EnvVarGuard>,
    server: Option<tokio::task::JoinHandle<anyhow::Result<Vec<u8>>>>,
    result: Option<Result<(), ClientError>>,
}

/// Build the default client arguments targeting `socket`.
fn base_args(socket: std::path::PathBuf) -> anyhow::Result<Args> {
    Ok(Args {
        repo_slug: "octocat/hello-world".parse().context("slug")?,
        pr_number: 1,
        comment_body: "Hi".into(),
        socket: Some(socket),
    })
}

#[given("a dummy daemon listening on a socket")]
fn dummy_daemon(world: &mut ClientWorld) -> anyhow::Result<()> {
    let dir = TempDir::new().context("tempdir")?;
    let socket = dir.path().join("sock");
    let listener = UnixListener::bind(&socket).context("bind")?;

    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.context("accept")?;
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.context("read")?;
        Ok(buf)
    });

    world.args = Some(base_args(socket)?);
    world.tempdir = Some(dir);
    world.server = Some(handle);
    Ok(())
}

#[given("a dummy daemon listening on a user runtime socket")]
fn user_runtime_daemon(world: &mut ClientWorld) -> anyhow::Result<()> {
    let dir = TempDir::new().context("tempdir")?;
    let socket = dir.path().join("comenq/comenq.sock");
    let parent = socket.parent().context("socket parent")?;
    fs::create_dir_all(parent).context("create runtime socket directory")?;
    let listener = UnixListener::bind(&socket).context("bind")?;

    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.context("accept")?;
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.context("read")?;
        Ok(buf)
    });

    world.args = Some(Args {
        repo_slug: "octocat/hello-world".parse().context("slug")?,
        pr_number: 1,
        comment_body: "Hi".into(),
        socket: None,
    });
    world.runtime_dir_guard = Some(EnvVarGuard::set(
        "XDG_RUNTIME_DIR",
        dir.path()
            .to_str()
            .context("runtime directory path is UTF-8")?,
    ));
    world.tempdir = Some(dir);
    world.server = Some(handle);
    Ok(())
}

#[given("no daemon is listening on a socket")]
fn no_daemon(world: &mut ClientWorld) -> anyhow::Result<()> {
    let dir = TempDir::new().context("tempdir")?;
    let socket = dir.path().join("sock");
    world.args = Some(base_args(socket)?);
    world.tempdir = Some(dir);
    Ok(())
}

#[when("the client sends the request")]
async fn send_request(world: &mut ClientWorld) -> anyhow::Result<()> {
    let args = world.args.clone().context("args")?;
    world.result = Some(run(args).await);
    Ok(())
}

#[then("the daemon receives the request")]
async fn daemon_receives(world: &mut ClientWorld) -> anyhow::Result<()> {
    let handle = world.server.take().context("server handle")?;
    let data = handle.await.context("join")??;
    let req: CommentRequest = serde_json::from_slice(&data).context("parse")?;
    assert_eq!(req.owner, "octocat");
    assert_eq!(req.repo, "hello-world");
    assert_eq!(req.pr_number, 1);
    assert_eq!(req.body, "Hi");
    let result = world.result.take().context("client result recorded")?;
    assert!(result.is_ok());
    Ok(())
}

#[then("an error occurs")]
fn an_error_occurs(world: &mut ClientWorld) {
    match world.result.take() {
        Some(Err(ClientError::Connect(_))) => {}
        other => panic!("unexpected result: {other:?}"),
    }
}
