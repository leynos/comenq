//! Behavioural steps for the release workflow.

use anyhow::Context as _;
use cucumber::{World, given, then, when};
use regex::Regex;
use serde_yaml::Value;
use std::fs;
use test_support::uses_shared_release_actions as workflow_uses_shared_actions;

#[derive(Debug, Default, World)]
pub struct ReleaseWorld {
    content: Option<String>,
    yaml: Option<Value>,
}

#[given("the release workflow file")]
fn the_workflow_file(world: &mut ReleaseWorld) -> anyhow::Result<()> {
    let text = fs::read_to_string(".github/workflows/release.yml").context("read workflow")?;
    world.content = Some(text);
    Ok(())
}

#[when("it is parsed as YAML")]
fn parse_yaml(world: &mut ReleaseWorld) -> anyhow::Result<()> {
    let text = world.content.as_deref().context("file loaded")?;
    world.yaml = Some(serde_yaml::from_str(text).context("parse yaml")?);
    Ok(())
}

#[then("the workflow uses the shared release actions")]
fn assert_uses_shared_actions(world: &mut ReleaseWorld) -> anyhow::Result<()> {
    let content = world.content.as_ref().context("file still loaded")?;
    assert!(workflow_uses_shared_actions(content).context("parse")?);
    Ok(())
}

#[then("the workflow triggers on tags")]
fn triggers_on_tags(world: &mut ReleaseWorld) -> anyhow::Result<()> {
    let yaml = world.yaml.as_ref().context("yaml parsed")?;
    let on = yaml.get("on").context("on")?;
    let push = on.get("push").context("push")?;
    let tags = push
        .get("tags")
        .context("tags")?
        .as_sequence()
        .context("sequence")?;
    let pattern = Regex::new(r"^v\*\.\*\.\*$").context("compile regex")?;
    assert!(
        tags.iter()
            .filter_map(|t| t.as_str())
            .any(|t| pattern.is_match(t)),
        "missing semantic version tag pattern",
    );
    Ok(())
}
