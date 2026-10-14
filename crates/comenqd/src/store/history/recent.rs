//! Reverse scanning for bounded recent-history reads and completion lookup.

use std::fs::File;
use std::io::{self, Read as _, Seek as _, SeekFrom};
use std::path::Path;

use super::{HistoryRecord, MAX_HISTORY_LINE_BYTES, Result, log_malformed, log_oversized};

const REVERSE_CHUNK_BYTES: usize = 16 * 1024;

/// Find the start of the newest `limit` valid records without scanning older history.
pub(super) fn recent_start(
    path: &Path,
    file: &mut File,
    end_offset: u64,
    limit: usize,
) -> Result<u64> {
    let mut count = 0;
    let mut oldest = end_offset;
    scan_reverse(path, file, end_offset, 0, |start, _| {
        oldest = start;
        count += 1;
        count == limit
    })?;
    Ok(oldest)
}

/// Return the newest matching record at or beyond a caller's history offset.
pub(super) fn scan_reverse(
    path: &Path,
    file: &mut File,
    end_offset: u64,
    minimum_offset: u64,
    mut accept: impl FnMut(u64, &HistoryRecord) -> bool,
) -> Result<Option<HistoryRecord>> {
    let mut chunk_end = end_offset;
    let mut reversed_line = Vec::new();
    let mut line_start = None;
    let mut oversized = false;
    while chunk_end > minimum_offset {
        let chunk_start = chunk_end
            .saturating_sub(REVERSE_CHUNK_BYTES as u64)
            .max(minimum_offset);
        let chunk_len = usize::try_from(chunk_end - chunk_start)
            .map_err(|_| io::Error::other("history chunk length exceeds address space"))?;
        file.seek(SeekFrom::Start(chunk_start))?;
        let mut chunk = vec![0; chunk_len];
        file.read_exact(&mut chunk)?;
        for (index, byte) in chunk.iter().enumerate().rev() {
            let offset = chunk_start + index as u64;
            if *byte == b'\n' {
                if let Some(record) = finish_line(path, line_start, &mut reversed_line, oversized)
                    && accept(offset + 1, &record)
                {
                    return Ok(Some(record));
                }
                reversed_line.clear();
                line_start = None;
                oversized = false;
            } else {
                line_start = Some(offset);
                if !oversized {
                    if reversed_line.len() == MAX_HISTORY_LINE_BYTES {
                        reversed_line.clear();
                        oversized = true;
                    } else {
                        reversed_line.push(*byte);
                    }
                }
            }
        }
        chunk_end = chunk_start;
    }
    if let Some(record) = finish_line(path, line_start, &mut reversed_line, oversized)
        && accept(minimum_offset, &record)
    {
        return Ok(Some(record));
    }
    Ok(None)
}

fn finish_line(
    path: &Path,
    line_start: Option<u64>,
    reversed_line: &mut [u8],
    oversized: bool,
) -> Option<HistoryRecord> {
    let start = line_start?;
    if oversized {
        log_oversized(path, start);
        return None;
    }
    reversed_line.reverse();
    if reversed_line.iter().all(u8::is_ascii_whitespace) {
        return None;
    }
    match serde_json::from_slice::<HistoryRecord>(reversed_line) {
        Ok(record) => Some(record),
        Err(error) => {
            log_malformed(path, start, &error);
            None
        }
    }
}
