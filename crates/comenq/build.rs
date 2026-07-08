//! Stages the packaged client man page for installation.
use std::{env, fs, path::PathBuf};

fn main() {
    if let Err(error) = copy_man_page() {
        panic!("failed to stage man page: {error}");
    }
}

/// Describes why staging the man page failed.
#[derive(Debug, thiserror::Error)]
enum StageError {
    /// A required Cargo environment variable was missing or invalid.
    #[error("environment variable {0}: {1}")]
    Env(&'static str, std::env::VarError),
    /// Copying the man page into `OUT_DIR` failed.
    #[error("copy failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Reads a Cargo-provided environment variable, reporting which one failed.
fn cargo_env(name: &'static str) -> Result<String, StageError> {
    env::var(name).map_err(|source| StageError::Env(name, source))
}

fn copy_man_page() -> Result<(), StageError> {
    let manifest_dir = PathBuf::from(cargo_env("CARGO_MANIFEST_DIR")?);
    let source = manifest_dir.join("../../packaging/man/comenq.1");
    let out_dir = PathBuf::from(cargo_env("OUT_DIR")?);
    let dest = out_dir.join("comenq.1");
    fs::copy(&source, &dest)?;
    println!("cargo:rerun-if-changed={}", source.display());
    Ok(())
}
