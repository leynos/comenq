//! Library utilities for the `comenq` CLI.

use clap::Parser;
use std::path::PathBuf;

mod client;

pub use client::{ClientError, run};

/// Command line arguments for the `comenq` client.
#[derive(Debug, Clone, Parser)]
#[command(name = "comenq", about = "Enqueue a GitHub PR comment")]
pub struct Args {
    /// The repository in 'owner/repo' format (e.g., "rust-lang/rust").
    #[arg(value_parser = validate_repo_slug)]
    pub repo_slug: String,

    /// The pull request number to comment on.
    pub pr_number: u64,

    /// The body of the comment. It is recommended to quote this argument.
    pub comment_body: String,

    /// Path to the daemon's Unix Domain Socket.
    #[arg(long, default_value = "/run/comenq/socket")]
    pub socket: PathBuf,
}

fn validate_repo_slug(s: &str) -> Result<String, String> {
    let parts: Vec<&str> = s.split('/').collect();
    if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
        Ok(s.to_owned())
    } else {
        Err(String::from("invalid repository format, use 'owner/repo'"))
    }
}

#[cfg(test)]
mod tests {
    use super::Args;
    use clap::Parser;
    use rstest::{case, rstest};

    #[rstest]
    #[case("octocat/hello-world", 1, "Hi")]
    fn parses_valid_arguments(#[case] slug: &str, #[case] pr: u64, #[case] body: &str) {
        let pr_str = pr.to_string();
        let args = Args::try_parse_from(["comenq", slug, &pr_str, body]);
        let args = args.expect("valid arguments should parse");
        assert_eq!(args.repo_slug, slug);
        assert_eq!(args.pr_number, pr);
        assert_eq!(args.comment_body, body);
    }

    #[rstest]
    #[case("octocat")]
    #[case("/repo")]
    #[case("owner/")]
    #[case("owner/repo/extra")]
    fn rejects_invalid_slug(#[case] slug: &str) {
        let result = Args::try_parse_from(["comenq", slug, "1", "Hi"]);
        assert!(result.is_err());
    }
}
