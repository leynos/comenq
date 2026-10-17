//! Rendering regressions for human-readable client output.

use super::{format_age, format_eta, one_line_summary, render_entry, render_history, render_put};
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

fn history(success: bool, error: Option<&str>) -> HistoryEntry {
    HistoryEntry {
        id: "1a2b3c4d".into(),
        posted_at: 1_000,
        success,
        error: error.map(str::to_owned),
        token_hash: "0123456789abcdef".repeat(4),
        owner: "octocat".into(),
        repo: "hello-world".into(),
        pr_number: 7,
        body: "Hi there".into(),
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
#[case(0, "just now")]
#[case(45, "45s ago")]
#[case(150, "2m 30s ago")]
#[case(3_720, "1h 02m ago")]
fn formats_age(#[case] seconds: u64, #[case] expected: &str) {
    assert_eq!(format_age(seconds), expected);
}

#[rstest]
fn renders_successful_history_line() {
    assert_eq!(
        render_history(&history(true, None), 1_150),
        "1a2b3c4d   2m 30s ago  ok    01234567  octocat/hello-world#7  Hi there"
    );
}

#[rstest]
fn renders_legacy_history_line_without_token_hash() {
    let mut entry = history(true, None);
    entry.token_hash.clear();

    assert_eq!(
        render_history(&entry, 1_150),
        "1a2b3c4d   2m 30s ago  ok    --------  octocat/hello-world#7  Hi there"
    );
}

#[rstest]
fn renders_failed_history_line_with_error() {
    assert_eq!(
        render_history(&history(false, Some("timeout\nafter 10s")), 1_045),
        "1a2b3c4d      45s ago  FAIL  01234567  octocat/hello-world#7  Hi there — timeout after 10s"
    );
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

#[rstest]
#[case(0, 60)]
#[case(1, 1)]
#[case(9, 9)]
#[case(10, 10)]
#[case(60, 10)]
#[case(61, 30)]
#[case(600, 30)]
#[case(601, 120)]
#[case(3_600, 120)]
#[case(3_601, 600)]
#[case(21_600, 600)]
#[case(21_601, 1_800)]
#[case(u64::MAX, 1_800)]
fn bounds_report_cadence(#[case] seconds: u64, #[case] expected: u64) {
    assert_eq!(super::report_interval(seconds), expected);
}

#[rstest]
#[case(0, "due now")]
#[case(45, "45s remaining")]
#[case(150, "2m 30s remaining")]
#[case(3_720, "1h 02m remaining")]
fn renders_countdown(#[case] seconds: u64, #[case] expected: &str) {
    assert_eq!(super::render_remaining(seconds), expected);
}
