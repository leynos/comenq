//! Tests for timeout helper utilities.

mod util;

use std::time::Duration;

use test_support::EnvVarGuard;
use tokio::time::sleep;

use util::{
    CI_MULTIPLIER, COVERAGE_MULTIPLIER, DEBUG_MULTIPLIER, MAX_TIMEOUT_SECS, MIN_TIMEOUT_SECS,
    TestComplexity, TimeoutConfig, timeout_with_retries,
};

#[test]
#[serial_test::serial]
fn calculate_timeout_caps_bounds() {
    // Test without CI environment
    {
        let _guard = EnvVarGuard::remove("CI");
        let cfg = TimeoutConfig::new(1, TestComplexity::Simple);
        assert_eq!(
            cfg.calculate_timeout(),
            Duration::from_secs(MIN_TIMEOUT_SECS)
        );
    }

    // Test with CI environment
    {
        let _guard = EnvVarGuard::set("CI", "1");
        let cfg = TimeoutConfig::new(400, TestComplexity::Complex);
        assert_eq!(
            cfg.calculate_timeout(),
            Duration::from_secs(MAX_TIMEOUT_SECS)
        );
    }
}

#[test]
#[serial_test::serial]
fn calculate_timeout_scales_with_ci_env() {
    let _guard = EnvVarGuard::set("CI", "1");
    let cfg = TimeoutConfig::new(10, TestComplexity::Simple);
    let mut expected = 10 * DEBUG_MULTIPLIER * CI_MULTIPLIER;
    if std::env::var("LLVM_PROFILE_FILE").is_ok() {
        expected *= COVERAGE_MULTIPLIER;
    }
    assert_eq!(cfg.calculate_timeout(), Duration::from_secs(expected));
}

#[test]
fn calculate_timeout_respects_complexity() {
    let simple = TimeoutConfig::new(10, TestComplexity::Simple).calculate_timeout();
    let moderate = TimeoutConfig::new(10, TestComplexity::Moderate).calculate_timeout();
    assert_eq!(moderate, Duration::from_secs(simple.as_secs() * 2));
}

#[test]
fn with_progressive_retry_scales_base() {
    let cfg = TimeoutConfig::new(10, TestComplexity::Simple);
    let base = cfg.calculate_timeout().as_secs();
    let expected = vec![
        Duration::from_secs(base * 50 / 100),
        Duration::from_secs(base * 100 / 100),
        Duration::from_secs(base * 150 / 100),
    ];
    assert_eq!(cfg.with_progressive_retry(), expected);
}

#[tokio::test(start_paused = true)]
async fn retries_after_timeout_then_succeeds() {
    let cfg = TimeoutConfig::new(10, TestComplexity::Simple);
    let first_timeout = cfg.with_progressive_retry()[0];
    use std::sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    };
    let attempts = Arc::new(AtomicU32::new(0));
    let handle_attempts = attempts.clone();
    let handle = tokio::spawn(timeout_with_retries(cfg, "demo", move || {
        let attempts = handle_attempts.clone();
        let first_timeout = first_timeout;
        async move {
            let current = attempts.fetch_add(1, Ordering::SeqCst) + 1;
            if current == 1 {
                sleep(first_timeout + Duration::from_secs(1)).await;
            }
            Ok(current)
        }
    }));

    tokio::time::advance(first_timeout).await;
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(1)).await;

    let result = handle.await.expect("join").expect("timeout_with_retries");
    assert_eq!(result, 2);
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn fails_after_all_retries() {
    let cfg = TimeoutConfig::new(10, TestComplexity::Simple);
    let timeouts = cfg.with_progressive_retry();
    let final_timeout = *timeouts.last().expect("timeouts");
    let handle = tokio::spawn(timeout_with_retries(cfg, "demo", move || async move {
        sleep(final_timeout + Duration::from_secs(1)).await;
        Ok(())
    }));

    tokio::time::advance(timeouts[0]).await;
    tokio::task::yield_now().await;
    tokio::time::advance(timeouts[1]).await;
    tokio::task::yield_now().await;
    tokio::time::advance(timeouts[2]).await;
    tokio::task::yield_now().await;

    let err = handle.await.expect("join").expect_err("should time out");
    assert!(err.contains("timed out"));
}
