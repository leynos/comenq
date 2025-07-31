//! Test support utilities.

pub mod util;

/// Wait for a file to appear, retrying with a fixed delay.
///
/// This is re-exported from [`util`] for convenience in tests.
pub use util::wait_for_file;
