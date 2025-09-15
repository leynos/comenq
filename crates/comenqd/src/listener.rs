//! Unix socket listener for comenqd.
//!
//! Accepts client connections, deserialises requests, and forwards them to the
//! persistent queue for processing by the worker.

use crate::config::Config;
use anyhow::{Context, Result};
use comenq_lib::CommentRequest;
use std::fs as stdfs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, watch};
use uuid::Uuid;

use crate::supervisor::backoff;

/// Prepare a Unix domain socket for the listener.
///
/// Atomically replaces any file at `path` by binding to a temporary socket and
/// renaming it into place with permissions `0o660`.
///
/// # Examples
///
/// ```rust,no_run
/// use comenqd::listener::prepare_listener;
/// use tempfile::tempdir;
/// let dir = tempdir().expect("create tempdir");
/// let sock = dir.path().join("sock");
/// let listener = prepare_listener(&sock).expect("prepare socket");
/// ```
pub fn prepare_listener(path: &Path) -> Result<UnixListener> {
    let parent = path.parent().context("socket path missing parent")?;
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

    stdfs::rename(&tmp, path).inspect_err(|_| {
        if let Err(e) = stdfs::remove_file(&tmp) {
            tracing::error!(
                "failed to remove orphaned socket file {}: {}",
                tmp.display(),
                e
            );
        }
    })?;
    stdfs::set_permissions(path, stdfs::Permissions::from_mode(0o660))?;
    Ok(listener)
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
pub async fn run_listener(
    config: Arc<Config>,
    tx: mpsc::Sender<Vec<u8>>,
    mut shutdown: watch::Receiver<()>,
) -> Result<()> {
    let listener = prepare_listener(&config.socket_path)?;
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
                    let tx_clone = tx.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_client(stream, tx_clone).await {
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
                        .expect("backoff should yield a duration");
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

pub async fn handle_client(stream: UnixStream, tx: mpsc::Sender<Vec<u8>>) -> Result<()> {
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
    tx.send(bytes)
        .await
        .map_err(|_| anyhow::anyhow!("queue writer dropped"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::OpenOptions;
    use std::os::unix::fs::FileTypeExt;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use tempfile::tempdir;

    #[test]
    fn prepare_listener_prevents_pre_bind_race() {
        let dir = tempdir().expect("create tempdir");
        let sock = dir.path().join("sock");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop);
        let sock_clone = sock.clone();
        let attacker = thread::spawn(move || {
            while !stop_clone.load(Ordering::SeqCst) {
                let _ = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&sock_clone);
                std::thread::yield_now();
            }
        });
        let listener = prepare_listener(&sock).expect("prepare listener");
        stop.store(true, Ordering::SeqCst);
        attacker.join().expect("attacker thread");
        // Avoid following symlinks when asserting the final on-disk type.
        let meta = std::fs::symlink_metadata(&sock).expect("metadata");
        assert!(meta.file_type().is_socket());
        drop(listener);
    }
}
