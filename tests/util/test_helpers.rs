//! Helper functions for tests.
//!
//! Provides constructors for daemon [`Config`] with temporary paths and a
//! simplified way to create an [`Octocrab`] client for a [`MockServer`].

#![expect(clippy::expect_used, reason = "simplify test setup")]

use std::sync::Arc;

use comenqd::config::Config;
use octocrab::Octocrab;
use tempfile::TempDir;
use wiremock::MockServer;

/// Build a daemon [`Config`] using paths inside the given temporary directory.
///
/// The returned configuration uses a dummy GitHub token and sets the
/// cooldown to one second.
pub fn temp_config(tmp: &TempDir) -> Config {
    Config {
        github_token: "t".into(),
        socket_path: tmp.path().join("sock"),
        queue_path: tmp.path().join("q"),
        cooldown_period_seconds: 1,
    }
}

/// Construct an [`Octocrab`] client targeting the provided [`MockServer`].
///
/// The client is built with a placeholder personal token and its base URL set
/// to the mock server URI.
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
