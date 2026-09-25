//! Human-readable rendering of daemon replies.
//!
//! Formats estimated posting times and one-line comment summaries for the
//! `put` and `list` subcommands.

use comenq_lib::protocol::PendingEntry;

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

/// Escape reply fields that could otherwise alter terminal output.
fn terminal_safe(value: &str) -> String {
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
mod tests {
    //! Unit tests for ETA and summary rendering.
    use super::{format_eta, one_line_summary, render_entry, render_put};
    use comenq_lib::protocol::PendingEntry;
    use rstest::rstest;

    fn entry(body: &str, eta: u64) -> PendingEntry {
        PendingEntry {
            id: "1a2b3c4d".into(),
            eta_seconds: eta,
            owner: "octocat".into(),
            repo: "hello-world".into(),
            pr_number: 7,
            body: body.into(),
        }
    }

    #[rstest]
    #[case(0, "now")]
    #[case(1, "1s")]
    #[case(59, "59s")]
    #[case(60, "1m 00s")]
    #[case(150, "2m 30s")]
    #[case(3_599, "59m 59s")]
    #[case(3_600, "1h 00m")]
    #[case(3_720, "1h 02m")]
    #[case(90_000, "25h 00m")]
    fn formats_eta(#[case] seconds: u64, #[case] expected: &str) {
        assert_eq!(format_eta(seconds), expected);
    }

    #[rstest]
    #[case("short", "short")]
    #[case("line\nbreaks\tand\rreturns", "line breaks and returns")]
    #[case("line\u{2028}and\u{2029}paragraph", "line and paragraph")]
    fn summarises_one_line(#[case] body: &str, #[case] expected: &str) {
        assert_eq!(one_line_summary(body), expected);
    }

    #[rstest]
    fn truncates_long_bodies_to_sixty_characters() {
        let body = "a".repeat(100);
        let summary = one_line_summary(&body);
        assert_eq!(summary.chars().count(), 60);
        assert!(summary.ends_with('…'));
    }

    #[rstest]
    fn sixty_character_bodies_are_untouched() {
        let body = "a".repeat(60);
        assert_eq!(one_line_summary(&body), body);
    }

    #[rstest]
    #[case('\u{2028}')]
    #[case('\u{2029}')]
    fn truncation_normalizes_unicode_line_separators(#[case] separator: char) {
        let body = format!("{}{}tail", "a".repeat(59), separator);
        let summary = one_line_summary(&body);
        assert_eq!(summary.chars().count(), 60);
        assert_eq!(summary.chars().nth(59), Some('…'));
    }

    #[rstest]
    fn renders_put_confirmation() {
        let line = render_put(&entry("Hi", 3_660));
        assert_eq!(
            line,
            "Queued 1a2b3c4d for octocat/hello-world#7 — posts in ~1h 01m"
        );
    }

    #[rstest]
    fn renders_list_line_with_truncated_body() {
        let body = "b".repeat(100);
        let line = render_entry(&entry(&body, 90));
        assert!(line.starts_with("1a2b3c4d"));
        assert!(line.contains("1m 30s"));
        assert!(line.contains("octocat/hello-world#7"));
        assert!(line.ends_with('…'));
        assert!(!line.contains('\n'));
    }

    #[rstest]
    fn escapes_untrusted_reply_target_fields() {
        let mut entry = entry("Hi", 0);
        entry.id = "1a2b\u{1b}[2J".into();
        entry.owner = "octo\ncat".into();
        entry.repo = "hello\u{2028}world".into();

        let rendered = render_put(&entry);
        assert!(!rendered.contains('\u{1b}'));
        assert!(!rendered.contains('\n'));
        assert!(!rendered.contains('\u{2028}'));
        assert!(rendered.contains("\\u{1b}"));
        assert!(rendered.contains("\\n"));
        assert!(rendered.contains("\\u{2028}"));
    }
}
