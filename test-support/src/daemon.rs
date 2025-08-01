//! Helper utilities for daemon tests.
//!
//! Provides constructors for temporary daemon [`Config`]s and simplified
//! creation of [`Octocrab`] clients targeting a [`MockServer`].

#![expect(clippy::expect_used, reason = "simplify test setup")]

use std::sync::Arc;

use comenqd::config::Config;
use octocrab::Octocrab;
use tempfile::TempDir;
use wiremock::MockServer;

/// Build a [`Config`] using paths inside `tmp`.
///
/// The configuration uses a dummy GitHub token and a one second cooldown.
pub fn temp_config(tmp: &TempDir) -> Config {
    Config {
        github_token: "t".into(),
        socket_path: tmp.path().join("sock"),
        queue_path: tmp.path().join("q"),
        cooldown_period_seconds: 1,
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
