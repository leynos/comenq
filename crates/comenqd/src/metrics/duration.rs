//! Fixed-label duration metrics for protocol and queue-store operations.
//!
//! This module keeps request and storage timings separate from the daemon's
//! counters and gauges while ensuring that every metric label comes from a
//! finite vocabulary.

use metrics::histogram;

const PROTOCOL_TRANSACTION_DURATION: &str = "comenqd_protocol_transaction_duration_seconds";
const QUEUE_STORE_OPERATION_DURATION: &str = "comenqd_queue_store_operation_duration_seconds";

/// Record a complete listener transaction with fixed, low-cardinality labels.
///
/// An unparsed request uses the `unknown` operation label. For example, a
/// malformed JSON request is recorded as
/// `operation="unknown",outcome="failed",error_kind="invalid_json"`.
pub(crate) fn record_protocol_duration(
    operation: Option<&'static str>,
    outcome: &'static str,
    error_kind: Option<&'static str>,
    duration: std::time::Duration,
) {
    histogram!(
        PROTOCOL_TRANSACTION_DURATION,
        "operation" => protocol_operation_label(operation),
        "outcome" => bounded_outcome_label(outcome),
        "error_kind" => bounded_error_label(error_kind),
    )
    .record(duration.as_secs_f64());
}

/// Record a blocking store result; failed deletions use a bounded label such as
/// `operation="del",outcome="failure"`.
pub(crate) fn record_queue_store_duration(
    operation: &'static str,
    outcome: &'static str,
    error_kind: Option<&'static str>,
    duration: std::time::Duration,
) {
    histogram!(
        QUEUE_STORE_OPERATION_DURATION,
        "operation" => queue_store_operation_label(operation),
        "outcome" => bounded_outcome_label(outcome),
        "error_kind" => bounded_error_label(error_kind),
    )
    .record(duration.as_secs_f64());
}

/// Map wire operations to a fixed label vocabulary; unparsed requests use
/// `unknown`.
fn protocol_operation_label(operation: Option<&'static str>) -> &'static str {
    match operation {
        Some("put") => "put",
        Some("list") => "list",
        Some("bump") => "bump",
        Some("bust") => "bust",
        Some("del") => "del",
        _ => "unknown",
    }
}

/// Map internal store operation names to a finite label vocabulary; for
/// example, `claim_next_due` is recorded as `claim`.
fn queue_store_operation_label(operation: &'static str) -> &'static str {
    match operation {
        "put" | "list" | "bump" | "bust" | "del" | "next_due" | "complete" => operation,
        "claim_next_due" => "claim",
        "release_claim" => "release",
        "recover_worker_state" => "recover",
        "queue_metrics_snapshot" => "metrics",
        _ => "unknown",
    }
}

/// Keep outcomes fixed even if a future caller supplies an unexpected value;
/// unrecognised values are recorded as `unknown`.
fn bounded_outcome_label(outcome: &'static str) -> &'static str {
    match outcome {
        "accepted" | "failed" | "rejected" | "success" | "failure" => outcome,
        _ => "unknown",
    }
}

/// Restrict error labels to known protocol and persistence categories; unknown
/// classifications are recorded as `unknown`.
fn bounded_error_label(error_kind: Option<&'static str>) -> &'static str {
    match error_kind {
        None | Some("none") => "none",
        Some(
            kind @ ("protocol_io"
            | "invalid_json"
            | "response_too_large"
            | "response_serialization"
            | "not_found"
            | "permission_denied"
            | "invalid_data"
            | "io_error"
            | "unknown_identifier"
            | "unsafe_identifier"
            | "invalid_repository_component"
            | "invalid_last_post"
            | "blocking_task"
            | "queue_full"
            | "entry_too_large"
            | "byte_budget_exceeded"),
        ) => kind,
        _ => "unknown",
    }
}
