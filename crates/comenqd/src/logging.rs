//! Logging utilities for the daemon.
//!
//! Initializes structured logging using `tracing` and
//! `tracing-subscriber`, reading filter settings from the `RUST_LOG`
//! environment variable.

use tracing_subscriber::{EnvFilter, fmt};

/// Initialize the global tracing subscriber.
pub fn init() {
    fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .json()
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_logging() {
        init();
    }
}
