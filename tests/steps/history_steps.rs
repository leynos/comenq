//! Behavioural test steps for the posting history.
//!
//! Exercises the daemon's `hist` protocol operation against a real
//! `SharedQueue`, simulating posting outcomes through the queue's
//! completion and failure-recording API.

use std::sync::Arc;

use anyhow::Context as _;
use comenq_lib::CommentRequest;
use comenq_lib::protocol::{HistoryEntry, Request, Response};
use comenqd::config::Config;
use comenqd::daemon::SharedQueue;
use comenqd::store::StoredEntry;
use cucumber::{World, given, then, when};
use tempfile::TempDir;
use test_support::temp_config;

/// Token hash recorded against simulated posting attempts.
const FIRST_TOKEN_HASH: &str = "feedbead0011";

#[derive(Default, World)]
pub struct HistoryWorld {
    dir: Option<TempDir>,
    queue: Option<Arc<SharedQueue>>,
    /// Identifiers in the order their posting attempts were recorded.
    recorded: Vec<String>,
    last_history: Option<Vec<HistoryEntry>>,
}

impl std::fmt::Debug for HistoryWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HistoryWorld").finish()
    }
}

impl HistoryWorld {
    fn queue(&self) -> anyhow::Result<&Arc<SharedQueue>> {
        self.queue.as_ref().context("queue not initialized")
    }

    async fn put(&mut self, body: &str) -> anyhow::Result<()> {
        let response = self
            .queue()?
            .execute(Request::Put {
                request: CommentRequest {
                    owner: "octocat".into(),
                    repo: "hello-world".into(),
                    pr_number: 7,
                    body: body.into(),
                },
                immediate: true,
            })
            .await;
        anyhow::ensure!(
            matches!(response, Response::Ok { .. }),
            "put should succeed, got {response:?}"
        );
        Ok(())
    }

    async fn head(&self) -> anyhow::Result<StoredEntry> {
        let due = self.queue()?.next_due().await.context("next due")?;
        let (entry, _) = due.context("queue should hold a head entry")?;
        Ok(entry)
    }

    async fn request_history(&mut self, limit: Option<usize>) -> anyhow::Result<()> {
        let response = self.queue()?.execute(Request::Hist { limit }).await;
        match response {
            Response::Ok {
                history: Some(history),
                ..
            } => {
                self.last_history = Some(history);
                Ok(())
            }
            other => anyhow::bail!("expected hist reply, got {other:?}"),
        }
    }

    fn history(&self) -> anyhow::Result<&[HistoryEntry]> {
        self.last_history
            .as_deref()
            .context("no hist reply recorded")
    }
}

#[given("an empty posting queue")]
fn empty_queue(world: &mut HistoryWorld) -> anyhow::Result<()> {
    let dir = TempDir::new().context("tempdir")?;
    let cfg = Arc::new(Config::from(temp_config(&dir).with_cooldown(600)));
    world.queue = Some(SharedQueue::open(cfg).context("open queue")?);
    world.dir = Some(dir);
    Ok(())
}

#[given(regex = r#"^the comments \"([^\"]+)\" and \"([^\"]+)\" are queued for posting$"#)]
async fn two_comments_queued(
    world: &mut HistoryWorld,
    first: String,
    second: String,
) -> anyhow::Result<()> {
    for body in [first, second] {
        world.put(&body).await?;
    }
    Ok(())
}

#[given(
    regex = r#"^the comments \"([^\"]+)\", \"([^\"]+)\" and \"([^\"]+)\" are queued for posting$"#
)]
async fn three_comments_queued(
    world: &mut HistoryWorld,
    first: String,
    second: String,
    third: String,
) -> anyhow::Result<()> {
    for body in [first, second, third] {
        world.put(&body).await?;
    }
    Ok(())
}

#[when("the head comment is posted successfully")]
async fn head_posted(world: &mut HistoryWorld) -> anyhow::Result<()> {
    let entry = world.head().await?;
    world
        .queue()?
        .complete(&entry, FIRST_TOKEN_HASH)
        .await
        .context("complete")?;
    world.recorded.push(entry.id);
    Ok(())
}

#[when("each queued comment is posted successfully")]
async fn all_posted(world: &mut HistoryWorld) -> anyhow::Result<()> {
    while world
        .queue()?
        .next_due()
        .await
        .context("next due")?
        .is_some()
    {
        head_posted(world).await?;
    }
    Ok(())
}

#[when(regex = r#"^the head comment fails to post with \"([^\"]+)\"$"#)]
async fn head_fails(world: &mut HistoryWorld, error: String) -> anyhow::Result<()> {
    let entry = world.head().await?;
    world
        .queue()?
        .record_failure(&entry, FIRST_TOKEN_HASH, &error)
        .await
        .context("record failure")?;
    world.recorded.push(entry.id);
    Ok(())
}

#[when("the posting history is requested")]
async fn history_requested(world: &mut HistoryWorld) -> anyhow::Result<()> {
    world.request_history(None).await
}

#[when(regex = r"^the posting history is requested with a limit of ([0-9]+)$")]
async fn history_requested_with_limit(
    world: &mut HistoryWorld,
    limit: usize,
) -> anyhow::Result<()> {
    world.request_history(Some(limit)).await
}

#[then("the history lists nothing")]
fn history_is_empty(world: &mut HistoryWorld) -> anyhow::Result<()> {
    anyhow::ensure!(
        world.history()?.is_empty(),
        "expected an empty history, got {:?}",
        world.history()?
    );
    Ok(())
}

#[then(regex = r"^the history lists ([0-9]+) records$")]
fn history_has_len(world: &mut HistoryWorld, expected: usize) -> anyhow::Result<()> {
    assert_eq!(world.history()?.len(), expected);
    Ok(())
}

#[then(regex = r#"^the history records a success then a failure with \"([^\"]+)\"$"#)]
fn history_outcomes(world: &mut HistoryWorld, error: String) -> anyhow::Result<()> {
    let history = world.history()?;
    let success = history.first().context("history should hold a record")?;
    assert!(success.success, "the first record should be a success");
    assert_eq!(success.error, None);
    let failure = history.get(1).context("history should hold two records")?;
    assert!(!failure.success, "the second record should be a failure");
    assert_eq!(failure.error, Some(error));
    for (record, id) in history.iter().zip(&world.recorded) {
        assert_eq!(&record.id, id, "records should appear in posting order");
    }
    Ok(())
}

#[then(regex = r"^the history lists the ([0-9]+) most recent records$")]
fn history_is_most_recent(world: &mut HistoryWorld, expected: usize) -> anyhow::Result<()> {
    let history = world.history()?;
    assert_eq!(history.len(), expected);
    let listed: Vec<&str> = history.iter().map(|record| record.id.as_str()).collect();
    let skip = world.recorded.len().saturating_sub(expected);
    let recent: Vec<&str> = world
        .recorded
        .iter()
        .skip(skip)
        .map(String::as_str)
        .collect();
    assert_eq!(listed, recent, "the oldest records should be dropped");
    Ok(())
}
