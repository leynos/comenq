//! Helper utilities for daemon tests.
//!
//! Provides constructors for temporary daemon configurations and simplified
//! creation of [`Octocrab`] clients targeting a [`MockServer`].

#![expect(clippy::expect_used, reason = "simplify test setup")]

use std::{path::PathBuf, sync::Arc};

use octocrab::Octocrab;
use serde::{Deserialize, Serialize};
use tempfile::TempDir;
use wiremock::MockServer;

/// Minimal configuration used in daemon tests.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TestConfig {
    /// GitHub Personal Access Token.
    pub github_token: String,
    /// Path to the Unix Domain Socket.
    pub socket_path: PathBuf,
    /// Directory for the persistent queue.
    pub queue_path: PathBuf,
    /// Cooldown between comment posts in seconds.
    pub cooldown_period_seconds: u64,
    /// Minimum delay in milliseconds applied between task restarts.
    pub restart_min_delay_ms: u64,
}

/// Build a [`TestConfig`] using paths inside `tmp`.
///
/// The configuration uses a dummy GitHub token and a one second cooldown.
pub fn temp_config(tmp: &TempDir) -> TestConfig {
    TestConfig {
        github_token: "t".into(),
        socket_path: tmp.path().join("sock"),
        queue_path: tmp.path().join("q"),
        cooldown_period_seconds: 1,
        restart_min_delay_ms: 1,
    }
}

impl TestConfig {
    /// Override the cooldown period and return the updated configuration.
    #[must_use]
    pub fn with_cooldown(mut self, secs: u64) -> Self {
        self.cooldown_period_seconds = secs;
        self
    }

    /// Override the minimum restart delay (milliseconds) and return the updated
    /// configuration.
    #[must_use]
    pub fn with_restart_min_delay_ms(mut self, ms: u64) -> Self {
        self.restart_min_delay_ms = ms;
        self
    }
}

/// Construct an [`Octocrab`] client for a [`MockServer`].
///
/// The client is initialised with a placeholder token and its base URL
/// configured to the mock server's URI.
pub fn octocrab_for(server: &MockServer) -> Arc<Octocrab> {
    Arc::new(
        Octocrab::builder()
            .personal_token("t".to_string())
            .base_uri(server.uri())
            .expect("base_uri")
            .build()
            .expect("build octocrab"),
    )
}
