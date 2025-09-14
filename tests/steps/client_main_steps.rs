//! Behavioural test steps for the client binary's interaction with the daemon.

use comenq::{Args, ClientError, run};
use comenq_lib::CommentRequest;
use cucumber::{World, given, then, when};
use tempfile::TempDir;
use tokio::io::AsyncReadExt;
use tokio::net::UnixListener;

#[derive(Debug, Default, World)]
pub struct ClientWorld {
    args: Option<Args>,
    tempdir: Option<TempDir>,
    server: Option<tokio::task::JoinHandle<Vec<u8>>>,
    result: Option<Result<(), ClientError>>,
}

#[given("a dummy daemon listening on a socket")]
#[expect(clippy::expect_used, reason = "simplify test failure output")]
fn dummy_daemon(world: &mut ClientWorld) {
    let dir = TempDir::new().expect("tempdir");
    let socket = dir.path().join("sock");
    let listener = UnixListener::bind(&socket).expect("bind");

    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.expect("read");
        buf
    });

    world.args = Some(Args {
        repo_slug: "octocat/hello-world".parse().expect("slug"),
        pr_number: 1,
        comment_body: "Hi".into(),
        socket: socket.clone(),
    });
    world.tempdir = Some(dir);
    world.server = Some(handle);
}

#[given("no daemon is listening on a socket")]
#[expect(clippy::expect_used, reason = "simplify test failure output")]
fn no_daemon(world: &mut ClientWorld) {
    let dir = TempDir::new().expect("tempdir");
    let socket = dir.path().join("sock");
    world.args = Some(Args {
        repo_slug: "octocat/hello-world".parse().expect("slug"),
        pr_number: 1,
        comment_body: "Hi".into(),
        socket,
    });
    world.tempdir = Some(dir);
}

#[when("the client sends the request")]
#[expect(clippy::expect_used, reason = "simplify test failure output")]
async fn send_request(world: &mut ClientWorld) {
    let args = world.args.clone().expect("args");
    world.result = Some(run(args).await);
}

#[then("the daemon receives the request")]
#[expect(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "simplify test failure output"
)]
async fn daemon_receives(world: &mut ClientWorld) {
    let handle = world.server.take().expect("server handle");
    let data = handle.await.expect("join");
    let req: CommentRequest = serde_json::from_slice(&data).expect("parse");
    assert_eq!(req.owner, "octocat");
    assert_eq!(req.repo, "hello-world");
    assert_eq!(req.pr_number, 1);
    assert_eq!(req.body, "Hi");
    assert!(world.result.take().unwrap().is_ok());
}

#[then("an error occurs")]
fn an_error_occurs(world: &mut ClientWorld) {
    match world.result.take() {
        Some(Err(ClientError::Connect(_))) => {}
        other => panic!("unexpected result: {other:?}"),
    }
}
