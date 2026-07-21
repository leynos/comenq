//! Resolution of the GitHub token rotation pool.
//!
//! Reads the configured token files into named tokens; the worker rotates
//! through the pool round-robin when posting comments.

use std::io::{self, Read as _};
use std::path::{Path, PathBuf};

/// Keep token-file reads bounded, matching the single-token credential path.
const MAX_TOKEN_FILE_BYTES: usize = 64 * 1024;

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
            let file = expand_env_prefix(configured_path)?;
            let token = read_token_file(&file)?;
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

fn read_token_file(path: &Path) -> io::Result<String> {
    let mut contents = String::with_capacity(MAX_TOKEN_FILE_BYTES);
    std::fs::File::open(path)
        .and_then(|file| {
            file.take((MAX_TOKEN_FILE_BYTES + 1) as u64)
                .read_to_string(&mut contents)
                .map(|_| ())
        })
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("token file {}: {error}", path.display()),
            )
        })?;
    if contents.len() > MAX_TOKEN_FILE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "token file {} exceeds {MAX_TOKEN_FILE_BYTES} bytes",
                path.display()
            ),
        ));
    }
    Ok(contents)
}

fn expand_env_prefix(path: &Path) -> io::Result<PathBuf> {
    let Some(text) = path.to_str() else {
        return Ok(path.to_path_buf());
    };
    let Some(rest) = text.strip_prefix("${") else {
        return Ok(path.to_path_buf());
    };
    let Some((variable, tail)) = rest.split_once('}') else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("unterminated environment placeholder in '{text}'"),
        ));
    };
    let value = std::env::var(variable).map_err(|_| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("environment variable '{variable}' is not set"),
        )
    })?;
    Ok(PathBuf::from(format!("{value}{tail}")))
}
