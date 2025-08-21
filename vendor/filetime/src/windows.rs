//! Windows backend for file timestamp manipulation.

use crate::FileTime;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::windows::prelude::*;
use std::path::Path;
use std::ptr;
use windows_sys::Win32::Foundation::{FILETIME, HANDLE};
use windows_sys::Win32::Storage::FileSystem::*;

const HUNDRED_NS_PER_SEC_I64: i64 = 10_000_000;
const HUNDRED_NS_PER_SEC_U64: u64 = 10_000_000;

/// Set both access and modification times for a file at `p`.
pub(crate) fn set_file_times(p: &Path, atime: FileTime, mtime: FileTime) -> io::Result<()> {
    let f = OpenOptions::new()
        .write(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(p)?;
    set_file_handle_times(&f, Some(atime), Some(mtime))
}

/// Set only the modification time for a file at `p`.
pub(crate) fn set_file_mtime(p: &Path, mtime: FileTime) -> io::Result<()> {
    let f = OpenOptions::new()
        .write(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(p)?;
    set_file_handle_times(&f, None, Some(mtime))
}

/// Set only the access time for a file at `p`.
pub(crate) fn set_file_atime(p: &Path, atime: FileTime) -> io::Result<()> {
    let f = OpenOptions::new()
        .write(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(p)?;
    set_file_handle_times(&f, Some(atime), None)
}

/// Set times for an open file handle.
pub(crate) fn set_file_handle_times(
    f: &File,
    atime: Option<FileTime>,
    mtime: Option<FileTime>,
) -> io::Result<()> {
    let atime = atime.map(to_filetime);
    let mtime = mtime.map(to_filetime);
    // SAFETY: `f.as_raw_handle()` yields a valid handle; optional FILETIME
    // pointers either reference stack values that live until the call returns
    // or are null. Windows does not retain the pointers.
    let ret = unsafe {
        SetFileTime(
            f.as_raw_handle() as HANDLE,
            ptr::null(),
            atime
                .as_ref()
                .map(|p| p as *const FILETIME)
                .unwrap_or(ptr::null()),
            mtime
                .as_ref()
                .map(|p| p as *const FILETIME)
                .unwrap_or(ptr::null()),
        )
    };
    if ret != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }

    fn to_filetime(ft: FileTime) -> FILETIME {
        let ticks = ft.seconds() * HUNDRED_NS_PER_SEC_I64 + (ft.nanoseconds() as i64) / 100;
        FILETIME {
            dwLowDateTime: ticks as u32,
            dwHighDateTime: (ticks >> 32) as u32,
        }
    }
}

/// Set times for a symlink at `p` without following it.
pub(crate) fn set_symlink_file_times(p: &Path, atime: FileTime, mtime: FileTime) -> io::Result<()> {
    use std::os::windows::fs::OpenOptionsExt;

    let f = OpenOptions::new()
        .write(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .open(p)?;
    set_file_handle_times(&f, Some(atime), Some(mtime))
}

/// Return the last modification time from `meta`.
pub(crate) fn from_last_modification_time(meta: &fs::Metadata) -> FileTime {
    from_intervals(meta.last_write_time())
}

/// Return the last access time from `meta`.
pub(crate) fn from_last_access_time(meta: &fs::Metadata) -> FileTime {
    from_intervals(meta.last_access_time())
}

/// Return the creation time from `meta`.
pub(crate) fn from_creation_time(meta: &fs::Metadata) -> Option<FileTime> {
    Some(from_intervals(meta.creation_time()))
}

fn from_intervals(ticks: u64) -> FileTime {
    // Windows stores times in 100 ns intervals, convert to seconds/nanoseconds.
    FileTime {
        seconds: (ticks / HUNDRED_NS_PER_SEC_U64) as i64,
        nanos: ((ticks % HUNDRED_NS_PER_SEC_U64) * 100) as u32,
    }
}
