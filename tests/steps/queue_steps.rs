//! Behavioural test steps for queue management operations.
//!
//! Exercises the daemon's protocol surface — put, list, bump, bust, and
//! del — against a real `SharedQueue`, plus the client's one-line summary
//! rendering for listings.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context as _;
use comenq_lib::CommentRequest;
use comenq_lib::protocol::{PendingEntry, Request, Response};
use comenqd::config::Config;
use comenqd::daemon::SharedQueue;
use cucumber::{World, given, then, when};
use tempfile::TempDir;
use test_support::temp_config;

/// Cooldown used by these scenarios, in seconds.
///
/// Long enough that nothing is ever due during a test run, so listings are
/// stable, and distinct entries have strictly increasing estimated times.
const COOLDOWN_SECONDS: u64 = 600;

#[derive(Default, World)]
pub struct QueueWorld {
    dir: Option<TempDir>,
    queue: Option<Arc<SharedQueue>>,
    /// Identifier of each queued comment, keyed by its body text.
    ids: HashMap<String, String>,
    last_put: Option<PendingEntry>,
    last_response: Option<Response>,
}

impl std::fmt::Debug for QueueWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueueWorld").finish()
    }
}

impl QueueWorld {
    fn queue(&self) -> anyhow::Result<&Arc<SharedQueue>> {
        self.queue.as_ref().context("queue not initialized")
    }

    fn id_for(&self, body: &str) -> anyhow::Result<&str> {
        self.ids
            .get(body)
            .map(String::as_str)
            .with_context(|| format!("no queued comment with body '{body}'"))
    }

    async fn put(&mut self, body: &str) -> anyhow::Result<PendingEntry> {
        let queue = self.queue()?.clone();
        let response = queue
            .execute(Request::Put {
                request: CommentRequest {
                    owner: "octocat".into(),
                    repo: "hello-world".into(),
                    pr_number: 7,
                    body: body.into(),
                },
            })
            .await;
        let Response::Ok {
            entry: Some(entry), ..
        } = response
        else {
            anyhow::bail!("expected put reply with entry, got {response:?}");
        };
        self.ids.insert(body.to_owned(), entry.id.clone());
        Ok(entry)
    }

    async fn entries(&self) -> anyhow::Result<Vec<PendingEntry>> {
        let response = self.queue()?.execute(Request::List).await;
        match response {
            Response::Ok {
                entries: Some(entries),
                ..
            } => Ok(entries),
            other => anyhow::bail!("expected list reply, got {other:?}"),
        }
    }
}

#[given("an empty comment queue")]
fn empty_queue(world: &mut QueueWorld) -> anyhow::Result<()> {
    let dir = TempDir::new().context("tempdir")?;
    let cfg = Arc::new(Config::from(
        temp_config(&dir).with_cooldown(COOLDOWN_SECONDS),
    ));
    world.queue = Some(SharedQueue::open(cfg).context("open queue")?);
    world.dir = Some(dir);
    Ok(())
}

#[given(regex = r#"^the comments \"(.+)\", \"(.+)\" and \"(.+)\" are queued$"#)]
async fn comments_are_queued(
    world: &mut QueueWorld,
    first: String,
    second: String,
    third: String,
) -> anyhow::Result<()> {
    for body in [first, second, third] {
        world.put(&body).await?;
    }
    Ok(())
}

#[when(regex = r#"^the comment \"(.+)\" is put$"#)]
async fn comment_is_put(world: &mut QueueWorld, body: String) -> anyhow::Result<()> {
    let entry = world.put(&body).await?;
    world.last_put = Some(entry);
    Ok(())
}

#[when(regex = r#"^a comment of ([0-9]+) \"(.)\" characters is put$"#)]
async fn long_comment_is_put(
    world: &mut QueueWorld,
    count: usize,
    character: String,
) -> anyhow::Result<()> {
    let body = character.repeat(count);
    let entry = world.put(&body).await?;
    world.last_put = Some(entry);
    Ok(())
}

#[when(regex = r#"^the comment \"(.+)\" is (bumped|busted|deleted)$"#)]
async fn comment_is_moved(
    world: &mut QueueWorld,
    body: String,
    operation: String,
) -> anyhow::Result<()> {
    let id = world.id_for(&body)?.to_owned();
    let request = match operation.as_str() {
        "bumped" => Request::Bump { id },
        "busted" => Request::Bust { id },
        "deleted" => Request::Del { id },
        other => anyhow::bail!("unsupported operation '{other}'"),
    };
    let response = world.queue()?.execute(request).await;
    anyhow::ensure!(
        matches!(response, Response::Ok { .. }),
        "operation should succeed, got {response:?}"
    );
    Ok(())
}

