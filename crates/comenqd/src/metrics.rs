//! Bounded Prometheus metrics for daemon reliability and throughput.
//!
//! The daemon attempts to expose a local scrape endpoint at
//! `127.0.0.1:9000/metrics`.
//! Metric labels are static, low-cardinality classifications so metrics never
//! include request content, repository names, file paths, or credentials.

use metrics::{counter, histogram};
use metrics_exporter_prometheus::{BuildError, PrometheusBuilder};

/// Local address of the daemon's Prometheus scrape endpoint.
pub const PROMETHEUS_LISTEN_ADDR: ([u8; 4], u16) = ([127, 0, 0, 1], 9000);

const TASK_RESTARTS: &str = "comenqd_task_restarts_total";
const REQUESTS: &str = "comenqd_requests_total";
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
pub(crate) fn record_request_outcome(outcome: &'static str) {
    counter!(REQUESTS, "outcome" => outcome).increment(1);
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
            record_request_outcome("accepted");
            record_request_outcome("rejected");
            record_request_outcome("failed");
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
            3
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
                        | ("outcome", "accepted" | "failed" | "rejected")
                        | ("outcome", "success" | "api_error" | "timeout")
                )
            })
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
