//! Library utilities for the `comenq` CLI.

use clap::{Parser, builder::ValueHint};
use std::{fmt, path::PathBuf, str::FromStr};
use thiserror::Error;

mod client;

pub use client::{ClientError, run};

/// A GitHub repository slug in `owner/repo` format.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RepoSlug {
    /// Repository owner.
    owner: String,
    /// Repository name.
    repo: String,
}

impl RepoSlug {
    /// Repository owner.
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// Repository name.
    pub fn repo(&self) -> &str {
        &self.repo
    }
}

/// Error returned when parsing a [`RepoSlug`] fails.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RepoSlugParseError {
    /// Missing slash separator.
    #[error("invalid repository format, use 'owner/repo'")]
    MissingSlash,
    /// Owner segment is empty.
    #[error("invalid repository format, use 'owner/repo'")]
    EmptyOwner,
    /// Repository segment is empty.
    #[error("invalid repository format, use 'owner/repo'")]
    EmptyRepo,
    /// Extra slash found in repository segment.
    #[error("invalid repository format, use 'owner/repo'")]
    ExtraSlashes,
}

impl FromStr for RepoSlug {
    type Err = RepoSlugParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let (owner, repo) = s.split_once('/').ok_or(RepoSlugParseError::MissingSlash)?;
        if owner.is_empty() {
            return Err(RepoSlugParseError::EmptyOwner);
        }
        if repo.is_empty() {
            return Err(RepoSlugParseError::EmptyRepo);
        }
        if repo.contains('/') {
            return Err(RepoSlugParseError::ExtraSlashes);
        }
        Ok(Self {
            owner: owner.to_owned(),
            repo: repo.to_owned(),
        })
    }
}

impl fmt::Display for RepoSlug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.repo)
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
    #[arg(
        long,
        value_hint = ValueHint::FilePath,
        default_value_os_t = PathBuf::from(comenq_lib::DEFAULT_SOCKET_PATH),
    )]
    pub socket: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::{Args, RepoSlug, RepoSlugParseError};
    use clap::Parser;
    use rstest::rstest;
    use std::path::PathBuf;

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

    #[rstest]
    #[case("octocat", RepoSlugParseError::MissingSlash)]
    #[case("/repo", RepoSlugParseError::EmptyOwner)]
    #[case("owner/", RepoSlugParseError::EmptyRepo)]
    #[case("owner/repo/extra", RepoSlugParseError::ExtraSlashes)]
    fn from_str_rejects_invalid_inputs(#[case] input: &str, #[case] expected: RepoSlugParseError) {
        let err = input
            .parse::<RepoSlug>()
            .expect_err("invalid slug should fail");
        assert_eq!(err, expected);
    }

    #[test]
    fn display_round_trips() {
        let slug: RepoSlug = "octocat/hello".parse().expect("slug parses");
        assert_eq!(slug.to_string(), "octocat/hello");
    }

    #[test]
    fn trims_whitespace() {
        let slug: RepoSlug = "  octocat/hello-world  ".parse().expect("slug parses");
        assert_eq!(slug.owner(), "octocat");
        assert_eq!(slug.repo(), "hello-world");
    }

    #[test]
    fn socket_default_matches_constant() {
        let args = Args::try_parse_from(["comenq", "octocat/hello-world", "1", "Hi"])
            .expect("valid arguments should parse");
        assert_eq!(args.socket, PathBuf::from(comenq_lib::DEFAULT_SOCKET_PATH));
    }
}
