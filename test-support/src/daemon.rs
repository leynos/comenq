//! Helper utilities for daemon tests.
//!
//! Provides constructors for temporary daemon configurations and simplified
//! creation of [`Octocrab`] clients targeting a [`MockServer`].

#![expect(clippy::expect_used, reason = "simplify test setup")]

use std::{path::PathBuf, sync::Arc, time::Duration};

use octocrab::Octocrab;
use serde::{Deserialize, Serialize};
use tempfile::TempDir;
use wiremock::MockServer;

/// Minimal configuration used in daemon tests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Timeout for GitHub API requests in seconds.
    pub github_api_timeout_secs: u64,
    /// Capacity of the channel buffering client requests.
    pub client_channel_capacity: usize,
}

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            github_token: String::new(),
            socket_path: PathBuf::new(),
            queue_path: PathBuf::new(),
            cooldown_period_seconds: 1,
            restart_min_delay_ms: 1,
            github_api_timeout_secs: 30,
            client_channel_capacity: 1024,
        }
    }
}

/// Build a [`TestConfig`] using paths inside `tmp`.
///
/// The configuration uses a dummy GitHub token and a one second cooldown.
/// The minimum restart delay is set to 1 ms to exercise supervision paths
/// without introducing flaky timing in tests.
pub fn temp_config(tmp: &TempDir) -> TestConfig {
    TestConfig {
        github_token: "t".into(),
        socket_path: tmp.path().join("sock"),
        queue_path: tmp.path().join("q"),
        cooldown_period_seconds: 1,
        restart_min_delay_ms: 1,
        github_api_timeout_secs: 30,
        client_channel_capacity: 1024,
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
        self.restart_min_delay_ms = ms.max(1);
        self
    }

    /// Override the minimum restart delay using a [`Duration`].
    #[must_use]
    pub fn with_restart_min_delay(mut self, d: Duration) -> Self {
        let ms = d.as_millis().max(1) as u64;
        self.restart_min_delay_ms = ms;
        self
    }

    /// Override the GitHub API timeout (seconds).
    #[must_use]
    pub fn with_github_api_timeout_secs(mut self, secs: u64) -> Self {
        self.github_api_timeout_secs = secs;
        self
    }

    /// Override the GitHub API timeout using a [`Duration`].
    #[must_use]
    pub fn with_github_api_timeout(mut self, d: Duration) -> Self {
        self.github_api_timeout_secs = d.as_secs();
        self
    }

    /// Override the client channel capacity.
    #[must_use]
    pub fn with_client_channel_capacity(mut self, cap: usize) -> Self {
        self.client_channel_capacity = cap;
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
