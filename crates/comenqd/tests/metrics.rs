//! End-to-end verification of the daemon Prometheus scrape endpoint.

use comenqd::config::Config;
use comenqd::daemon::listener::handle_client;
use comenqd::daemon::{SharedQueue, listener::dispatch_request};
use comenqd::metrics::{PROMETHEUS_LISTEN_ADDR, install_prometheus};
use std::net::Ipv4Addr;
use std::path::Path;
use std::sync::Arc;
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UnixStream};

/// Recompute the queue gauge from persisted bytes and unused mutation reserve.
fn persisted_accounting(queue_path: &Path) -> (usize, u64) {
    const ENTRY_MUTATION_HEADROOM_BYTES: u64 = 80;

    std::fs::read_dir(queue_path.join("entries"))
        .expect("read queue entries")
        .map(|dirent| {
            let path = dirent.expect("read entry directory item").path();
            let bytes = std::fs::read(&path).expect("read persisted entry");
            let entry: serde_json::Value =
                serde_json::from_slice(&bytes).expect("decode persisted entry");
            let order = entry["order"].as_i64().expect("stored order");
            let order_growth = order.to_string().len().saturating_sub(1);
            let claim_growth = entry["claim_token"]
                .as_str()
                .map(|token| {
                    serde_json::to_vec(token)
                        .expect("encode claim token")
                        .len()
                        .saturating_sub(4)
                })
                .unwrap_or(0);
            let remaining_headroom = ENTRY_MUTATION_HEADROOM_BYTES
                .saturating_sub(u64::try_from(order_growth).expect("order growth fits u64"))
                .saturating_sub(u64::try_from(claim_growth).expect("claim growth fits u64"));
            let persisted_bytes = u64::try_from(bytes.len()).expect("entry size fits u64");
            persisted_bytes + remaining_headroom
        })
        .fold((0, 0), |(count, bytes), entry_bytes| {
            (count + 1, bytes + entry_bytes)
        })
}

/// Assert both queue gauges against the queue's persisted representation.
async fn assert_queue_gauges(queue_path: &Path, expected_count: usize) {
    let (count, bytes) = persisted_accounting(queue_path);
    assert_eq!(count, expected_count);
    let response = scrape_metrics().await;
    assert!(
        response
            .lines()
            .any(|line| { line == format!("comenqd_queue_entries {count}") })
    );
    assert!(
        response
            .lines()
            .any(|line| { line == format!("comenqd_queue_bytes {bytes}") })
    );
}

/// Fetch the local metrics endpoint for integration assertions.
async fn scrape_metrics() -> String {
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
    response
}

/// Find a Prometheus histogram count with the expected fixed labels.
fn has_histogram_count(response: &str, name: &str, labels: &[(&str, &str)], count: u64) -> bool {
    response.lines().any(|line| {
        line.starts_with(&format!("{name}_count{{"))
            && line.ends_with(&format!("}} {count}"))
            && labels
                .iter()
                .all(|(key, value)| line.contains(&format!("{key}=\"{value}\"")))
    })
}

