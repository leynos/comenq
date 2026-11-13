//! Unix-socket history paging tests for the client.

use super::super::{run_with_env, run_with_writer};
use crate::{Args, Command};
use comenq_lib::protocol::{HistoryEntry, Request, Response};
#[cfg(unix)]
use std::ffi::OsString;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt as _;
use tempfile::tempdir;
use test_support::mock_env;
#[cfg(unix)]
use test_support::split_env;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

#[tokio::test]
async fn run_sends_hist_request_and_renders_its_reply() {
    let dir = tempdir().expect("temp dir");
    let socket = dir.path().join("sock");
    let listener = UnixListener::bind(&socket).expect("bind socket");
    let accept = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).await.expect("read request");
        let request = serde_json::from_slice::<Request>(&bytes).expect("decode request");
        let reply = Response::history(
            vec![HistoryEntry {
                id: "1a2b3c4d".into(),
                posted_at: 1_000,
                success: true,
                error: None,
                token_hash: "0123456789abcdef".repeat(4),
                owner: "octocat".into(),
                repo: "hello-world".into(),
                pr_number: 1,
                body: "Hi".into(),
            }],
            None,
        );
        stream
            .write_all(&serde_json::to_vec(&reply).expect("encode reply"))
            .await
            .expect("write reply");
        request
    });

    let args = Args {
        socket: Some(socket),
        command: Command::Hist { limit: Some(2) },
    };
    let mut output = Vec::new();
    run_with_writer(args, &mut output)
        .await
        .expect("history request succeeds");

    assert_eq!(
        accept.await.expect("join daemon"),
        Request::Hist {
            limit: Some(2),
            cursor: None,
        }
    );
    let output = String::from_utf8(output).expect("UTF-8 output");
    assert!(output.contains("ok    01234567  octocat/hello-world#1  Hi"));
}

/// The injected-environment entry point must take the same history path as
/// the compatibility wrapper.
#[tokio::test]
async fn run_with_env_sends_a_hist_request() {
    let dir = tempdir().expect("temp dir");
    let socket = dir.path().join("sock");
    let listener = UnixListener::bind(&socket).expect("bind socket");
    let accept = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).await.expect("read request");
        let request = serde_json::from_slice::<Request>(&bytes).expect("decode request");
        let reply = Response::history(Vec::new(), None);
        stream
            .write_all(&serde_json::to_vec(&reply).expect("encode reply"))
            .await
            .expect("write reply");
        request
    });

    let args = Args {
        socket: Some(socket),
        command: Command::Hist { limit: None },
    };
    run_with_env(args, &mock_env([] as [(&str, &str); 0]))
        .await
        .expect("history request succeeds");

    assert_eq!(
        accept.await.expect("join daemon"),
        Request::Hist {
            limit: None,
            cursor: None,
        }
    );
}

/// A non-Unicode `COMENQ_SOCKET` must select that exact socket when the
/// command runs, even though the Unicode view reports a decoy path.
///
/// The daemon receives a `list` request, proving the client connected to the
/// injected runtime socket rather than to a discovered or decoy one.
#[cfg(unix)]
#[tokio::test]
async fn run_with_env_connects_to_a_non_unicode_socket_override() {
    let dir = tempdir().expect("temp dir");
    let socket = dir
        .path()
        .join(OsString::from_vec(b"comenq-\xff.sock".to_vec()));
    let listener = UnixListener::bind(&socket).expect("bind socket");
    let accept = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).await.expect("read request");
        let request = serde_json::from_slice::<Request>(&bytes).expect("decode request");
        stream
            .write_all(&serde_json::to_vec(&Response::entries(Vec::new())).expect("encode reply"))
            .await
            .expect("write reply");
        request
    });

    let env = split_env(
        [("COMENQ_SOCKET", "/tmp/decoy.sock")],
        [("COMENQ_SOCKET", socket.into_os_string())],
    );
    let args = Args {
        socket: None,
        command: Command::List,
    };

    run_with_env(args, &env)
        .await
        .expect("list request succeeds");

    assert_eq!(accept.await.expect("join daemon"), Request::List);
}

#[tokio::test]
async fn run_follows_history_cursors_until_the_snapshot_is_complete() {
    let dir = tempdir().expect("temp dir");
    let socket = dir.path().join("sock");
    let listener = UnixListener::bind(&socket).expect("bind socket");
    let accept = tokio::spawn(async move {
        let mut requests = Vec::new();
        for (index, next_cursor) in [
            (
                0,
                Some(comenq_lib::protocol::HistoryCursor {
                    offset: 10,
                    end_offset: 20,
                }),
            ),
            (1, None),
        ] {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut bytes = Vec::new();
            stream.read_to_end(&mut bytes).await.expect("read request");
            requests.push(serde_json::from_slice::<Request>(&bytes).expect("decode request"));
            let reply = Response::history(
                vec![HistoryEntry {
                    id: format!("0000000{index}"),
                    posted_at: 1_000 + index,
                    success: true,
                    error: None,
                    token_hash: "0123456789abcdef".repeat(4),
                    owner: "octocat".into(),
                    repo: "hello-world".into(),
                    pr_number: 1,
                    body: format!("comment {index}"),
                }],
                next_cursor,
            );
            stream
                .write_all(&serde_json::to_vec(&reply).expect("encode reply"))
                .await
                .expect("write reply");
        }
        requests
    });

    let args = Args {
        socket: Some(socket),
        command: Command::Hist { limit: Some(3) },
    };
    let mut output = Vec::new();
    run_with_writer(args, &mut output)
        .await
        .expect("paged history request succeeds");

    assert_eq!(
        accept.await.expect("join daemon"),
        vec![
            Request::Hist {
                limit: Some(3),
                cursor: None,
            },
            Request::Hist {
                limit: None,
                cursor: Some(comenq_lib::protocol::HistoryCursor {
                    offset: 10,
                    end_offset: 20,
                }),
            },
        ]
    );
    let output = String::from_utf8(output).expect("UTF-8 output");
    assert!(output.contains("comment 0"));
    assert!(output.contains("comment 1"));
}
