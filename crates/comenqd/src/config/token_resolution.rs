//! Resolve GitHub credentials into the ordered startup token pool.

use super::single_token::{expand_env_prefix, read_github_token_file};
use super::tokens;
use super::{Config, NamedToken};
use std::io;

impl Config {
    /// Resolve the effective GitHub token, reading `github_token_file` when
    /// set and validating that a non-empty token is available.
    #[expect(clippy::result_large_err, reason = "propagate ortho_config errors")]
    #[tracing::instrument(skip(self))]
    pub(super) fn resolve_github_token(
        &mut self,
        has_cli_token: bool,
    ) -> Result<Vec<NamedToken>, ortho_config::OrthoError> {
        if has_cli_token {
            // An explicit command-line token is the strongest credential
            // override, including over a configured token pool.
            self.github_token_file = None;
            self.github_token_files.clear();
            tracing::debug!(credential_source = "cli", "Using GitHub credential");
        } else if !self.github_token_files.is_empty() {
            return self
                .github_tokens()
                .map_err(|error| ortho_config::OrthoError::Validation {
                    key: "github_token_files".into(),
                    message: error.to_string(),
                });
        } else if let Some(file) = &self.github_token_file {
            let path = expand_env_prefix(file)?;
            tracing::debug!(
                credential_source = "file",
                path = %path.display(),
                "Loading GitHub credential",
            );
            let token = read_github_token_file(&path)?;
            let token = token.trim();
            if token.is_empty() {
                return Err(ortho_config::OrthoError::Validation {
                    key: "github_token_file".into(),
                    message: format!("token file {} is empty", path.display()),
                });
            }
            self.github_token = token.to_owned();
        } else {
            tracing::debug!(
                credential_source = "configuration",
                "Using GitHub credential"
            );
        }
        if self.github_token.trim().is_empty() {
            return Err(ortho_config::OrthoError::Validation {
                key: "github_token".into(),
                message: "provide github_token, github_token_file, or github_token_files".into(),
            });
        }
        self.github_tokens()
            .map_err(|error| ortho_config::OrthoError::Validation {
                key: "github_token".into(),
                message: error.to_string(),
            })
    }

    /// Resolve the ordered rotation pool, falling back to the single token.
    ///
    /// Each configured token file is read and validated at startup. With no
    /// token files, the existing inline or single-file token is a pool of one.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use comenqd::config::Config;
    /// let config = Config::load().expect("load daemon config");
    /// let tokens = config.github_tokens().expect("resolve GitHub tokens");
    /// assert!(!tokens.is_empty());
    /// ```
    pub fn github_tokens(&self) -> io::Result<Vec<NamedToken>> {
        if self.github_token_files.is_empty() && self.github_token.trim().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "GitHub token is empty",
            ));
        }
        tokens::resolve_pool(&self.github_token_files, &self.github_token)
    }
}
