# Adaptive timeouts for asynchronous tests

Fixed timeouts often cause flaky tests, particularly on slower machines, under
coverage instrumentation, or in continuous integration environments. To address
this, the daemon tests use a small `smart_timeouts` module that scales and caps
timeouts based on build configuration and the detected environment. Tunable
constants expose the multipliers and bounds, allowing projects to tweak
behaviour without touching call sites. Constants: `MIN_TIMEOUT_SECS`,
`MAX_TIMEOUT_SECS`, `DEBUG_MULTIPLIER`, `COVERAGE_MULTIPLIER`, `CI_MULTIPLIER`,
and `PROGRESSIVE_RETRY_PERCENTS`.

The `timeout_with_retries` helper executes an operation with a progressively
increasing timeout. By default, it retries with 50%, 100%, and 150% of the
calculated timeout, offering clearer diagnostics while avoiding spurious
failures from transient delays.

Note: coverage scaling activates when the environment variable
`LLVM_PROFILE_FILE` is set.

```rust
// Note: These utilities are only available in test builds (#[cfg(test)])
# use crate::daemon::{smart_timeouts, timeout_with_retries};
# async fn example() {
let result = timeout_with_retries(
    smart_timeouts::WORKER_SUCCESS,
    "worker join",
    || async { Ok(()) },
)
.await
.expect("worker completed");
# }
```

By centralising these rules, the tests remain concise yet adapt to differing
runtime conditions.
