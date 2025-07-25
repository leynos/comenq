//! CLI client for the Comenq service.
//! Parses user input and forwards it to the daemon.

use clap::Parser;
use comenq::Args;

fn main() {
    let _args = Args::parse();
    println!("Hello from Comenq!");
}
