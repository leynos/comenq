//! CLI client for the Comenq service.
//! Parses user input and forwards it to the daemon.

use clap::Parser;
use comenq::{Args, run};
use std::process;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    let args = Args::parse();
    if let Err(e) = run(args).await {
        eprintln!("{e}");
        process::exit(1);
    }
}
