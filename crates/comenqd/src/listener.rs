//! Unix socket listener for comenqd.
//!
//! Accepts client connections, deserializes requests, and forwards them to the
//! persistent queue for processing by the worker.

use crate::config::Config;
use crate::metrics;
use anyhow::{Context, Result};
use comenq_lib::CommentRequest;
use std::fs as stdfs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Mutex, mpsc, watch};
use uuid::Uuid;

use crate::supervisor::backoff;

/// The current bounded channel sender shared by the listener and its handlers.
///
/// A handler locks this state only after reading and serializing a client
/// request, then clones the current sender before awaiting the channel send.
pub type ClientSender = Arc<Mutex<mpsc::Sender<Vec<u8>>>>;

/// Prepare a Unix domain socket for the listener.
///
/// Atomically replaces any file at `path` by binding to a temporary socket in
/// the same parent directory (so `rename(2)` is atomic on the same filesystem),
/// setting its permissions to `0o660`, and then renaming it into place. The
/// final permissions are enforced again after the rename.
///
/// # Examples
///
/// ```rust,no_run
/// use comenqd::daemon::listener::prepare_listener;
/// use tempfile::tempdir;
/// let dir = tempdir().expect("create tempdir");
/// let sock = dir.path().join("sock");
/// let listener = prepare_listener(&sock).expect("prepare socket");
/// ```
pub fn prepare_listener(path: &Path) -> Result<UnixListener> {
    let parent = path.parent().context("socket path missing parent")?;
    // Create the socket directory when absent so a user-hosted daemon works
    // without systemd's RuntimeDirectory= support. An empty parent means the
    // path is relative to the working directory, which already exists.
    if !parent.as_os_str().is_empty() {
        create_socket_parent(parent)?;
    }
    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("socket path missing file name"))?;
    let tmp = parent.join(format!(
        ".{}.{}",
        file_name.to_string_lossy(),
        Uuid::new_v4()
    ));
    let listener = UnixListener::bind(&tmp)
        .with_context(|| format!("binding to temp socket {}", tmp.display()))?;
    // Ensure correct permissions before the temp socket becomes visible at the final path.
    stdfs::set_permissions(&tmp, stdfs::Permissions::from_mode(0o660))
        .with_context(|| format!("setting permissions on {}", tmp.display()))?;

    stdfs::rename(&tmp, path)
        .inspect_err(|_| {
            if let Err(e) = stdfs::remove_file(&tmp) {
                tracing::error!(
                    "failed to remove orphaned socket file {}: {}",
                    tmp.display(),
                    e
                );
            }
        })
        .with_context(|| format!("renaming socket {} -> {}", tmp.display(), path.display()))?;
    // Belt-and-braces: enforce final permissions in case of ACL quirkiness.
    stdfs::set_permissions(path, stdfs::Permissions::from_mode(0o660))
        .with_context(|| format!("setting permissions on {}", path.display()))?;
    Ok(listener)
}

/// Create missing socket-parent components without changing existing modes.
fn create_socket_parent(parent: &Path) -> Result<()> {
    let mut component_path = PathBuf::new();
    for component in parent.components() {
        component_path.push(component.as_os_str());
        let created_by_this_process = match stdfs::create_dir(&component_path) {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("creating socket directory {}", component_path.display())
                });
            }
        };
        if created_by_this_process {
            stdfs::set_permissions(&component_path, stdfs::Permissions::from_mode(0o700))
                .with_context(|| {
                    format!(
                        "setting permissions on socket directory {}",
                        component_path.display()
                    )
                })?;
        }
    }
    Ok(())
}

