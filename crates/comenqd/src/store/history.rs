//! Posting-history records and bounded access to their JSON Lines log.
//!
//! The store owns durable attempt records and file positions; protocol adapters
//! convert records to wire types and decide how many fit in a response.

use comenq_lib::CommentRequest;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read as _, Seek as _, SeekFrom, Write as _};
use std::path::Path;
use uuid::Uuid;

use super::{Result, StoredEntry};

mod recent;

/// Bound error text so one failed attempt always fits within a history page.
const MAX_HISTORY_ERROR_BYTES: usize = 64 * 1024;
/// Bound memory used to inspect one history line, including corrupt lines.
const MAX_HISTORY_LINE_BYTES: usize = 2 * 1024 * 1024;

/// One posting attempt, as recorded in the history log.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct HistoryRecord {
    /// Stable identifier used to make completion recovery idempotent.
    #[serde(default)]
    pub attempt_id: String,
    /// Identifier the entry carried while queued.
    pub id: String,
    /// Unix time the posting attempt finished, in seconds.
    pub posted_at: u64,
    /// Whether GitHub accepted the comment.
    pub success: bool,
    /// Failure description when `success` is false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The comment that was posted (or attempted).
    pub request: CommentRequest,
}

impl HistoryRecord {
    /// Record a successful post of `entry` at `posted_at`.
    #[must_use]
    pub(crate) fn success(entry: &StoredEntry, posted_at: u64) -> Self {
        Self {
            attempt_id: Uuid::new_v4().to_string(),
            id: entry.id.clone(),
            posted_at,
            success: true,
            error: None,
            request: entry.request.clone(),
        }
    }

    /// Record a failed posting attempt of `entry` at `posted_at`.
    #[must_use]
    pub(crate) fn failure(entry: &StoredEntry, posted_at: u64, error: impl Into<String>) -> Self {
        Self {
            attempt_id: Uuid::new_v4().to_string(),
            id: entry.id.clone(),
            posted_at,
            success: false,
            error: Some(bounded_error(error.into())),
            request: entry.request.clone(),
        }
    }
}

fn bounded_error(mut error: String) -> String {
    if error.len() <= MAX_HISTORY_ERROR_BYTES {
        return error;
    }
    let mut end = MAX_HISTORY_ERROR_BYTES - 3;
    while !error.is_char_boundary(end) {
        end -= 1;
    }
    error.truncate(end);
    error.push_str("...");
    error
}

/// Position in a fixed snapshot of the append-only history file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HistoryPosition {
    pub offset: u64,
    pub end_offset: u64,
}

/// A persisted record paired with the position immediately after its line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HistoryRecordAt {
    pub record: HistoryRecord,
    pub position_after: HistoryPosition,
}

/// One bounded page from the persisted history log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HistoryPage {
    pub records: Vec<HistoryRecordAt>,
    pub next: Option<HistoryPosition>,
}

impl HistoryPage {
    fn empty() -> Self {
        Self {
            records: Vec::new(),
            next: None,
        }
    }
}

/// Append `record` as one line to the log at `path`.
///
/// A missing final delimiter is repaired before the new line, so a partial
/// previous write cannot consume the next valid record.
pub(super) fn append(path: &Path, record: &HistoryRecord) -> Result<()> {
    let mut line = serde_json::to_vec(record)?;
    line.push(b'\n');
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .read(true)
        .open(path)?;
    let file_len = file.metadata()?.len();
    append_line(&mut file, &line, file_len)?;
    if file_len == 0 {
        sync_parent_directory(path)?;
    }
    Ok(())
}

fn append_line(file: &mut File, line: &[u8], file_len: u64) -> Result<()> {
    if file_len > 0 && !has_final_newline(file, file_len)? {
        file.write_all(b"\n")?;
    }
    file.write_all(line)?;
    file.sync_data()?;
    Ok(())
}

fn has_final_newline(file: &mut File, file_len: u64) -> io::Result<bool> {
    file.seek(SeekFrom::Start(file_len - 1))?;
    let mut last_byte = [0_u8; 1];
    file.read_exact(&mut last_byte)?;
    Ok(last_byte[0] == b'\n')
}

fn sync_parent_directory(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "history path has no parent"))?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

/// Append a completion record unless its stable attempt identifier is present.
pub(super) fn append_once(path: &Path, record: &HistoryRecord, search_from: u64) -> Result<()> {
    if contains_attempt(path, &record.attempt_id, search_from)? {
        return Ok(());
    }
    append(path, record)
}

/// Read a bounded page, optionally starting at the newest `recent_limit` records.
pub(super) fn read_page(
    path: &Path,
    position: Option<HistoryPosition>,
    recent_limit: Option<usize>,
    page_size: usize,
) -> Result<HistoryPage> {
    if page_size == 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "page size is zero").into());
    }
    let Some((mut file, file_len)) = open_history_snapshot(path, position.is_none())? else {
        return Ok(HistoryPage::empty());
    };
    let (offset, end_offset) = page_offsets(path, &mut file, file_len, position, recent_limit)?;
    read_snapshot_page(
        path, file, file_len, position, offset, end_offset, page_size,
    )
}

fn open_history_snapshot(path: &Path, missing_is_empty: bool) -> Result<Option<(File, u64)>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if missing_is_empty && error.kind() == io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => return Err(error.into()),
    };
    let file_len = file.metadata()?.len();
    Ok(Some((file, file_len)))
}

