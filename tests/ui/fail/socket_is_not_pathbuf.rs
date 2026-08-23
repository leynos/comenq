//! Compile-fail fixture proving that `Args::socket` is an optional path.

use comenq::{Args, Command};
use std::path::PathBuf;

fn main() {
    let args = Args {
        socket: None,
        command: Command::Put {
            repo_slug: "octocat/hello-world".parse().expect("parse repository slug"),
            pr_number: 1,
            comment_body: "Hello".into(),
            now: false,
        },
    };
    let _: PathBuf = args.socket;
}
