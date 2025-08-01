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

/// Build a [`Config`] using paths inside `tmp` with a one-second cooldown.
///
/// # Examples
///
/// ```
/// use tempfile::tempdir;
/// use comenqd::config::Config;
/// use test_support::temp_config;
///
/// let dir = tempdir().unwrap();
/// let cfg: Config = temp_config(&dir);
/// assert_eq!(cfg.cooldown_period_seconds, 1);
/// ```
pub fn temp_config(tmp: &TempDir) -> Config {
    temp_config_with(tmp, 1)
}

/// Build a [`Config`] using paths inside `tmp`.
///
/// # Parameters
/// - `tmp`: temporary directory for socket and queue paths.
/// - `cooldown_period_seconds`: cooldown period between GitHub API calls.
///
/// # Examples
///
/// ```
/// use tempfile::tempdir;
/// use comenqd::config::Config;
/// use test_support::temp_config_with;
///
/// let dir = tempdir().unwrap();
/// let fast_cfg: Config = temp_config_with(&dir, 0);
/// assert_eq!(fast_cfg.cooldown_period_seconds, 0);
/// ```
pub fn temp_config_with(tmp: &TempDir, cooldown_period_seconds: u64) -> Config {
    Config {
        github_token: "t".into(),
        socket_path: tmp.path().join("sock"),
        queue_path: tmp.path().join("q"),
        cooldown_period_seconds,
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
