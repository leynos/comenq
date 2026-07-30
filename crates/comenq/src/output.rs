//! Human-readable rendering of daemon replies.
//!
//! Formats estimated posting times and one-line comment summaries for the
//! `put` and `list` subcommands.

use comenq_lib::protocol::PendingEntry;

/// Maximum characters of comment text shown by `list`.
const SUMMARY_LIMIT: usize = 60;

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
    if seconds == 0 {
        return "now".to_owned();
    }
    if seconds < MINUTE {
        return format!("{seconds}s");
    }
    if seconds < HOUR {
        let minutes = seconds / MINUTE;
        let rest = seconds % MINUTE;
        return format!("{minutes}m {rest:02}s");
    }
    let hours = seconds / HOUR;
    let minutes = (seconds % HOUR) / MINUTE;
    format!("{hours}h {minutes:02}m")
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
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if flat.chars().count() <= SUMMARY_LIMIT {
        return flat;
    }
    let mut truncated: String = flat.chars().take(SUMMARY_LIMIT - 1).collect();
    truncated.push('…');
    truncated
}

/// Render the `put` confirmation line.
pub(crate) fn render_put(entry: &PendingEntry) -> String {
    format!(
        "Queued {} for {}/{}#{} — posts in ~{}",
        entry.id,
        entry.owner,
        entry.repo,
        entry.pr_number,
        format_eta(entry.eta_seconds)
    )
}

/// Render one `list` line for a pending entry.
pub(crate) fn render_entry(entry: &PendingEntry) -> String {
    format!(
        "{}  {:>7}  {}/{}#{}  {}",
        entry.id,
        format_eta(entry.eta_seconds),
        entry.owner,
        entry.repo,
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
}
