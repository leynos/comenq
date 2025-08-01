mod steps;
mod util;
mod support;
mod util;
use cucumber::World as _;
use steps::{
    CliWorld, ClientWorld, CommentWorld, ConfigWorld, ListenerWorld, PackagingWorld, WorkerWorld,
};

#[tokio::main]
async fn main() {
    tokio::join!(
        CliWorld::run("tests/features/cli.feature"),
        ClientWorld::run("tests/features/client_main.feature"),
        CommentWorld::run("tests/features/comment_request.feature"),
        ConfigWorld::run("tests/features/config.feature"),
        ListenerWorld::run("tests/features/listener.feature"),
        PackagingWorld::run("tests/features/packaging.feature"),
        WorkerWorld::run("tests/features/worker.feature"),
    );
}
