#![expect(clippy::expect_used, reason = "tests use expect for brevity")]

mod support;

use std::time::Duration;
use support::fs::wait_for_path;
use tempfile::tempdir;
use tokio::fs;
use tokio::time::sleep;

#[tokio::test]
async fn returns_true_when_path_preexists() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("file");
    fs::write(&path, b"content").await.expect("create file");

    let found = wait_for_path(&path, 10).await;
    assert!(found, "should detect existing path");
}

#[tokio::test]
async fn returns_true_when_path_created_later() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("file");

    let creator = tokio::spawn({
        let path = path.clone();
        async move {
            sleep(Duration::from_millis(20)).await;
            fs::write(&path, b"content").await.expect("create file");
        }
    });

    let found = wait_for_path(&path, 100).await;
    creator.await.expect("create task");
    assert!(found, "should detect newly created path");
}

#[tokio::test]
async fn returns_false_when_timeout_expires() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("file");

    let found = wait_for_path(&path, 30).await;
    assert!(!found, "no file should be detected");
}
