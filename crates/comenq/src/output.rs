//! Human-readable rendering of daemon replies.
//!
//! Formats estimated posting times, posting ages, and one-line summaries for
//! the `put`, `list`, and `hist` subcommands.

use comenq_lib::protocol::{HistoryEntry, PendingEntry};

/// Maximum characters of comment text shown by `list`.
const SUMMARY_LIMIT: usize = 60;

#[derive(Clone, Copy)]
enum EtaUnit {
    Now,
    Seconds,
    Minutes,
    Hours,
}

/// Classify an ETA before rendering its compact duration.
///
/// The resulting unit determines the precision used by [`format_eta`].
fn classify_eta(seconds: u64) -> EtaUnit {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    match seconds {
        0 => EtaUnit::Now,
        1..MINUTE => EtaUnit::Seconds,
        MINUTE..HOUR => EtaUnit::Minutes,
        _ => EtaUnit::Hours,
    }
}

/// Render an ETA in seconds as a compact human duration.
///
/// # Examples
///
/// ```rust
/// assert_eq!(comenq::format_eta(0), "now");
/// assert_eq!(comenq::format_eta(45), "45s");
/// assert_eq!(comenq::format_eta(150), "2m 30s");
/// assert_eq!(comenq::format_eta(3_720), "1h 02m");
/// ```
#[must_use]
pub fn format_eta(seconds: u64) -> String {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    match classify_eta(seconds) {
        EtaUnit::Now => "now".to_owned(),
        EtaUnit::Seconds => format!("{seconds}s"),
        EtaUnit::Minutes => {
            let minutes = seconds / MINUTE;
            let rest = seconds % MINUTE;
            format!("{minutes}m {rest:02}s")
        }
        EtaUnit::Hours => {
            let hours = seconds / HOUR;
            let minutes = (seconds % HOUR) / MINUTE;
            format!("{hours}h {minutes:02}m")
        }
    }
}

/// Choose a countdown cadence, capped at the remaining wait and at least one second.
///
/// # Examples
///
/// ```rust
/// assert_eq!(comenq::report_interval(45), 10);
/// assert_eq!(comenq::report_interval(3), 3);
/// ```
#[must_use]
pub fn report_interval(seconds: u64) -> u64 {
    let cadence = match seconds {
        0..=60 => 10,
        61..=600 => 30,
        601..=3_600 => 120,
        3_601..=21_600 => 600,
        _ => 1_800,
    };
    cadence.min(seconds.max(1))
}

/// Render a remaining wait using the usual ETA precision.
///
/// # Examples
///
/// ```rust
/// assert_eq!(comenq::render_remaining(150), "2m 30s remaining");
/// assert_eq!(comenq::render_remaining(0), "due now");
/// ```
#[must_use]
pub fn render_remaining(seconds: u64) -> String {
    terminal_safe(&if seconds == 0 {
        "due now".to_owned()
    } else {
        format!("{} remaining", format_eta(seconds))
    })
}

/// Collapse a comment body to a single line of at most 60 characters.
///
/// Control characters (including newlines and tabs) become spaces so the
/// summary never spans lines; longer bodies are truncated with an ellipsis.
///
/// # Examples
///
/// ```rust
/// assert_eq!(comenq::one_line_summary("Hi there"), "Hi there");
/// assert_eq!(comenq::one_line_summary("a\nb\tc"), "a b c");
/// let long = "x".repeat(80);
/// let summary = comenq::one_line_summary(&long);
/// assert_eq!(summary.chars().count(), 60);
/// assert!(summary.ends_with('…'));
/// ```
#[must_use]
pub fn one_line_summary(body: &str) -> String {
    let flat: String = body
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') {
                ' '
            } else {
                c
            }
        })
        .collect();
    if flat.chars().count() <= SUMMARY_LIMIT {
        return flat;
    }
    let mut truncated: String = flat.chars().take(SUMMARY_LIMIT - 1).collect();
    truncated.push('…');
    truncated
}

/// Render how long ago something happened as a compact human duration.
///
/// # Examples
///
/// ```rust
/// assert_eq!(comenq::format_age(0), "just now");
/// assert_eq!(comenq::format_age(45), "45s ago");
/// assert_eq!(comenq::format_age(3_720), "1h 02m ago");
/// ```
#[must_use]
pub fn format_age(seconds: u64) -> String {
    if seconds == 0 {
        "just now".to_owned()
    } else {
        format!("{} ago", format_eta(seconds))
    }
}

/// Render one `hist` line for a past posting attempt.
///
/// `now` is the current Unix time, used to show the attempt's age. Failed
/// attempts include their failure description on the same line.
pub(crate) fn render_history(entry: &HistoryEntry, now: u64) -> String {
    let age = format_age(now.saturating_sub(entry.posted_at));
    let status = if entry.success { "ok" } else { "FAIL" };
    let token_hash: String = entry.token_hash.chars().take(8).collect();
    let token_hash = if token_hash.is_empty() {
        "--------".to_owned()
    } else {
        terminal_safe(&token_hash)
    };
    let mut line = format!(
        "{}  {:>11}  {:<4}  {}  {}/{}#{}  {}",
        terminal_safe(&entry.id),
        age,
        status,
        token_hash,
        terminal_safe(&entry.owner),
        terminal_safe(&entry.repo),
        entry.pr_number,
        one_line_summary(&entry.body)
    );
    if let Some(error) = &entry.error {
        line.push_str(" — ");
        line.push_str(&one_line_summary(error));
    }
    line
}

/// Escape reply fields that could otherwise alter terminal output.
///
/// Control characters are rendered visibly so untrusted identifiers and
/// repository names cannot add terminal lines or control sequences.
pub(crate) fn terminal_safe(value: &str) -> String {
    value.chars().fold(String::new(), |mut safe, character| {
        if character.is_control() || matches!(character, '\u{2028}' | '\u{2029}') {
            safe.extend(character.escape_default());
        } else {
            safe.push(character);
        }
        safe
    })
}

/// Render the `put` confirmation line.
pub(crate) fn render_put(entry: &PendingEntry) -> String {
    format!(
        "Queued {} for {}/{}#{} — posts in ~{}",
        terminal_safe(&entry.id),
        terminal_safe(&entry.owner),
        terminal_safe(&entry.repo),
        entry.pr_number,
        format_eta(entry.eta_seconds)
    )
}

/// Render one `list` line for a pending entry.
pub(crate) fn render_entry(entry: &PendingEntry) -> String {
    format!(
        "{}  {:>7}  {}/{}#{}  {}",
        terminal_safe(&entry.id),
        format_eta(entry.eta_seconds),
        terminal_safe(&entry.owner),
        terminal_safe(&entry.repo),
        entry.pr_number,
        one_line_summary(&entry.body)
    )
}

#[cfg(test)]
mod tests;
