mod steps;
use cucumber::World as _;
use steps::CommentWorld;

#[tokio::main]
async fn main() {
    CommentWorld::run("tests/features").await;
}
