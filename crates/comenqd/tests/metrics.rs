//! End-to-end verification of the daemon Prometheus scrape endpoint.

use comenqd::config::Config;
use comenqd::daemon::SharedQueue;
use comenqd::daemon::listener::handle_client;
use comenqd::metrics::{PROMETHEUS_LISTEN_ADDR, install_prometheus};
use std::net::Ipv4Addr;
use std::sync::Arc;
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UnixStream};

#[tokio::test(flavor = "current_thread")]
async fn exporter_serves_listener_request_metrics() {
    install_prometheus().expect("install local Prometheus exporter");
    let dir = tempdir().expect("create temporary queue directory");
    let queue = SharedQueue::open(Arc::new(Config {
        github_token: "token".into(),
        github_token_file: None,
        socket_path: dir.path().join("comenq.sock"),
        queue_path: dir.path().join("queue"),
        cooldown_period_seconds: 0,
        cooldown_flutter_seconds: 0,
        restart_min_delay_ms: 0,
        github_api_timeout_secs: 1,
    }))
    .expect("open queue");
    let (mut client, server) = UnixStream::pair().expect("create Unix stream pair");
    let request = comenq_lib::protocol::Request::Put {
        request: comenq_lib::CommentRequest {
            owner: "owner".into(),
            repo: "repo".into(),
            pr_number: 1,
            body: "body".into(),
        },
        immediate: true,
    };
    client
        .write_all(&serde_json::to_vec(&request).expect("serialize request"))
        .await
        .expect("write request");
    client.shutdown().await.expect("close request");
    handle_client(server, queue)
        .await
        .expect("accept client request");
    let mut reply = Vec::new();
    client.read_to_end(&mut reply).await.expect("read reply");
    assert!(serde_json::from_slice::<comenq_lib::protocol::Response>(&reply).is_ok());

    let mut stream = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        TcpStream::connect((
            Ipv4Addr::from(PROMETHEUS_LISTEN_ADDR.0),
            PROMETHEUS_LISTEN_ADDR.1,
        )),
    )
    .await
    .expect("metrics endpoint should accept connections")
    .expect("connect to metrics endpoint");
    stream
        .write_all(b"GET /metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .expect("request metrics");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .await
        .expect("read metrics response");

    assert!(response.starts_with("HTTP/1.1 200"));
    assert!(response.contains("comenqd_requests_total{outcome=\"accepted\"}"));
}