/// Listen on the Unix socket and spawn a handler for each client.
///
/// The listener accepts connections on the path configured in [`Config`]. Each
/// connection is handled concurrently by [`handle_client`], forwarding valid
/// requests to the queue writer. The function exits when the `shutdown` watch
/// channel is triggered.
///
/// # Errors
/// Returns an error if the socket cannot be created or if accepting a
/// connection fails after retries. Exiting due to a shutdown signal is normal
/// and not treated as an error.
#[tracing::instrument(
    skip(config, client_tx, shutdown),
    fields(task = "listener", socket = %config.socket_path.display())
)]
pub async fn run_listener(
    config: Arc<Config>,
    client_tx: ClientSender,
    mut shutdown: watch::Receiver<()>,
) -> Result<()> {
    let socket_path = config.socket_path.clone();
    let listener = tokio::task::spawn_blocking(move || prepare_listener(&socket_path))
        .await
        .context("listener preparation task failed")??;
    let min_delay = Duration::from_millis(config.restart_min_delay_ms);
    let mut accept_backoff = backoff(min_delay);

    loop {
        tokio::select! {
            res = listener.accept() => match res {
                Ok((stream, _)) => {
                    accept_backoff = backoff(min_delay);
                    let cred = stream.peer_cred().ok();
                    let pid = cred.as_ref().map(|c| c.pid());
                    let uid = cred.as_ref().map(|c| c.uid());
                    let client_tx = Arc::clone(&client_tx);
                    tokio::spawn(async move {
                        if let Err(e) = handle_client_with_sender(stream, client_tx).await {
                            match (pid, uid) {
                                (Some(pid), Some(uid)) => {
                                    tracing::warn!(pid, uid, error = %e, "Client handling failed");
                                }
                                _ => tracing::warn!(error = %e, "Client handling failed"),
                            }
                        }
                    });
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to accept client connection");
                    let delay = accept_backoff
                        .next()
                        .unwrap_or(crate::supervisor::BACKOFF_FALLBACK_DELAY);
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {},
                        _ = shutdown.changed() => break,
                    }
                }
            },
            _ = shutdown.changed() => {
                break;
            }
        }
    }
    Ok(())
}

/// Read a single request from `stream` and forward it to the queue.
///
/// Expects the client to send a JSON encoded [`CommentRequest`] and then close
/// the connection. The request is re-encoded to bytes and sent over `tx` for the
/// queue writer to persist.
///
/// # Errors
/// Fails if reading from the socket or parsing JSON fails, or if the queue
/// writer has shut down.
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024; // 1 MiB
pub const CLIENT_READ_TIMEOUT_SECS: u64 = 5;

/// Handle a client request using an independent bounded channel sender.
///
/// This compatibility wrapper is useful for direct callers. The daemon uses
/// [`run_listener`], whose handlers share [`ClientSender`] so they acquire the
/// current sender only after the asynchronous client read completes.
pub async fn handle_client(stream: UnixStream, tx: mpsc::Sender<Vec<u8>>) -> Result<()> {
    handle_client_with_sender(stream, Arc::new(Mutex::new(tx))).await
}

#[tracing::instrument(skip(stream, tx), fields(task = "listener", outcome = tracing::field::Empty))]
async fn handle_client_with_sender(stream: UnixStream, tx: ClientSender) -> Result<()> {
    let result = handle_client_inner(stream, tx).await;
    record_request_outcome(result)
}

fn record_request_outcome(result: Result<()>) -> Result<()> {
    let outcome = if result.is_ok() {
        "accepted"
    } else {
        "rejected"
    };
    tracing::Span::current().record("outcome", outcome);
    metrics::record_request_outcome(outcome);
    result
}

async fn handle_client_inner(stream: UnixStream, tx: ClientSender) -> Result<()> {
    handle_client_inner_with_before_read(stream, tx, || {}).await
}

async fn handle_client_inner_with_before_read<F>(
    stream: UnixStream,
    tx: ClientSender,
    before_read: F,
) -> Result<()>
where
    F: FnOnce(),
{
    before_read();
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
        anyhow::bail!("client payload exceeds {} bytes", MAX_REQUEST_BYTES);
    }
    let request: CommentRequest = serde_json::from_slice(&buffer)?;
    let bytes = serde_json::to_vec(&request)?;
    let tx = {
        let current = tx.lock().await;
        current.clone()
    };
    tx.send(bytes)
        .await
        .map_err(|_| anyhow::anyhow!("queue writer dropped"))?;
    let depth = tx.max_capacity().saturating_sub(tx.capacity());
    metrics::record_client_channel_depth(depth);
    Ok(())
}

#[cfg(test)]
mod tests;