/// Exercise queue metrics across durable state transitions and malformed input.
#[tokio::test(flavor = "current_thread")]
async fn exporter_serves_listener_request_metrics() {
    install_prometheus().expect("install local Prometheus exporter");
    let dir = tempdir().expect("create temporary queue directory");
    let config = Arc::new(Config {
        github_token: "token".into(),
        github_token_file: None,
        socket_path: dir.path().join("comenq.sock"),
        queue_path: dir.path().join("queue"),
        cooldown_period_seconds: 0,
        cooldown_flutter_seconds: 0,
        restart_min_delay_ms: 0,
        github_api_timeout_secs: 1,
    });
    let queue = SharedQueue::open(Arc::clone(&config)).expect("open queue");
    assert_queue_gauges(&dir.path().join("queue"), 0).await;

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
    handle_client(server, Arc::clone(&queue))
        .await
        .expect("accept client request");
    let mut reply = Vec::new();
    client.read_to_end(&mut reply).await.expect("read reply");
    let put_response = serde_json::from_slice::<comenq_lib::protocol::Response>(&reply)
        .expect("decode put response");
    let comenq_lib::protocol::Response::Ok {
        entry: Some(_entry),
        ..
    } = put_response
    else {
        panic!("expected put entry, got {put_response:?}");
    };
    assert_queue_gauges(&dir.path().join("queue"), 1).await;

    let second_response = dispatch_request(
        &queue,
        comenq_lib::protocol::Request::Put {
            request: comenq_lib::CommentRequest {
                owner: "owner".into(),
                repo: "repo".into(),
                pr_number: 2,
                body: "second body".into(),
            },
            immediate: true,
        },
    )
    .await;
    let comenq_lib::protocol::Response::Ok {
        entry: Some(second_entry),
        ..
    } = second_response
    else {
        panic!("expected second put entry, got {second_response:?}");
    };
    assert_queue_gauges(&dir.path().join("queue"), 2).await;

    assert!(matches!(
        dispatch_request(
            &queue,
            comenq_lib::protocol::Request::Bump {
                id: second_entry.id.clone(),
            },
        )
        .await,
        comenq_lib::protocol::Response::Ok { .. }
    ));
    assert_queue_gauges(&dir.path().join("queue"), 2).await;
    assert!(matches!(
        dispatch_request(
            &queue,
            comenq_lib::protocol::Request::Bust {
                id: second_entry.id.clone(),
            },
        )
        .await,
        comenq_lib::protocol::Response::Ok { .. }
    ));
    assert_queue_gauges(&dir.path().join("queue"), 2).await;

    let (claimed, _) = queue
        .claim_next_due()
        .await
        .expect("claim due entry")
        .expect("queue has a due entry");
    let claim_token = claimed.claim_token.clone().expect("claim token persisted");
    assert_queue_gauges(&dir.path().join("queue"), 2).await;
    queue
        .release_claim(&claimed.id, &claim_token)
        .await
        .expect("release failed post claim");
    assert_queue_gauges(&dir.path().join("queue"), 2).await;

    let (_claimed, _) = queue
        .claim_next_due()
        .await
        .expect("claim entry before recovery")
        .expect("queue has a due entry");
    assert_queue_gauges(&dir.path().join("queue"), 2).await;
    drop(queue);
    let queue = SharedQueue::open(Arc::clone(&config)).expect("reopen queue and recover claim");
    assert_queue_gauges(&dir.path().join("queue"), 2).await;

    let (claimed, _) = queue
        .claim_next_due()
        .await
        .expect("claim entry for completion")
        .expect("queue has a due entry");
    queue
        .complete(&claimed.id, claimed.claim_token.clone())
        .await
        .expect("complete posted entry");
    assert_queue_gauges(&dir.path().join("queue"), 1).await;
    assert!(matches!(
        dispatch_request(
            &queue,
            comenq_lib::protocol::Request::Del {
                id: second_entry.id,
            },
        )
        .await,
        comenq_lib::protocol::Response::Ok { .. }
    ));
    assert_queue_gauges(&dir.path().join("queue"), 0).await;

    assert!(matches!(
        dispatch_request(
            &queue,
            comenq_lib::protocol::Request::Del {
                id: "deadbeef".into(),
            },
        )
        .await,
        comenq_lib::protocol::Response::Error { .. }
    ));

    let (mut malformed_client, malformed_server) =
        UnixStream::pair().expect("create malformed-request stream pair");
    malformed_client
        .write_all(b"not json")
        .await
        .expect("write malformed request");
    malformed_client
        .shutdown()
        .await
        .expect("close malformed request");
    handle_client(malformed_server, Arc::clone(&queue))
        .await
        .expect("reply to malformed request");
    let mut malformed_reply = Vec::new();
    malformed_client
        .read_to_end(&mut malformed_reply)
        .await
        .expect("read malformed-request response");
    assert!(matches!(
        serde_json::from_slice::<comenq_lib::protocol::Response>(&malformed_reply),
        Ok(comenq_lib::protocol::Response::Error { .. })
    ));

    let (mut io_client, io_server) = UnixStream::pair().expect("create I/O-error stream pair");
    io_client
        .write_all(&serde_json::to_vec(&comenq_lib::protocol::Request::List).expect("serialize"))
        .await
        .expect("write request before closing client");
    io_client
        .shutdown()
        .await
        .expect("close request before closing client");
    drop(io_client);
    assert!(handle_client(io_server, Arc::clone(&queue)).await.is_err());

    let response = scrape_metrics().await;

    assert!(response.starts_with("HTTP/1.1 200"));
    assert!(response.contains("comenqd_requests_total{operation=\"put\",outcome=\"accepted\"}"));
    assert!(response.contains("comenqd_requests_total{outcome=\"failed\"} 1"));
    assert!(!response.contains("comenqd_requests_total{outcome=\"accepted\"}"));
    assert!(has_histogram_count(
        &response,
        "comenqd_protocol_transaction_duration_seconds",
        &[
            ("operation", "put"),
            ("outcome", "accepted"),
            ("error_kind", "none"),
        ],
        1,
    ));
    assert!(has_histogram_count(
        &response,
        "comenqd_protocol_transaction_duration_seconds",
        &[
            ("operation", "unknown"),
            ("outcome", "failed"),
            ("error_kind", "invalid_json"),
        ],
        1,
    ));
    assert!(has_histogram_count(
        &response,
        "comenqd_protocol_transaction_duration_seconds",
        &[
            ("operation", "list"),
            ("outcome", "rejected"),
            ("error_kind", "protocol_io"),
        ],
        1,
    ));
    assert!(has_histogram_count(
        &response,
        "comenqd_queue_store_operation_duration_seconds",
        &[
            ("operation", "put"),
            ("outcome", "success"),
            ("error_kind", "none"),
        ],
        2,
    ));
    assert!(has_histogram_count(
        &response,
        "comenqd_queue_store_operation_duration_seconds",
        &[
            ("operation", "del"),
            ("outcome", "failure"),
            ("error_kind", "unknown_identifier"),
        ],
        1,
    ));
}
