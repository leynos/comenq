//! Resolution of the GitHub token rotation pool.
//!
//! Reads the configured token files into named tokens; the worker rotates
//! through the pool round-robin when posting comments.

use super::single_token::{
    MAX_GITHUB_TOKEN_FILE_BYTES, TokenFileReadError, TokenPathError, expand_token_path,
    read_token_file,
};
use std::io;
use std::path::PathBuf;

/// A GitHub token paired with the name of the file it was read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedToken {
    /// The token file's name, or `default` for a single inline token.
    pub name: String,
    /// The token value.
    pub token: String,
}

/// Resolve the rotation pool from `files`, falling back to `inline`.
///
/// Reads each path in `files`, naming the token after its file; each file
/// must hold a non-empty token. When `files` is empty, `inline` forms a
/// pool of one named `default`.
pub(super) fn resolve_pool(files: &[PathBuf], inline: &str) -> io::Result<Vec<NamedToken>> {
    if files.is_empty() {
        return Ok(vec![NamedToken {
            name: "default".to_owned(),
            token: inline.to_owned(),
        }]);
    }
    files
        .iter()
        .map(|configured_path| {
            let file = expand_token_path(configured_path).map_err(token_path_io_error)?;
            let token =
                read_token_file(&file).map_err(|error| token_file_io_error(&file, error))?;
            let token = token.trim();
            if token.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("token file {} is empty", file.display()),
                ));
            }
            let name = configured_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("token")
                .to_owned();
            Ok(NamedToken {
                name,
                token: token.to_owned(),
            })
        })
        .collect()
}

fn token_file_io_error(path: &std::path::Path, error: TokenFileReadError) -> io::Error {
    match error {
        TokenFileReadError::Io(error) => io::Error::new(
            error.kind(),
            format!("token file {}: {error}", path.display()),
        ),
        TokenFileReadError::TooLarge => io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "token file {} exceeds {MAX_GITHUB_TOKEN_FILE_BYTES} bytes",
                path.display()
            ),
        ),
    }
}

fn token_path_io_error(error: TokenPathError) -> io::Error {
    match error {
        TokenPathError::Malformed(text) => io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("unterminated environment placeholder in '{text}'"),
        ),
        TokenPathError::Unset(variable) => io::Error::new(
            io::ErrorKind::NotFound,
            format!("environment variable '{variable}' is not set"),
        ),
    }
}
