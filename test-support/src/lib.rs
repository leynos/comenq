//! Test support utilities.

pub mod util;

/// Maximum number of times to poll for an expected file.
pub use util::SOCKET_RETRY_COUNT;

/// Delay between polls when waiting for a file to appear.
///
/// Multiply by [`SOCKET_RETRY_COUNT`] for the worst-case wait duration.
pub use util::SOCKET_RETRY_DELAY;

/// Wait for a file to appear, retrying with a fixed delay.
///
/// This is re-exported from [`util`] for convenience in tests.
///
/// # Arguments
/// * `path` – Path to the file that is expected to be created.
/// * `tries` – Maximum number of polling attempts.
/// * `delay` – Pause between attempts as a [`std::time::Duration`].
///   The total wait time is `tries * delay`.
///
/// # Returns
/// `true` if the file appears within `tries` attempts, otherwise `false`.
///
/// # Examples
/// ```rust,no_run
/// use std::path::Path;
/// use test_support::{wait_for_file, SOCKET_RETRY_COUNT, SOCKET_RETRY_DELAY};
///
/// #[tokio::main]
/// async fn main() {
///     let path = Path::new("/tmp/example.sock");
///     let found = wait_for_file(path, SOCKET_RETRY_COUNT, SOCKET_RETRY_DELAY).await;
///     assert!(found);
/// }
/// ```
pub use util::wait_for_file;
