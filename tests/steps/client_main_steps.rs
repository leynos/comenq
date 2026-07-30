//! Behavioural test steps for the client binary's interaction with the daemon.

use anyhow::Context as _;
use comenq::{Args, ClientError, Command, run};
use comenq_lib::protocol::{PendingEntry, Request, Response};
use cucumber::{World, given, then, when};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

#[derive(Debug, Default, World)]
pub struct ClientWorld {
    args: Option<Args>,
    tempdir: Option<TempDir>,
    server: Option<tokio::task::JoinHandle<anyhow::Result<Vec<u8>>>>,
    result: Option<Result<(), ClientError>>,
}

/// Build the default client arguments targeting `socket`.
fn base_args(socket: std::path::PathBuf) -> anyhow::Result<Args> {
    Ok(Args {
        socket: Some(socket),
        command: Command::Put {
            repo_slug: "octocat/hello-world".parse().context("slug")?,
            pr_number: 1,
            comment_body: "Hi".into(),
            now: false,
        },
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
        // Reply so the client's transaction completes.
        let reply = Response::entry(PendingEntry {
            id: "1a2b3c4d".into(),
            eta_seconds: 0,
            owner: "octocat".into(),
            repo: "hello-world".into(),
            pr_number: 1,
            body: "Hi".into(),
        });
        let bytes = serde_json::to_vec(&reply).context("serialize reply")?;
        stream.write_all(&bytes).await.context("write reply")?;
        Ok(buf)
    });

    world.args = Some(base_args(socket)?);
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
    let request: Request = serde_json::from_slice(&data).context("parse")?;
    let Request::Put {
        request: req,
        immediate,
    } = request
    else {
        anyhow::bail!("expected put request, got {request:?}");
    };
    anyhow::ensure!(!immediate, "plain put must not request immediate posting");
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
