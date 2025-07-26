//! Configuration loading for the Comenqd daemon.
//!
//! The configuration is stored in `/etc/comenqd/config.toml`. Values may be
//! overridden by environment variables using the `COMENQD_` prefix.

use figment::providers::Env;
use serde::Deserialize;
use std::io;
use std::path::{Path, PathBuf};

/// Runtime configuration for the daemon.
#[derive(Debug, Deserialize, PartialEq, Eq)]
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

fn default_socket_path() -> PathBuf {
    PathBuf::from("/run/comenq/comenq.sock")
}

fn default_queue_path() -> PathBuf {
    PathBuf::from("/var/lib/comenq/queue")
}

impl Config {
    /// Default location of the daemon configuration file.
    pub const DEFAULT_PATH: &'static str = "/etc/comenqd/config.toml";

    /// Load the configuration from `DEFAULT_PATH`.
    #[allow(clippy::result_large_err)]
    pub fn load() -> Result<Self, ortho_config::OrthoError> {
        Self::from_file(Path::new(Self::DEFAULT_PATH))
    }

    /// Load the configuration from the specified path, merging `COMENQD_*`
    /// environment variables over file values.
    #[allow(clippy::result_large_err)]
    pub fn from_file(path: &Path) -> Result<Self, ortho_config::OrthoError> {
        let mut fig = ortho_config::load_config_file(path)?.ok_or_else(|| {
            ortho_config::OrthoError::File {
                path: path.to_path_buf(),
                source: Box::new(io::Error::from(io::ErrorKind::NotFound)),
            }
        })?;
        fig = fig.merge(Env::prefixed("COMENQD_").split("__"));
        fig.extract().map_err(ortho_config::OrthoError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use std::fs;
    use tempfile::tempdir;

    #[rstest]
    fn loads_from_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            "github_token='abc'\nsocket_path='/tmp/s.sock'\nqueue_path='/tmp/q'",
        )
        .unwrap();
        let cfg = Config::from_file(&path).unwrap();
        assert_eq!(cfg.github_token, "abc");
        assert_eq!(cfg.socket_path, PathBuf::from("/tmp/s.sock"));
        assert_eq!(cfg.queue_path, PathBuf::from("/tmp/q"));
    }

    #[rstest]
    fn error_when_missing_file() {
        let path = PathBuf::from("/nonexistent/file.toml");
        let res = Config::from_file(&path);
        assert!(res.is_err());
    }
}
