//! Behavioural steps for packaging configuration.

use cucumber::{World, given, then, when};
use serde_yaml::Value;
use std::fs;

#[derive(Debug, Default, World)]
pub struct PackagingWorld {
    content: Option<String>,
    yaml: Option<Value>,
}

#[given("the goreleaser configuration file")]
#[expect(clippy::expect_used, reason = "simplify test failure output")]
fn the_goreleaser_file(world: &mut PackagingWorld) {
    let text = fs::read_to_string(".goreleaser.yaml").expect("read goreleaser");
    world.content = Some(text);
}

#[when("it is parsed as YAML")]
#[expect(clippy::expect_used, reason = "simplify test failure output")]
fn parse_yaml(world: &mut PackagingWorld) {
    let text = world.content.take().expect("file loaded");
    world.yaml = Some(serde_yaml::from_str(&text).expect("parse yaml"));
}

#[then("the nfpms section exists")]
#[expect(clippy::expect_used, reason = "simplify test failure output")]
fn nfpms_exists(world: &mut PackagingWorld) {
    let yaml = world.yaml.as_ref().expect("yaml parsed");
    assert!(yaml.get("nfpms").is_some(), "missing nfpms section");
}

#[given("the systemd unit file")]
#[expect(clippy::expect_used, reason = "simplify test failure output")]
fn the_systemd_file(world: &mut PackagingWorld) {
    let text = fs::read_to_string("packaging/linux/comenqd.service").expect("read service");
    world.content = Some(text);
}

#[then("it includes hardening directives")]
#[expect(clippy::expect_used, reason = "simplify test failure output")]
fn includes_hardening(world: &mut PackagingWorld) {
    let text = world.content.take().expect("service loaded");
    assert!(text.contains("ProtectSystem=strict"));
    assert!(text.contains("PrivateTmp=true"));
    assert!(text.contains("NoNewPrivileges=true"));
}
