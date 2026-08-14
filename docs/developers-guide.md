# Developers' guide

## Architecture

The workspace separates shared protocol types and socket discovery in the root
`comenq-lib` crate from the `comenq` client and `comenqd` daemon adapters. The
daemon composes configuration, the Unix socket listener, persistent `yaque`
queue, GitHub worker, and task supervisor. The detailed component and lifecycle
design is maintained in [Comenq design](comenq-design.md), especially
[Daemon architecture](comenq-design.md#section-3-design-of-the-comenqd-daemon)
and
[Source code for `comenqd`](comenq-design.md#55-source-code-for-comenqd-daemon).

The worker uses `rand` to choose a new uniformly distributed flutter for each
cooldown. Flutter is added to the complete base cooldown and never shortens it.
Keep this operational rule aligned across configuration, worker tests, the
[users' guide](users-guide.md), and the design document.

Structured tracing records socket probes, the selected credential source (but
never the token value), queue-side opens and task restarts, and each effective
cooldown wait. Operators and developers can select the emitted detail through
`RUST_LOG`.

## Testing support

Run repository gates through the Makefile. The principal code gates are
`make check-fmt`, `make lint`, `make typecheck`, and `make test`; documentation
changes additionally require `make markdownlint` and `make nixie`.

The `test-support` workspace crate owns helpers shared by unit, integration,
and behavioural tests, including temporary daemon configuration, environment
guards, logging capture, socket polling, and mock GitHub clients. The `comenqd`
`test-support` feature exposes only the daemon integration seams needed by
external tests. Production code must not depend on that feature being enabled.

Tests that mutate process-wide environment variables use `serial_test` together
with `test_support::EnvVarGuard`. Mark every such test `#[serial_test::serial]`
so it cannot race another environment-mutating test, and let the guard restore
the previous value. `proptest` exercises cooldown arithmetic across arbitrary
`u64` values: a selected flutter must never shorten the base cooldown or exceed
the base plus the configured flutter, using saturating addition at overflow.
Prefer the shared helpers over duplicating fixtures in an individual crate.
Further patterns are documented in
[the Cucumber guide](behavioural-testing-in-rust-with-cucumber.md) and
[Rust testing with rstest fixtures](rust-testing-with-rstest-fixtures.md).

## Spelling gate

Run `make spelling` to enforce en-GB-oxendict spelling in tracked Markdown
prose. The target regenerates `typos.toml`, verifies that the generated file is
tracked and unchanged, then runs the pinned `typos` release over tracked
Markdown files. `make markdownlint` depends on this gate, and `make all` runs
it with the repository's release build.

The generated configuration combines the shared estate dictionary bundled with
the pinned `typos-config-builder` revision and the repository-specific
`typos.local.toml` overlay. Do not edit `typos.toml` by hand. Add only narrow
identifier, API, proper-name, or immutable-fixture exceptions to the local
overlay; ordinary prose belongs in Oxford spelling.

Run `make spelling-config-write` to refresh the untracked
`.typos-oxendict-base.toml` cache and regenerate `typos.toml`. Run
`make spelling-config` to check for generated drift without rewriting the
tracked file. The focused builder refreshes a valid cache only when its bundled
authority is newer; the repository retains phrase enforcement in
`scripts/typos_rollout_check.py` because Typos cannot represent exact phrase
corrections faithfully.