#[when(regex = r#"^the unknown identifier \"(.+)\" is bumped$"#)]
async fn unknown_id_is_bumped(world: &mut QueueWorld, id: String) -> anyhow::Result<()> {
    let response = world.queue()?.execute(Request::Bump { id }).await;
    world.last_response = Some(response);
    Ok(())
}

#[then("the reply carries an eight character identifier")]
fn reply_has_identifier(world: &mut QueueWorld) -> anyhow::Result<()> {
    let entry = world.last_put.as_ref().context("no put reply recorded")?;
    assert_eq!(entry.id.len(), 8, "identifier '{}' length", entry.id);
    assert!(entry.id.chars().all(|c| c.is_ascii_hexdigit()));
    Ok(())
}

#[then("the reply reports an immediate ETA")]
fn reply_reports_eta(world: &mut QueueWorld) -> anyhow::Result<()> {
    let entry = world.last_put.as_ref().context("no put reply recorded")?;
    // Nothing has ever been posted, so the queue head is due immediately.
    assert_eq!(entry.eta_seconds, 0);
    Ok(())
}

#[then("listing shows the same identifier as the put reply")]
async fn listing_matches_put(world: &mut QueueWorld) -> anyhow::Result<()> {
    let entry = world.last_put.clone().context("no put reply recorded")?;
    let entries = world.entries().await?;
    assert_eq!(entries.len(), 1);
    let listed = entries.first().context("listing should hold the comment")?;
    assert_eq!(listed.id, entry.id);
    Ok(())
}

#[then(regex = r"^listing shows ([0-9]+) entries with strictly increasing ETAs$")]
async fn listing_shows_schedule(world: &mut QueueWorld, expected: usize) -> anyhow::Result<()> {
    let entries = world.entries().await?;
    assert_eq!(entries.len(), expected);
    for (earlier, later) in entries.iter().zip(entries.iter().skip(1)) {
        assert!(
            earlier.eta_seconds < later.eta_seconds,
            "ETAs should increase: {} then {}",
            earlier.eta_seconds,
            later.eta_seconds,
        );
    }
    Ok(())
}

#[then(regex = r"^the listed body summarizes to one line of ([0-9]+) characters$")]
async fn listed_body_truncates(world: &mut QueueWorld, limit: usize) -> anyhow::Result<()> {
    let entries = world.entries().await?;
    let entry = entries.first().context("queue should hold the comment")?;
    let summary = comenq::one_line_summary(&entry.body);
    assert_eq!(summary.chars().count(), limit);
    assert!(summary.ends_with('…'));
    assert!(!summary.contains('\n'));
    Ok(())
}

#[then(regex = r#"^the queue order is \"([^\"]+)\", \"([^\"]+)\", \"([^\"]+)\"$"#)]
async fn queue_order_is_three(
    world: &mut QueueWorld,
    first: String,
    second: String,
    third: String,
) -> anyhow::Result<()> {
    assert_queue_order(world, &[first, second, third]).await
}

#[then(regex = r#"^the queue order is \"([^\"]+)\", \"([^\"]+)\"$"#)]
async fn queue_order_is_two(
    world: &mut QueueWorld,
    first: String,
    second: String,
) -> anyhow::Result<()> {
    assert_queue_order(world, &[first, second]).await
}

async fn assert_queue_order(world: &mut QueueWorld, bodies: &[String]) -> anyhow::Result<()> {
    let entries = world.entries().await?;
    let listed: Vec<&str> = entries.iter().map(|entry| entry.body.as_str()).collect();
    assert_eq!(
        listed,
        bodies.iter().map(String::as_str).collect::<Vec<_>>()
    );
    Ok(())
}

#[then("the daemon reports an unknown identifier error")]
fn daemon_reports_error(world: &mut QueueWorld) -> anyhow::Result<()> {
    match world.last_response.take() {
        Some(Response::Error { message }) => {
            assert!(
                message.contains("deadbeef"),
                "error should name the identifier: {message}"
            );
        }
        other => anyhow::bail!("expected error reply, got {other:?}"),
    }
    Ok(())
}
