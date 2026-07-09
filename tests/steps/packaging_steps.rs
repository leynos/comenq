//! Behavioural steps for packaging configuration.

use anyhow::Context as _;
use cucumber::{World, given, then, when};
use serde_yaml::Value;
use std::fs;

#[derive(Debug, Default, World)]
pub struct PackagingWorld {
    content: Option<String>,
    yaml: Option<Value>,
}

#[given("the goreleaser configuration file")]
fn the_goreleaser_file(world: &mut PackagingWorld) -> anyhow::Result<()> {
    let text = fs::read_to_string(".goreleaser.yaml").context("read goreleaser")?;
    world.content = Some(text);
    Ok(())
}

#[when("it is parsed as YAML")]
fn parse_yaml(world: &mut PackagingWorld) -> anyhow::Result<()> {
    let text = world.content.take().context("file loaded")?;
    world.yaml = Some(serde_yaml::from_str(&text).context("parse yaml")?);
    Ok(())
}

#[then("the nfpms section exists")]
fn nfpms_exists(world: &mut PackagingWorld) -> anyhow::Result<()> {
    let yaml = world.yaml.as_ref().context("yaml parsed")?;
    assert!(yaml.get("nfpms").is_some(), "missing nfpms section");
    Ok(())
}

#[given("the systemd unit file")]
fn the_systemd_file(world: &mut PackagingWorld) -> anyhow::Result<()> {
    let text = fs::read_to_string("packaging/linux/comenqd.service").context("read service")?;
    world.content = Some(text);
    Ok(())
}

#[then("it includes hardening directives")]
fn includes_hardening(world: &mut PackagingWorld) -> anyhow::Result<()> {
    let text = world.content.take().context("service loaded")?;
    assert!(text.contains("ProtectSystem=strict"));
    assert!(text.contains("PrivateTmp=true"));
    assert!(text.contains("NoNewPrivileges=true"));
    Ok(())
}
