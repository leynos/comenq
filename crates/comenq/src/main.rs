//! CLI client for the Comenq service.
//! Parses user input and forwards it to the daemon.

use clap::Parser;
use comenq::Args;

fn main() {
    let args = Args::parse();
    todo!(
        "Connect to daemon at {} to enqueue comment for {}",
        args.socket.display(),
        args.repo_slug
    );
}
