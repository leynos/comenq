//! Redox backend for file timestamp manipulation.
//!
//! Provides helpers to update access and modification times. When only one
//! time is supplied, the other is derived from the file's existing metadata.

use crate::FileTime;
use std::fs::{self, File};
use std::io;
use std::os::unix::prelude::*;
use std::path::Path;

use libredox::{
    call, errno,
    error::{Error, Result as RedoxResult},
    flag, Fd,
};

/// Set both access and modification times for a file at `p`.
///
/// # Errors
/// Returns `EINVAL` if `p` is not UTF-8. Propagates OS errors from
/// libredox when the underlying syscall fails.
pub(crate) fn set_file_times(p: &Path, atime: FileTime, mtime: FileTime) -> io::Result<()> {
    let fd = open_redox(p, 0)?;
    set_file_times_redox(fd.raw(), atime, mtime)
}

/// Set only the modification time for a file at `p`.
///
/// # Errors
/// Returns `EINVAL` if `p` is not UTF-8. Propagates OS errors from
/// libredox when the underlying syscall fails.
pub(crate) fn set_file_mtime(p: &Path, mtime: FileTime) -> io::Result<()> {
    let fd = open_redox(p, 0)?;
    let st = fd.stat()?;

    set_file_times_redox(
        fd.raw(),
        FileTime {
            seconds: st.st_atime as i64,
            nanos: st.st_atime_nsec as u32,
        },
        mtime,
    )?;
    Ok(())
}

/// Set only the access time for a file at `p`.
///
/// # Errors
/// Returns `EINVAL` if `p` is not UTF-8. Propagates OS errors from
/// libredox when the underlying syscall fails.
pub(crate) fn set_file_atime(p: &Path, atime: FileTime) -> io::Result<()> {
    let fd = open_redox(p, 0)?;
    let st = fd.stat()?;

    set_file_times_redox(
        fd.raw(),
        atime,
        FileTime {
            seconds: st.st_mtime as i64,
            nanos: st.st_mtime_nsec as u32,
        },
    )?;
    Ok(())
}

/// Set access and modification times for a symlink at `p` (no-follow).
///
/// # Errors
/// Returns `EINVAL` if `p` is not UTF-8. Propagates OS errors from
/// libredox when the underlying syscall fails.
pub(crate) fn set_symlink_file_times(
    p: &Path,
    atime: FileTime,
    mtime: FileTime,
) -> io::Result<()> {
    let fd = open_redox(p, flag::O_NOFOLLOW)?;
    set_file_times_redox(fd.raw(), atime, mtime)?;
    Ok(())
}

/// Set times for an open file handle.
///
/// If only one of `atime` or `mtime` is provided, the missing value is read
/// from the handle's current metadata.
///
/// # Errors
/// Propagates OS errors from metadata queries or the underlying syscall.
pub(crate) fn set_file_handle_times(
    f: &File,
    atime: Option<FileTime>,
    mtime: Option<FileTime>,
) -> io::Result<()> {
    let (atime1, mtime1) = match (atime, mtime) {
        (Some(a), Some(b)) => (a, b),
        (None, None) => return Ok(()),
        (Some(a), None) => {
            let meta = f.metadata()?;
            (a, FileTime::from_last_modification_time(&meta))
        }
        (None, Some(b)) => {
            let meta = f.metadata()?;
            (FileTime::from_last_access_time(&meta), b)
        }
    };
    set_file_times_redox(f.as_raw_fd() as usize, atime1, mtime1)
}

/// Open `path` with the provided Redox-specific `flags`.
///
/// # Errors
/// Returns `EINVAL` if `path` is not UTF-8. Propagates OS errors from
/// `Fd::open` when the underlying syscall fails.
fn open_redox(path: &Path, flags: i32) -> RedoxResult<Fd> {
    match path.to_str() {
        Some(string) => Fd::open(string, flags, 0),
        // Redox requires UTF-8 paths; non-UTF-8 paths cannot be represented and
        // yield `EINVAL`.
        None => Err(Error::new(errno::EINVAL)),
    }
}

/// Low-level helper using `futimens` on Redox.
fn set_file_times_redox(fd: usize, atime: FileTime, mtime: FileTime) -> io::Result<()> {
    use libredox::data::TimeSpec;

    fn to_timespec(ft: &FileTime) -> TimeSpec {
        TimeSpec {
            tv_sec: ft.seconds(),
            tv_nsec: ft.nanoseconds() as _,
        }
    }

    let times = [to_timespec(&atime), to_timespec(&mtime)];

    call::futimens(fd, &times)?;
    Ok(())
}

/// Return the last modification time from `meta`.
pub(crate) fn from_last_modification_time(meta: &fs::Metadata) -> FileTime {
    FileTime {
        seconds: meta.mtime(),
        nanos: meta.mtime_nsec() as u32,
    }
}

/// Return the last access time from `meta`.
pub(crate) fn from_last_access_time(meta: &fs::Metadata) -> FileTime {
    FileTime {
        seconds: meta.atime(),
        nanos: meta.atime_nsec() as u32,
    }
}

/// Redox does not expose creation time. Always returns `None`.
pub(crate) fn from_creation_time(_meta: &fs::Metadata) -> Option<FileTime> {
    None
}
