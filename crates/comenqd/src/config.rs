//! Configuration loading for the Comenqd daemon.
//!
//! The configuration is stored in `/etc/comenqd/config.toml`. Values may be
//! overridden by environment variables using the `COMENQD_` prefix.

use clap::Parser;
use figment::providers::Env;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

/// Default socket path when none is provided.
const DEFAULT_SOCKET_PATH: &str = "/run/comenq/comenq.sock";
/// Default queue directory when none is provided.
const DEFAULT_QUEUE_PATH: &str = "/var/lib/comenq/queue";
/// Default cooldown in seconds between comment posts.
///
/// The period was increased from 15 to 16 minutes to provide a larger
/// buffer against GitHub's secondary rate limits.
const DEFAULT_COOLDOWN: u64 = 960;
/// Default minimum delay between task restarts in milliseconds.
const DEFAULT_RESTART_MIN_DELAY_MS: u64 = 100;
/// Default timeout in seconds for GitHub API calls.
const DEFAULT_GITHUB_API_TIMEOUT_SECS: u64 = 30;

/// Runtime configuration for the daemon.
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, Clone)]
pub struct Config {
    /// GitHub Personal Access Token.
    pub github_token: String,
    /// Path to the Unix Domain Socket.
    #[serde(default = "default_socket_path")]
    pub socket_path: PathBuf,
    /// Directory for the persistent queue.
    #[serde(default = "default_queue_path")]
    pub queue_path: PathBuf,
    /// Cooldown between comment posts in seconds.
    #[serde(default = "default_cooldown")]
    pub cooldown_period_seconds: u64,
    /// Minimum delay in milliseconds applied between task restarts.
    #[serde(default = "default_restart_min_delay_ms")]
    pub restart_min_delay_ms: u64,
    /// Timeout applied to GitHub API requests in seconds.
    #[serde(default = "default_github_api_timeout_secs")]
    pub github_api_timeout_secs: u64,
}

/// Convert a [`test_support::daemon::TestConfig`] into a [`Config`].
///
/// This implementation is only available when the `test-support` feature is
/// enabled and allows `TestConfig` to be converted via `Config::from(test_cfg)`
/// or `test_cfg.into()`.
///
/// ```rust,no_run
/// use comenqd::config::Config;
/// use test_support::temp_config;
///
/// let tmp = tempfile::tempdir().expect("create tempdir");
/// let cfg: Config = temp_config(&tmp).into();
/// assert_eq!(cfg.cooldown_period_seconds, 1);
/// ```
#[cfg(any(test, feature = "test-support"))]
#[cfg_attr(docsrs, doc(cfg(feature = "test-support")))]
impl From<test_support::daemon::TestConfig> for Config {
    fn from(value: test_support::daemon::TestConfig) -> Self {
        Self {
            github_token: value.github_token,
            socket_path: value.socket_path,
            queue_path: value.queue_path,
            cooldown_period_seconds: value.cooldown_period_seconds,
            restart_min_delay_ms: value.restart_min_delay_ms,
            github_api_timeout_secs: value.github_api_timeout_secs,
        }
    }
}

/// Convert a borrowed [`test_support::daemon::TestConfig`] into a [`Config`].
///
/// Requires the `test-support` feature and clones only the owned fields,
/// avoiding an intermediate allocation of a full
/// [`test_support::daemon::TestConfig`] clone.
///
/// # Examples
///
/// ```rust,no_run
/// use comenqd::config::Config;
/// use tempfile::tempdir;
/// use test_support::temp_config;
///
/// let tmp = tempdir().expect("create temp directory");
/// let test_cfg = temp_config(&tmp);
///
/// // Convert by reference without cloning.
/// let cfg = Config::from(&test_cfg);
///
/// assert_eq!(cfg.socket_path, test_cfg.socket_path);
/// assert_eq!(cfg.queue_path, test_cfg.queue_path);
/// ```
#[cfg(any(test, feature = "test-support"))]
#[cfg_attr(docsrs, doc(cfg(feature = "test-support")))]
impl From<&test_support::daemon::TestConfig> for Config {
    fn from(value: &test_support::daemon::TestConfig) -> Self {
        Self {
            github_token: value.github_token.clone(),
            socket_path: value.socket_path.clone(),
            queue_path: value.queue_path.clone(),
            cooldown_period_seconds: value.cooldown_period_seconds,
            restart_min_delay_ms: value.restart_min_delay_ms,
            github_api_timeout_secs: value.github_api_timeout_secs,
        }
    }
}

