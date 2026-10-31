# Migration guide: clap-mangen to cargo-orthohelp

## Introduction

This guide explains how to migrate an `ortho-config` consumer from
`clap_mangen`-driven man-page generation in `build.rs` to the `cargo-orthohelp`
workflow.

The migration keeps command-line interface (CLI) documentation generation out
of application build scripts, supports localized outputs, and removes the need
to depend on `clap_mangen`.

The examples below are based on the `../netsuke/build.rs` pattern used with
`ortho-config` v0.7.0.

## What changes in practice

With `clap_mangen`, man-page generation normally happens inside `build.rs`
while compiling the application.

With `cargo-orthohelp`, documentation generation is a separate command, usually
run in packaging scripts or Continuous Integration (CI):

```bash
cargo orthohelp --format man --out-dir target/docs --locale en-US
```

This separation is intentional. `cargo-orthohelp` builds a small bridge binary
that reads `OrthoConfigDocs` metadata and emits documentation artefacts.

## 1. Remove clap-mangen dependencies

Update `Cargo.toml` and remove `clap_mangen` from `build-dependencies`.

Before:

```toml
[build-dependencies]
clap = { version = "4.5.0", features = ["derive"] }
clap_mangen = "0.2.29"
time = { version = "0.3.44", features = ["formatting"] }
```

After:

```toml
[build-dependencies]
clap = { version = "4.5.0", features = ["derive"] }
```

If `time` was only used to format the manual page date for `clap_mangen`,
remove it from `build-dependencies` as well.

## 2. Add or verify orthohelp metadata

`cargo-orthohelp` needs a root config type and locale list. Declare these in
`Cargo.toml` so the command can run without repeating long flags:

```toml
[package.metadata.ortho_config]
root_type = "example_app::cli::AppConfig"
locales = ["en-US"]
```

If metadata is not available, pass `--root-type` and locale flags explicitly
when calling `cargo orthohelp`.

## 3. Remove man-page generation from build.rs

The `../netsuke/build.rs` pattern currently uses `clap_mangen::Man` directly:

```rust
use clap::{ArgMatches, CommandFactory};
use clap_mangen::Man;

// ...

let cmd = cli::Cli::command();
let man = Man::new(cmd)
    .section("1")
    .source(format!("{cargo_bin} {version}"))
    .date(manual_date());
let mut buf = Vec::new();
man.render(&mut buf)?;
write_man_page(&buf, &out_dir, &page_name)?;
```

After migration, remove this man-page path from `build.rs` and keep only logic
that must run at compile time (for example, localization audits).

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    build_l10n_audit::audit_localization_keys()?;
    Ok(())
}
```

### Remove build-script lint suppressions

After updating to `ortho_config` v0.10.0 or later, delete the following
suppression from `build.rs`:

```rust
#![allow(
    unfulfilled_lint_expectations,
    reason = "OrthoConfig derive behaves differently in build script"
)]
```

Version 0.10.0 changes the generated `compose_layers` and
`compose_layers_from_iter` helpers from `#[expect(dead_code, ...)]` to
`#[allow(dead_code, ...)]`. The generated helpers therefore remain valid when
the build script already allows `dead_code`.

The following suppressions apply only because `clap_mangen` compiles the CLI
types inside `build.rs`:

```rust
#![allow(dead_code, reason = "CLI types unused in build script context")]
#![allow(unused_imports, reason = "CLI module imports unused in build script")]
```

Delete `clap_mangen` man-page generation from `build.rs` and generate the
documentation with `cargo orthohelp` instead, as described in the next section.
This removes the build-script compilation boundary that requires both remaining
suppressions.

## 4. Add a docs generation command to CI or packaging

Create a release or packaging step that runs `cargo orthohelp`.

Example for one locale:

```bash
cargo orthohelp \
  --package netsuke \
  --bin netsuke \
  --format man \
  --locale en-US \
  --out-dir target/generated-docs \
  --man-section 1
```

Example with reproducible date from `SOURCE_DATE_EPOCH`:

```bash
MAN_DATE="$(date -u -d "@${SOURCE_DATE_EPOCH:?}" +%F)"
cargo orthohelp \
  --package netsuke \
  --bin netsuke \
  --format man \
  --locale en-US \
  --out-dir target/generated-docs \
  --man-section 1 \
  --man-date "$MAN_DATE"
```

If the environment does not support `date -d`, compute the date using the
platform-equivalent command and pass the same `YYYY-MM-DD` value via
`--man-date`.

## 5. Update expected output paths

`clap_mangen` workflows often write a single file such as:

```plaintext
target/generated-man/<target>/<profile>/<bin>.1
```

`cargo-orthohelp` writes standard man-page layout under the selected output
root:

```plaintext
<out-dir>/man/man<section>/<bin>.<section>
```

For example, with `--out-dir target/generated-docs --man-section 1`, expect:

```plaintext
target/generated-docs/man/man1/netsuke.1
```

If a packaging job expects the old flat path, update the packaging script to
read from the new path, or copy the generated file into the legacy location as
a compatibility shim.

## 6. Validate the migration

Run the documentation command and verify that:

- The command exits successfully.
- The expected man page exists under `<out-dir>/man/man<section>/`.
- Packaging or install jobs consume the new path correctly.

Recommended verification command:

```bash
cargo orthohelp --format man --locale en-US --out-dir target/generated-docs
```

## Summary

Migrating from `clap_mangen` to `cargo-orthohelp` for `ortho-config` v0.7.0
primarily means:

- moving man-page generation out of `build.rs`,
- removing `clap_mangen` build dependencies,
- declaring `ortho_config` metadata for documentation generation, and
- generating man pages in CI or packaging using `cargo orthohelp`.

This keeps runtime builds focused, reduces documentation drift risk, and aligns
with the intermediate representation (IR)-driven documentation model introduced
for `ortho-config` tooling.
