//! CLI client for the Comenq service.
//! Parses user input and forwards it to the daemon.

use clap::Parser;
use comenq::{Args, run};
use std::process;
use tracing_subscriber::EnvFilter;

/// Build the runtime explicitly so a failure is reported rather than a panic.
fn main() {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("failed to build the Tokio runtime: {error}");
            process::exit(1);
        }
    };
    if let Err(e) = runtime.block_on(run_cli()) {
        eprintln!("{e}");
        process::exit(1);
    }
}

/// Initialise logging, parse the arguments and run the client.
async fn run_cli() -> Result<(), comenq::ClientError> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    let args = Args::parse();
    run(args).await
}
