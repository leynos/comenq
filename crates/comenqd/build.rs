//! Stages the packaged daemon man page for installation.
use std::{env, fs, path::PathBuf};

fn main() {
    if let Err(error) = copy_man_page() {
        panic!("failed to stage man page: {error}");
    }
}

/// Describes why staging the man page failed.
#[derive(Debug)]
enum StageError {
    /// A required Cargo environment variable was missing or invalid.
    Env(&'static str, std::env::VarError),
    /// Copying the man page into `OUT_DIR` failed.
    Io(std::io::Error),
}

impl std::fmt::Display for StageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Env(name, source) => write!(f, "environment variable {name}: {source}"),
            Self::Io(source) => write!(f, "copy failed: {source}"),
        }
    }
}

impl From<std::io::Error> for StageError {
    fn from(source: std::io::Error) -> Self {
        Self::Io(source)
    }
}

/// Reads a Cargo-provided environment variable, reporting which one failed.
fn cargo_env(name: &'static str) -> Result<String, StageError> {
    env::var(name).map_err(|source| StageError::Env(name, source))
}

fn copy_man_page() -> Result<(), StageError> {
    let manifest_dir = PathBuf::from(cargo_env("CARGO_MANIFEST_DIR")?);
    let source = manifest_dir.join("../../packaging/man/comenqd.1");
    let out_dir = PathBuf::from(cargo_env("OUT_DIR")?);
    let dest = out_dir.join("comenqd.1");
    fs::copy(&source, &dest)?;
    println!("cargo:rerun-if-changed={}", source.display());
    Ok(())
}
