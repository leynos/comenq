//! Human-readable rendering of daemon replies.
//!
//! Formats estimated posting times, past-attempt ages, and one-line comment
//! summaries for the `put`, `list`, and `hist` subcommands.

use comenq_lib::protocol::{HistoryEntry, PendingEntry};

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
        return "just now".to_owned();
    }
    format!("{} ago", format_eta(seconds))
}

/// Render one `hist` line for a past posting attempt.
///
/// `now` is the current Unix time, used to show the attempt's age. The
/// posting token appears as the first eight characters of its hash. Failed
/// attempts carry the failure description, collapsed to one line.
pub(crate) fn render_history(entry: &HistoryEntry, now: u64) -> String {
    let age = format_age(now.saturating_sub(entry.posted_at));
    let status = if entry.success { "ok" } else { "FAIL" };
    let token: String = entry.token_hash.chars().take(8).collect();
    let mut line = format!(
        "{}  {:>11}  {:<4}  {:<8}  {}/{}#{}  {}",
        entry.id,
        age,
        status,
        token,
        entry.owner,
        entry.repo,
        entry.pr_number,
        one_line_summary(&entry.body)
    );
    if let Some(error) = &entry.error {
        line.push_str(" — ");
        line.push_str(&one_line_summary(error));
    }
    line
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
    //! Unit tests for ETA, age, and summary rendering.
    use super::{
        format_age, format_eta, one_line_summary, render_entry, render_history, render_put,
    };
    use comenq_lib::protocol::{HistoryEntry, PendingEntry};
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

    fn history(success: bool, error: Option<&str>) -> HistoryEntry {
        HistoryEntry {
            id: "1a2b3c4d".into(),
            posted_at: 1_000,
            success,
            error: error.map(str::to_owned),
            token_hash: "cafe0123".repeat(8),
            owner: "octocat".into(),
            repo: "hello-world".into(),
            pr_number: 7,
            body: "Hi there".into(),
        }
    }

    #[rstest]
    #[case(0, "just now")]
    #[case(45, "45s ago")]
    #[case(150, "2m 30s ago")]
    #[case(3_720, "1h 02m ago")]
    fn formats_age(#[case] seconds: u64, #[case] expected: &str) {
        assert_eq!(format_age(seconds), expected);
    }

    #[rstest]
    fn renders_successful_history_line() {
        let line = render_history(&history(true, None), 1_150);
        assert_eq!(
            line,
            "1a2b3c4d   2m 30s ago  ok    cafe0123  octocat/hello-world#7  Hi there"
        );
    }

    #[rstest]
    fn renders_failed_history_line_with_error() {
        let line = render_history(&history(false, Some("timeout\nafter 10s")), 1_045);
        assert_eq!(
            line,
            "1a2b3c4d      45s ago  FAIL  cafe0123  octocat/hello-world#7  Hi there — timeout after 10s"
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
