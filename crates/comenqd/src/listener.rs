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
use tokio::sync::{mpsc, watch};
use uuid::Uuid;

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
pub async fn run_listener(
    config: Arc<Config>,
    client_tx: mpsc::Sender<Vec<u8>>,
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
                    let client_tx = client_tx.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_client(stream, client_tx).await {
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

pub async fn handle_client(stream: UnixStream, tx: mpsc::Sender<Vec<u8>>) -> Result<()> {
    let result = handle_client_inner(stream, tx).await;
    metrics::record_request_outcome(if result.is_ok() {
        "accepted"
    } else {
        "rejected"
    });
    result
}

async fn handle_client_inner(stream: UnixStream, tx: mpsc::Sender<Vec<u8>>) -> Result<()> {
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
    let depth = tx.max_capacity().saturating_sub(tx.capacity());
    metrics::record_client_channel_depth(depth);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::metrics::set_default_local_recorder;
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};
    use std::fs::OpenOptions;
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use tempfile::tempdir;

    fn request_outcomes(
        metrics: &[(
            metrics_util::CompositeKey,
            Option<::metrics::Unit>,
            Option<::metrics::SharedString>,
            DebugValue,
        )],
    ) -> Vec<&str> {
        metrics
            .iter()
            .filter(|(key, _, _, _)| key.key().name() == "comenqd_requests_total")
            .flat_map(|(key, _, _, _)| key.key().labels())
            .filter_map(|label| (label.key() == "outcome").then_some(label.value()))
            .collect()
    }

    #[tokio::test]
    async fn prepare_listener_creates_missing_parent_directory() {
        let dir = tempdir().expect("create tempdir");
        let sock = dir.path().join("missing/nested/comenq.sock");
        let listener = prepare_listener(&sock).expect("prepare listener");
        let meta = std::fs::symlink_metadata(&sock).expect("metadata");
        assert!(meta.file_type().is_socket());
        assert_eq!(meta.permissions().mode() & 0o777, 0o660);
        let parent = sock.parent().expect("socket parent");
        let parent_meta = std::fs::symlink_metadata(parent).expect("parent metadata");
        assert_eq!(parent_meta.permissions().mode() & 0o777, 0o700);
        let nested_parent = parent.parent().expect("nested socket parent");
        let nested_parent_meta =
            std::fs::symlink_metadata(nested_parent).expect("nested parent metadata");
        assert_eq!(nested_parent_meta.permissions().mode() & 0o777, 0o700);
        drop(listener);
    }

    #[tokio::test]
    async fn prepare_listener_preserves_existing_parent_permissions() {
        let dir = tempdir().expect("create tempdir");
        let parent = dir.path().join("existing");
        std::fs::create_dir(&parent).expect("create parent");
        std::fs::set_permissions(&parent, stdfs::Permissions::from_mode(0o755))
            .expect("set parent permissions");

        let listener = prepare_listener(&parent.join("comenq.sock")).expect("prepare listener");
        let metadata = std::fs::symlink_metadata(&parent).expect("parent metadata");
        assert_eq!(metadata.permissions().mode() & 0o777, 0o755);
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

    #[tokio::test(flavor = "current_thread")]
    async fn handle_client_records_accepted_and_rejected_request_metrics() {
        use tokio::io::AsyncWriteExt;
        use tokio::sync::mpsc;

        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let _recorder_guard = set_default_local_recorder(&recorder);
        let (tx, mut rx) = mpsc::channel(2);

        let (mut valid_client, valid_server) = UnixStream::pair().expect("create valid pair");
        let request = CommentRequest {
            owner: "owner".into(),
            repo: "repo".into(),
            pr_number: 1,
            body: "body".into(),
        };
        valid_client
            .write_all(&serde_json::to_vec(&request).expect("serialize request"))
            .await
            .expect("write valid request");
        valid_client.shutdown().await.expect("close valid request");
        handle_client(valid_server, tx.clone())
            .await
            .expect("accept valid request");
        let _ = rx.recv().await.expect("receive valid request");

        let (mut invalid_client, invalid_server) = UnixStream::pair().expect("create invalid pair");
        invalid_client
            .write_all(b"not json")
            .await
            .expect("write invalid request");
        invalid_client
            .shutdown()
            .await
            .expect("close invalid request");
        assert!(handle_client(invalid_server, tx).await.is_err());

        let metrics = snapshotter.snapshot().into_vec();
        let outcomes = request_outcomes(&metrics);
        assert!(outcomes.contains(&"accepted"));
        assert!(outcomes.contains(&"rejected"));
    }
}
