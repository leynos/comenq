//! Output and user-visible behaviour tests for the client.
mod connect;
mod history_transport;
mod transport;

use super::{ClientError, render_response};
use crate::Command;
use comenq_lib::protocol::{HistoryEntry, PendingEntry};
use std::io::{self, Write};

struct FailingWriter;

impl Write for FailingWriter {
    fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
        Err(io::Error::from(io::ErrorKind::PermissionDenied))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn sample_entry() -> PendingEntry {
    PendingEntry {
        id: "1a2b3c4d".into(),
        eta_seconds: 0,
        owner: "octocat".into(),
        repo: "hello-world".into(),
        pr_number: 1,
        body: "Hi".into(),
    }
}

fn sample_history(success: bool, error: Option<&str>) -> HistoryEntry {
    HistoryEntry {
        id: "1a2b3c4d".into(),
        posted_at: 1_000,
        success,
        error: error.map(str::to_owned),
        token_hash: "0123456789abcdef".repeat(4),
        owner: "octocat".into(),
        repo: "hello-world".into(),
        pr_number: 1,
        body: "Hi".into(),
    }
}

#[test]
fn output_failures_are_returned_to_the_caller() {
    let mut output = FailingWriter;
    let err = render_response(&Command::List, None, Some(vec![]), None, 1_100, &mut output)
        .expect_err("non-broken output failure must surface");
    assert!(
        matches!(err, ClientError::Output(error) if error.kind() == io::ErrorKind::PermissionDenied)
    );
}

fn render_to_string(
    command: &Command,
    entry: Option<PendingEntry>,
    entries: Option<Vec<PendingEntry>>,
    history: Option<Vec<comenq_lib::protocol::HistoryEntry>>,
) -> String {
    let mut output = Vec::new();
    render_response(command, entry, entries, history, 1_100, &mut output).expect("render response");
    String::from_utf8(output).expect("rendered output is UTF-8")
}

#[test]
fn renders_put_output_exactly() {
    let output = render_to_string(
        &Command::Put {
            repo_slug: "octocat/hello-world".parse().expect("slug"),
            pr_number: 1,
            comment_body: "Hi".into(),
            now: false,
        },
        Some(sample_entry()),
        None,
        None,
    );
    assert_eq!(
        output,
        "Queued 1a2b3c4d for octocat/hello-world#1 — posts in ~now\n"
    );
}

#[test]
fn renders_non_empty_list_output_exactly() {
    let mut second = sample_entry();
    second.id = "deadbeef".into();
    second.eta_seconds = 90;
    second.body = "Later".into();
    let output = render_to_string(
        &Command::List,
        None,
        Some(vec![sample_entry(), second]),
        None,
    );
    assert_eq!(
        output,
        concat!(
            "1a2b3c4d      now  octocat/hello-world#1  Hi\n",
            "deadbeef   1m 30s  octocat/hello-world#1  Later\n",
        )
    );
}

#[test]
fn renders_empty_list_output_exactly() {
    let output = render_to_string(&Command::List, None, Some(vec![]), None);
    assert_eq!(output, "No comments queued.\n");
}

#[test]
fn renders_empty_history_output_exactly() {
    let output = render_to_string(&Command::Hist { limit: None }, None, None, Some(vec![]));
    assert_eq!(output, "No posting history recorded.\n");
}

#[test]
fn renders_success_and_failure_history_lines() {
    let output = render_to_string(
        &Command::Hist { limit: None },
        None,
        None,
        Some(vec![
            sample_history(true, None),
            sample_history(false, Some("timeout\nafter 10s")),
        ]),
    );
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("ok    octocat/hello-world#1  Hi"));
    assert!(lines[0].contains("1m 40s ago"));
    assert!(lines[1].contains("FAIL  octocat/hello-world#1  Hi"));
    assert!(lines[1].ends_with("— timeout after 10s"));
}

#[test]
fn renders_bump_output_exactly() {
    let output = render_to_string(
        &Command::Bump {
            id: "1a2b3c4d".into(),
        },
        None,
        None,
        None,
    );
    assert_eq!(output, "Moved 1a2b3c4d to the head of the queue.\n");
}

#[test]
fn renders_bust_output_exactly() {
    let output = render_to_string(
        &Command::Bust {
            id: "1a2b3c4d".into(),
        },
        None,
        None,
        None,
    );
    assert_eq!(output, "Moved 1a2b3c4d to the tail of the queue.\n");
}

#[test]
fn renders_del_output_exactly() {
    let output = render_to_string(
        &Command::Del {
            id: "1a2b3c4d".into(),
        },
        None,
        None,
        None,
    );
    assert_eq!(output, "Removed 1a2b3c4d from the queue.\n");
}
