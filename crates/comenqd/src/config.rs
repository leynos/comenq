//! Configuration loading for the Comenqd daemon.
//!
//! The configuration is stored in `/etc/comenqd/config.toml`. Values may be
//! overridden by environment variables using the `COMENQD_` prefix.

use clap::Parser;
use figment::providers::Env;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

/// Runtime configuration for the daemon.
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Config {
    /// GitHub Personal Access Token.
    pub github_token: String,
    /// Path to the Unix Domain Socket.
    #[serde(default = "default_socket_path")]
    pub socket_path: PathBuf,
    /// Directory for the persistent queue.
    #[serde(default = "default_queue_path")]
    pub queue_path: PathBuf,
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
    PathBuf::from("/run/comenq/comenq.sock")
}

fn default_queue_path() -> PathBuf {
    PathBuf::from("/var/lib/comenq/queue")
}

impl Config {
    /// Default location of the daemon configuration file.
    pub const DEFAULT_PATH: &'static str = "/etc/comenqd/config.toml";

    /// Load the configuration using command-line overrides and environment
    /// variables.
    #[allow(clippy::result_large_err)]
    pub fn load() -> Result<Self, ortho_config::OrthoError> {
        let args = CliArgs::parse();
        Self::from_file_with_cli(&args.config, &args)
    }

    /// Load the configuration from the specified path, merging `COMENQD_*`
    /// environment variables and CLI arguments over file values.
    #[allow(clippy::result_large_err)]
    pub fn from_file(path: &Path) -> Result<Self, ortho_config::OrthoError> {
        Self::from_file_with_cli(path, &CliArgs::default())
    }

    #[allow(clippy::result_large_err)]
    fn from_file_with_cli(path: &Path, cli: &CliArgs) -> Result<Self, ortho_config::OrthoError> {
        let mut fig = ortho_config::load_config_file(path)?.ok_or_else(|| {
            ortho_config::OrthoError::File {
                path: path.to_path_buf(),
                source: Box::new(io::Error::from(io::ErrorKind::NotFound)),
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
        unsafe {
            std::env::remove_var("COMENQD_SOCKET_PATH");
        }
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
        unsafe {
            std::env::set_var("COMENQD_SOCKET_PATH", "/tmp/override.sock");
        }
        let cfg = Config::from_file(&path).unwrap();
        unsafe {
            std::env::remove_var("COMENQD_SOCKET_PATH");
        }
        assert_eq!(cfg.socket_path, PathBuf::from("/tmp/override.sock"));
    }
}
