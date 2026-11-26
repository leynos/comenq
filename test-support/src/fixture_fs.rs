//! Capability-scoped filesystem helpers for staging and inspecting fixtures.
//!
//! Tests stage files at paths they have just created, usually inside a
//! `TempDir`. These helpers open the file's parent directory as a
//! `cap_std::fs::Dir` handle and perform the operation through it, so test
//! code needs no ambient `std::fs` calls. The ambient authority is requested
//! here, once, rather than scattered through the tests.

use std::{
    io,
    path::{Path, PathBuf},
};

use cap_std::{
    ambient_authority,
    fs::{Dir, PermissionsExt as _},
};

/// Split `path` into the directory that holds it and its file name.
fn parent_and_name(path: &Path) -> io::Result<(PathBuf, PathBuf)> {
    let name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} has no file name", path.display()),
        )
    })?;
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    Ok((parent, PathBuf::from(name)))
}

/// Open the directory that holds `path` and return it with the file name.
fn open_parent(path: &Path) -> io::Result<(Dir, PathBuf)> {
    let (parent, name) = parent_and_name(path)?;
    Ok((Dir::open_ambient_dir(parent, ambient_authority())?, name))
}

/// Write `contents` to the file at `path`, creating or truncating it.
///
/// # Errors
///
/// Returns an error if the parent directory cannot be opened or the file
/// cannot be written.
pub fn write_file(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    let (dir, name) = open_parent(path)?;
    dir.write(name, contents)
}

/// Read the whole file at `path` into a byte vector.
///
/// # Errors
///
/// Returns an error if the parent directory or the file cannot be opened.
pub fn read_file(path: &Path) -> io::Result<Vec<u8>> {
    let (dir, name) = open_parent(path)?;
    dir.read(name)
}

/// List the paths of the entries directly inside the directory `dir`.
///
/// # Errors
///
/// Returns an error if the directory cannot be opened or read.
pub fn list_dir(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let handle = Dir::open_ambient_dir(dir, ambient_authority())?;
    handle
        .entries()?
        .map(|entry| entry.map(|entry| dir.join(entry.file_name())))
        .collect()
}

/// Return the permission bits (`mode & 0o777`) of the file at `path`.
///
/// # Errors
///
/// Returns an error if the file's metadata cannot be read.
pub fn permission_bits(path: &Path) -> io::Result<u32> {
    let (dir, name) = open_parent(path)?;
    Ok(dir.metadata(name)?.permissions().mode() & 0o777)
}

#[cfg(test)]
mod tests {
    //! Unit tests for the fixture filesystem helpers.

    use rstest::rstest;
    use tempfile::TempDir;

    use super::*;

    #[rstest]
    fn written_file_reads_back() {
        let dir = TempDir::new().expect("temporary directory");
        let path = dir.path().join("file.txt");

        write_file(&path, "hello").expect("write file");

        assert_eq!(
            read_file(&path).expect("read file"),
            b"hello",
            "the file should hold what was written"
        );
    }

    #[rstest]
    fn listing_returns_entry_paths() {
        let dir = TempDir::new().expect("temporary directory");
        let path = dir.path().join("entry");
        write_file(&path, b"").expect("write file");

        let entries = list_dir(dir.path()).expect("list directory");

        assert_eq!(entries, vec![path], "the one entry should be listed");
    }

    #[rstest]
    fn permission_bits_report_the_mode() {
        let dir = TempDir::new().expect("temporary directory");
        let path = dir.path().join("file");
        write_file(&path, b"").expect("write file");

        let bits = permission_bits(&path).expect("read permissions");

        assert_ne!(bits, 0, "a freshly created file has some permission bits");
    }

    #[rstest]
    fn a_path_without_a_file_name_is_rejected() {
        let error = write_file(Path::new("/"), b"x").expect_err("root has no file name");

        assert_eq!(
            error.kind(),
            io::ErrorKind::InvalidInput,
            "a path with no file name is invalid input"
        );
    }
}
