//! Tests for listener behaviour. Freezes Tokio time, advances beyond the client read timeout, and asserts idle connections time out.

use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tokio::task::yield_now;
use tokio::time::advance;

use comenq_lib::CommentRequest;
use comenqd::daemon::listener::{CLIENT_READ_TIMEOUT_SECS, MAX_REQUEST_BYTES, handle_client};

#[tokio::test(start_paused = true)]
async fn handle_client_times_out_if_client_does_not_close() {
    let (client_tx, _rx) = mpsc::channel(1);
    let (client, server) = UnixStream::pair().expect("pair");
    let handle = tokio::spawn(handle_client(server, client_tx));
    // Ensure the handler registers its timeout before advancing time.
    yield_now().await;
    advance(Duration::from_secs(CLIENT_READ_TIMEOUT_SECS + 1)).await;
    let err = handle.await.expect("join").expect_err("expected timeout");
    assert!(err.to_string().contains("client read timed out"));
    drop(client);
}

#[tokio::test]
async fn handle_client_accepts_exact_max_request_bytes() {
    let (tx, mut rx) = mpsc::channel(1);
    let (mut client, server) = UnixStream::pair().expect("pair");
    let handle = tokio::spawn(handle_client(server, tx));

    let base = CommentRequest {
        owner: String::new(),
        repo: String::new(),
        pr_number: 0,
        body: String::new(),
    };
    let base_len = serde_json::to_vec(&base).expect("serialise").len();
    let body_len = MAX_REQUEST_BYTES - base_len;
    let mut request = base;
    request.body = "a".repeat(body_len);
    let payload = serde_json::to_vec(&request).expect("serialise");

    client.write_all(&payload).await.expect("write");
    client.shutdown().await.expect("shutdown");

    handle.await.expect("join").expect("handle");
    let msg = rx.recv().await.expect("recv");
    assert_eq!(msg, payload);
}

#[tokio::test]
async fn handle_client_rejects_request_exceeding_limit() {
    let (tx, _rx) = mpsc::channel(1);
    let (mut client, server) = UnixStream::pair().expect("pair");
    let handle = tokio::spawn(handle_client(server, tx));

    let base = CommentRequest {
        owner: String::new(),
        repo: String::new(),
        pr_number: 0,
        body: String::new(),
    };
    let base_len = serde_json::to_vec(&base).expect("serialise").len();
    let body_len = MAX_REQUEST_BYTES - base_len + 1;
    let mut request = base;
    request.body = "a".repeat(body_len);
    let payload = serde_json::to_vec(&request).expect("serialise");

    client.write_all(&payload).await.expect("write");
    client.shutdown().await.expect("shutdown");

    let err = handle
        .await
        .expect("join")
        .expect_err("expected size error");
    assert!(err.to_string().contains("client payload exceeds"));
}
