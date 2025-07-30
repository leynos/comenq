#![expect(clippy::expect_used, reason = "simplify test failure output")]

mod support;

use support::fs::wait_for_path;
use tempfile::tempdir;
use tokio::fs::File;
use tokio::time::{Duration, sleep};

#[tokio::test]
async fn returns_true_when_file_exists() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("exists");
    File::create(&file_path).await.expect("create");

    assert!(wait_for_path(&file_path, 50).await);
}

#[tokio::test]
async fn returns_true_when_file_appears_later() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("delayed");
    let path_clone = file_path.clone();
    let create_task = tokio::spawn(async move {
        sleep(Duration::from_millis(20)).await;
        File::create(path_clone).await.expect("create");
    });

    let found = wait_for_path(&file_path, 100).await;
    create_task.await.expect("join");

    assert!(found);
}

#[tokio::test]
async fn returns_false_when_file_never_appears() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("missing");

    assert!(!wait_for_path(&file_path, 20).await);
}
