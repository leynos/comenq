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
const REQUESTS: &str = "comenqd_requests_total";
const QUEUE_ENTRIES: &str = "comenqd_queue_entries";
const QUEUE_BYTES: &str = "comenqd_queue_bytes";
const COOLDOWN_WAIT_DURATION: &str = "comenqd_cooldown_wait_duration_seconds";
const GITHUB_POSTS: &str = "comenqd_github_posts_total";
const GITHUB_POST_DURATION: &str = "comenqd_github_post_duration_seconds";

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

/// Record whether a client request reached the daemon queue.
pub(crate) fn record_request(operation: Option<&'static str>, outcome: &'static str) {
    if let Some(operation) = operation {
        counter!(REQUESTS, "operation" => operation, "outcome" => outcome).increment(1);
    } else {
        counter!(REQUESTS, "outcome" => outcome).increment(1);
    }
}

/// Record the current number of persisted pending entries without labels.
pub(crate) fn record_queue_entries(count: usize) {
    gauge!(QUEUE_ENTRIES).set(count as f64);
}

/// Record persisted entry bytes plus reserved mutation headroom without labels.
pub(crate) fn record_queue_bytes(bytes: u64) {
    gauge!(QUEUE_BYTES).set(bytes as f64);
}

/// Record the configured duration of a cooldown wait.
pub(crate) fn record_cooldown_wait(seconds: u64) {
    histogram!(COOLDOWN_WAIT_DURATION).record(seconds as f64);
}

/// Record the bounded result class of a GitHub comment request.
pub(crate) fn record_github_post_outcome(outcome: &'static str) {
    counter!(GITHUB_POSTS, "outcome" => outcome).increment(1);
}

/// Record the elapsed duration of a GitHub comment request.
pub(crate) fn record_github_post_duration(duration: std::time::Duration) {
    histogram!(GITHUB_POST_DURATION).record(duration.as_secs_f64());
}

#[cfg(test)]
mod tests {
    //! Tests bounded metric names, values, and labels emitted by this module.

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
            record_request(Some("put"), "accepted");
            record_request(Some("list"), "rejected");
            record_request(Some("bump"), "failed");
            record_request(Some("bust"), "accepted");
            record_request(Some("del"), "accepted");
            record_github_post_outcome("success");
            record_github_post_outcome("api_error");
            record_github_post_outcome("timeout");
        });

        let metrics = snapshotter.snapshot().into_vec();
        let names = metric_names(&metrics);

        assert!(names.contains(&TASK_RESTARTS));
        assert!(names.contains(&GITHUB_POSTS));
        assert_eq!(
            metrics
                .iter()
                .filter(|(key, _, _, _)| key.key().name() == REQUESTS)
                .count(),
            5
        );
        assert_eq!(
            metrics
                .iter()
                .filter(|(key, _, _, _)| key.key().name() == GITHUB_POSTS)
                .count(),
            3
        );
        assert!(metrics.iter().all(|(key, _, _, _)| {
            key.key().labels().all(|label| {
                matches!(
                    (label.key(), label.value()),
                    ("task", "listener" | "worker")
                        | ("operation", "put" | "list" | "bump" | "bust" | "del")
                        | ("outcome", "accepted" | "failed" | "rejected")
                        | ("outcome", "success" | "api_error" | "timeout")
                )
            })
        }));
    }

    #[test]
    fn records_label_free_queue_depth_and_accounted_bytes_gauges() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        with_local_recorder(&recorder, || {
            record_queue_entries(7);
            record_queue_bytes(4096);
        });
        let metrics = snapshotter.snapshot().into_vec();
        let queue_metrics: Vec<_> = metrics
            .iter()
            .filter(|(key, _, _, _)| matches!(key.key().name(), QUEUE_ENTRIES | QUEUE_BYTES))
            .collect();
        assert_eq!(queue_metrics.len(), 2);
        for (key, _, _, _) in &queue_metrics {
            assert!(key.key().labels().next().is_none());
        }
        assert!(queue_metrics.iter().any(|(key, _, _, value)| {
            key.key().name() == QUEUE_ENTRIES
                && matches!(value, DebugValue::Gauge(value) if value.into_inner() == 7.0)
        }));
        assert!(queue_metrics.iter().any(|(key, _, _, value)| {
            key.key().name() == QUEUE_BYTES
                && matches!(value, DebugValue::Gauge(value) if value.into_inner() == 4096.0)
        }));
    }

    #[test]
    fn records_cooldown_duration() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        with_local_recorder(&recorder, || {
            record_cooldown_wait(45);
            record_github_post_duration(std::time::Duration::from_secs(2));
        });

        let metrics = snapshotter.snapshot().into_vec();
        assert!(metric_names(&metrics).contains(&COOLDOWN_WAIT_DURATION));
        assert!(metric_names(&metrics).contains(&GITHUB_POST_DURATION));
    }
}
