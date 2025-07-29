mod steps;
mod support;
use cucumber::World as _;
use steps::{CliWorld, ClientWorld, CommentWorld, ConfigWorld, ListenerWorld, WorkerWorld};

#[tokio::main]
async fn main() {
    tokio::join!(
        CliWorld::run("tests/features/cli.feature"),
        ClientWorld::run("tests/features/client_main.feature"),
        CommentWorld::run("tests/features/comment_request.feature"),
        ConfigWorld::run("tests/features/config.feature"),
        ListenerWorld::run("tests/features/listener.feature"),
        WorkerWorld::run("tests/features/worker.feature"),
    );
}
