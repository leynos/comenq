//! Unix socket listener for comenqd.
//!
//! Accepts client connections, deserializes protocol requests, executes them
//! against the shared queue, and writes the JSON reply back to the client.

use anyhow::{Context, Result};
pub use comenq_lib::protocol::MAX_REQUEST_BYTES;
use std::fs as stdfs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UnixListener;
use tokio::sync::{Semaphore, watch};
use uuid::Uuid;

use crate::queue::SharedQueue;
use crate::supervisor::backoff;

mod await_handler;
mod connection;
pub use connection::{CLIENT_READ_TIMEOUT_SECS, CLIENT_WRITE_TIMEOUT_SECS, handle_client};
pub(crate) mod protocol;
#[cfg(test)]
pub(crate) use protocol::dispatch_request;

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
    let mut component_path = std::path::PathBuf::new();
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
    let socket_path = cfg.socket_path.clone();
    let listener = tokio::task::spawn_blocking(move || prepare_listener(&socket_path))
        .await
        .context("joining socket preparation task")??;
    let min_delay = Duration::from_millis(cfg.restart_min_delay_ms);
    let mut accept_backoff = backoff(min_delay);

    let await_permits = Arc::new(Semaphore::new(await_handler::MAX_AWAITERS));
    loop {
        tokio::select! {
            res = listener.accept() => match res {
                Ok((stream, _)) => {
                    accept_backoff = backoff(min_delay);
                    let cred = stream.peer_cred().ok();
                    let pid = cred.as_ref().map(|c| c.pid());
                    let uid = cred.as_ref().map(|c| c.uid());
                    let queue = queue.clone();
                    let shutdown = shutdown.clone();
                    let permits = Arc::clone(&await_permits);
                    tokio::spawn(async move {
                        if let Err(e) = connection::handle_client_with_control(stream, queue, shutdown, permits).await {
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

#[cfg(test)]
mod tests {
    //! Socket preparation tests for the listener adapter.

    use super::*;
    use rstest::rstest;
    use std::fs::OpenOptions;
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use tempfile::tempdir;

    #[rstest]
    #[case::missing("missing/nested", None, 0o700)]
    #[case::existing("existing", Some(0o755), 0o755)]
    #[tokio::test]
    async fn prepare_listener_preserves_or_sets_parent_directory_mode(
        #[case] parent_path: &str,
        #[case] existing_mode: Option<u32>,
        #[case] expected_mode: u32,
    ) {
        let dir = tempdir().expect("create tempdir");
        let parent = dir.path().join(parent_path);
        if let Some(mode) = existing_mode {
            std::fs::create_dir_all(&parent).expect("create parent directory");
            std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(mode))
                .expect("set parent directory mode");
        }
        let sock = parent.join("comenq.sock");
        let listener = prepare_listener(&sock).expect("prepare listener");
        let meta = std::fs::symlink_metadata(&sock).expect("metadata");
        assert!(meta.file_type().is_socket());
        assert_eq!(meta.permissions().mode() & 0o777, 0o660);
        let parent_meta = std::fs::symlink_metadata(&parent).expect("parent metadata");
        assert_eq!(parent_meta.permissions().mode() & 0o777, expected_mode);
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

#[cfg(test)]
mod observability;
