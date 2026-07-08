//! Cucumber test entry point.
//!
//! This module spawns all test worlds concurrently so scenarios run in
//! parallel.

mod steps;
use cucumber::World as _;
use steps::{
    CliWorld, ClientWorld, CommentWorld, ConfigWorld, ListenerWorld, PackagingWorld, ReleaseWorld,
    WorkerWorld,
};

fn main() -> anyhow::Result<()> {
    // Build the runtime explicitly so runtime construction errors propagate
    // instead of panicking inside the `#[tokio::main]` expansion.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        tokio::join!(
            CliWorld::run("tests/features/cli.feature"),
            ReleaseWorld::run("tests/features/release.feature"),
            ClientWorld::run("tests/features/client_main.feature"),
            CommentWorld::run("tests/features/comment_request.feature"),
            ConfigWorld::run("tests/features/config.feature"),
            ListenerWorld::run("tests/features/listener.feature"),
            PackagingWorld::run("tests/features/packaging.feature"),
            WorkerWorld::run("tests/features/worker.feature"),
        );
    });
    Ok(())
}
