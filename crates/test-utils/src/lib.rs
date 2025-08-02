//! Shared test utilities.
//!
//! Provides helpers for constructing temporary daemon configuration and
//! mock Octocrab clients for use with `wiremock` servers.

use std::sync::Arc;

use comenqd::config::Config;
use octocrab::Octocrab;
use tempfile::TempDir;
use wiremock::MockServer;

/// Build a [`Config`] using paths inside `tmp`.
pub fn temp_config(tmp: &TempDir) -> Config {
    Config {
        github_token: String::from("t"),
        socket_path: tmp.path().join("sock"),
        queue_path: tmp.path().join("q"),
        cooldown_period_seconds: 1,
    }
}

/// Construct an [`Octocrab`] client for a [`MockServer`].
#[expect(clippy::expect_used, reason = "simplify test helper setup")]
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
