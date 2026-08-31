//! Round-trip tests for the client transport.
use super::{ClientError, render_response, run, run_with_writer, transact_with_timeout};
use crate::{Args, Command};
use comenq_lib::protocol::{MAX_RESPONSE_BYTES, PendingEntry, Request, Response};
use std::io::{self, Write};
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::sync::Notify;

struct BrokenPipeWriter;

impl Write for BrokenPipeWriter {
    fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
        Err(io::Error::from(io::ErrorKind::BrokenPipe))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct FailingWriter;

impl Write for FailingWriter {
    fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
        Err(io::Error::from(io::ErrorKind::PermissionDenied))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn put_args(socket: std::path::PathBuf) -> Args {
    Args {
        socket: Some(socket),
        command: Command::Put {
            repo_slug: "octocat/hello-world".parse().expect("slug"),
            pr_number: 1,
            comment_body: "Hi".into(),
            now: false,
        },
    }
}

fn args(socket: std::path::PathBuf, command: Command) -> Args {
    Args {
        socket: Some(socket),
        command,
    }
}

fn sample_entry() -> PendingEntry {
    PendingEntry {
        id: "1a2b3c4d".into(),
        eta_seconds: 0,
        owner: "octocat".into(),
        repo: "hello-world".into(),
        pr_number: 1,
        body: "Hi".into(),
    }
}

/// Accept one connection, capture the request, and reply.
fn spawn_daemon(listener: UnixListener, reply: Response) -> tokio::task::JoinHandle<Request> {
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.expect("read");
        let request = serde_json::from_slice::<Request>(&buf).expect("deserialize");
        let bytes = serde_json::to_vec(&reply).expect("serialize reply");
        stream.write_all(&bytes).await.expect("write reply");
        request
    })
}

#[tokio::test]
async fn run_sends_put_request_and_accepts_reply() {
    let dir = tempdir().expect("temp dir");
    let socket = dir.path().join("sock");
    let listener = UnixListener::bind(&socket).expect("bind socket");
    let reply = Response::entry(sample_entry());
    let accept = spawn_daemon(listener, reply);

    run(put_args(socket)).await.expect("run succeeds");
    let request = accept.await.expect("join");
    let Request::Put { request, immediate } = request else {
        panic!("expected put request, got {request:?}");
    };
    assert_eq!(request.owner, "octocat");
    assert_eq!(request.repo, "hello-world");
    assert_eq!(request.pr_number, 1);
    assert_eq!(request.body, "Hi");
    assert!(!immediate, "put must default to deferred posting");
}

#[tokio::test]
async fn successful_put_ignores_a_broken_output_pipe() {
    let dir = tempdir().expect("temp dir");
    let socket = dir.path().join("sock");
    let listener = UnixListener::bind(&socket).expect("bind socket");
    let accept = spawn_daemon(listener, Response::entry(sample_entry()));
    let mut output = BrokenPipeWriter;

    run_with_writer(put_args(socket), &mut output)
        .await
        .expect("broken output pipe should complete the command");
    let request = accept.await.expect("join");
    assert!(matches!(request, Request::Put { .. }));
}

#[test]
fn output_failures_are_returned_to_the_caller() {
    let mut output = FailingWriter;
    let err = render_response(&Command::List, None, Some(vec![]), &mut output)
        .expect_err("non-broken output failure must surface");
    assert!(
        matches!(err, ClientError::Output(error) if error.kind() == io::ErrorKind::PermissionDenied)
    );
}

#[tokio::test]
async fn run_surfaces_daemon_errors() {
    let dir = tempdir().expect("temp dir");
    let socket = dir.path().join("sock");
    let listener = UnixListener::bind(&socket).expect("bind socket");
    let accept = spawn_daemon(listener, Response::error("queue unavailable"));

    let err = run(put_args(socket)).await.expect_err("should error");
    assert!(matches!(err, ClientError::Daemon(m) if m == "queue unavailable"));
    accept.await.expect("join");
}

#[tokio::test]
async fn run_rejects_mismatched_reply() {
    let dir = tempdir().expect("temp dir");
    let socket = dir.path().join("sock");
    let listener = UnixListener::bind(&socket).expect("bind socket");
    // A bare Ok reply lacks the entry a put expects.
    let accept = spawn_daemon(listener, Response::ok());

    let err = run(put_args(socket)).await.expect_err("should error");
    assert!(matches!(err, ClientError::UnexpectedResponse));
    accept.await.expect("join");
}

