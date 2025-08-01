//! Behavioural steps for the release workflow.
#![expect(clippy::expect_used, reason = "simplify test failure output")]

use cucumber::{World, given, then, when};
use serde_yaml::Value;
use std::fs;

#[derive(Debug, Default, World)]
pub struct ReleaseWorld {
    content: Option<String>,
    yaml: Option<Value>,
}

#[given("the release workflow file")]
fn the_workflow_file(world: &mut ReleaseWorld) {
    let text = fs::read_to_string(".github/workflows/release.yml").expect("read workflow");
    world.content = Some(text);
}

#[when("it is parsed as YAML")]
fn parse_yaml(world: &mut ReleaseWorld) {
    let text = world.content.take().expect("file loaded");
    world.yaml = Some(serde_yaml::from_str(&text).expect("parse yaml"));
}

#[then("the workflow uses goreleaser")]
fn uses_goreleaser(world: &mut ReleaseWorld) {
    let yaml = world.yaml.as_ref().expect("yaml parsed");
    let jobs = yaml.get("jobs").expect("jobs");
    let goreleaser = jobs.get("goreleaser").expect("goreleaser job");
    let steps = goreleaser
        .get("steps")
        .expect("steps")
        .as_sequence()
        .expect("sequence");
    assert!(steps.iter().any(|s| {
        s.get("uses")
            .and_then(|u| u.as_str())
            .is_some_and(|u| u.contains("goreleaser-action"))
    }));
}

#[then("the workflow triggers on tags")]
fn triggers_on_tags(world: &mut ReleaseWorld) {
    let yaml = world.yaml.as_ref().expect("yaml parsed");
    let on = yaml.get("on").expect("on");
    let push = on.get("push").expect("push");
    let tags = push
        .get("tags")
        .expect("tags")
        .as_sequence()
        .expect("sequence");
    assert!(tags.iter().any(|t| t.as_str() == Some("v*")));
}
