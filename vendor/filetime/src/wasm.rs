//! Unimplemented filetime backend for wasm targets.
//!
//! Provides stub implementations that either return a canonical error or
//! panic, documenting unsupported operations on this platform.

use crate::FileTime;
use std::fs::{self, File};
use std::io;
use std::path::Path;

#[inline]
fn wasm_not_implemented() -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Other,
        "Wasm backend not implemented",
    ))
}

pub(crate) fn set_file_times(
    _p: &Path,
    _atime: FileTime,
    _mtime: FileTime,
) -> io::Result<()> {
    wasm_not_implemented()
}

pub(crate) fn set_symlink_file_times(
    _p: &Path,
    _atime: FileTime,
    _mtime: FileTime,
) -> io::Result<()> {
    wasm_not_implemented()
}

pub(crate) fn set_file_mtime(_p: &Path, _mtime: FileTime) -> io::Result<()> {
    wasm_not_implemented()
}

pub(crate) fn set_file_atime(_p: &Path, _atime: FileTime) -> io::Result<()> {
    wasm_not_implemented()
}

/// Not supported on the wasm backend.
///
/// # Panics
/// Always panics; this target has no filesystem metadata.
pub(crate) fn from_last_modification_time(_meta: &fs::Metadata) -> FileTime {
    panic!("filetime: from_last_modification_time is unsupported on wasm target")
}

/// Not supported on the wasm backend.
///
/// # Panics
/// Always panics; this target has no filesystem metadata.
pub(crate) fn from_last_access_time(_meta: &fs::Metadata) -> FileTime {
    panic!("filetime: from_last_access_time is unsupported on wasm target")
}

/// Not supported on the wasm backend. Always returns `None`.
pub(crate) fn from_creation_time(_meta: &fs::Metadata) -> Option<FileTime> {
    None
}

pub(crate) fn set_file_handle_times(
    _f: &File,
    _atime: Option<FileTime>,
    _mtime: Option<FileTime>,
) -> io::Result<()> {
    wasm_not_implemented()
}
