//! Cucumber test entry point.
//!
//! This module runs independent worlds concurrently and serializes worlds
//! that mutate process-wide environment variables.

mod steps;
use cucumber::World as _;
use steps::{
    CliWorld, ClientWorld, CommentWorld, ConfigWorld, ListenerWorld, PackagingWorld, ReleaseWorld,
    WorkerWorld,
};

fn main() -> anyhow::Result<()> {
    use anyhow::Context as _;

    // Build the runtime explicitly so runtime construction errors propagate
    // instead of panicking inside the `#[tokio::main]` expansion.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to build the Tokio runtime for the cucumber test binary")?;
    runtime.block_on(async {
        tokio::join!(
            CliWorld::run("tests/features/cli.feature"),
            ReleaseWorld::run("tests/features/release.feature"),
            CommentWorld::run("tests/features/comment_request.feature"),
            ListenerWorld::run("tests/features/listener.feature"),
            PackagingWorld::run("tests/features/packaging.feature"),
            WorkerWorld::run("tests/features/worker.feature"),
        );

        // Both worlds temporarily set XDG_RUNTIME_DIR, which is process-wide.
        // Run them separately so their socket-discovery assertions cannot race.
        ClientWorld::run("tests/features/client_main.feature").await;
        ConfigWorld::run("tests/features/config.feature").await;
    });
    Ok(())
}
