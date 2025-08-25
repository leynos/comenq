//! Shared test utilities for adaptive timeouts.

use std::time::Duration;

pub const MIN_TIMEOUT_SECS: u64 = 10;
pub const MAX_TIMEOUT_SECS: u64 = 600;
pub const DEBUG_MULTIPLIER: u64 = 2;
pub const COVERAGE_MULTIPLIER: u64 = 5;
pub const CI_MULTIPLIER: u64 = 2;
pub const PROGRESSIVE_RETRY_PERCENTS: [u64; 3] = [50, 100, 150];

#[derive(Debug, Clone, Copy)]
pub enum TestComplexity {
    Simple,
    Moderate,
    Complex,
}

#[derive(Debug, Clone, Copy)]
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
