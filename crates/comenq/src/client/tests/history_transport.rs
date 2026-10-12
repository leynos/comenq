//! Unix-socket history paging tests for the client.

use super::super::run_with_writer;
use crate::{Args, Command};
use comenq_lib::protocol::{HistoryEntry, Request, Response};
use tempfile::tempdir;
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
    assert!(output.contains("ok    octocat/hello-world#1  Hi"));
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
