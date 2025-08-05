//! Utility helpers for asynchronous tests.
//!
//! Provides functions to synchronize with background tasks in tests.

use std::path::Path;
use std::time::Duration;
use tokio::time::{interval, sleep, timeout};

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

/// Poll an asynchronous predicate until it succeeds or a timeout elapses.
///
/// This helper underpins deterministic asynchronous tests, replacing brittle
/// sleeps with condition-based polling.
///
/// # Arguments
/// - `timeout_duration`: overall deadline before abandoning the poll.
/// - `poll_interval`: delay between predicate invocations.
/// - `predicate`: async closure returning `true` once the condition holds.
///
/// # Returns
/// `true` if the predicate returns `true` within the timeout; otherwise
/// `false`.
///
/// # Examples
/// ```rust,no_run
/// use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
/// use std::time::Duration;
/// use test_support::util::poll_until;
///
/// #[tokio::main]
/// async fn main() {
///     let flag = Arc::new(AtomicBool::new(false));
///     let flag_clone = flag.clone();
///
///     tokio::spawn(async move {
///         tokio::time::sleep(Duration::from_millis(50)).await;
///         flag_clone.store(true, Ordering::SeqCst);
///     });
///
///     let condition_met = poll_until(
///         Duration::from_millis(100),
///         Duration::from_millis(10),
///         || async { flag.load(Ordering::SeqCst) },
///     ).await;
///
///     assert!(condition_met);
/// }
/// ```
pub async fn poll_until<F, Fut>(
    timeout_duration: Duration,
    poll_interval: Duration,
    predicate: F,
) -> bool
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    timeout(timeout_duration, async {
        let mut ticker = interval(poll_interval);
        loop {
            if predicate().await {
                break;
            }
            ticker.tick().await;
        }
    })
    .await
    .is_ok()
}
