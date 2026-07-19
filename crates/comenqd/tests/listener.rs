//! Tests for listener behaviour. Freezes Tokio time, advances beyond the client
//! read timeout, and asserts idle connections time out. Also exercises the
//! request size limit at and just past the boundary.

use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::task::yield_now;
use tokio::time::advance;

use comenq_lib::CommentRequest;
use comenq_lib::protocol::{Request, Response};
use comenqd::config::Config;
use comenqd::daemon::SharedQueue;
use comenqd::daemon::listener::{CLIENT_READ_TIMEOUT_SECS, MAX_REQUEST_BYTES, handle_client};
use test_support::temp_config;

/// Convert a test configuration into the runtime `Config`.
///
/// Mirrors the helper in `daemon.rs`: the `From` conversion needs the
/// `test-support` feature, which coverage builds disable.
#[cfg(feature = "test-support")]
fn cfg_from(cfg: test_support::daemon::TestConfig) -> Config {
    Config::from(cfg)
}

#[cfg(not(feature = "test-support"))]
fn cfg_from(cfg: test_support::daemon::TestConfig) -> Config {
    Config {
        github_token: cfg.github_token,
        github_token_file: None,
        socket_path: cfg.socket_path,
        queue_path: cfg.queue_path,
        cooldown_period_seconds: cfg.cooldown_period_seconds,
        cooldown_flutter_seconds: 0,
        restart_min_delay_ms: cfg.restart_min_delay_ms,
        github_api_timeout_secs: cfg.github_api_timeout_secs,
    }
}

fn open_queue(dir: &tempfile::TempDir) -> Arc<SharedQueue> {
    let cfg: Arc<Config> = Arc::new(cfg_from(temp_config(dir)));
    SharedQueue::open(cfg).expect("open queue")
}

/// Build a `put` payload whose serialized size is exactly `target` bytes.
fn payload_of_size(target: usize) -> Vec<u8> {
    let base = Request::Put {
        request: CommentRequest {
            owner: String::new(),
            repo: String::new(),
            pr_number: 0,
            body: String::new(),
        },
    };
    let base_len = serde_json::to_vec(&base).expect("serialize").len();
    let body_len = target - base_len;
    let request = Request::Put {
        request: CommentRequest {
            owner: String::new(),
            repo: String::new(),
            pr_number: 0,
            body: "a".repeat(body_len),
        },
    };
    let payload = serde_json::to_vec(&request).expect("serialize");
    assert_eq!(payload.len(), target);
    payload
}

#[tokio::test(start_paused = true)]
async fn handle_client_times_out_if_client_does_not_close() {
    let dir = tempdir().expect("tempdir");
    let queue = open_queue(&dir);
    let (client, server) = UnixStream::pair().expect("pair");
    let handle = tokio::spawn(handle_client(server, queue));
    // Ensure the handler registers its timeout before advancing time.
    yield_now().await;
    advance(Duration::from_secs(CLIENT_READ_TIMEOUT_SECS + 1)).await;
    let err = handle.await.expect("join").expect_err("expected timeout");
    assert!(err.to_string().contains("client read timed out"));
    drop(client);
}

#[tokio::test]
async fn handle_client_accepts_exact_max_request_bytes() {
    let dir = tempdir().expect("tempdir");
    let queue = open_queue(&dir);
    let (mut client, server) = UnixStream::pair().expect("pair");
    let handle = tokio::spawn(handle_client(server, queue));

    let payload = payload_of_size(MAX_REQUEST_BYTES);
    client.write_all(&payload).await.expect("write");
    client.shutdown().await.expect("shutdown");

    let mut reply = Vec::new();
    client.read_to_end(&mut reply).await.expect("read reply");
    handle.await.expect("join").expect("handle");
    let response: Response = serde_json::from_slice(&reply).expect("parse reply");
    assert!(
        matches!(response, Response::Ok { .. }),
        "boundary-sized request should be accepted: {response:?}"
    );
}

#[tokio::test]
async fn handle_client_rejects_request_exceeding_limit() {
    let dir = tempdir().expect("tempdir");
    let queue = open_queue(&dir);
    let (mut client, server) = UnixStream::pair().expect("pair");
    let handle = tokio::spawn(handle_client(server, queue));

    let payload = payload_of_size(MAX_REQUEST_BYTES + 1);
    client.write_all(&payload).await.expect("write");
    client.shutdown().await.expect("shutdown");

    let err = handle
        .await
        .expect("join")
        .expect_err("expected size error");
    assert!(err.to_string().contains("client payload exceeds"));
}
