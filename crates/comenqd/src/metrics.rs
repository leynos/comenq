//! Bounded Prometheus metrics for daemon reliability and throughput.
//!
//! The daemon attempts to expose a local scrape endpoint at
//! `127.0.0.1:9000/metrics`.
//! Metric labels are static, low-cardinality classifications so metrics never
//! include request content, repository names, file paths, or credentials.

use metrics::{counter, gauge, histogram};
use metrics_exporter_prometheus::{BuildError, PrometheusBuilder};

/// Local address of the daemon's Prometheus scrape endpoint.
pub const PROMETHEUS_LISTEN_ADDR: ([u8; 4], u16) = ([127, 0, 0, 1], 9000);

const TASK_RESTARTS: &str = "comenqd_task_restarts_total";
const QUEUE_WRITER_FAILURES: &str = "comenqd_queue_writer_failures_total";
const CLIENT_CHANNEL_DEPTH: &str = "comenqd_client_channel_depth";
const REQUESTS: &str = "comenqd_requests_total";
const COOLDOWN_WAIT_DURATION: &str = "comenqd_cooldown_wait_duration_seconds";

/// Install the Prometheus recorder and local scrape endpoint.
///
/// # Errors
///
/// Returns an error when the metrics listener cannot bind or another recorder
/// is already installed.
pub fn install_prometheus() -> Result<(), BuildError> {
    PrometheusBuilder::new()
        .with_http_listener(PROMETHEUS_LISTEN_ADDR)
        .install()
}

/// Record a supervised task restart using a fixed task-name label.
pub(crate) fn record_task_restart(task: &'static str) {
    counter!(TASK_RESTARTS, "task" => task).increment(1);
}

/// Record an enqueue failure from the persistent queue writer.
pub(crate) fn record_queue_writer_failure() {
    counter!(QUEUE_WRITER_FAILURES, "queue_side" => "sender").increment(1);
}

/// Record the currently buffered client requests as a bounded depth proxy.
pub(crate) fn record_client_channel_depth(depth: usize) {
    gauge!(CLIENT_CHANNEL_DEPTH).set(depth as f64);
}

/// Record whether a client request reached the daemon queue.
pub(crate) fn record_request_outcome(outcome: &'static str) {
    counter!(REQUESTS, "outcome" => outcome).increment(1);
}

/// Record the configured duration of a cooldown wait.
pub(crate) fn record_cooldown_wait(seconds: u64) {
    histogram!(COOLDOWN_WAIT_DURATION).record(seconds as f64);
}

#[cfg(test)]
mod tests {
    use super::*;
    use metrics::with_local_recorder;
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    fn metric_names(
        metrics: &[(
            metrics_util::CompositeKey,
            Option<metrics::Unit>,
            Option<metrics::SharedString>,
            DebugValue,
        )],
    ) -> Vec<&str> {
        metrics
            .iter()
            .map(|(key, _, _, _)| key.key().name())
            .collect()
    }

    #[test]
    fn records_success_and_failure_metrics_with_bounded_labels() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        with_local_recorder(&recorder, || {
            record_task_restart("worker");
            record_queue_writer_failure();
            record_request_outcome("accepted");
            record_request_outcome("rejected");
        });

        let metrics = snapshotter.snapshot().into_vec();
        let names = metric_names(&metrics);

        assert!(names.contains(&TASK_RESTARTS));
        assert!(names.contains(&QUEUE_WRITER_FAILURES));
        assert_eq!(
            metrics
                .iter()
                .filter(|(key, _, _, _)| key.key().name() == REQUESTS)
                .count(),
            2
        );
        assert!(metrics.iter().all(|(key, _, _, _)| {
            key.key().labels().all(|label| {
                matches!(
                    (label.key(), label.value()),
                    ("task", "listener" | "worker" | "writer")
                        | ("queue_side", "sender")
                        | ("outcome", "accepted" | "rejected")
                )
            })
        }));
    }

    #[test]
    fn records_bounded_depth_and_cooldown_duration() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        with_local_recorder(&recorder, || {
            record_client_channel_depth(3);
            record_cooldown_wait(45);
        });

        let metrics = snapshotter.snapshot().into_vec();
        assert!(metric_names(&metrics).contains(&CLIENT_CHANNEL_DEPTH));
        assert!(metric_names(&metrics).contains(&COOLDOWN_WAIT_DURATION));
    }
}
