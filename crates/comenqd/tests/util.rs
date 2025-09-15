//! Shared test utilities for adaptive timeouts and task join diagnostics.

use rstest::rstest;
use std::time::Duration;
use tokio::task::JoinError;

pub const MIN_TIMEOUT_SECS: u64 = 10;
pub const MAX_TIMEOUT_SECS: u64 = 600;
pub const DEBUG_MULTIPLIER: u64 = 2;
pub const COVERAGE_MULTIPLIER: u64 = 5;
pub const CI_MULTIPLIER: u64 = 2;
pub const PROGRESSIVE_RETRY_PERCENTS: [u64; 3] = [50, 100, 150];

#[derive(Debug, Clone)]
pub enum TestComplexity {
    Simple,
    Moderate,
    Complex,
}

#[derive(Debug, Clone)]
pub struct TimeoutConfig {
    base_seconds: u64,
    complexity: TestComplexity,
}

impl TimeoutConfig {
    pub const fn new(base_seconds: u64, complexity: TestComplexity) -> Self {
        Self {
            base_seconds,
            complexity,
        }
    }

    pub fn calculate_timeout(&self) -> Duration {
        let mut timeout = self.base_seconds;
        timeout = timeout.saturating_mul(match self.complexity {
            TestComplexity::Simple => 1,
            TestComplexity::Moderate => 2,
            TestComplexity::Complex => 3,
        });
        #[cfg(debug_assertions)]
        {
            timeout = timeout.saturating_mul(DEBUG_MULTIPLIER);
        }
        if std::env::var("LLVM_PROFILE_FILE").is_ok() {
            timeout = timeout.saturating_mul(COVERAGE_MULTIPLIER);
        }
        if std::env::var("CI").is_ok() {
            timeout = timeout.saturating_mul(CI_MULTIPLIER);
        }
        timeout = timeout.max(MIN_TIMEOUT_SECS);
        timeout = timeout.min(MAX_TIMEOUT_SECS);
        Duration::from_secs(timeout)
    }

    pub fn with_progressive_retry(&self) -> Vec<Duration> {
        let base = self.calculate_timeout().as_secs();
        PROGRESSIVE_RETRY_PERCENTS
            .iter()
            .map(|p| Duration::from_secs(base * p / 100))
            .collect()
    }
}

pub async fn timeout_with_retries<F, Fut, T>(
    config: TimeoutConfig,
    operation_name: &str,
    mut operation: F,
) -> Result<T, String>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, String>> + Send,
    T: Send,
{
    let timeouts = config.with_progressive_retry();
    for (attempt, timeout_duration) in timeouts.iter().enumerate() {
        let attempt_num = attempt + 1;
        match tokio::time::timeout(*timeout_duration, operation()).await {
            Ok(Ok(result)) => return Ok(result),
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                if attempt_num == timeouts.len() {
                    return Err(format!(
                        "{operation_name} timed out after all retry attempts"
                    ));
                }
            }
        }
    }
    Err(format!("{operation_name} exhausted all retry attempts"))
}

#[rstest]
#[case(TestComplexity::Simple)]
#[case(TestComplexity::Moderate)]
#[case(TestComplexity::Complex)]
fn uses_all_test_complexity_variants(#[case] complexity: TestComplexity) {
    drop(TimeoutConfig::new(1, complexity));
}

/// Map a task [`JoinError`] into a concise diagnostic message.
///
/// ```ignore
/// let handle = tokio::spawn(async {});
/// handle.abort();
/// let err = handle.await.unwrap_err();
/// assert_eq!(join_err("worker", err), "worker task cancelled");
/// ```
pub(crate) fn join_err(name: &str, e: JoinError) -> String {
    if e.is_panic() {
        format!("{name} task panicked")
    } else if e.is_cancelled() {
        format!("{name} task cancelled")
    } else {
        format!("{name} task failed: {e}")
    }
}

#[derive(Debug, Clone)]
enum JoinScenario {
    Cancelled,
    Panicked,
}

#[rstest]
#[case::cancelled(
    JoinScenario::Cancelled,
    "failed to join cancelled task",
    "worker task cancelled"
)]
#[case::panicked(
    JoinScenario::Panicked,
    "failed to join panicked task",
    "worker task panicked"
)]
#[tokio::test]
async fn join_err_maps_panic_and_cancel(
    #[case] scenario: JoinScenario,
    #[case] join_msg: &str,
    #[case] expected: &str,
) {
    let handle = match scenario {
        JoinScenario::Cancelled => {
            let h = tokio::spawn(async {
                tokio::task::yield_now().await;
            });
            h.abort();
            h
        }
        JoinScenario::Panicked => tokio::spawn(async { panic!("boom") }),
    };
    let err = handle.await.expect_err(join_msg);
    assert_eq!(join_err("worker", err), expected);
}
