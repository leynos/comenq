//! Filesystem utilities for tests.
//!
//! These helpers poll the filesystem while asynchronous tasks create
//! files or directories.

use std::path::Path;
use std::time::{Duration, Instant};
use tokio::time::sleep;

/// Wait until `path` appears within the provided `timeout`.
///
/// Returns `true` if the path was detected before the timeout expired.
///
/// # Examples
/// ```no_run
/// # use std::time::Duration;
/// # use std::path::Path;
/// # async fn example() {
/// let ready = wait_for_path(Path::new("/tmp/file"), Duration::from_millis(100)).await;
/// assert!(ready);
/// # }
/// ```
pub async fn wait_for_path(path: &Path, timeout: Duration) -> bool {
    const POLL_INTERVAL: Duration = Duration::from_millis(20);

    let start = Instant::now();
    while !path.exists() && start.elapsed() < timeout {
        sleep(POLL_INTERVAL).await;
    }
    path.exists()
}
