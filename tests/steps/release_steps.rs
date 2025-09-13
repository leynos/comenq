//! Behavioural steps for the release workflow.

use cucumber::{World, given, then, when};
use regex::Regex;
use serde_yaml::Value;
use std::fs;
use test_support::uses_goreleaser as workflow_uses_goreleaser;

#[derive(Debug, Default, World)]
pub struct ReleaseWorld {
    content: Option<String>,
    yaml: Option<Value>,
}

#[given("the release workflow file")]
#[expect(clippy::expect_used, reason = "simplify test failure output")]
fn the_workflow_file(world: &mut ReleaseWorld) {
    let text = fs::read_to_string(".github/workflows/release.yml").expect("read workflow");
    world.content = Some(text);
}

#[when("it is parsed as YAML")]
#[expect(clippy::expect_used, reason = "simplify test failure output")]
fn parse_yaml(world: &mut ReleaseWorld) {
    let text = world.content.as_deref().expect("file loaded");
    world.yaml = Some(serde_yaml::from_str(text).expect("parse yaml"));
}

#[then("the workflow uses goreleaser")]
#[expect(clippy::expect_used, reason = "simplify test failure output")]
fn assert_uses_goreleaser(world: &mut ReleaseWorld) {
    let content = world.content.as_ref().expect("file still loaded");
    assert!(workflow_uses_goreleaser(content).expect("parse"));
}

#[then("the workflow triggers on tags")]
#[expect(clippy::expect_used, reason = "simplify test failure output")]
fn triggers_on_tags(world: &mut ReleaseWorld) {
    let yaml = world.yaml.as_ref().expect("yaml parsed");
    let on = yaml.get("on").expect("on");
    let push = on.get("push").expect("push");
    let tags = push
        .get("tags")
        .expect("tags")
        .as_sequence()
        .expect("sequence");
    let pattern = Regex::new(r"^v\*\.\*\.\*$").expect("compile regex");
    assert!(
        tags.iter()
            .filter_map(|t| t.as_str())
            .any(|t| pattern.is_match(t)),
        "missing semantic version tag pattern",
    );
}
