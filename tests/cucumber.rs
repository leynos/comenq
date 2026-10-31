//! Cucumber test entry point.
//!
//! All worlds run concurrently. Environment-dependent behaviour uses injected
//! `mockable::Env` providers, so no world mutates process-wide state.

mod steps;
use cucumber::World as _;
use steps::{
    AwaitWorld, CliWorld, ClientWorld, CommentWorld, ConfigWorld, HistoryWorld, ListenerWorld,
    PackagingWorld, QueueWorld, ReleaseWorld, WorkerWorld,
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
            AwaitWorld::run("tests/features/await.feature"),
            ReleaseWorld::run("tests/features/release.feature"),
            CommentWorld::run("tests/features/comment_request.feature"),
            ListenerWorld::run("tests/features/listener.feature"),
            PackagingWorld::run("tests/features/packaging.feature"),
            HistoryWorld::run("tests/features/history.feature"),
            QueueWorld::run("tests/features/queue.feature"),
            WorkerWorld::run("tests/features/worker.feature"),
            CliWorld::run("tests/features/cli.feature"),
            ClientWorld::run("tests/features/client_main.feature"),
            ConfigWorld::run("tests/features/config.feature"),
        );
    });
    Ok(())
}
