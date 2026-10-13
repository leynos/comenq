//! Resolution of the GitHub token rotation pool.
//!
//! Reads the configured token files into named tokens; the worker rotates
//! through the pool round-robin when posting comments.

use super::single_token::{
    MAX_GITHUB_TOKEN_FILE_BYTES, TokenFileReadError, TokenPathError, expand_token_path,
    read_token_file,
};
use std::collections::HashMap;
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
///
/// # Examples
///
/// With no token files, the inline token forms a one-entry pool:
///
/// ```rust,ignore
/// let pool = resolve_pool(&[], "inline-token")?;
/// assert_eq!(pool.len(), 1);
/// assert_eq!(pool[0].name, "default");
/// assert_eq!(pool[0].token, "inline-token");
/// # Ok::<(), std::io::Error>(())
/// ```
pub(super) fn resolve_pool(files: &[PathBuf], inline: &str) -> io::Result<Vec<NamedToken>> {
    if files.is_empty() {
        return Ok(vec![NamedToken {
            name: "default".to_owned(),
            token: inline.to_owned(),
        }]);
    }
    let mut paths_by_token: HashMap<String, PathBuf> = HashMap::with_capacity(files.len());
    files.iter().try_fold(
        Vec::with_capacity(files.len()),
        |mut tokens, configured_path| {
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
            if let Some(previous_path) = paths_by_token.get(token) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "token files {} and {} contain the same token",
                        previous_path.display(),
                        file.display()
                    ),
                ));
            }
            paths_by_token.insert(token.to_owned(), file);
            let name = configured_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("token")
                .to_owned();
            tokens.push(NamedToken {
                name,
                token: token.to_owned(),
            });
            Ok(tokens)
        },
    )
}

/// Add token-file path context while preserving read error kinds.
///
/// Oversized files become `InvalidData`; ordinary filesystem errors retain
/// their original kind and include the path in the message.
///
/// # Examples
///
/// A permission failure keeps its kind and identifies the affected file:
///
/// ```rust,ignore
/// let path = std::path::Path::new("/run/secrets/github-token");
/// let error = token_file_io_error(
///     path,
///     TokenFileReadError::Io(io::Error::from(io::ErrorKind::PermissionDenied)),
/// );
/// assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
/// assert!(error.to_string().contains("/run/secrets/github-token"));
/// ```
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

/// Map token-file path expansion failures to useful I/O errors.
///
/// An unset environment variable becomes `NotFound`, while a malformed
/// placeholder becomes `InvalidInput`; both messages retain the useful name.
///
/// # Examples
///
/// An unset path variable is reported as a missing configuration value:
///
/// ```rust,ignore
/// let error = token_path_io_error(TokenPathError::Unset("SECRETS_DIR".into()));
/// assert_eq!(error.kind(), io::ErrorKind::NotFound);
/// assert!(error.to_string().contains("SECRETS_DIR"));
/// ```
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
