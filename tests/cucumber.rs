mod steps;
use cucumber::World as _;
use steps::{CliWorld, CommentWorld};

#[tokio::main]
async fn main() {
    tokio::join!(
        CommentWorld::run("tests/features/comment_request.feature"),
        CliWorld::run("tests/features/cli.feature")
    );
}
