# Migration guide: v0.5.0 to v0.6.0

## Table of contents

- [Introduction](#introduction)
- [At-a-glance breaking changes](#at-a-glance-breaking-changes)
- [1. Update crate versions and feature flags](#1-update-crate-versions-and-feature-flags)
- [2. Simplify imports using the new re-exports](#2-simplify-imports-using-the-new-re-exports)
- [3. Adopt declarative configuration discovery](#3-adopt-declarative-configuration-discovery)
- [4. Handle stricter discovery outcomes](#4-handle-stricter-discovery-outcomes)
- [5. Switch to the stricter YAML provider](#5-switch-to-the-new-stricter-yaml-provider)
- [6. Review documentation and release notes](#6-review-documentation-and-release-notes)

## Introduction

This guide describes how to upgrade applications from `ortho-config` v0.5.0 to
v0.6.0. The release focuses on removing redundant dependencies, improving
configuration discovery, and tightening YAML parsing semantics. Each section
explains the change, why it matters, and how to adapt code. For concrete usage,
review the hello_world example
(<https://github.com/leynos/ortho-config/tree/main/examples/hello_world>)
throughout.

## At-a-glance breaking changes

| Area               | Impact                                                                                       | Section                                                                                            |
| ------------------ | -------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| Dependency surface | Macro crate now inherits parser features and consumes the re-exported dependency graph.      | [1](#1-update-crate-versions-and-feature-flags), [2](#2-simplify-imports-using-the-new-re-exports) |
| Discovery          | Declarative attributes replace bespoke builders, altering discovery configuration points.    | [3](#3-adopt-declarative-configuration-discovery)                                                  |
| Error handling     | `ConfigDiscovery::load_first` now errors when every candidate fails, affecting control flow. | [4](#4-handle-stricter-discovery-outcomes)                                                         |
| YAML parsing       | `SaphyrYaml` replaces the Figment YAML provider and enforces YAML 1.2 semantics.             | [5](#5-switch-to-the-new-stricter-yaml-provider)                                                   |

## 1. Update crate versions and feature flags

### Before: v0.5.0 dependencies

```toml
ortho_config = "0.5"
ortho_config_macros = { version = "0.5", features = ["yaml"] }
```

### After: v0.6.0 dependencies

```toml
ortho_config = { version = "0.6.0", features = ["yaml"] }
ortho_config_macros = "0.6.0"
```

1. Change every `ortho_config` dependency (workspace metadata, application
   crates, and supporting tools) to `"0.6.0"`. This keeps the runtime crate and
   the derive macro in lockstep, ensuring generated code matches the new
   library behaviour.
2. Retain any optional features (such as `json5`, `yaml`, or `toml`) on the
   main `ortho_config` dependency. The macro crate now inherits those flags,
   removing the need for duplicate feature declarations on
   `ortho_config_macros`.[^forwarded-features]
3. Rebuild the project to confirm the upgraded macro compiles cleanly before
   proceeding with behavioural changes.

The `hello_world` example continues to expose feature toggles via the parent
crate so that enabling `json5`, `yaml`, or `toml` automatically propagates to
both the runtime and macro crates.[^hello-world-cargo]

## 2. Simplify imports using the new re-exports

Version 0.6.0 re-exports `figment`, `uncased`, `xdg`, and the optional format
parsers directly from `ortho_config`, meaning downstream crates can remove
explicit dependencies on those packages.[^reexports] Update import paths to use
`ortho_config::figment` (and similar paths) instead of referring to the crates
independently. The `hello_world` example demonstrates this pattern when loading
configuration files and writing integration tests.[^hello-world-figment]

### Before: external Figment import

```rust
use figment::providers::{Format, Toml};
use ortho_config::ConfigBuilder;
```

### After: re-exported Figment import

```rust
use ortho_config::figment::providers::{Format, Toml};
use ortho_config::ConfigBuilder;
```

Once the imports are adjusted, prune redundant dependencies from each
`Cargo.toml`. Rebuilding after the clean-up confirms the slimmer dependency set.

## 3. Adopt declarative configuration discovery

A new `#[ortho_config(discovery(...))]` attribute enables customized discovery
without bespoke builders for every CLI entry point.[^discovery-attr] This
attribute mirrors the builder capabilities: specify the config file name,
dotfile name, project-level override, the generated `--config` flag, and the
corresponding environment variable prefix.

### Before: manual discovery builder

```rust
impl HelloWorldCli {
    pub fn discovery() -> ConfigDiscovery {
        ConfigDiscovery::builder()
            .global_file_name("hello-world.toml")
            .dotfile_name(".hello-world.toml")
            .build()
    }
}
```

### After: declarative discovery attribute

```rust
#[derive(ortho_config::Config)]
#[ortho_config(discovery(
    global_file_name = "hello-world.toml",
    dotfile_name = ".hello-world.toml",
    env_prefix = "HELLO_WORLD"
))]
pub struct HelloWorldCli {
    // fields
}
```

Apply the attribute to root config structs to remove manual builder calls. The
`hello_world` CLI uses the attribute to declare its TOML locations while
retaining a shared helper for non-derive consumers that need direct access to
the discovery builder.[^hello-world-discovery]

When programmatic discovery is required elsewhere (for example, to present the
candidate paths), keep helper functions that reuse `ConfigDiscovery::builder`.
Those helpers now benefit from the clarified error behaviour described next.

## 4. Handle stricter discovery outcomes

`ConfigDiscovery::load_first` now returns an error when every candidate path
fails to load and at least one of them produced an error. It only returns
`Ok(None)` when no candidates were available. Callers should bubble up the new
error instead of silently falling back to defaults.

In the `hello_world` example the shared discovery helper maps the error into
`HelloWorldError`[^discovery-errors], ensuring the CLI exits with actionable
diagnostics when misconfigured files exist.[^hello-world-discover-config]

### Before: ignoring discovery errors

```rust
match discovery.load_first()? {
    Some(config) => Ok(config),
    None => Ok(default_config()),
}
```

### After: bubbling discovery errors

```rust
let config = discovery.load_first().map_err(HelloWorldError::Load)?;
match config {
    Some(config) => Ok(config),
    None => Ok(default_config()),
}
```

Call sites that previously matched on `Ok(None)` to continue running with
defaults should be audited. After upgrading, consider whether those branches
now ought to abort with an error, so broken configuration files do not pass
unnoticed.

## 5. Switch to the new stricter YAML provider

The legacy `serde_yaml` integration has been replaced with a new `SaphyrYaml`
provider backed by `serde-saphyr`. The parser enforces YAML 1.2 rules, so
tokens like `yes` remain strings unless quoted and duplicate mapping keys
produce structured errors.[^saphyr]

If an application reads YAML, enable the `yaml` feature on `ortho_config` and
switch to `SaphyrYaml::file` (or `::string` for inline fixtures) wherever the
Figment YAML provider was previously used. The new provider supports the same
profile controls, so most call sites simply update the constructor name.

### Before: Figment YAML provider

```rust
use ortho_config::figment::providers::YamlExtended;

Figment::from(YamlExtended::file("hello-world.yaml"))
```

### After: Saphyr YAML provider

```rust
use ortho_config::file::SaphyrYaml;

Figment::from(SaphyrYaml::file("hello-world.yaml"))
```

The `hello_world` example exposes the YAML provider through integration tests
that merge inline YAML fragments into the global configuration, exercising the
strict parsing mode.[^hello-world-yaml]

## 6. Review documentation and release notes

After the code changes, internal documentation or runbooks should be updated to
reflect the new dependency graph, discovery behaviour, and YAML semantics. The
v0.6.0 CHANGELOG entries provide a concise summary for release
announcements.[^changelog]

### Before: documentation before v0.6.0

```markdown
* ortho-config v0.5.0 provides YAML parsing via serde_yaml.
```

### After: documentation updated for v0.6.0

```markdown
* ortho-config v0.6.0 provides YAML parsing via SaphyrYaml (serde-saphyr).
```

Once everything compiles and tests pass, the upgraded configuration experience
is ready for release.

[^forwarded-features]: Optional parser features on `ortho_config` automatically
enable matching flags on the macro crate, keeping generated code in sync with
runtime capabilities.【F:ortho_config/Cargo.toml†L41-L45】
[^hello-world-cargo]: The `hello_world` crate forwards its parser feature flags
to `ortho_config`, so enabling a format once covers both runtime and macro
usage.【F:examples/hello_world/Cargo.toml†L23-L33】
[^reexports]: `ortho_config` re-exports Figment, optional parser crates, and
supporting utilities for consumers, eliminating redundant direct
dependencies.【F:ortho_config/src/lib.rs†L11-L61】
[^hello-world-figment]: The `hello_world` example pulls Figment providers from
the `ortho_config` namespace when layering configuration data.
【F:examples/hello_world/src/cli/config_loading.rs†L1-L60】 It reuses the same
imports in tests to assert behaviour under YAML overrides.
【F:examples/hello_world/src/cli/tests/overrides.rs†L125-L155】
[^discovery-attr]: The derive macro accepts a `discovery(...)` attribute on
config structs, enabling declarative discovery
policies.【F:examples/hello_world/src/cli/mod.rs†L174-L211】
[^hello-world-discovery]: The CLI struct uses the discovery attribute to define
file names, CLI flags, and environment overrides without manual builder
plumbing.【F:examples/hello_world/src/cli/mod.rs†L174-L211】
[^discovery-errors]: `ConfigDiscovery::load_first` now aggregates discovery
errors, returning `Err` whenever every candidate fails but at least one error
occurred.【F:ortho_config/src/discovery/mod.rs†L305-L318】
[^hello-world-discover-config]: The shared discovery helper wraps
`ConfigDiscovery::load_first` and maps aggregated errors into `HelloWorldError`
for callers.【F:examples/hello_world/src/cli/discovery.rs†L1-L36】
[^saphyr]: The `SaphyrYaml` provider reads files with strict YAML 1.2 semantics
and backs the format-specific branch of `parse_config_by_format`.
【F:ortho_config/src/file/mod.rs†L34-L86】
【F:ortho_config/src/file/mod.rs†L253-L296】
[^hello-world-yaml]: Behavioural tests in `hello_world` create YAML fixtures,
load them through `ortho_config::load_config_file`, and assert strict parsing
behaviour.【F:examples/hello_world/src/cli/tests/overrides.rs†L125-L155】
[^changelog]: The Unreleased changelog summarizes the v0.6.0 additions and
behaviour changes discussed in this guide.【F:CHANGELOG.md†L6-L26】
