//! CLI client for the Comenq service.
//! Parses user input and forwards it to the daemon.

use clap::Parser;
use comenq::{Args, run};
use std::process;

#[tokio::main]
async fn main() {
    let args = Args::parse();
    if let Err(e) = run(args).await {
        eprintln!("{e}");
        process::exit(1);
    }
}
