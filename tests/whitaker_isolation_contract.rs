//! Contract for the Whitaker isolation the development build standard asks for.
//!
//! Whitaker runs under the toolchain its installer manages, so `make lint` assigns it the literal
//! `RUSTFLAGS="-D warnings"`, which displaces every `rustflags` source (no parallel frontend flag,
//! no `mold`) and inherits nothing from the caller. These tests read the real `make -n lint`
//! output, so a recipe that drops the assignment fails here.

use std::process::Command;

use rstest::rstest;

/// The warning-only `RUSTFLAGS` assignment that isolates Whitaker, with its trailing space.
const LITERAL_RUSTFLAGS: &str = "RUSTFLAGS=\"-D warnings\" ";

/// Returns whether a command, after any leading `NAME=value` assignments, runs the Whitaker
/// executable: the first word's basename is `whitaker` or a versioned `whitaker-*` name, so a
/// `cargo` path that merely contains the word does not match.
fn runs_whitaker(command: &str) -> bool {
    let mut words = command
        .split_whitespace()
        .skip_while(|word| word.contains('='));
    words.next().is_some_and(|binary| {
        let name = binary.rsplit('/').next().unwrap_or(binary);
        name == "whitaker" || name.starts_with("whitaker-")
    })
}

/// Returns the `make -n lint` line that runs Whitaker, or an empty line when `make` fails or the
/// recipe runs none, which `isolation_problems` then reports.
fn whitaker_line() -> String {
    let output = Command::new("make")
        .args(["-n", "-B", "lint"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default();
    output
        .lines()
        .find(|line| {
            line.split("&&").any(|command| {
                command
                    .trim()
                    .strip_prefix(LITERAL_RUSTFLAGS)
                    .is_some_and(runs_whitaker)
            })
        })
        .unwrap_or_default()
        .to_owned()
}

/// Returns the complaints about a Whitaker line that is not isolated: it must clear an inherited
/// `CARGO_ENCODED_RUSTFLAGS`, which outranks `RUSTFLAGS`, and then assign the literal warning-only
/// `RUSTFLAGS`, which displaces every `rustflags` source and inherits nothing from the caller.
fn isolation_problems(line: &str) -> Vec<String> {
    let commands: Vec<&str> = line.split("&&").map(str::trim).collect();
    let clear_at = commands
        .iter()
        .position(|command| *command == "unset CARGO_ENCODED_RUSTFLAGS");
    let whitaker_at = commands.iter().position(|command| {
        command
            .strip_prefix(LITERAL_RUSTFLAGS)
            .is_some_and(runs_whitaker)
    });
    let mut problems = Vec::new();
    // The clear must run before Whitaker starts, or the encoded flags are still in force then.
    if clear_at.is_none_or(|clear| whitaker_at.is_some_and(|whitaker| clear > whitaker)) {
        problems.push(format!(
            "the Whitaker command must clear `CARGO_ENCODED_RUSTFLAGS` first: {line}"
        ));
    }
    if whitaker_at.is_none() {
        problems.push(format!(
            "the Whitaker command must assign the literal `RUSTFLAGS=\"-D warnings\"`: {line}"
        ));
    }
    problems
}

/// Scenario: the real `make -n lint` output.
///
/// Invariant: Whitaker clears the encoded flags and assigns the literal `RUSTFLAGS`, so it takes none of
/// the development standard and inherits nothing from the caller.
#[test]
fn make_lint_isolates_whitaker_with_the_literal_rustflags() {
    let problems = isolation_problems(&whitaker_line());
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// Scenario: Whitaker commands that assign no, inherited or development `RUSTFLAGS`.
///
/// Invariant: only the cleared, literal warning-only assignment passes.
#[rstest]
#[case::isolated(
    "unset CARGO_ENCODED_RUSTFLAGS && RUSTFLAGS=\"-D warnings\" whitaker --all -- --all-targets",
    true
)]
#[case::versioned_installer_name(
    "unset CARGO_ENCODED_RUSTFLAGS && RUSTFLAGS=\"-D warnings\" whitaker-0.2.9 --all",
    true
)]
#[case::a_path_that_only_contains_the_word(
    "unset CARGO_ENCODED_RUSTFLAGS && RUSTFLAGS=\"-D warnings\" /home/whitaker/.cargo/bin/cargo clippy",
    false
)]
#[case::encoded_flags_cleared_after_whitaker(
    "RUSTFLAGS=\"-D warnings\" whitaker --all -- --all-targets && unset CARGO_ENCODED_RUSTFLAGS",
    false
)]
#[case::encoded_flags_not_cleared(
    "RUSTFLAGS=\"-D warnings\" whitaker --all -- --all-targets",
    false
)]
#[case::none("whitaker --all", false)]
#[case::development_flags("RUSTFLAGS=\"-D warnings -Zthreads=8\" whitaker --all", false)]
#[case::inherited("RUSTFLAGS=\"$RUSTFLAGS -D warnings\" whitaker --all", false)]
fn only_the_literal_rustflags_isolate_whitaker(#[case] line: &str, #[case] isolated: bool) {
    assert_eq!(
        isolation_problems(line).is_empty(),
        isolated,
        "isolated should be {isolated}: {line}"
    );
}
