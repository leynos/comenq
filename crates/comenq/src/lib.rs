//! Library utilities for the `comenq` CLI.

use clap::Parser;
use std::{path::PathBuf, str::FromStr};

mod client;

pub use client::{ClientError, run};

/// A GitHub repository slug in `owner/repo` format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoSlug {
    /// Repository owner.
    pub owner: String,
    /// Repository name.
    pub repo: String,
}

impl FromStr for RepoSlug {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Perform the split once to avoid redundant parsing elsewhere.
        let (owner, repo) = s
            .split_once('/')
            .ok_or_else(|| String::from("invalid repository format, use 'owner/repo'"))?;
        if owner.is_empty() || repo.is_empty() || repo.contains('/') {
            return Err(String::from("invalid repository format, use 'owner/repo'"));
        }
        Ok(Self {
            owner: owner.to_owned(),
            repo: repo.to_owned(),
        })
    }
}

/// Command line arguments for the `comenq` client.
#[derive(Debug, Clone, Parser)]
#[command(name = "comenq", about = "Enqueue a GitHub PR comment")]
pub struct Args {
    /// The repository in 'owner/repo' format (e.g., "rust-lang/rust").
    pub repo_slug: RepoSlug,

    /// The pull request number to comment on.
    pub pr_number: u64,

    /// The body of the comment. It is recommended to quote this argument.
    pub comment_body: String,

    /// Path to the daemon's Unix Domain Socket.
    #[arg(long, default_value = "/run/comenq/socket")]
    pub socket: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::{Args, RepoSlug};
    use clap::Parser;
    use rstest::rstest;

    #[rstest]
    #[case("octocat/hello-world", 1, "Hi")]
    fn parses_valid_arguments(#[case] slug: &str, #[case] pr: u64, #[case] body: &str) {
        let pr_str = pr.to_string();
        let args = Args::try_parse_from(["comenq", slug, &pr_str, body]);
        let args = args.expect("valid arguments should parse");
        let expected: RepoSlug = slug.parse().expect("slug parses");
        assert_eq!(args.repo_slug, expected);
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
