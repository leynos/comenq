//! Tests for listener behaviour.

use std::time::Duration;
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tokio::time::advance;

use comenqd::daemon::listener::handle_client;

#[tokio::test(start_paused = true)]
async fn handle_client_times_out_if_client_does_not_close() {
    let (client_tx, _rx) = mpsc::channel(1);
    let (client, server) = UnixStream::pair().expect("pair");
    let handle = tokio::spawn(handle_client(server, client_tx));

    advance(Duration::from_secs(6)).await;

    let res = handle.await.expect("join");
    assert!(res.is_err());
    drop(client);
}
