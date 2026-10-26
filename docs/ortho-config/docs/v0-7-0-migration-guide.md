# Migration guide: v0.6.0 to v0.7.0

## Table of contents

- [Introduction](#introduction)
- [At-a-glance breaking changes](#at-a-glance-breaking-changes)
- [1. Update versions](#1-update-crate-versions-and-feature-flags)
- [2. Compose layers separately](#2-compose-layers-separately-from-merging)
- [3. Add post-merge hooks where needed](#3-add-post-merge-hooks-where-needed)
- [4. Localize CLI copy and errors](#4-localize-cli-copy-and-errors)
- [5. Treat clap defaults](#5-treat-clap-defaults-as-absent)
- [6. Refresh error handling](#6-refresh-error-handling-and-tests)

## Introduction

This guide describes how to upgrade applications from `ortho-config` v0.6.0 to
v0.7.0. The release is additive: it introduces layer composition helpers,
post-merge hooks, localized CLI output, and additional error-handling
utilities, without changing the core configuration model. Use the sections
below to adopt the new helpers where they match your configuration style and
testing setup.

## At-a-glance breaking changes

| Area              | Impact                                                                                                  | Section                                  |
| ----------------- | ------------------------------------------------------------------------------------------------------- | ---------------------------------------- |
| Core API          | No mandatory breaking changes; v0.7.0 is additive for typical usage.                                    | N/A                                      |
| CLI defaults      | `cli_default_as_absent` is opt-in and changes precedence for clap defaults when used with `ArgMatches`. | [5](#5-treat-clap-defaults-as-absent)    |
| Behavioural tests | The `hello_world` behavioural suite now uses `rstest-bdd` instead of `cucumber-rs`.                     | [6](#6-refresh-error-handling-and-tests) |

## 1. Update crate versions and feature flags

### Before: v0.6.0 dependencies

```toml
ortho_config = { version = "0.6.0", features = ["yaml"] }
ortho_config_macros = "0.6.0"
```

### After: v0.7.0 dependencies

```toml
ortho_config = { version = "0.7.0", features = ["yaml"] }
ortho_config_macros = "0.7.0"
```

1. Update every `ortho_config` and `ortho_config_macros` dependency to `0.7.0`.
2. Keep format features (`toml`, `json5`, `yaml`) on `ortho_config` as before.
3. If you disable default features, enable `serde_json` explicitly when you
   want `cli_default_as_absent` or selected-subcommand merging helpers. The
   default feature set already enables it.
4. Expect new transitive dependencies (`fluent-bundle`, `fluent-syntax`,
   `unic-langid`, and `tracing`) to land with v0.7.0, as they power CLI
   localization support.[^deps-0-7]

## 2. Compose layers separately from merging

Derived configurations now expose `compose_layers()` and
`compose_layers_from_iter(...)` helpers that return a `LayerComposition`. This
separates discovery and capture from the actual merge step, making it easier to
add custom layers or aggregate errors before producing the final config.

### Example: merge after inspecting the composition

```rust
use ortho_config::OrthoConfig;

let composition = AppConfig::compose_layers_from_iter(["app", "--port", "4040"]);
let merged = composition.into_merge_result(AppConfig::merge_from_layers)?;
# let _ = merged;
```

Use `LayerComposition::into_parts` when you need to push extra layers or report
errors alongside the final merge. The Hello World example uses this separation
to keep layer discovery distinct from CLI overrides.[^compose-layers]

## 3. Add post-merge hooks where needed

If your configuration needs cross-field adjustments or validation that depends
on merged data, implement `PostMergeHook` and enable it with
`#[ortho_config(post_merge_hook)]`. The hook receives a `PostMergeContext`
containing the prefix, loaded file paths, and a flag for CLI input.

### Example: normalize a derived field

```rust
use ortho_config::{OrthoConfig, OrthoResult, PostMergeContext, PostMergeHook};
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Deserialize, Serialize, OrthoConfig)]
#[ortho_config(prefix = "APP_", post_merge_hook)]
struct GreetArgs {
    #[ortho_config(default = String::from("!"))]
    punctuation: String,
    preamble: Option<String>,
}

impl PostMergeHook for GreetArgs {
    fn post_merge(&mut self, _ctx: &PostMergeContext) -> OrthoResult<()> {
        if self.preamble.as_ref().is_some_and(|p| p.trim().is_empty()) {
            self.preamble = None;
        }
        Ok(())
    }
}
```

Use hooks sparingly. Prefer field-level attributes where possible, and only
reach for `PostMergeHook` when behaviour depends on multiple merged
fields.[^post-merge]

## 4. Localize CLI copy and errors

v0.7.0 introduces a `Localizer` trait plus a Fluent-backed `FluentLocalizer`
implementation. This lets applications localize `clap` usage strings while
falling back to the stock English copy when a translation is missing. Use
`localize_clap_error_with_command` to rewrite `clap` errors with the same
localizer.

### Example: build a Fluent localizer and attach it to clap

```rust
use clap::CommandFactory;
use ortho_config::{FluentLocalizer, Localizer, langid, localize_clap_error_with_command};

static APP_EN: &str = include_str!("../locales/en-US/app.ftl");

let localizer = FluentLocalizer::builder(langid!("en-US"))
    .with_consumer_resources([APP_EN])
    .try_build()?;

let mut command = Cli::command().localize(&localizer);
let _matches = command.try_get_matches().map_err(|err| {
    localize_clap_error_with_command(err, &localizer, Some(&command))
})?;
```

The `LocalizationArgs` helper mirrors Fluent placeholders, making it easy to
pass argument-aware values into lookups when needed.[^localizer][^localize-clap]

## 5. Treat clap defaults as absent

Non-`Option` fields backed by `default_value_t` always appear in parsed CLI
structs, which previously forced the CLI to override file and environment
values. Add `#[ortho_config(cli_default_as_absent)]` to treat clap defaults as
absent unless the user provides a value explicitly.

The derive macro infers struct defaults from clap defaults when
`cli_default_as_absent` is enabled, so a duplicate
`#[ortho_config(default = ...)]` is no longer required. Typed `default_value_t`
and `default_values_t` retain their existing direct inference. String
`default_value` is replayed through clap's value parser at load time,
preserving clap parsing semantics and reporting conversion failures as
`OrthoError::DefaultValueConversion` rather than panicking.

String-default inference supports scalar fields, `Option<T>`, and `Vec<T>`
fields, including primitive and standard types, `ValueEnum` fields, and fields
with `#[arg(value_parser = ...)]`. Nested `Option`/`Vec` wrappers and map
fields are rejected at compile time; use an explicit
`#[ortho_config(default = ...)]` for those shapes.

When you use this attribute, pass `ArgMatches` so `value_source()` can detect
which fields were truly provided:

```rust
let matches = GreetArgs::command().get_matches();
let cli = GreetArgs::from_arg_matches(&matches)?;
let merged = cli.load_and_merge_with_matches(&matches)?;
# let _ = merged;
```

For subcommand enums, derive `SelectedSubcommandMerge` and call
`load_and_merge_selected(&matches)` or
`load_globals_and_merge_selected_subcommand` to merge the active variant in one
step. Variants that depend on `cli_default_as_absent` should be annotated with
`#[ortho_subcommand(with_matches)]` so the merge can access the subcommand
`ArgMatches`.[^cli-defaults][^selected-subcommand]

## 6. Refresh error handling and tests

- Use `is_display_request` before mapping `clap::Error` into your own error
  types, so `--help` and `--version` still exit with code 0.[^display-request]
- `OrthoJsonMergeExt::into_ortho_merge_json()` lets you attribute JSON parsing
  failures to the merge phase while preserving line and column
  details.[^json-merge]
- The `hello_world` behavioural suite now uses `rstest-bdd` instead of
  `cucumber-rs`. If you copied the example harness or pinned CI scripts to the
  old runner, update them to use the `rstest-bdd` macros and compile-time tag
  filters (for example, gating YAML scenarios).[^rstest-bdd]

[^deps-0-7]: v0.7.0 adds Fluent and localization dependencies alongside the
existing feature flags in the runtime crate and macro crate metadata.
【F:ortho_config/Cargo.toml†L1-L49】 【F:ortho_config_macros/Cargo.toml†L1-L29】
[^compose-layers]: Derived configuration structs now expose `compose_layers()`
and `compose_layers_from_iter(...)` and return `LayerComposition` for staged
merging and error aggregation.
【F:ortho_config_macros/src/derive/load_impl.rs†L281-L339】
【F:ortho_config/src/declarative.rs†L236-L305】
[^post-merge]: `PostMergeHook` and `PostMergeContext` enable post-merge
adjustments and are invoked when `#[ortho_config(post_merge_hook)]` is present.
【F:ortho_config/src/post_merge.rs†L1-L64】
【F:ortho_config/src/post_merge.rs†L67-L159】
[^localizer]: `Localizer`, `LocalizationArgs`, and `FluentLocalizer` define the
CLI localization surface and provide a Fluent-backed implementation.
【F:ortho_config/src/localizer/mod.rs†L1-L215】
[^localize-clap]: `localize_clap_error_with_command` rewrites clap error
messages while keeping the existing rendered tail intact.
【F:ortho_config/src/localizer/clap_error.rs†L1-L90】
[^cli-defaults]: `cli_default_as_absent` is implemented via the
`CliValueExtractor` trait to exclude clap defaults from the CLI layer unless
explicitly provided. 【F:ortho_config/src/merge.rs†L134-L181】
[^selected-subcommand]: `SelectedSubcommandMerge` and
`load_globals_and_merge_selected_subcommand` let you merge the chosen
subcommand without manual `match` scaffolding.
【F:ortho_config/src/subcommand/selected.rs†L1-L191】
[^display-request]: `is_display_request` preserves clap's help and version exit
behaviour when callers use `try_parse()`.
【F:ortho_config/src/error.rs†L59-L73】
[^json-merge]: `OrthoJsonMergeExt` maps `serde_json::Error` into merge failures
while preserving detailed diagnostics.
【F:ortho_config/src/result_ext.rs†L71-L112】
[^rstest-bdd]: The behavioural suites moved from `cucumber-rs` to `rstest-bdd`,
including tag-aware filtering for YAML scenarios.
【F:docs/adrs/adr-0002-replace-cucumber-with-rstest-bdd.md†L9-L47】
