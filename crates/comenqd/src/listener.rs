//! Unix socket listener for comenqd.
//!
//! Accepts client connections, deserializes protocol requests, executes them
//! against the shared queue, and writes the JSON reply back to the client.

use anyhow::{Context, Result};
use comenq_lib::protocol::{Request, Response};
use std::fs as stdfs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::watch;
use uuid::Uuid;

use crate::queue::SharedQueue;
use crate::supervisor::backoff;

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
        stdfs::create_dir_all(parent)
            .with_context(|| format!("creating socket directory {}", parent.display()))?;
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

/// Listen on the Unix socket and spawn a handler for each client.
///
/// The listener accepts connections on the path configured in the shared
/// queue's configuration. Each connection is handled concurrently by
/// [`handle_client`], which executes the request and replies. The function
/// exits when the `shutdown` watch channel is triggered.
///
/// # Errors
/// Returns an error if the socket cannot be created or if accepting a
/// connection fails after retries. Exiting due to a shutdown signal is normal
/// and not treated as an error.
pub async fn run_listener(
    queue: Arc<SharedQueue>,
    mut shutdown: watch::Receiver<()>,
) -> Result<()> {
    let cfg = queue.config();
    let listener = prepare_listener(&cfg.socket_path)?;
    let min_delay = Duration::from_millis(cfg.restart_min_delay_ms);
    let mut accept_backoff = backoff(min_delay);

    loop {
        tokio::select! {
            res = listener.accept() => match res {
                Ok((stream, _)) => {
                    accept_backoff = backoff(min_delay);
                    let cred = stream.peer_cred().ok();
                    let pid = cred.as_ref().map(|c| c.pid());
                    let uid = cred.as_ref().map(|c| c.uid());
                    let queue = queue.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_client(stream, queue).await {
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

/// Maximum accepted request payload, in bytes.
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024; // 1 MiB
/// Seconds a client has to transmit its request.
pub const CLIENT_READ_TIMEOUT_SECS: u64 = 5;

/// Read a single request from `stream`, execute it, and reply.
///
/// Expects the client to send one JSON encoded [`Request`] and then close its
/// write side. The reply is a JSON encoded [`Response`] written back over the
/// same connection. A malformed request receives an error reply rather than
/// silently closing the connection.
///
/// # Errors
/// Fails if reading from or writing to the socket fails, or if the payload
/// exceeds [`MAX_REQUEST_BYTES`].
pub async fn handle_client(stream: UnixStream, queue: Arc<SharedQueue>) -> Result<()> {
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
    let response = match serde_json::from_slice::<Request>(&buffer) {
        Ok(request) => queue.execute(request).await,
        Err(e) => Response::error(format!("invalid request: {e}")),
    };
    let bytes = serde_json::to_vec(&response)?;
    let mut stream = limited.into_inner();
    stream.write_all(&bytes).await.context("write response")?;
    stream.shutdown().await.context("close connection")?;
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

    #[tokio::test]
    async fn prepare_listener_creates_missing_parent_directory() {
        let dir = tempdir().expect("create tempdir");
        let sock = dir.path().join("missing/nested/comenq.sock");
        let listener = prepare_listener(&sock).expect("prepare listener");
        let meta = std::fs::symlink_metadata(&sock).expect("metadata");
        assert!(meta.file_type().is_socket());
        drop(listener);
    }

    #[tokio::test]
    async fn prepare_listener_prevents_pre_bind_race() {
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
