//! Utility helpers for asynchronous tests.
//!
//! Provides functions to synchronize with background tasks in tests.

use std::path::Path;
use std::time::Duration;
use tokio::time::sleep;

/// Maximum number of times to poll for an expected file.
pub const SOCKET_RETRY_COUNT: u32 = 10;

/// Delay between polls when waiting for a file to appear.
///
/// Each attempt sleeps for this duration; multiply by
/// [`SOCKET_RETRY_COUNT`] to obtain the worst-case total wait.
/// The value is ten milliseconds.
pub const SOCKET_RETRY_DELAY: Duration = Duration::from_millis(10);

/// Wait for a file to appear within the given number of tries.
///
/// # Examples
///
/// ```rust,ignore
/// use std::path::Path;
/// use std::time::Duration;
/// use test_support::wait_for_file;
///
/// let path = Path::new("/tmp/example.sock");
/// let found = wait_for_file(path, 5, Duration::from_millis(10)).await;
/// assert!(found);
/// ```
pub async fn wait_for_file(path: &Path, tries: u32, delay: Duration) -> bool {
    for _ in 0..tries {
        if path.exists() {
            return true;
        }
        sleep(delay).await;
    }
    path.exists()
}
