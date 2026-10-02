//! Connection candidate selection tests.

use super::super::{ClientError, connect_first};
use tempfile::tempdir;
use tokio::net::UnixListener;

/// A stale socket file must not shadow a live daemon later in the list.
#[tokio::test]
async fn connect_first_skips_stale_sockets() {
    let dir = tempdir().expect("temp dir");
    let stale = dir.path().join("stale.sock");
    drop(UnixListener::bind(&stale).expect("bind stale socket"));
    assert!(stale.exists(), "stale socket file should remain on disk");

    let live = dir.path().join("live.sock");
    let listener = UnixListener::bind(&live).expect("bind live socket");

    let stream = connect_first(&[stale, live])
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

    let err = connect_first(&[stale, missing])
        .await
        .expect_err("all candidates should fail");
    let ClientError::Connect(source) = err else {
        panic!("expected connection error, got {err:?}");
    };
    assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
}
