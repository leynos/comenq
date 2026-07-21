//! Resolution of the GitHub token rotation pool.
//!
//! Reads the configured token files into named tokens; the worker rotates
//! through the pool round-robin when posting comments.

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
        .map(|file| {
            let token = std::fs::read_to_string(file).map_err(|e| {
                io::Error::new(e.kind(), format!("token file {}: {e}", file.display()))
            })?;
            let token = token.trim();
            if token.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("token file {} is empty", file.display()),
                ));
            }
            let name = file
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
