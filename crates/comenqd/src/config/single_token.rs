//! Loading and path expansion for the legacy single-token credential.

use std::io::Read as _;
use std::path::{Path, PathBuf};

/// Largest credential file accepted at daemon startup, in bytes.
pub(super) const MAX_GITHUB_TOKEN_FILE_BYTES: usize = 64 * 1024;

/// Read a GitHub credential without allowing an unbounded startup allocation.
#[expect(clippy::result_large_err, reason = "propagate ortho_config errors")]
pub(super) fn read_github_token_file(path: &Path) -> Result<String, ortho_config::OrthoError> {
    let file = std::fs::File::open(path).map_err(|source| ortho_config::OrthoError::File {
        path: path.to_path_buf(),
        source: Box::new(source),
    })?;
    let mut contents = String::with_capacity(MAX_GITHUB_TOKEN_FILE_BYTES);
    file.take((MAX_GITHUB_TOKEN_FILE_BYTES + 1) as u64)
        .read_to_string(&mut contents)
        .map_err(|source| ortho_config::OrthoError::File {
            path: path.to_path_buf(),
            source: Box::new(source),
        })?;
    if contents.len() > MAX_GITHUB_TOKEN_FILE_BYTES {
        return Err(ortho_config::OrthoError::Validation {
            key: "github_token_file".into(),
            message: format!(
                "token file {} exceeds {MAX_GITHUB_TOKEN_FILE_BYTES} bytes",
                path.display()
            ),
        });
    }
    Ok(contents)
}

/// Expand a leading `${VAR}` placeholder in `path` from the environment.
///
/// Only a placeholder at the very start of the path is expanded; paths
/// without one are returned unchanged. An unset variable or a malformed
/// placeholder is an error so misconfiguration fails loudly at startup.
#[expect(clippy::result_large_err, reason = "propagate ortho_config errors")]
pub(super) fn expand_env_prefix(path: &Path) -> Result<PathBuf, ortho_config::OrthoError> {
    let Some(text) = path.to_str() else {
        return Ok(path.to_path_buf());
    };
    let Some(rest) = text.strip_prefix("${") else {
        return Ok(path.to_path_buf());
    };
    let Some((var, tail)) = rest.split_once('}') else {
        return Err(ortho_config::OrthoError::Validation {
            key: "github_token_file".into(),
            message: format!("unterminated environment placeholder in '{text}'"),
        });
    };
    let value = std::env::var(var).map_err(|_| ortho_config::OrthoError::Validation {
        key: "github_token_file".into(),
        message: format!("environment variable '{var}' is not set"),
    })?;
    Ok(PathBuf::from(format!("{value}{tail}")))
}