/// Command-line overrides for configuration values.
#[derive(Debug, Default, Parser, Serialize)]
struct CliArgs {
    /// Path to the configuration file.
    #[arg(short, long, value_name = "FILE", default_value = Config::DEFAULT_PATH)]
    config: PathBuf,
    /// GitHub Personal Access Token.
    #[arg(long)]
    github_token: Option<String>,
    /// Override the Unix Domain Socket path.
    #[arg(long)]
    socket_path: Option<PathBuf>,
    /// Override the queue directory.
    #[arg(long)]
    queue_path: Option<PathBuf>,
}

fn default_socket_path() -> PathBuf {
    PathBuf::from(DEFAULT_SOCKET_PATH)
}

fn default_queue_path() -> PathBuf {
    PathBuf::from(DEFAULT_QUEUE_PATH)
}

fn default_cooldown() -> u64 {
    DEFAULT_COOLDOWN
}

fn default_restart_min_delay_ms() -> u64 {
    DEFAULT_RESTART_MIN_DELAY_MS
}

fn default_github_api_timeout_secs() -> u64 {
    DEFAULT_GITHUB_API_TIMEOUT_SECS
}

impl Config {
    /// Default location of the daemon configuration file.
    pub const DEFAULT_PATH: &'static str = "/etc/comenqd/config.toml";

    /// Load the configuration using command-line overrides and environment
    /// variables.
    #[expect(clippy::result_large_err, reason = "propagate figment errors")]
    pub fn load() -> Result<Self, ortho_config::OrthoError> {
        let args = CliArgs::parse();
        Self::from_file_with_cli(&args.config, &args)
    }

    /// Test-only helper that loads the configuration from the specified path,
    /// merging `COMENQD_*` environment variables and CLI arguments over file
    /// values.
    #[cfg(any(test, feature = "test-support"))]
    #[cfg_attr(docsrs, doc(cfg(feature = "test-support")))]
    #[cfg_attr(not(test), allow(dead_code, reason = "test-only helper"))]
    #[expect(clippy::result_large_err, reason = "propagate figment errors")]
    pub fn from_file(path: &Path) -> Result<Self, ortho_config::OrthoError> {
        Self::from_file_with_cli(path, &CliArgs::default())
    }

