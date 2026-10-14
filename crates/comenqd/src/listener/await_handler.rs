//! Read-only streaming waits, independent of worker wake notifications.

use super::CLIENT_WRITE_TIMEOUT_SECS;
use crate::queue::SharedQueue;
use crate::store::{AwaitCursor, AwaitState};
use anyhow::Result;
use comenq_lib::protocol::{AWAIT_HEARTBEAT_SECS, AwaitEvent, MAX_AWAIT_FRAME_BYTES};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio::sync::{Semaphore, watch};

pub(super) const MAX_AWAITERS: usize = 64;

/// Acquire a listener-wide permit and report exactly one bounded terminal outcome.
pub(super) async fn handle_await(
    mut stream: UnixStream,
    queue: Arc<SharedQueue>,
    id: String,
    shutdown: watch::Receiver<()>,
    permits: Arc<Semaphore>,
) -> &'static str {
    let Ok(_permit) = permits.try_acquire_owned() else {
        return send_terminal(
            &mut stream,
            "too many concurrent await requests",
            "rejected_capacity",
        )
        .await;
    };
    await_entry(&mut stream, &queue, &id, shutdown).await
}

async fn await_entry(
    stream: &mut UnixStream,
    queue: &SharedQueue,
    id: &str,
    mut shutdown: watch::Receiver<()>,
) -> &'static str {
    // Register before lookup: a mutation during the read must remain unseen.
    let mut state = queue.subscribe_state();
    state.borrow_and_update();
    let mut cursor = None;
    loop {
        if shutdown.has_changed().unwrap_or(true) {
            return send_terminal(stream, "daemon is shutting down", "shutdown").await;
        }
        match queue.resolve_await(id, cursor.clone()).await {
            Ok(AwaitState::Pending {
                eta_seconds,
                cursor: selected,
            }) => {
                cursor = Some(selected);
                if send_frame(stream, &AwaitEvent::Waiting { eta_seconds })
                    .await
                    .is_err()
                {
                    return "client_gone";
                }
            }
            Ok(AwaitState::Posted {
                comment_url: Some(comment_url),
                ..
            }) => {
                let event = AwaitEvent::Posted { comment_url };
                if serde_json::to_vec(&event)
                    .map_or(true, |bytes| bytes.len() >= MAX_AWAIT_FRAME_BYTES)
                {
                    return send_terminal(
                        stream,
                        "posted comment URL exceeds the await frame limit",
                        "removed",
                    )
                    .await;
                }
                return if send_frame(stream, &event).await.is_ok() {
                    "posted"
                } else {
                    "client_gone"
                };
            }
            Ok(AwaitState::Posted {
                comment_url: None, ..
            }) => {
                return send_terminal(stream, "posted comment has no recorded URL", "removed")
                    .await;
            }
            Ok(AwaitState::Unknown) => return missing_entry(stream, id, cursor.as_ref()).await,
            Err(error) => {
                return send_terminal(stream, &error.to_string(), "unknown_identifier").await;
            }
        }
        tokio::select! {
            biased;
            _ = shutdown.changed() => {
                return send_terminal(stream, "daemon is shutting down", "shutdown").await;
            }
            _ = state.changed() => {}
            _ = tokio::time::sleep(Duration::from_secs(AWAIT_HEARTBEAT_SECS)) => {}
        }
    }
}

async fn missing_entry(
    stream: &mut UnixStream,
    id: &str,
    cursor: Option<&AwaitCursor>,
) -> &'static str {
    let (message, outcome) = if cursor.is_some() {
        (format!("comment '{id}' was removed"), "removed")
    } else {
        (
            format!("unknown comment identifier '{id}'"),
            "unknown_identifier",
        )
    };
    send_terminal(stream, &message, outcome).await
}

async fn send_terminal(
    stream: &mut UnixStream,
    message: &str,
    outcome: &'static str,
) -> &'static str {
    if send_frame(
        stream,
        &AwaitEvent::Error {
            message: message.to_owned(),
        },
    )
    .await
    .is_ok()
    {
        outcome
    } else {
        "client_gone"
    }
}

async fn send_frame(stream: &mut UnixStream, event: &AwaitEvent) -> Result<()> {
    let mut bytes = serde_json::to_vec(event)?;
    anyhow::ensure!(
        bytes.len() < MAX_AWAIT_FRAME_BYTES,
        "await frame exceeds limit"
    );
    bytes.push(b'\n');
    tokio::time::timeout(
        Duration::from_secs(CLIENT_WRITE_TIMEOUT_SECS),
        stream.write_all(&bytes),
    )
    .await
    .map_err(|_| anyhow::anyhow!("await frame write timed out"))??;
    Ok(())
}

#[cfg(test)]
mod tests;
