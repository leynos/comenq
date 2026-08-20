//! Compile-fail fixture proving that `Args::socket` is an optional path.

use comenq::Args;
use std::path::PathBuf;

fn main() {
    let args = Args {
        repo_slug: "octocat/hello-world".parse().expect("parse repository slug"),
        pr_number: 1,
        comment_body: "Hello".into(),
        socket: None,
    };
    let _: PathBuf = args.socket;
}
