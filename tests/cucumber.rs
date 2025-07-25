mod steps;
use cucumber::World as _;
use steps::{CliWorld, CommentWorld};

#[tokio::main]
async fn main() {
    CommentWorld::run("tests/features/comment_request.feature").await;
    CliWorld::run("tests/features/cli.feature").await;
}
