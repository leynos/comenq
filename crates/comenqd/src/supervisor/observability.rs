//! Structured tracing helpers for supervised daemon tasks.

/// Log any failure from a supervised task.
///
/// This is a no-op when the task completes successfully. Failures carry the
/// task name and a stable kind so operators can distinguish application errors
/// from Tokio join failures.
pub(super) fn log_task_failure<T, E>(task: &str, result: &std::result::Result<anyhow::Result<T>, E>)
where
    E: std::fmt::Display,
{
    match result {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => tracing::error!(
            task,
            kind = "inner_error",
            error = %error,
            "Task failed",
        ),
        Err(error) => tracing::error!(
            task,
            kind = "join_error",
            error = %error,
            "Task failed",
        ),
    }
}
