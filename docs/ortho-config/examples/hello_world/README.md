# Hello World example

This crate will showcase a minimal, end-to-end, configuration-driven
command-line application. It focuses on demonstrating the orthogonal
configuration concepts that power the wider project without adding
production-ready complexity.

## Demonstrated capabilities

- **Global parameters (switches and arrays)**: illustrate how the command-line
  parser exposes top-level configuration that applies to every command,
  covering boolean feature switches, repeated values, and precedence between
  defaults and caller-supplied input. The loader now reuses the derive-generated
  `compose_layers_from_iter` output and clears salutation defaults when
  callers pass `-s/--salutation`, so CLI vectors replace file or environment
  entries.
- **Collection merge strategies**: demonstrate vector appends alongside map
  replacement semantics. The `greeting_templates` field in `GlobalArgs` uses the
  `merge_strategy = "replace"` attribute, so configuration files can swap the
  entire template set without leaking defaults from other layers. This keeps
  the example defaults isolated when consumers override templates.
- **Subcommands**: implement a friendly `greet` command that accepts a name and
  configurable greeting, alongside a `take-leave` workflow that combines
  switches, optional arguments, and shared greeting customizations to decide
  how a farewell is delivered. The `Commands` enum derives
  `SelectedSubcommandMerge` so the entry point can merge the selected
  subcommand configuration without duplicating `load_and_merge()` calls in a
  `match`.
- **Testing disciplines**: add `rstest`-powered unit tests for deterministic
  components and `rstest-bdd` (Behaviour-Driven Development) behavioural
  specifications that exercise the binary as a user would, capturing
  configuration precedence and cross-platform quirks. The unit suite now
  includes a declarative merging fixture that enumerates precedence
  permutations. This pairs with the JSON-layer scenario, which is bound via
  compile-time tag filters.
- **Graceful help/version exits**: the entry point parses Command-Line Interface
  (CLI) arguments with `clap::Parser::try_parse` and uses
  `ortho_config::is_display_request` to detect `--help` / `--version` requests.
  It delegates to `clap::Error::exit` so shells and completion generation keep
  their expected zero exit status.
- **Declarative merging**: demonstrate how `MergeComposer` and
  `merge_from_layers` build layered configuration without invoking the CLI by
  driving a behavioural scenario that composes JSON-described layers into
  `GlobalArgs`, asserting that default salutations are preserved when
  environment layers append new values.
- **Localized help text**: ship a `DemoLocalizer` backed by
  `FluentLocalizer`, layer the example’s bundled catalogue over `ortho_config`
  ’s defaults, and thread it through
  `CommandLine::command().with_base("hello_world.cli").localize(&localizer)`
  plus `ortho_config::parse_localized_command`. Formatting errors are logged,
  and the default bundle is used as a fallback, illustrating how applications
  can adopt Fluent without sacrificing existing help copy while keeping custom
  catalogue roots.
- **Documentation IR emission**: serialize documentation metadata using
  `OrthoConfigDocs::get_doc_metadata` so `cargo-orthohelp` can generate man
  pages and PowerShell help without scraping `--help` output. The `emit_docs`
  helper binary prints the IR as JSON.
- **Shell and Windows automation**: provide paired `.sh` and `.cmd` scripts
  highlighting how environment variables, configuration files, and command-line
  overrides interact. Include examples covering default configuration,
  per-subcommand overrides, and the precedence order across the sources.
- **YAML 1.2 parsing**: exercise the new `serde-saphyr` provider with
  behavioural coverage that keeps unquoted scalars such as `yes` as strings via
  strict boolean parsing and rejects duplicate mapping keys, mirroring the
  semantics library users observe.

## Planned project layout

- `src/` will contain a small `main.rs` and supporting modules for
  option-parsing, command dispatch, and domain logic.
- `tests/` will host `rstest-bdd` steps, fixtures, and scenario bindings for
  behavioural coverage.
- `scripts/` will offer automation snippets, with mirrored POSIX shell and
  Windows `.cmd` scripts to showcase configuration strategies on each platform.
- `config/` will collect sample configuration files that the scripts reference
  during demonstrations.

## Implementation considerations

- Keep the greeting and farewell flows intentionally simple so that the focus
  remains on configuration handling rather than application behaviour.
- Ensure every configuration source is represented in both documentation and
  automated coverage to demonstrate reproducibility.
- Document how to run the example from a fresh checkout through the scripts and
  behavioural tests.

## Localizer demonstration

The `src/localizer.rs` module builds a `FluentLocalizer` from the embedded
`examples/hello_world/locales/en-US/messages.ftl` catalogue and layers it over
`ortho_config`’s default messages. The binary instantiates this localizer
before parsing arguments, localizes the command tree with
`with_base("hello_world.cli")`, and calls
`ortho_config::parse_localized_command`, ensuring `--help` output reflects the
translated copy. If catalogue parsing fails, the demo logs a warning and falls
back to `NoOpLocalizer`, keeping the stock `clap` strings available while
translations are repaired. Consumers who are not ready to ship real strings can
explicitly choose `DemoLocalizer::noop()` for the same effect.

