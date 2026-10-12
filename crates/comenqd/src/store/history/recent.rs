//! Reverse scanning for bounded recent-history reads.

use std::fs::File;
use std::io::{self, Read as _, Seek as _, SeekFrom};
use std::path::Path;

use super::{HistoryRecord, MAX_HISTORY_LINE_BYTES, Result, log_malformed, log_oversized};

/// Keep reverse reads small while avoiding one seek per byte.
const REVERSE_CHUNK_BYTES: usize = 16 * 1024;

/// Find the start of the newest `limit` valid records without scanning older history.
pub(super) fn recent_start(
    path: &Path,
    file: &mut File,
    end_offset: u64,
    limit: usize,
) -> Result<u64> {
    let mut chunk_end = end_offset;
    let mut reversed_line = Vec::new();
    let mut line_start = None;
    let mut oversized = false;
    let mut record_starts = Vec::with_capacity(limit.min(1024));

    while chunk_end > 0 {
        let chunk_start = chunk_end.saturating_sub(REVERSE_CHUNK_BYTES as u64);
        let chunk_len = usize::try_from(chunk_end - chunk_start)
            .map_err(|_| io::Error::other("history chunk length exceeds address space"))?;
        file.seek(SeekFrom::Start(chunk_start))?;
        let mut chunk = vec![0; chunk_len];
        file.read_exact(&mut chunk)?;

        for (index, byte) in chunk.iter().enumerate().rev() {
            let offset = chunk_start + index as u64;
            if *byte == b'\n' {
                if let Some(start) = finish_line(path, line_start, &mut reversed_line, oversized) {
                    record_starts.push(start);
                    if record_starts.len() == limit {
                        return Ok(start);
                    }
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

    if let Some(start) = finish_line(path, line_start, &mut reversed_line, oversized) {
        record_starts.push(start);
    }
    Ok(record_starts.last().copied().unwrap_or(end_offset))
}

fn finish_line(
    path: &Path,
    line_start: Option<u64>,
    reversed_line: &mut [u8],
    oversized: bool,
) -> Option<u64> {
    let start = line_start?;
    accept_line(path, start, reversed_line, oversized)
}

fn accept_line(path: &Path, start: u64, reversed_line: &mut [u8], oversized: bool) -> Option<u64> {
    if oversized {
        log_oversized(path, start);
        return None;
    }
    reversed_line.reverse();
    if reversed_line.iter().all(u8::is_ascii_whitespace) {
        return None;
    }
    match serde_json::from_slice::<HistoryRecord>(reversed_line) {
        Ok(_) => Some(start),
        Err(error) => {
            log_malformed(path, start, &error);
            None
        }
    }
}
