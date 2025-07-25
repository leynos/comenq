mod steps;
use cucumber::World as _;
use steps::{CliWorld, ClientWorld, CommentWorld};

#[tokio::main]
async fn main() {
    tokio::join!(
        CommentWorld::run("tests/features/comment_request.feature"),
        CliWorld::run("tests/features/cli.feature"),
        ClientWorld::run("tests/features/client_main.feature")
    );
}
