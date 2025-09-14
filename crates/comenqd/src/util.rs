//! Internal utilities shared by daemon components.
//!
//! Provides helpers used across production code and tests.

use std::ffi::OsStr;

/// Names of files storing queue metadata.
///
/// Extend this list when new metadata files are introduced.
pub(crate) const METADATA_FILE_NAMES: [&str; 2] = ["version", "recv.lock"];

/// Returns whether a file name represents queue metadata.
///
/// # Examples
///
/// ```
/// use comenqd::daemon::is_metadata_file;
/// use std::ffi::OsStr;
/// assert!(is_metadata_file(OsStr::new("version")));
/// assert!(!is_metadata_file(OsStr::new("0001")));
/// ```
pub fn is_metadata_file(name: impl AsRef<OsStr>) -> bool {
    let name = name.as_ref();
    METADATA_FILE_NAMES.iter().any(|m| OsStr::new(m) == name)
}
