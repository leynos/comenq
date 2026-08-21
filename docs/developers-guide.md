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

`user_socket_path()` returns a per-user socket only when `XDG_RUNTIME_DIR` is a
non-empty absolute path. `default_socket_path()` selects that path for a daemon
or the system default otherwise. `socket_candidates()` returns the user path
first, then `/run/comenq/comenq.sock`, without duplicates. The client probes
those candidates by connecting rather than checking for socket files. An
explicit `--socket` or `COMENQ_SOCKET` value becomes the sole candidate.

The supervisor owns the `yaque::Sender` used by the queue writer; it opens that
side at startup and whenever the writer restarts. Each worker start opens only
the matching `yaque::Receiver`. This one-side-per-task topology avoids yaque's
per-side lock contention. Restart tracing includes the task name, attempt,
queue path, queue side, and backoff delay where applicable.

The worker uses `rand` to choose a new uniformly distributed flutter for each
cooldown. Flutter is added to the complete base cooldown and never shortens it.
Keep this operational rule aligned across configuration, worker tests, the
[users' guide](users-guide.md), and the design document.

### Configuration API

`comenqd::config::Config` is the public runtime configuration model. The
`Config::load()` entry point reads the daemon's `--config` file, merges
`COMENQD_*` environment variables, applies supported CLI overrides, and
resolves the effective GitHub credential. Its public fields cover the token
sources, socket and queue paths, cooldown and flutter, restart delay, GitHub
API timeout, and client-channel capacity.

Tests and integrations built with the `test-support` feature can use
`Config::from_file(path)` to load a particular file while retaining the
`COMENQD_*` environment merge. Both entry points return a configuration error
when the file is missing or invalid, or when no usable GitHub credential is
available.

Structured tracing records socket probes, the selected credential source (but
never the token value), queue-side opens and task restarts, and each effective
cooldown wait. Operators and developers can select the emitted detail through
`RUST_LOG`. The credential reader rejects files larger than 64 KiB before
trimming their contents.

Prometheus metrics are exposed at `127.0.0.1:9000/metrics`. The stable metric
vocabulary is:

- `comenqd_task_restarts_total{task=listener|worker|writer}` for supervised
  task restarts.
- `comenqd_queue_writer_failures_total{queue_side=sender}` for queue-writer
  failures.
- `comenqd_client_channel_depth` for the bounded client-channel depth proxy.
- `comenqd_requests_total{outcome=accepted|rejected}` for request outcomes.
- `comenqd_cooldown_wait_duration_seconds` for cooldown wait durations.

## Testing support

Run repository gates through the Makefile. The principal code gates are
`make check-fmt`, `make lint`, `make typecheck`, and `make test`; documentation
changes additionally require `make fmt`, `make markdownlint`, and `make nixie`.

The `make test` target includes the compiler-facing UI suite through its
workspace `nextest` run. `tests/public_api.rs` uses `trybuild` to compile every
pass fixture in `tests/ui/pass/*.rs` and to require compilation failure for
every fixture in `tests/ui/fail/*.rs`, with each failure matched against its
checked-in `.stderr` file. `trybuild` compiles an isolated fixture workspace;
the first cold run is allowed a five-minute slow-test period in
`.config/nextest.toml`. The complete `nextest` run has a 10-minute global
timeout, after which `make test` runs the Cucumber test target separately.

## Automated packaging

`make release` builds a local optimized binary and requires the Rust toolchain.
The tag-triggered [release workflow](../.github/workflows/release.yml) uses the
pinned shared release actions to provision each Rust target, stage generated
man pages, and produce `.deb` and `.rpm` artefacts. It also uploads the
transient `nfpm.yaml` manifest for each target so package contents are
inspectable. Run the Makefile gates before a release; the workflow then
validates its generated packages during the build-and-package jobs. See the
[automated packaging guide](automated-cross-platform-packaging.md) for the
release topology and system-package details.

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
