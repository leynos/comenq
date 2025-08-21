//! Unix `utimensat` backend for setting timestamps on paths, handles, and
//! symlinks.
//!
//! Functions here are crate-internal helpers invoked by the public API in
//! `lib.rs`. Paths must not contain interior NUL bytes. On emscripten, symlink
//! updates are not supported and return an error.

use crate::FileTime;
use std::ffi::CString;
use std::fs::File;
use std::io;
use std::os::unix::prelude::*;
use std::path::Path;

/// Set both access and modification times for a file at `p`.
///
/// # Errors
/// Returns an error if `p` contains interior NUL bytes or if the underlying
/// `utimensat` call fails.
pub(crate) fn set_file_times(p: &Path, atime: FileTime, mtime: FileTime) -> io::Result<()> {
    set_times(p, Some(atime), Some(mtime), false)
}

/// Set only the modification time for a file at `p`.
///
/// # Errors
/// Returns an error if `p` contains interior NUL bytes or if the underlying
/// `utimensat` call fails.
pub(crate) fn set_file_mtime(p: &Path, mtime: FileTime) -> io::Result<()> {
    set_times(p, None, Some(mtime), false)
}

/// Set only the access time for a file at `p`.
///
/// # Errors
/// Returns an error if `p` contains interior NUL bytes or if the underlying
/// `utimensat` call fails.
pub(crate) fn set_file_atime(p: &Path, atime: FileTime) -> io::Result<()> {
    set_times(p, Some(atime), None, false)
}

/// Set times for an open file handle.
///
/// # Errors
/// Propagates errors from the underlying `futimens` call.
pub(crate) fn set_file_handle_times(
    f: &File,
    atime: Option<FileTime>,
    mtime: Option<FileTime>,
) -> io::Result<()> {
    let times = [super::to_timespec(&atime), super::to_timespec(&mtime)];
    let rc = unsafe { libc::futimens(f.as_raw_fd(), times.as_ptr()) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Set access and modification times for a symlink at `p` (no-follow).
///
/// # Errors
/// Returns an error if `p` contains interior NUL bytes or if the underlying
/// `utimensat` call fails.
pub(crate) fn set_symlink_file_times(p: &Path, atime: FileTime, mtime: FileTime) -> io::Result<()> {
    set_times(p, Some(atime), Some(mtime), true)
}

/// Low-level helper around `utimensat`/`lutimens`.
///
/// # Safety
/// The path is converted to a `CString`, guaranteeing a terminating NUL and no
/// interior NUL bytes before calling the FFI.
fn set_times(
    p: &Path,
    atime: Option<FileTime>,
    mtime: Option<FileTime>,
    symlink: bool,
) -> io::Result<()> {
    let flags = if symlink {
        if cfg!(target_os = "emscripten") {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "emscripten does not support utimensat for symlinks",
            ));
        }
        libc::AT_SYMLINK_NOFOLLOW
    } else {
        0
    };

    let p_cstr = CString::new(p.as_os_str().as_bytes())?;
    let times = [super::to_timespec(&atime), super::to_timespec(&mtime)];
    // SAFETY:
    // - `p_cstr` is a valid NUL-terminated C string.
    // - `times` points to two initialised `timespec` values.
    // - `flags` is 0 or `AT_SYMLINK_NOFOLLOW` as determined by `symlink`.
    let rc = unsafe { libc::utimensat(libc::AT_FDCWD, p_cstr.as_ptr(), times.as_ptr(), flags) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
