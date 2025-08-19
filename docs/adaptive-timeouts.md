# Adaptive timeouts for asynchronous tests

Fixed timeouts often cause flaky tests, particularly on slower machines, under
coverage instrumentation, or in continuous integration environments. To address
this the daemon tests use a small `smart_timeouts` module that scales and caps
timeouts based on build configuration and the detected environment.

The `timeout_with_retries` helper executes an operation with a progressively
increasing timeout. This offers clearer diagnostics while avoiding spurious
failures from transient delays.

```rust
# use comenqd::daemon::{smart_timeouts, timeout_with_retries};
# async fn example() {
let result = timeout_with_retries(
    smart_timeouts::WORKER_SUCCESS,
    "worker join",
    || {
        // operation producing a `Future`
        Box::pin(async { Ok(()) })
    },
)
.await
.expect("worker completed");
# }
```

By centralising these rules the tests remain concise yet adapt to differing
runtime conditions.
