//! Filesystem helpers for tests.
use std::path::Path;
use tokio::time::{Duration, Instant, sleep};

/// Wait for a path to exist within the given timeout.
///
/// Polls the filesystem every 10ms until `timeout_ms` has elapsed.
/// Returns `true` if the path appears before the timeout.
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
