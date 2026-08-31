# Developers' guide

## Architecture

The root `comenq-lib` crate owns only shared protocol types. The
`comenq-transport` crate owns socket-discovery policy because it reads
`XDG_RUNTIME_DIR`; the `comenq` client and `comenqd` daemon use its helpers in
their transport and configuration adapters. The daemon composes configuration,
the Unix socket listener, persistent `QueueStore`, GitHub worker, and task
supervisor. The detailed component and lifecycle design is maintained in
[Comenq design](comenq-design.md), especially
[Daemon architecture](comenq-design.md#section-3-design-of-the-comenqd-daemon)
and
[Source code for `comenqd`](comenq-design.md#55-source-code-for-comenqd-daemon).

`comenq_transport::user_socket_path()` returns a per-user socket only when
`XDG_RUNTIME_DIR` is a non-empty absolute path.
`comenq_transport::default_socket_path()` selects that path for a daemon or the
system default otherwise. `comenq_transport::socket_candidates()` returns the
user path first, then `/run/comenq/comenq.sock`, without duplicates. The client
probes those candidates by connecting rather than checking for socket files. An
explicit `--socket` or `COMENQ_SOCKET` value becomes the sole candidate.

The supervisor starts the listener and worker against the same
`Arc<SharedQueue>`. `SharedQueue` serializes access to the filesystem-backed
`QueueStore` with a `std::sync::Mutex`; each synchronous store operation runs
inside `spawn_blocking` so filesystem work does not occupy Tokio runtime
threads. A `tokio::sync::Notify` wakes the worker after queue mutations.
Restart tracing includes the task name, attempt, and backoff delay where
applicable. Recovery is bounded to five restart attempts; when that limit is
exhausted, the supervisor signals daemon shutdown.

### Client and daemon API

The `comenq::Args` parser exposes a global `--socket` option and the `put`,
`list`, `bump`, `bust`, and `del` [`Command`](../crates/comenq/src/lib.rs)
subcommands. `Command::to_request()` maps each command to the shared
`comenq_lib::protocol::Request` enum. `comenq::ClientError` distinguishes
connection, serialization, I/O, daemon-reported, and unexpected-response
failures; `comenq::run()` renders successful replies for users.

The Unix socket carries exactly one tagged JSON `Request` per connection. The
daemon returns exactly one tagged JSON `Response`: `Response::Ok` contains an
`entry` for `put`, an `entries` list for `list`, or neither field for `bump`,
`bust`, and `del`; `Response::Error` contains the daemon's human-readable
failure message. `PendingEntry` carries the deterministic eight-character ID,
ETA in seconds, repository target, pull request number, and full comment body.
Clients must treat response fields as untrusted and reject a successful reply
whose payload shape does not match the request.

The listener adapter passes valid requests to `SharedQueue::execute`. It
performs queue mutations and scheduling through `QueueStore`, then notifies the
worker after a successful mutation. `SharedQueue::next_due()` selects the head
entry for posting, and `SharedQueue::complete()` removes an entry after a
successful GitHub post while recording the posting timestamp. Completion first
writes a durable recovery record containing the entry identifier and posting
time; startup reconciliation finishes the deletion and `last_post` update
before clearing that record. Queue-store failures become daemon error
responses; failed GitHub posts leave their entries in place for a later
full-cooldown retry.

When a comment is enqueued, the worker chooses a uniformly distributed flutter
and stores it with that entry. The stored flutter is added to the complete base
cooldown and never shortens it, keeping the queue's cooldown-derived ETA
stable. Keep this operational rule aligned across configuration, worker tests,
the [users' guide](users-guide.md), and the design document.

### Configuration API

`comenqd::config::Config` is the public runtime configuration model. The
`Config::load()` entry point reads the daemon's `--config` file, merges
`COMENQD_*` environment variables, applies supported CLI overrides, and
resolves the effective GitHub credential. Its public fields cover the token
sources, socket and queue paths, cooldown and flutter, restart delay, and
GitHub API timeout.

Tests and integrations built with the `test-support` feature can use
`Config::from_file(path)` to load a particular file while retaining the
`COMENQD_*` environment merge. Both entry points return a configuration error
when the file is missing or invalid, or when no usable GitHub credential is
available.

Structured tracing records socket probes, the selected credential source (but
never the token value), queue-side opens and task restarts, each effective
cooldown wait, and GitHub post spans. The post span contains only
`task="worker"` and an `outcome` of `success`, `api_error`, or `timeout`; spans
exclude credentials, payloads, repository names, paths, request identifiers,
and raw errors. Operators and developers can select the emitted detail through
`RUST_LOG`. The credential reader rejects files larger than 64 KiB before
trimming their contents.

The daemon attempts to expose Prometheus metrics at `127.0.0.1:9000/metrics`.
The stable metric vocabulary is:

- `comenqd_task_restarts_total{task=listener|worker}` for supervised
  task restarts.
- `comenqd_requests_total{outcome=accepted|failed|rejected}` for request
  outcomes.
- `comenqd_cooldown_wait_duration_seconds` for cooldown wait durations.
- `comenqd_github_posts_total{outcome=success|api_error|timeout}` for GitHub
  comment-post outcomes.
- `comenqd_github_post_duration_seconds` for GitHub comment-post durations.

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