    #[expect(clippy::result_large_err, reason = "propagate figment errors")]
    fn from_file_with_cli(path: &Path, cli: &CliArgs) -> Result<Self, ortho_config::OrthoError> {
        let mut fig = ortho_config::load_config_file(path)?.ok_or_else(|| {
            ortho_config::OrthoError::File {
                path: path.to_path_buf(),
                source: Box::new(io::Error::new(
                    io::ErrorKind::NotFound,
                    "Configuration file not found",
                )),
            }
        })?;

        fig = fig.merge(Env::prefixed("COMENQD_").split("__"));
        let mut cfg: Self = fig.extract().map_err(ortho_config::OrthoError::from)?;

        if let Some(token) = &cli.github_token {
            cfg.github_token = token.clone();
        }
        if let Some(socket) = &cli.socket_path {
            cfg.socket_path = socket.clone();
        }
        if let Some(queue) = &cli.queue_path {
            cfg.queue_path = queue.clone();
        }
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use std::fs;
    use tempfile::tempdir;

    use test_support::env_guard::{EnvVarGuard, remove_env_var};

    #[rstest]
    #[serial_test::serial]
    fn loads_from_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            "github_token='abc'\nsocket_path='/tmp/s.sock'\nqueue_path='/tmp/q'",
        )
        .unwrap();
        remove_env_var("COMENQD_SOCKET_PATH");
        let cfg = Config::from_file(&path).unwrap();
        assert_eq!(cfg.github_token, "abc");
        assert_eq!(cfg.socket_path, PathBuf::from("/tmp/s.sock"));
        assert_eq!(cfg.queue_path, PathBuf::from("/tmp/q"));
    }

    #[rstest]
    #[serial_test::serial]
    fn error_when_missing_file() {
        let path = PathBuf::from("/nonexistent/file.toml");
        let res = Config::from_file(&path);
        assert!(res.is_err());
    }

    #[rstest]
    #[serial_test::serial]
    fn env_vars_override_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "github_token='abc'\nsocket_path='/tmp/s.sock'").unwrap();
        let _guard = EnvVarGuard::set("COMENQD_SOCKET_PATH", "/tmp/override.sock");
        let cfg = Config::from_file(&path).unwrap();
        assert_eq!(cfg.socket_path, PathBuf::from("/tmp/override.sock"));
    }

    #[rstest]
    #[serial_test::serial]
    fn error_with_invalid_toml() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "github_token='abc' this is not toml").unwrap();
        let res = Config::from_file(&path);
        assert!(res.is_err());
    }

    #[rstest]
    #[serial_test::serial]
    fn error_when_missing_token() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "socket_path='/tmp/s.sock'").unwrap();
        let res = Config::from_file(&path);
        assert!(res.is_err());
    }

    #[rstest]
    #[serial_test::serial]
    fn defaults_are_applied() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "github_token='abc'").unwrap();
        let cfg = Config::from_file(&path).unwrap();
        assert_eq!(cfg.socket_path, PathBuf::from("/run/comenq/comenq.sock"));
        assert_eq!(cfg.queue_path, PathBuf::from("/var/lib/comenq/queue"));
        assert_eq!(cfg.cooldown_period_seconds, DEFAULT_COOLDOWN);
        assert_eq!(cfg.restart_min_delay_ms, DEFAULT_RESTART_MIN_DELAY_MS);
        assert_eq!(cfg.github_api_timeout_secs, DEFAULT_GITHUB_API_TIMEOUT_SECS);
    }

    /// CLI arguments should take precedence over environment variables
    /// and configuration file values when building the daemon `Config`.
    #[rstest]
    #[serial_test::serial]
    fn cli_overrides_env_and_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "github_token='abc'\nsocket_path='/tmp/file.sock'").unwrap();
        let _guard = EnvVarGuard::set("COMENQD_SOCKET_PATH", "/tmp/env.sock");
        let cli = CliArgs {
            config: path.clone(),
            github_token: None,
            socket_path: Some(PathBuf::from("/tmp/cli.sock")),
            queue_path: None,
        };
        let cfg = Config::from_file_with_cli(&path, &cli).unwrap();
        assert_eq!(cfg.socket_path, PathBuf::from("/tmp/cli.sock"));
    }

    #[cfg(feature = "test-support")]
    #[rstest]
    #[case(|cfg: &test_support::daemon::TestConfig| Config::from(cfg))]
    #[case(|cfg: &test_support::daemon::TestConfig| Config::from(cfg.clone()))]
    #[serial_test::serial]
    fn converts_from_test_config(#[case] conv: fn(&test_support::daemon::TestConfig) -> Config) {
        use test_support::temp_config;

        let tmp = tempdir().expect("create tempdir");
        let test_cfg = temp_config(&tmp).with_cooldown(42);
        let cfg = conv(&test_cfg);

        assert_eq!(cfg.github_token, test_cfg.github_token);
        assert_eq!(cfg.socket_path, test_cfg.socket_path);
        assert_eq!(cfg.queue_path, test_cfg.queue_path);
        assert_eq!(
            cfg.cooldown_period_seconds,
            test_cfg.cooldown_period_seconds
        );
        assert_eq!(cfg.restart_min_delay_ms, test_cfg.restart_min_delay_ms);
        assert_eq!(
            cfg.github_api_timeout_secs,
            test_cfg.github_api_timeout_secs
        );
    }
}
