//! Shared token-file reading and path expansion for GitHub credentials.

use std::io;
use std::io::Read as _;
use std::path::{Path, PathBuf};

/// Largest credential file accepted at daemon startup, in bytes.
pub(super) const MAX_GITHUB_TOKEN_FILE_BYTES: usize = 64 * 1024;

/// Failure while reading a bounded token file.
pub(super) enum TokenFileReadError {
    /// Opening or decoding the file failed.
    Io(io::Error),
    /// The file exceeded the shared startup size limit.
    TooLarge,
}

/// Failure while expanding a leading environment-variable path.
pub(super) enum TokenPathError {
    /// The placeholder did not have a closing brace.
    Malformed(String),
    /// The named environment variable was not set.
    Unset(String),
}

/// Read one UTF-8 token file while limiting the amount of input retained.
pub(super) fn read_token_file(path: &Path) -> Result<String, TokenFileReadError> {
    let mut contents = String::with_capacity(MAX_GITHUB_TOKEN_FILE_BYTES);
    std::fs::File::open(path)
        .and_then(|file| {
            file.take((MAX_GITHUB_TOKEN_FILE_BYTES + 1) as u64)
                .read_to_string(&mut contents)
                .map(|_| ())
        })
        .map_err(TokenFileReadError::Io)?;
    if contents.len() > MAX_GITHUB_TOKEN_FILE_BYTES {
        return Err(TokenFileReadError::TooLarge);
    }
    Ok(contents)
}

/// Expand a leading `${VAR}` placeholder in a token-file path.
pub(super) fn expand_token_path(path: &Path) -> Result<PathBuf, TokenPathError> {
    let Some(text) = path.to_str() else {
        return Ok(path.to_path_buf());
    };
    let Some(rest) = text.strip_prefix("${") else {
        return Ok(path.to_path_buf());
    };
    let Some((variable, tail)) = rest.split_once('}') else {
        return Err(TokenPathError::Malformed(text.to_owned()));
    };
    let value = std::env::var(variable).map_err(|_| TokenPathError::Unset(variable.to_owned()))?;
    Ok(PathBuf::from(format!("{value}{tail}")))
}

/// Read a GitHub credential without allowing an unbounded startup allocation.
#[expect(clippy::result_large_err, reason = "propagate ortho_config errors")]
pub(super) fn read_github_token_file(path: &Path) -> Result<String, ortho_config::OrthoError> {
    match read_token_file(path) {
        Ok(contents) => Ok(contents),
        Err(TokenFileReadError::Io(source)) => Err(ortho_config::OrthoError::File {
            path: path.to_path_buf(),
            source: Box::new(source),
        }),
        Err(TokenFileReadError::TooLarge) => Err(ortho_config::OrthoError::Validation {
            key: "github_token_file".into(),
            message: format!(
                "token file {} exceeds {MAX_GITHUB_TOKEN_FILE_BYTES} bytes",
                path.display()
            ),
        }),
    }
}

/// Expand a leading `${VAR}` placeholder in `path` from the environment.
///
/// Only a placeholder at the very start of the path is expanded; paths
/// without one are returned unchanged. An unset variable or a malformed
/// placeholder is an error so misconfiguration fails loudly at startup.
#[expect(clippy::result_large_err, reason = "propagate ortho_config errors")]
pub(super) fn expand_env_prefix(path: &Path) -> Result<PathBuf, ortho_config::OrthoError> {
    expand_token_path(path).map_err(|error| {
        let message = match error {
            TokenPathError::Malformed(text) => {
                format!("unterminated environment placeholder in '{text}'")
            }
            TokenPathError::Unset(variable) => {
                format!("environment variable '{variable}' is not set")
            }
        };
        ortho_config::OrthoError::Validation {
            key: "github_token_file".into(),
            message,
        }
    })
}
