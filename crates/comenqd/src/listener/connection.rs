//! Request decoding, bounded replies, and operation observability.

use super::protocol;
use crate::metrics;
use crate::queue::SharedQueue;
use anyhow::Result;
use comenq_lib::protocol::{MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES, Request, Response};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::{Semaphore, watch};

/// Seconds a client has to transmit its request.
pub const CLIENT_READ_TIMEOUT_SECS: u64 = 5;
/// Seconds a client has to receive the daemon response and close cleanly.
pub const CLIENT_WRITE_TIMEOUT_SECS: u64 = 5;

#[derive(Clone, Copy)]
enum ClientOutcome {
    Accepted,
    Failed,
    Await(&'static str),
}

/// Read a single request from `stream`, execute it, and reply.
///
/// Expects the client to send one JSON encoded [`Request`] and then close its
/// write side. The reply is a JSON encoded [`Response`] written back over the
/// same connection. `Await` streams NDJSON frames until posted or unrecoverable.
/// A malformed request receives an error reply rather than
/// silently closing the connection.
///
/// # Errors
/// Fails if reading from or writing to the socket fails, or if the payload
/// exceeds [`MAX_REQUEST_BYTES`].
pub async fn handle_client(stream: UnixStream, queue: Arc<SharedQueue>) -> Result<()> {
    let (_shutdown_tx, shutdown) = watch::channel(());
    handle_client_with_control(
        stream,
        queue,
        shutdown,
        Arc::new(Semaphore::new(super::await_handler::MAX_AWAITERS)),
    )
    .await
}

#[tracing::instrument(
    name = "handle_client",
    skip(stream, queue, shutdown, permits),
    fields(
        task = "listener",
        operation = tracing::field::Empty,
        elapsed_ms = tracing::field::Empty,
        outcome = tracing::field::Empty,
        error_kind = tracing::field::Empty,
    )
)]
pub(super) async fn handle_client_with_control(
    stream: UnixStream,
    queue: Arc<SharedQueue>,
    shutdown: watch::Receiver<()>,
    permits: Arc<Semaphore>,
) -> Result<()> {
    let started = Instant::now();
    let mut operation = None;
    let result = handle_client_inner(stream, queue, &mut operation, shutdown, permits).await;
    let (outcome, error_kind) = match &result {
        Ok((ClientOutcome::Await(outcome), error_kind)) => (*outcome, *error_kind),
        Ok((ClientOutcome::Accepted, error_kind)) => ("accepted", *error_kind),
        Ok((ClientOutcome::Failed, error_kind)) => ("failed", *error_kind),
        Err(_) => ("rejected", Some("protocol_io")),
    };
    let span = tracing::Span::current();
    span.record("operation", operation.unwrap_or("unknown"));
    let duration = started.elapsed();
    span.record(
        "elapsed_ms",
        u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
    );
    span.record("outcome", outcome);
    span.record("error_kind", error_kind.unwrap_or("none"));
    metrics::record_request(operation, outcome);
    metrics::record_protocol_duration(operation, outcome, error_kind, duration);
    result.map(|_| ())
}

async fn handle_client_inner(
    stream: UnixStream,
    queue: Arc<SharedQueue>,
    operation: &mut Option<&'static str>,
    shutdown: watch::Receiver<()>,
    permits: Arc<Semaphore>,
) -> Result<(ClientOutcome, Option<&'static str>)> {
    let mut buffer = Vec::with_capacity(8 * 1024);
    // Read up to LIMIT+1 to detect oversize payloads without relying on client EOF.
    let mut limited = stream.take((MAX_REQUEST_BYTES as u64) + 1);
    tokio::time::timeout(
        Duration::from_secs(CLIENT_READ_TIMEOUT_SECS),
        limited.read_to_end(&mut buffer),
    )
    .await
    .map_err(|_| anyhow::anyhow!("client read timed out"))??;
    if buffer.len() > MAX_REQUEST_BYTES {
        anyhow::bail!("client payload exceeds {MAX_REQUEST_BYTES} bytes");
    }
    let (response, mut outcome, mut error_kind) = match serde_json::from_slice::<Request>(&buffer) {
        Ok(request) => {
            *operation = Some(request_operation(&request));
            if let Request::Await { id } = request {
                let outcome = super::await_handler::handle_await(
                    limited.into_inner(),
                    queue,
                    id,
                    shutdown,
                    permits,
                )
                .await;
                return Ok((ClientOutcome::Await(outcome), None));
            }
            let (response, error_kind) =
                protocol::dispatch_request_with_error_kind(&queue, request).await;
            let outcome = if matches!(&response, Response::Error { .. }) {
                ClientOutcome::Failed
            } else {
                ClientOutcome::Accepted
            };
            (response, outcome, error_kind)
        }
        Err(e) => (
            Response::error(format!("invalid request: {e}")),
            ClientOutcome::Failed,
            Some("invalid_json"),
        ),
    };
    let mut bytes = serde_json::to_vec(&response)?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        outcome = ClientOutcome::Failed;
        error_kind = Some("response_too_large");
        bytes = serde_json::to_vec(&Response::error(format!(
            "response exceeds {MAX_RESPONSE_BYTES} bytes"
        )))?;
    }
    let mut stream = limited.into_inner();
    tokio::time::timeout(
        Duration::from_secs(CLIENT_WRITE_TIMEOUT_SECS),
        stream.write_all(&bytes),
    )
    .await
    .map_err(|_| anyhow::anyhow!("client response write timed out"))??;
    tokio::time::timeout(
        Duration::from_secs(CLIENT_WRITE_TIMEOUT_SECS),
        stream.shutdown(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("client connection shutdown timed out"))??;
    Ok((outcome, error_kind))
}

fn request_operation(request: &Request) -> &'static str {
    match request {
        Request::Put { .. } => "put",
        Request::List => "list",
        Request::Bump { .. } => "bump",
        Request::Bust { .. } => "bust",
        Request::Del { .. } => "del",
        Request::Hist { .. } => "hist",
        Request::Await { .. } => "await",
    }
}
