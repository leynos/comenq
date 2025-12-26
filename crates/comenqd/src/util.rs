//! Internal utilities shared by daemon components.
//!
//! Provides helpers used across production code and tests.

use std::ffi::OsStr;

/// Names of files storing queue metadata.
///
/// Extend this list when new metadata files are introduced.
pub(crate) const METADATA_FILE_NAMES: [&str; 3] = ["version", "recv.lock", "send.lock"];

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

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::version("version")]
    #[case::recv_lock("recv.lock")]
    #[case::send_lock("send.lock")]
    fn is_metadata_file_recognises_metadata(#[case] name: &str) {
        assert!(is_metadata_file(name));
    }

    #[rstest]
    #[case::segment_0000("0000")]
    #[case::segment_0001("0001")]
    #[case::segment_9999("9999")]
    #[case::data_json("data.json")]
    #[case::lock("lock")]
    #[case::empty("")]
    fn is_metadata_file_rejects_non_metadata(#[case] name: &str) {
        assert!(!is_metadata_file(name));
    }
}
