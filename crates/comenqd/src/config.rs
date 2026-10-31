//! Configuration loading for the Comenqd daemon.
//!
//! The configuration is stored in `/etc/comenqd/config.toml`. Values may be
//! overridden by environment variables using the `COMENQD_` prefix.

use clap::Parser;
use mockable::{DefaultEnv, Env};
use ortho_config::OrthoResult;
use ortho_config::figment::Figment;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

mod env_layer;
mod single_token;
mod token_resolution;
mod tokens;
pub use tokens::NamedToken;

use env_layer::{env_overrides, socket_path_defaults};

#[cfg(test)]
use single_token::MAX_GITHUB_TOKEN_FILE_BYTES;

/// Default queue directory when none is provided.
const DEFAULT_QUEUE_PATH: &str = "/var/lib/comenq/queue";
/// Default cooldown in seconds between comment posts.
///
/// The period was increased from 15 to 16 minutes to provide a larger
/// buffer against GitHub's secondary rate limits.
const DEFAULT_COOLDOWN: u64 = 960;
/// Default random flutter added to the cooldown, in seconds (disabled).
const DEFAULT_COOLDOWN_FLUTTER: u64 = 0;
/// Default minimum delay between task restarts in milliseconds.
const DEFAULT_RESTART_MIN_DELAY_MS: u64 = 100;
/// Default timeout in seconds for GitHub API calls.
const DEFAULT_GITHUB_API_TIMEOUT_SECS: u64 = 30;

