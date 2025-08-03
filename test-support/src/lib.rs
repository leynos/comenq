//! Test support utilities.

pub mod util;

pub use util::{SOCKET_RETRY_COUNT, SOCKET_RETRY_DELAY};

/// Wait for a file to appear, retrying with a fixed delay.
///
/// This is re-exported from [`util`] for convenience in tests.
///
/// # Arguments
/// * `path` – Path to the file that is expected to be created.
/// * `tries` – Maximum number of polling attempts.
/// * `delay` – Pause between attempts as a [`std::time::Duration`].
///
/// # Returns
/// `true` if the file appears within `tries` attempts, otherwise `false`.
///
/// # Examples
/// ```rust,ignore
/// use std::path::Path;
/// use test_support::{wait_for_file, SOCKET_RETRY_COUNT, SOCKET_RETRY_DELAY};
///
/// let path = Path::new("/tmp/example.sock");
/// let found = wait_for_file(path, SOCKET_RETRY_COUNT, SOCKET_RETRY_DELAY).await;
/// assert!(found);
/// ```
pub use util::wait_for_file;
