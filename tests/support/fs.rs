//! Filesystem utilities for tests.
//!
//! These helpers poll the filesystem while asynchronous tasks create
//! files or directories.

use std::path::Path;
use std::time::{Duration, Instant};
use tokio::time::sleep;

/// Wait until `path` exists within `timeout_ms` milliseconds.
///
/// Returns `true` if the path was found.
///
/// # Examples
/// ```no_run
/// # use std::path::Path;
/// # async fn example() {
/// let ready = wait_for_path(Path::new("/tmp/file"), 100).await;
/// assert!(ready);
/// # }
/// ```
pub async fn wait_for_path(path: &Path, timeout_ms: u64) -> bool {
    let start = Instant::now();
    let timeout = Duration::from_millis(timeout_ms);
    while !path.exists() && start.elapsed() < timeout {
        sleep(Duration::from_millis(10)).await;
    }
    path.exists()
}
