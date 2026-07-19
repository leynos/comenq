//! Configuration loading for the Comenqd daemon.
//!
//! The configuration is stored in `/etc/comenqd/config.toml`. Values may be
//! overridden by environment variables using the `COMENQD_` prefix.

use clap::Parser;
use figment::providers::Env;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

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
/// Default capacity for the listener channel buffering client requests.
const DEFAULT_CLIENT_CHANNEL_CAPACITY: usize = 1024;

/// Runtime configuration for the daemon.
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, Clone)]
pub struct Config {
    /// GitHub Personal Access Token.
    pub github_token: String,
    /// Path to the Unix Domain Socket.
    ///
    /// Defaults to `$XDG_RUNTIME_DIR/comenq/comenq.sock` when a user runtime
    /// directory is available, falling back to the system-wide path
    /// otherwise. See [`comenq_lib::default_socket_path`].
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
    /// Capacity of the channel buffering client requests.
    #[serde(default = "default_client_channel_capacity")]
    pub client_channel_capacity: usize,
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
        let test_support::daemon::TestConfig {
            github_token,
            socket_path,
            queue_path,
            cooldown_period_seconds,
            restart_min_delay_ms,
            github_api_timeout_secs,
            client_channel_capacity,
        } = value;
        Self {
            github_token,
            socket_path,
            queue_path,
            cooldown_period_seconds,
            restart_min_delay_ms,
            github_api_timeout_secs,
            client_channel_capacity,
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
            client_channel_capacity: value.client_channel_capacity,
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
    /// Override GitHub API timeout (seconds).
    #[arg(long)]
    github_api_timeout_secs: Option<u64>,
}

fn default_socket_path() -> PathBuf {
    comenq_lib::default_socket_path()
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

fn default_client_channel_capacity() -> usize {
    DEFAULT_CLIENT_CHANNEL_CAPACITY
}

impl Config {
    /// Default location of the daemon configuration file.
    pub const DEFAULT_PATH: &'static str = "/etc/comenqd/config.toml";

    /// Load the configuration using command-line overrides and environment
    /// variables.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use comenqd::config::Config;
    /// let cfg = Config::load().expect("load config");
    /// println!("{}", cfg.socket_path.display());
    /// ```
    #[expect(clippy::result_large_err, reason = "propagate figment errors")]
    pub fn load() -> Result<Self, ortho_config::OrthoError> {
        let args = CliArgs::parse();
        Self::from_file_with_cli(&args.config, &args)
    }

    /// Test-only helper that loads the configuration from the specified path,
    /// merging `COMENQD_*` environment variables and CLI arguments over file
    /// values.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use comenqd::config::Config;
    /// use std::env;
    /// let config_path = env::temp_dir().join("comenqd_config.toml");
    /// let cfg = Config::from_file(&config_path)
    ///     .expect("load config");
    /// ```
    #[cfg(any(test, feature = "test-support"))]
    #[cfg_attr(docsrs, doc(cfg(feature = "test-support")))]
    #[cfg_attr(
        all(not(test), not(feature = "test-support")),
        expect(dead_code, reason = "test-only helper")
    )]
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
        if let Some(secs) = cli.github_api_timeout_secs {
            cfg.github_api_timeout_secs = secs;
        }
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests;