fn page_offsets(
    path: &Path,
    file: &mut File,
    file_len: u64,
    position: Option<HistoryPosition>,
    recent_limit: Option<usize>,
) -> Result<(u64, u64)> {
    if let Some(position) = position {
        return Ok((position.offset, position.end_offset));
    }
    let offset = match recent_limit {
        Some(0) => file_len,
        Some(limit) => recent::recent_start(path, file, file_len, limit)?,
        None => 0,
    };
    Ok((offset, file_len))
}

fn read_snapshot_page(
    path: &Path,
    file: File,
    file_len: u64,
    position: Option<HistoryPosition>,
    offset: u64,
    end_offset: u64,
    page_size: usize,
) -> Result<HistoryPage> {
    let mut reader = BufReader::new(file);
    if let Some(position) = position {
        validate_position(&mut reader, position, file_len)?;
    }
    if offset >= end_offset {
        return Ok(HistoryPage::empty());
    }
    reader.seek(SeekFrom::Start(offset))?;
    let (records, current_offset) = read_records(path, &mut reader, offset, end_offset, page_size)?;
    let next = (current_offset < end_offset).then_some(HistoryPosition {
        offset: current_offset,
        end_offset,
    });
    Ok(HistoryPage { records, next })
}

fn read_records(
    path: &Path,
    reader: &mut BufReader<File>,
    offset: u64,
    end_offset: u64,
    page_size: usize,
) -> Result<(Vec<HistoryRecordAt>, u64)> {
    let mut current_offset = offset;
    let mut records = Vec::with_capacity(page_size);
    let mut line = Vec::new();
    while current_offset < end_offset && records.len() < page_size {
        if let Some(record) = read_record(path, reader, &mut current_offset, end_offset, &mut line)?
        {
            records.push(record);
        }
    }
    Ok((records, current_offset))
}

fn read_record(
    path: &Path,
    reader: &mut BufReader<File>,
    current_offset: &mut u64,
    end_offset: u64,
    line: &mut Vec<u8>,
) -> Result<Option<HistoryRecordAt>> {
    let line_start = *current_offset;
    let oversized = read_snapshot_line(reader, current_offset, end_offset, line)?;
    if oversized {
        log_oversized(path, line_start);
        return Ok(None);
    }
    if line.iter().all(u8::is_ascii_whitespace) {
        return Ok(None);
    }
    match serde_json::from_slice::<HistoryRecord>(line) {
        Ok(record) => Ok(Some(HistoryRecordAt {
            record,
            position_after: HistoryPosition {
                offset: *current_offset,
                end_offset,
            },
        })),
        Err(error) => {
            log_malformed(path, line_start, &error);
            Ok(None)
        }
    }
}

fn read_snapshot_line(
    reader: &mut BufReader<File>,
    current_offset: &mut u64,
    end_offset: u64,
    line: &mut Vec<u8>,
) -> io::Result<bool> {
    line.clear();
    let mut oversized = false;
    while *current_offset < end_offset {
        let remaining = (end_offset - *current_offset).min(usize::MAX as u64) as usize;
        let (consumed, line_ended) = {
            let available = reader.fill_buf()?;
            if available.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "history log changed while reading a snapshot",
                ));
            }
            let visible = available.len().min(remaining);
            let line_end = available[..visible].iter().position(|byte| *byte == b'\n');
            let line_len = line_end.map_or(visible, |index| index + 1);
            if !oversized && line.len().saturating_add(line_len) <= MAX_HISTORY_LINE_BYTES {
                line.extend_from_slice(&available[..line_len]);
            } else {
                oversized = true;
            }
            (line_len, line_end.is_some())
        };
        reader.consume(consumed);
        *current_offset += consumed as u64;
        if line_ended {
            break;
        }
    }
    Ok(oversized)
}

fn validate_position(
    reader: &mut BufReader<File>,
    position: HistoryPosition,
    file_len: u64,
) -> Result<()> {
    if position.offset > position.end_offset || position.end_offset > file_len {
        return Err(invalid_position().into());
    }
    if position.offset == 0 {
        return Ok(());
    }
    reader.seek(SeekFrom::Start(position.offset - 1))?;
    let mut previous_byte = [0_u8; 1];
    reader.read_exact(&mut previous_byte)?;
    if previous_byte[0] != b'\n' {
        return Err(invalid_position().into());
    }
    Ok(())
}

fn contains_attempt(path: &Path, attempt_id: &str, search_from: u64) -> Result<bool> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let file_len = file.metadata()?.len();
    let offset = search_from.min(file_len);
    let mut reader = BufReader::new(file);
    reader.seek(SeekFrom::Start(offset))?;
    let mut current_offset = offset;
    let mut line = Vec::new();
    while current_offset < file_len {
        let oversized = read_snapshot_line(&mut reader, &mut current_offset, file_len, &mut line)?;
        if oversized {
            continue;
        }
        if let Ok(record) = serde_json::from_slice::<HistoryRecord>(&line)
            && record.attempt_id == attempt_id
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn log_malformed(path: &Path, offset: u64, error: &serde_json::Error) {
    tracing::error!(
        path = %path.display(),
        offset,
        error = %error,
        "Skipping unreadable history record"
    );
}

fn log_oversized(path: &Path, offset: u64) {
    tracing::error!(
        path = %path.display(),
        offset,
        "Skipping oversized history record"
    );
}

fn invalid_position() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "history cursor is invalid")
}