Parsing failures are also routed through
`ortho_config::localize_clap_error_with_command`, so missing subcommands or
arguments reuse the same catalogue while still receiving context from the
command tree. The demo catalogue overrides `clap-error-missing-argument` and
`clap-error-missing-subcommand`, demonstrating how application text supersedes
the embedded defaults while still deferring to stock `clap` messages whenever a
translation is absent.

### Running with Japanese locale

The example ships with both English (`en-US`) and Japanese (`ja`) catalogues.
Locale selection inspects the `LC_ALL`, `LC_MESSAGES`, and `LANG` environment
variables in priority order, falling back to English when no supported locale
is detected.

To run the example with Japanese help text:

```sh
# View top-level help in Japanese
LANG=ja_JP.UTF-8 cargo run -p hello_world -- --help

# View greet subcommand help in Japanese
LANG=ja_JP.UTF-8 cargo run -p hello_world -- greet --help

# Trigger a localised error message (missing required argument)
LANG=ja_JP.UTF-8 cargo run -p hello_world -- greet
```

On Windows Command Prompt, set the variable before the command:

```cmd
set LANG=ja_JP.UTF-8
cargo run -p hello_world -- --help
```

The `LC_ALL` variable takes precedence over `LC_MESSAGES`, which in turn takes
precedence over `LANG`. This allows fine-grained control when multiple locale
variables are set.

## Generating documentation IR

`cargo-orthohelp` reads the `package.metadata.ortho_config` metadata declared in
`Cargo.toml` to locate the root configuration type and supported locales. Run
it from the repository root to emit per-locale JSON:

```sh
cargo orthohelp --package hello_world --out-dir target/orthohelp --locale en-US
```

Add `--cache` to reuse the cached IR, or `--no-build` to skip compiling the
bridge when the cache is already populated.

## Generating PowerShell help

`cargo-orthohelp` can also generate PowerShell help files and a wrapper module
from the same IR. The output is written under `powershell/<ModuleName>` in the
chosen output directory:

```sh
cargo orthohelp --package hello_world --format ps --out-dir target/orthohelp --locale en-US
```

To try the generated module in PowerShell:

```powershell
Import-Module target/orthohelp/powershell/HelloWorld/HelloWorld.psd1 -Force
Get-Help hello-world -Full
```

## Configuration samples and scripts

The `config/` directory contains `baseline.toml` and `overrides.toml`. The
baseline file defines the defaults exercised by the behavioural tests and the
demo scripts. `overrides.toml` extends the baseline to demonstrate
configuration inheritance by changing the recipient and salutation while
preserving the original repository state.

When present, `.hello_world.toml` overrides both global excitement and nested
`cmds.greet` fields. Discovery prefers `HELLO_WORLD_CONFIG_PATH`, then standard
user configuration folders (`$XDG_CONFIG_HOME`, entries in `$XDG_CONFIG_DIRS`,
and `%APPDATA%`), and finally falls back to `$HOME/.hello_world.toml` and the
working directory. The shipped overrides enable a `Layered hello` preamble and
triple exclamation marks, so the behavioural suite and demo scripts assert the
shouted output (`HEY CONFIG FRIENDS, EXCITED CREW!!!`) to guard the layering.
The derive uses `#[ortho_config(prefix = "HELLO_WORLD")]`; the macro appends
the trailing underscore automatically, so environment variables continue to use
the `HELLO_WORLD_` prefix.

Once the workspace is built, `scripts/demo.sh` (or `scripts/demo.cmd` on
Windows) can be executed. Each script creates an isolated temporary directory,
copies the sample configuration files, and then invokes
`cargo run -p hello_world` multiple times to show the precedence order: file
defaults, environment overrides, and CLI flags. The scripts leave the
repository tree untouched so they are safe to rerun. The derived CLI also
exposes a `--config` / `-c` flag, so ad hoc configuration files can be layered
without mutating the working directory.

## Getting started

- Prerequisites: Rust toolchain (via rustup), Cargo, make, and
  markdownlint-cli2.
- Build: `cargo build`.
- Run formatting and Markdown lint checks: `make fmt && make markdownlint`.
- Validate Mermaid diagrams (if present): `make nixie`.
- Run static analysis: `cargo clippy -D warnings` (or `make lint`).
- Execute tests (unit and behavioural): `make test`.
- Emit documentation metadata as JSON:
  `cargo run -p hello_world --bin emit_docs`.

## Implementation checklist

- [x] Scaffold the crate with `Cargo.toml`, `src/main.rs`, and supporting
  modules.
- [x] Define global command-line parameters, switches, and array parameters with
  defaults and validation.
- [x] Implement the `greet` subcommand with its arguments and options.
- [x] Implement the `take-leave` subcommand with its arguments and options.
- [x] Add `rstest` unit tests covering parsing, validation, and command logic.
- [x] Add `rstest-bdd` behavioural tests covering end-to-end workflows and
  configuration precedence.
- [x] Create shell and Windows `.cmd` scripts showcasing configuration file
  usage and overrides.
- [x] Provide sample configuration files aligned with the scripts and tests.
- [x] Update documentation to reference the example and describe how to run it.
