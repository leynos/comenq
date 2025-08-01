//! Helper utilities for behavioural and unit tests.
//!
//! These functions create temporary daemon configuration objects and
//! Octocrab clients tailored for a `wiremock::MockServer`.

use std::sync::Arc;

use comenqd::config::Config;
use octocrab::Octocrab;
use tempfile::TempDir;
use wiremock::MockServer;

/// Build a temporary [`Config`] using paths under `tmp`.
///
/// # Examples
///
/// ```
/// use tempfile::tempdir;
/// use comenqd::config::Config;
/// use util::test_helpers::temp_config;
///
/// let dir = tempdir().unwrap();
/// let cfg: Config = temp_config(&dir);
/// assert!(cfg.socket_path.ends_with("sock"));
/// ```
pub fn temp_config(tmp: &TempDir) -> Config {
    Config {
        github_token: String::from("t"),
        socket_path: tmp.path().join("sock"),
        queue_path: tmp.path().join("q"),
        cooldown_period_seconds: 1,
    }
}

/// Create an [`Octocrab`] instance configured for `server`.
///
/// # Examples
///
/// ```
/// use wiremock::MockServer;
/// use util::test_helpers::octocrab_for;
///
/// # tokio_test::block_on(async {
/// let server = MockServer::start().await;
/// let octocrab = octocrab_for(&server);
/// assert!(octocrab.base_url().as_str().contains(&server.uri()));
/// # });
/// ```
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
