//! Tests for task supervision and failure logging.

use super::log_task_failure;
use anyhow::anyhow;
use rstest::rstest;
use serde_json::Value;
use std::io::Write;
use std::sync::{Arc, Mutex};
use tokio::task::JoinError;
use tracing_subscriber::fmt::MakeWriterFn;

/// In-memory writer used to capture JSON-formatted tracing events.
#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Write for Buffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("lock buffer").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Create a [`JoinError`] representing a cancelled task.
fn create_cancelled_join_error() -> JoinError {
    tokio::runtime::Runtime::new()
        .expect("create runtime")
        .block_on(async {
            let handle = tokio::spawn(async {});
            handle.abort();
            handle.await.unwrap_err()
        })
}

#[rstest]
#[case(Ok(Ok(())), None)]
#[case(Ok(Err(anyhow!("boom"))), Some(("inner_error", "boom")))]
#[case(Err(create_cancelled_join_error()), Some(("join_error", "cancel")))]
fn logs_failures(
    #[case] res: std::result::Result<anyhow::Result<()>, JoinError>,
    #[case] expected: Option<(&str, &str)>,
) {
    use tracing_subscriber::prelude::*;

    let buf = Buffer::default();
    let make_writer = {
        let writer = buf.clone();
        MakeWriterFn::new(move || writer.clone())
    };
    let subscriber = tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer()
            .json()
            .with_writer(make_writer)
            .with_filter(tracing_subscriber::filter::LevelFilter::ERROR),
    );
    tracing::subscriber::with_default(subscriber, || {
        log_task_failure("task", &res);
    });

    let output = String::from_utf8(buf.0.lock().expect("read buffer").clone()).expect("utf8");
    match expected {
        None => assert!(output.is_empty()),
        Some((kind, err)) => {
            let line = output.lines().next().expect("log entry");
            let v: Value = serde_json::from_str(line).expect("json");
            let fields = &v["fields"];
            assert_eq!(fields["task"], "task");
            assert_eq!(fields["kind"], kind);
            assert!(fields["error"].as_str().expect("error str").contains(err));
            assert_eq!(fields["message"], "Task failed");
        }
    }
}