/// Runtime configuration for the daemon.
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, Clone)]
pub struct Config {
    /// GitHub Personal Access Token.
    ///
    /// May be supplied directly or via [`Config::github_token_file`]; one of
    /// the two must be provided.
    #[serde(default)]
    pub github_token: String,
    /// Optional path to a file containing the GitHub Personal Access Token.
    ///
    /// When set, the file is read at startup and its trimmed contents
    /// override [`Config::github_token`] unless an explicit `--github-token`
    /// argument is supplied. Selecting a file never falls back to the inline
    /// token: an unreadable or empty file is a configuration error. A leading
    /// `${VAR}` placeholder is expanded from the environment, so systemd
    /// credentials work with
    /// `LoadCredential=token:...` and
    /// `github_token_file = "${CREDENTIALS_DIRECTORY}/token"`.
    #[serde(default)]
    pub github_token_file: Option<PathBuf>,
    /// Ordered paths to files containing the GitHub token rotation pool.
    ///
    /// When non-empty, each file must contain one non-empty token and the
    /// worker rotates through them in this order. This setting takes
    /// precedence over the single-token settings.
    #[serde(default)]
    pub github_token_files: Vec<PathBuf>,
    /// Path to the Unix Domain Socket.
    ///
    /// The loader supplies an environment-aware default as its lowest
    /// precedence layer: `$XDG_RUNTIME_DIR/comenq/comenq.sock` when a user
    /// runtime directory is available, falling back to the system-wide path.
    /// An injected environment therefore resolves the default without reading
    /// the process environment. See
    /// [`comenq_transport::default_socket_path_with_env`].
    ///
    /// Direct Serde deserialization uses the deterministic system-wide
    /// [`comenq_transport::DEFAULT_SOCKET_PATH`]; it deliberately does not
    /// consult the process environment.
    #[serde(default = "system_socket_path")]
    pub socket_path: PathBuf,
    /// Directory for the persistent queue.
    #[serde(default = "default_queue_path")]
    pub queue_path: PathBuf,
    /// Cooldown between comment posts in seconds.
    #[serde(default = "default_cooldown")]
    pub cooldown_period_seconds: u64,
    /// Maximum random flutter added to each cooldown, in seconds.
    ///
    /// The daemon samples flutter once when it enqueues an entry and stores
    /// the value with that entry. It only lengthens the cooldown-derived ETA;
    /// zero (the default) disables flutter.
    #[serde(default = "default_cooldown_flutter")]
    pub cooldown_flutter_seconds: u64,
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
        let test_support::daemon::TestConfig {
            github_token,
            socket_path,
            queue_path,
            cooldown_period_seconds,
            restart_min_delay_ms,
            github_api_timeout_secs,
        } = value;
        Self {
            github_token,
            github_token_file: None,
            github_token_files: Vec::new(),
            socket_path,
            queue_path,
            cooldown_period_seconds,
            cooldown_flutter_seconds: DEFAULT_COOLDOWN_FLUTTER,
            restart_min_delay_ms,
            github_api_timeout_secs,
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
            github_token_file: None,
            github_token_files: Vec::new(),
            socket_path: value.socket_path.clone(),
            queue_path: value.queue_path.clone(),
            cooldown_period_seconds: value.cooldown_period_seconds,
            cooldown_flutter_seconds: DEFAULT_COOLDOWN_FLUTTER,
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
    /// Path to a file containing the GitHub Personal Access Token.
    #[arg(long, value_name = "FILE")]
    github_token_file: Option<PathBuf>,
    /// Override the Unix Domain Socket path.
    #[arg(long)]
    socket_path: Option<PathBuf>,
    /// Override the queue directory.
    #[arg(long)]
    queue_path: Option<PathBuf>,
    /// Override the cooldown between comment posts (seconds).
    #[arg(long)]
    cooldown_period_seconds: Option<u64>,
    /// Override GitHub API timeout (seconds).
    #[arg(long)]
    github_api_timeout_secs: Option<u64>,
}

/// Deterministic socket path for direct Serde deserialization.
///
/// The loader replaces this with an environment-aware value; using the
/// process environment here would reintroduce the ambient read that the
/// injected-provider design removes.
fn system_socket_path() -> PathBuf {
    PathBuf::from(comenq_transport::DEFAULT_SOCKET_PATH)
}

fn default_queue_path() -> PathBuf {
    PathBuf::from(DEFAULT_QUEUE_PATH)
}

fn default_cooldown() -> u64 {
    DEFAULT_COOLDOWN
}

fn default_cooldown_flutter() -> u64 {
    DEFAULT_COOLDOWN_FLUTTER
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
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use comenqd::config::Config;
    /// let cfg = Config::load().expect("load config");
    /// println!("{}", cfg.socket_path.display());
    /// ```
    pub fn load() -> OrthoResult<Self> {
        let args = CliArgs::parse();
        Self::from_file_with_cli(&args.config, &args, &DefaultEnv)
    }

    /// Load runtime configuration and resolve its GitHub token pool once.
    ///
    /// The daemon entry point passes the returned tokens to the supervisor so
    /// startup validation and client construction use the same file contents.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use comenqd::config::Config;
    /// let (config, tokens) = Config::load_with_tokens().expect("load daemon config");
    /// assert!(!tokens.is_empty());
    /// println!("{}", config.socket_path.display());
    /// ```
    pub fn load_with_tokens() -> OrthoResult<(Self, Vec<NamedToken>)> {
        let args = CliArgs::parse();
        Self::from_file_with_cli_and_tokens(&args.config, &args, &DefaultEnv)
    }

    /// Test-only helper that loads the configuration from the specified path,
    /// merging ambient `COMENQD_*` environment variables and CLI arguments over
    /// file values.
    ///
    /// Prefer [`Config::from_file_with_env`] in tests so environment values
    /// are injected rather than mutated process-wide.
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
    pub fn from_file(path: &Path) -> OrthoResult<Self> {
        Self::from_file_with_env(path, &DefaultEnv)
    }

    /// Test-only helper that loads the configuration from `path` using the
    /// environment values supplied by `env`.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use comenqd::config::Config;
    /// use mockable::DefaultEnv;
    /// use std::env;
    /// let config_path = env::temp_dir().join("comenqd_config.toml");
    /// let cfg = Config::from_file_with_env(&config_path, &DefaultEnv)
    ///     .expect("load config");
    /// ```
    #[cfg(any(test, feature = "test-support"))]
    #[cfg_attr(docsrs, doc(cfg(feature = "test-support")))]
    pub fn from_file_with_env(path: &Path, env: &dyn Env) -> OrthoResult<Self> {
        Self::from_file_with_cli(path, &CliArgs::default(), env)
    }

    fn from_file_with_cli(path: &Path, cli: &CliArgs, env: &dyn Env) -> OrthoResult<Self> {
        Self::from_file_with_cli_and_tokens(path, cli, env).map(|(config, _)| config)
    }

    fn from_file_with_cli_and_tokens(
        path: &Path,
        cli: &CliArgs,
        env: &dyn Env,
    ) -> OrthoResult<(Self, Vec<NamedToken>)> {
        let file = ortho_config::load_config_file(path)?.ok_or_else(|| {
            Arc::new(ortho_config::OrthoError::File {
                path: path.to_path_buf(),
                source: Box::new(io::Error::new(
                    io::ErrorKind::NotFound,
                    "Configuration file not found",
                )),
            })
        })?;

        // Later merges win, so the order is default socket, file, environment.
        let fig = Figment::from(socket_path_defaults(env))
            .merge(file)
            .merge(env_overrides(env));
        let mut cfg: Self = fig.extract().map_err(ortho_config::OrthoError::from)?;

        Self::apply_cli_overrides(&mut cfg, cli);
        let tokens = cfg.resolve_github_token(cli.github_token.is_some(), env)?;
        Ok((cfg, tokens))
    }

    /// Apply command-line values that explicitly override loaded configuration.
    fn apply_cli_overrides(cfg: &mut Self, cli: &CliArgs) {
        if let Some(token) = &cli.github_token {
            cfg.github_token = token.clone();
        }
        if let Some(file) = &cli.github_token_file {
            cfg.github_token_file = Some(file.clone());
            cfg.github_token_files.clear();
        }
        if let Some(socket) = &cli.socket_path {
            cfg.socket_path = socket.clone();
        }
        if let Some(queue) = &cli.queue_path {
            cfg.queue_path = queue.clone();
        }
        if let Some(secs) = cli.cooldown_period_seconds {
            cfg.cooldown_period_seconds = secs;
        }
        if let Some(secs) = cli.github_api_timeout_secs {
            cfg.github_api_timeout_secs = secs;
        }
    }
}

#[cfg(test)]
mod tests;
