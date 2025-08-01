//! Filesystem helpers for tests.
use std::path::Path;
use tokio::time::{Duration, Instant, sleep};

/// Wait for a path to appear within the timeout.
///
/// The filesystem is polled every 10&nbsp;ms until `timeout_ms` has elapsed.
/// Returns `true` once the path exists.
///
/// # Examples
///
/// ```rust,no_run
/// use std::path::Path;
/// use crate::support::fs::wait_for_path;
///
/// # tokio_test::block_on(async move {
/// let path = Path::new("/tmp/sock");
/// assert!(wait_for_path(path, 100).await);
/// # });
/// ```
pub async fn wait_for_path(path: &Path, timeout_ms: u64) -> bool {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    while Instant::now() < deadline {
        if path.exists() {
            return true;
        }
        sleep(Duration::from_millis(10)).await;
    }
    path.exists()
}