#[tokio::test]
async fn run_rejects_surplus_response_payloads() {
    let commands_and_replies = [
        (
            Command::Put {
                repo_slug: "octocat/hello-world".parse().expect("slug"),
                pr_number: 1,
                comment_body: "Hi".into(),
                now: false,
            },
            Response::Ok {
                entry: Some(sample_entry()),
                entries: Some(vec![]),
            },
        ),
        (
            Command::List,
            Response::Ok {
                entry: Some(sample_entry()),
                entries: Some(vec![]),
            },
        ),
        (
            Command::Bump {
                id: "1a2b3c4d".into(),
            },
            Response::entry(sample_entry()),
        ),
    ];

    for (command, reply) in commands_and_replies {
        let dir = tempdir().expect("temp dir");
        let socket = dir.path().join("sock");
        let listener = UnixListener::bind(&socket).expect("bind socket");
        let accept = spawn_daemon(listener, reply);

        let err = run(args(socket, command))
            .await
            .expect_err("surplus payload must fail");
        assert!(matches!(err, ClientError::UnexpectedResponse));
        accept.await.expect("join");
    }
}

#[tokio::test]
async fn run_accepts_mutation_responses_without_payloads() {
    let commands = [
        Command::Bump {
            id: "1a2b3c4d".into(),
        },
        Command::Bust {
            id: "1a2b3c4d".into(),
        },
        Command::Del {
            id: "1a2b3c4d".into(),
        },
    ];

    for command in commands {
        let dir = tempdir().expect("temp dir");
        let socket = dir.path().join("sock");
        let listener = UnixListener::bind(&socket).expect("bind socket");
        let accept = spawn_daemon(listener, Response::ok());

        run(args(socket, command)).await.expect("run succeeds");
        accept.await.expect("join");
    }
}

#[tokio::test]
async fn run_errors_when_socket_missing() {
    let dir = tempdir().expect("temp dir");
    let socket = dir.path().join("nosock");

    let err = run(put_args(socket)).await.expect_err("should error");
    assert!(matches!(err, ClientError::Connect(_)));
}

#[tokio::test]
async fn transaction_times_out_when_the_daemon_keeps_the_reply_open() {
    let dir = tempdir().expect("temp dir");
    let socket = dir.path().join("sock");
    let listener = UnixListener::bind(&socket).expect("bind socket");
    let request_received = Arc::new(Notify::new());
    let peer_received = Arc::clone(&request_received);
    let peer = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let mut request = Vec::new();
        stream
            .read_to_end(&mut request)
            .await
            .expect("read request");
        peer_received.notify_one();
        std::future::pending::<()>().await;
    });

    let err = transact_with_timeout(&[socket], &Request::List, Duration::from_millis(10))
        .await
        .expect_err("open reply must time out");
    assert!(matches!(err, ClientError::ReplyTimeout));
    request_received.notified().await;
    peer.abort();
}

#[tokio::test]
async fn transaction_rejects_an_oversized_daemon_reply() {
    let dir = tempdir().expect("temp dir");
    let socket = dir.path().join("sock");
    let listener = UnixListener::bind(&socket).expect("bind socket");
    let peer = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let mut request = Vec::new();
        stream
            .read_to_end(&mut request)
            .await
            .expect("read request");
        stream
            .write_all(&vec![b'x'; MAX_RESPONSE_BYTES + 1])
            .await
            .expect("write oversized reply");
    });

    let err = transact_with_timeout(&[socket], &Request::List, Duration::from_secs(1))
        .await
        .expect_err("oversized reply must fail");
    assert!(matches!(err, ClientError::ReplyTooLarge));
    peer.await.expect("join");
}

/// A stale socket file must not shadow a live daemon later in the list.
#[tokio::test]
async fn connect_first_skips_stale_sockets() {
    let dir = tempdir().expect("temp dir");
    let stale = dir.path().join("stale.sock");
    drop(UnixListener::bind(&stale).expect("bind stale socket"));
    assert!(stale.exists(), "stale socket file should remain on disk");

    let live = dir.path().join("live.sock");
    let listener = UnixListener::bind(&live).expect("bind live socket");

    let stream = super::connect_first(&[stale, live])
        .await
        .expect("should fall back to the live socket");
    drop(stream);
    drop(listener);
}

/// Every failed candidate must still report a connection error.
#[tokio::test]
async fn connect_first_reports_failure_when_all_candidates_fail() {
    let dir = tempdir().expect("temp dir");
    let stale = dir.path().join("stale.sock");
    drop(UnixListener::bind(&stale).expect("bind stale socket"));
    let missing = dir.path().join("missing.sock");

    let err = super::connect_first(&[stale, missing])
        .await
        .expect_err("all candidates should fail");
    let ClientError::Connect(source) = err else {
        panic!("expected connection error, got {err:?}");
    };
    assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
}
