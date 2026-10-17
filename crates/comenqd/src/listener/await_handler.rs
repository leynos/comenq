//! Read-only streaming waits, independent of worker wake notifications.

use super::CLIENT_WRITE_TIMEOUT_SECS;
use crate::queue::{AwaitOutcome, AwaitToken, SharedQueue};
use anyhow::Result;
use comenq_lib::protocol::{AWAIT_HEARTBEAT_SECS, AwaitEvent, MAX_AWAIT_FRAME_BYTES};
use std::ops::ControlFlow;
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
    await_entry(
        &mut stream,
        &queue,
        &id,
        shutdown,
        Duration::from_secs(AWAIT_HEARTBEAT_SECS),
    )
    .await
}

/// Stream ETA updates until the queue resolves, shutdown begins, or the client leaves.
async fn await_entry(
    stream: &mut UnixStream,
    queue: &SharedQueue,
    id: &str,
    mut shutdown: watch::Receiver<()>,
    heartbeat: Duration,
) -> &'static str {
    // Register before lookup: a mutation during the read must remain unseen.
    let mut state = queue.subscribe_state();
    state.mark_seen();
    let mut token = None;
    loop {
        if shutdown.has_changed().unwrap_or(true) {
            return send_terminal(stream, "daemon is shutting down", "shutdown").await;
        }
        if let ControlFlow::Break(outcome) = respond(stream, queue, id, &mut token).await {
            return outcome;
        }
        // The client has already half-closed its request side: read readiness
        // means EOF even for a live waiter. Detect departure on bounded writes
        // instead, without mistaking that protocol EOF for a cancelled wait.
        tokio::select! {
            biased;
            _ = shutdown.changed() => {
                return send_terminal(stream, "daemon is shutting down", "shutdown").await;
            }
            _ = state.changed() => {}
            _ = tokio::time::sleep(heartbeat) => {}
        }
    }
}

/// Resolve one waiter state and send its progress or terminal frame.
async fn respond(
    stream: &mut UnixStream,
    queue: &SharedQueue,
    id: &str,
    token: &mut Option<AwaitToken>,
) -> ControlFlow<&'static str> {
    match queue.resolve_await(id, token.clone()).await {
        Ok(AwaitOutcome::Pending {
            eta_seconds,
            token: selected,
        }) => {
            *token = Some(selected);
            if send_frame(stream, &AwaitEvent::Waiting { eta_seconds })
                .await
                .is_err()
            {
                ControlFlow::Break("client_gone")
            } else {
                ControlFlow::Continue(())
            }
        }
        Ok(AwaitOutcome::Posted {
            comment_url: Some(comment_url),
            ..
        }) => ControlFlow::Break(send_posted(stream, comment_url).await),
        Ok(AwaitOutcome::Posted {
            comment_url: None, ..
        }) => ControlFlow::Break(
            send_terminal(
                stream,
                "posted comment has no recorded URL",
                "posted_without_url",
            )
            .await,
        ),
        Ok(AwaitOutcome::Unknown) => {
            ControlFlow::Break(missing_entry(stream, token.as_ref()).await)
        }
        Err(error) => {
            let category = error.category();
            tracing::warn!(error_kind = category, "await lookup failed");
            ControlFlow::Break(send_terminal(stream, &error.message(), category).await)
        }
    }
}

/// Send a terminal URL frame, or an explicit error when its representation cannot fit.
async fn send_posted(stream: &mut UnixStream, comment_url: String) -> &'static str {
    let event = AwaitEvent::Posted { comment_url };
    let fits = serde_json::to_vec(&event).is_ok_and(|bytes| bytes.len() < MAX_AWAIT_FRAME_BYTES);
    if !fits {
        return send_terminal(
            stream,
            "posted comment URL exceeds the await frame limit",
            "posted_url_too_large",
        )
        .await;
    }
    if send_frame(stream, &event).await.is_ok() {
        "posted"
    } else {
        "client_gone"
    }
}

/// Report whether an identifier was initially unknown or removed during a wait.
async fn missing_entry(stream: &mut UnixStream, token: Option<&AwaitToken>) -> &'static str {
    let (message, outcome) = if token.is_some() {
        ("comment was removed", "removed")
    } else {
        ("unknown comment identifier", "unknown_identifier")
    };
    send_terminal(stream, message, outcome).await
}

/// Send one terminal error frame and return its bounded outcome label.
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

/// Serialize and write one newline-delimited frame within the listener deadline.
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
