//! Diagnostic logging for malformed and oversized history records.

use std::path::Path;

pub(super) fn log_malformed(path: &Path, offset: u64, error: &serde_json::Error) {
    tracing::error!(
        path = %path.display(),
        offset,
        error = %error,
        "Skipping unreadable history record"
    );
}

pub(super) fn log_oversized(path: &Path, offset: u64) {
    tracing::error!(
        path = %path.display(),
        offset,
        "Skipping oversized history record"
    );
}
