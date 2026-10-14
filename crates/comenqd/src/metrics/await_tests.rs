//! Recorder-level checks for bounded await duration labels.

use super::{
    record_protocol_duration, record_queue_store_duration, tests::assert_histogram_labels,
};
use metrics::with_local_recorder;
use metrics_util::debugging::DebuggingRecorder;
use rstest::rstest;
use std::time::Duration;

#[rstest]
#[case("posted", "posted")]
#[case("unknown_identifier", "unknown_identifier")]
#[case("removed", "removed")]
#[case("rejected_capacity", "rejected_capacity")]
#[case("shutdown", "shutdown")]
#[case("client_gone", "client_gone")]
#[case("posted_without_url", "posted_without_url")]
#[case("posted_url_too_large", "posted_url_too_large")]
#[case("invalid_json", "invalid_json")]
#[case("unsafe_identifier", "unsafe_identifier")]
#[case("io_error", "io_error")]
#[case("https://github.com/private/comment", "unknown")]
fn records_bounded_await_durations(#[case] outcome: &'static str, #[case] expected: &str) {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    with_local_recorder(&recorder, || {
        record_protocol_duration(Some("await"), outcome, None, Duration::from_secs(1));
        record_queue_store_duration("resolve_await", "success", None, Duration::from_millis(1));
    });
    let metrics = snapshotter.snapshot().into_vec();
    assert_histogram_labels(
        &metrics,
        "comenqd_protocol_transaction_duration_seconds",
        &[
            ("operation", "await"),
            ("outcome", expected),
            ("error_kind", "none"),
        ],
    );
    assert_histogram_labels(
        &metrics,
        "comenqd_queue_store_operation_duration_seconds",
        &[
            ("operation", "resolve_await"),
            ("outcome", "success"),
            ("error_kind", "none"),
        ],
    );
}
