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
`Arc<SharedQueue>`. `SharedQueue` serializes short filesystem-backed
`QueueStore` operations with a `std::sync::Mutex`; each synchronous store
operation runs inside `spawn_blocking` so filesystem work does not occupy Tokio
runtime threads. A `tokio::sync::Notify` wakes the worker after queue
mutations. Await waiters use a separate generation subscription, exposed
through a queue change port rather than Tokio's watch type. The supervisor
restarts failed tasks with jittered exponential backoff and no maximum attempt
count; shutdown interrupts the wait, while normal task completion does not
trigger a restart. Each restart event records `task`, a one-based
`restart_attempt` that continues across stable runs, `selected_delay_ms`,
`stable_run_duration_ms`, and `backoff_reset`. A stable run resets the delay
sequence but does not reset the attempt count.

Queue listing logs unreadable or corrupt entry files and skips them. These
bounds logs use static error categories and omit filesystem paths and stored
identifiers.

### Client and daemon API

The `comenq::Args` parser exposes a global `--socket` option and the `put`,
`list`, `bump`, `bust`, `del`, `hist`, and `await`
[`Command`](../crates/comenq/src/lib.rs) subcommands. `hist` accepts `-n` or
`--limit` to select recent attempts. `Command::to_request()` maps each command
to the shared `comenq_lib::protocol::Request` enum. `comenq::ClientError`
distinguishes connection, serialization, I/O, daemon-reported, and
unexpected-response failures; `comenq::run()` renders successful replies for
users.

The Unix socket carries one tagged JSON `Request` per connection. Ordinary
requests receive one tagged JSON `Response`: `Response::Ok` contains an `entry`
for `put`, an `entries` list for `list`, a `history` page and optional cursor
for `hist`, or neither payload for `bump`, `bust`, and `del`; `Response::Error`
contains the daemon's human-readable failure message. `await` is the streaming
exception: it returns newline-delimited JSON `Waiting` frames followed by
exactly one terminal `Posted` or `Error` frame. The listener caps each frame at
64 KiB, sends a heartbeat at least every 15 seconds, and limits connected
waiters to 64. The client allows 45 seconds between frames. Dropping the
connection releases its waiter and does not alter the queued entry.
`PendingEntry` carries the deterministic eight-character ID, ETA in seconds,
repository target, pull request number, and full comment body. History requests
follow cursors over a fixed file snapshot so the CLI can render every record
without exceeding the 2 MiB response limit. Clients must treat response fields
as untrusted and reject a successful reply whose payload shape does not match
the request.

The listener's `listener::protocol::dispatch_request` adapter maps each valid
ordinary request to a typed `SharedQueue` operation: `put`, `list`, `bump`,
`bust`, `del`, or `hist`. It converts the operation result to a protocol
`Response`; queue mutations notify the worker after success. The queue performs
scheduling and persistence through `QueueStore`.
`SharedQueue::claim_next_due()` selects the due head and durably claims it
while holding the store lock; the lock is released before the worker calls
GitHub. A failed post records its timestamp before releasing its claim, so the
cooldown remains in the persisted schedule after a daemon restart. Queue
changes cannot shorten the worker's in-process retry deadline. `bump` and
`bust` may reorder a claimed entry without changing its claim, while `del`
removes it. Completion checks the claim token so a successful post cannot
delete a replacement entry.

`SharedQueue::complete(entry, token_hash, comment_url)` records the successful
attempt, selected token hash, full comment URL, and posting timestamp before
removing the claimed entry. Its durable recovery record contains the history
payload and log offset as well as the entry identifier, claim token and posting
time; reconciliation idempotently appends the history record, finishes the
deletion and `last_post` update, then clears that record. Startup performs this
recovery before reclaiming claims left by an interrupted worker. Failed
attempts are recorded with
`SharedQueue::record_failure(entry, token_hash, error)`; scheduling uses the
later of the successful-post and failed-attempt timestamps as its cooldown
anchor. Failure history writes are logged but do not interrupt claim release or
retry pacing. Other queue-store failures become daemon error responses.

The await use case returns `queue::AwaitOutcome` and an opaque `AwaitToken` for
pending work. The token keeps the wait bound to one queue-entry lifetime; the
filesystem history offset and incarnation stay inside the store adapter. The
queue checks pending work under its mutex, scans history in `spawn_blocking`
outside that mutex, then rechecks the state generation before returning a
result. A retry on a changed generation avoids reporting a stale unknown result
when a concurrent completion lands during the scan. Successful completion
recovery clears a deletion marker only for its own or a queued successor's
incarnation; a durable marker prevents a later lookup from resurfacing an older
URL after a reused entry is deleted. Legacy successful history without a URL
produces an explicit terminal error.

The worker receives an ordered pool of `TokenClient` values. Before each post,
it reads the latest history record's `token_hash` and selects the next matching
pool entry, wrapping at the end. Empty, unreadable, or unrecognized history
selects the first client. `run_worker` requires a non-empty `Vec<TokenClient>`.
`TokenClient` keeps the token's file name for logs, its SHA-256 hash for
history, and the authenticated `Octocrab` client together; the worker never
reads credential paths or environment variables. The public `token_hash()`
helper uses lowercase hexadecimal SHA-256 so rotation state survives token-file
renames without writing credentials to history.

The store accepts at most 1,024 pending entries. `list` returns at most 1,024
entries, and both client and daemon enforce a 2 MiB response limit. A full
queue rejects `put` before persisting a new entry.

When the daemon accepts a comment for enqueueing, it chooses a uniformly
distributed flutter and stores it with that entry. The stored flutter is added
to the complete base cooldown and never shortens it, keeping the queue's
cooldown-derived ETA stable. Keep this operational rule aligned across
configuration, worker tests, the [users' guide](users-guide.md), and the design
document.

### Configuration API

`comenqd::config::Config` is the public runtime configuration model. The daemon
binary uses `Config::load_with_tokens()` to read the daemon's `--config` file,
merge `COMENQD_*` environment variables, apply supported CLI overrides, and
return both the `Config` and its resolved ordered token pool. Production
startup passes that owned pool to `daemon::run_with_tokens()` for client
construction, keeping the files validated during loading identical to the
credentials used by the worker. `Config::load()` and `daemon::run()` remain
compatibility entry points for callers that compose those steps themselves;
`Config::load()` returns only the configuration, and `daemon::run()` resolves
its tokens when called.

`Config::github_tokens()` is a fallible resolver, not an accessor for a cached
pool. Each call reads and validates the configured token files again. The
`NamedToken` values returned by the resolver own both the token string and its
non-secret name: the token file's name, or `default` for a single inline token.
Filesystem and environment reads remain within the configuration adapter.

`github_token_files` is an ordered list of file paths, and takes precedence over
`github_token_file` and `github_token`. Each file is expanded for a leading
`${VAR}`, read as UTF-8 with a 64 KiB limit, trimmed, and rejected if empty. An
explicit CLI `--github-token` overrides every source; otherwise
`--github-token-file` clears the configured pool and selects one file. The
single-token settings resolve to a one-element pool when no list is configured.

Tests and integrations built with the `test-support` feature can use
`Config::from_file(path)` to load a particular file while retaining the
`COMENQD_*` environment merge. Both entry points return a configuration error
when the file is missing or invalid, or when no usable GitHub credential is
available.

Structured tracing records socket probes, the selected credential source (but
never the token value), listener transactions and blocking queue-store
operations, task restarts, each effective cooldown wait, and GitHub post spans.
The post span contains only `task="worker"` and an `outcome` of `success`,
`api_error`, or `timeout`; spans exclude credentials, payloads, repository
names, paths, request identifiers, and raw errors. Operators and developers can
select the emitted detail through `RUST_LOG`. The credential reader rejects
files larger than 64 KiB before trimming their contents. History rendering
shows the first eight token-hash characters; legacy records without a hash use
the `--------` marker.

The daemon attempts to expose Prometheus metrics at `127.0.0.1:9000/metrics`.
The stable metric vocabulary is:

- `comenqd_task_restarts_total{task=listener|worker}` for supervised
  task restarts.
- `comenqd_requests_total` with bounded `operation` labels (`put`, `list`,
  `bump`, `bust`, `del`, `hist`, `await`) and `outcome` labels (`accepted`,
  `failed`, `rejected`); requests without a parsed operation omit the
  `operation` label.
- `comenqd_protocol_transaction_duration_seconds` measures complete listener
  transactions. Its bounded `operation` labels are `put`, `list`, `bump`,
  `bust`, `del`, `hist`, `await`, and `unknown`; ordinary `outcome` values are
  `accepted`, `failed`, `rejected`, or `unknown`. Await terminal outcomes are
  `posted`, `unknown_identifier`, `removed`, `rejected_capacity`, `shutdown`,
  `client_gone`, `posted_without_url`, and `posted_url_too_large`, plus bounded
  store-error categories such as `invalid_identifier`. `error_kind` uses fixed
  classifications and is `none` on success.
- `comenqd_queue_store_operation_duration_seconds` measures queue-store
  operations. Its bounded `operation` labels are `put`, `list`, `bump`, `bust`,
  `del`, `hist`, `history`, `append_history`, `latest_history_token`,
  `next_due`, `claim`, `complete`, `resolve_await`, `release`, `recover`,
  `metrics`, and `unknown`; `outcome` is `success`, `failure`, or `unknown`.
  `error_kind` uses fixed classifications and is `none` on success.
- `comenqd_await_schedule_cache_total{outcome=hit|rebuild|invalidation}` counts
  reuse, rebuild, and invalidation of the schedule projection used for await
  ETAs. The outcome label is limited to those three values.
- These histograms never use request values, paths, identifiers, payloads,
  tokens, or raw errors as labels.
- `comenqd_queue_entries` for the current pending-entry count, without labels.
- `comenqd_queue_bytes` for persisted entry-record bytes plus reserved
  mutation headroom, without labels.
- `comenqd_cooldown_wait_duration_seconds` for cooldown wait durations.
- `comenqd_github_posts_total{outcome=success|api_error|timeout}` for GitHub
  comment-post outcomes.
- `comenqd_github_post_duration_seconds` for GitHub comment-post durations.

## Testing support

Run repository gates through the Makefile. The principal code gates are
`make check-fmt`, `make lint`, `make typecheck`, and `make test`; documentation
changes additionally require `make fmt`, `make markdownlint`, and `make nixie`.

The `comenq` dev-dependency enables Tokio's `test-util` feature for
paused-clock tests of countdown cadence and client frame deadlines. Production
builds use the workspace Tokio dependency without that dev-only feature.

The `make test` target includes public API checks through its workspace
`nextest` run. `tests/public_api.rs` uses explicit type annotations for the
client socket candidates, optional token file, token-file list, and cooldown
flutter. These assignments make the expected public types part of the compiler
checks while reusing the workspace build artefacts. The previous `trybuild`
suite rebuilt the daemon's production dependency graph in a separate target
directory and exceeded the cold-test timeout. The complete `nextest` run has a
10-minute global timeout, after which `make test` runs the Cucumber test target
separately.

Pull requests run the suite once. In `ci.yml`'s `build-test` job the coverage
step selects every workspace target and every feature (`all-targets` and
`all-features`), the scope `make test`'s `nextest` run has, so it is the only
step that runs the `nextest` suite. The Cucumber harness has its own `main`,
which `nextest` cannot execute, so `build-test` runs it in a step of its own
with `make test-cucumber`, the target `make test` calls after `nextest`.
`tests/workflow_contracts/suite_runs_once_test.py` holds the split: it reads
each pull-request step by the command it runs (through
`tests/workflow_contracts/suite_commands.py`, which splits a command the way
the shell does), refuses a second suite run, and requires the coverage step's
scope and the Cucumber step to stay. `make test-workflow-contracts` runs it.

A bare `make` runs the Makefile's default goal, so the reader takes that goal
from the Makefile (`default_goal` in `suite_commands.py`). It applies the
`.DEFAULT_GOAL` assignments in order, as GNU make does (manual, "Other Special
Variables"). `=` and `:=` replace the value, so the last one wins. `?=` changes
nothing because make defines `.DEFAULT_GOAL` itself, empty, before it reads a
makefile. `+=` appends a word, and an empty value clears it. A value of several
words, which make refuses, is not read, and the reader falls back to the first
rule that is not a special or pattern target. `make test-cucumber` counts as a
suite run to the reader; the contract exempts the one expected `build-test`
step by workflow, job and exact command. The tests that pin the reader to GNU
make (`test_the_reader_agrees_with_gnu_make`) run `make -f - -pn` on each
fixture and compare the goal make settles on; they skip, with the reason, on a
host where `make` is absent or is not GNU make.

`tests/workflow_contracts/suite_properties_test.py` states what must hold over
generated input: a suite command is found however it is joined and prefixed, a
harmless command never is, quoted text never adds a suite run, and the default
goal is what applying the assignments in order leaves. It also compares the
reader with real GNU make over every sequence of up to three assignments, which
is exhaustive rather than sampled. The properties use Hypothesis, which
`make test-workflow-contracts` adds to the run.

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

## The build standard

Development, test, lint, and typecheck builds use the parallel `rustc` frontend
(`-Zthreads=8`) and, on Linux, the `mold` linker (`-Clink-arg=-fuse-ld=mold`).
These are defaults in `.cargo/config.toml`, which Cargo discovers on its own,
so a bare `cargo build` gets them. `mold` ships for Linux only, so the linker
flag lives in a Linux-only table and macOS and Windows keep their platform
linker. Cargo selects one `rustflags` source rather than merging them, so every
source repeats the same flags apart from the linker.

An assigned `RUSTFLAGS` replaces the configuration's flags, so the Makefile
recipes that set it compose the standard's flags onto any inherited value (CI's
`setup-rust` exports one). Two builds are deliberately excluded: coverage
assigns `RUSTFLAGS` without the fast flags, because a measurement should not
depend on them, and the release recipe and workflow keep the platform linker,
because they assign `RUSTFLAGS` (even an empty value displaces the
configuration). Cargo has no per-profile `rustflags`, so a direct
`cargo build --release` takes the configuration's flags unless `RUSTFLAGS` is
assigned too.

On Linux, install `mold` before building: the configuration names it, so a
build without it fails at link time. CI installs it through `setup-rust`'s
`install-mold` input. `tests/build_standard_contract.rs` holds the standard. It
reads the configuration sources, the commands `make -n` prints for each
development target on a Linux host and a macOS host (each keeping the caller's
own `RUSTFLAGS`) and for each coverage and release target on a Linux host, and
the `setup-rust` steps of the CI workflows (each must pass `install-mold`), so
a flag lost through a recipe or workflow edit fails there.
`tests/workflow_contracts/ci_rustflags_test.py` holds the two CI steps that call
`cargo` themselves, `Install Merman CLI` and `Test and Measure Coverage`. It
parses `ci.yml` with PyYAML and requires each to assign exactly
`RUSTFLAGS: -D warnings` in its own `env` block: the Merman install builds on
the isolated stable 1.95.0 toolchain, which must never see `-Zthreads`, and a
coverage build is a measurement that takes neither standard flag. An assignment
on a sibling step, the job or the workflow does not count. Run it with
`make test-workflow-contracts`.

### Cranelift

Exception: Cranelift is not the development-profile backend. The repository pins
`nightly-2025-07-22`, but the CI step `cargo +1.95.0 install merman-cli`,
which runs from the checkout, builds on a stable toolchain against
`.cargo/config.toml`, and stable Cargo refuses a
`[profile.dev] codegen-backend` key ("config profile `dev` is not valid"), so
selecting the backend would break that build (recorded 2026-09-29). That build
assigns `RUSTFLAGS`, so the nightly-only `-Zthreads` flag never reaches it.
Revisit if that build moves to the pinned nightly.

## Spelling gate

Run `make spelling` to enforce en-GB-oxendict spelling in tracked Markdown
prose. The target regenerates `typos.toml` from the live shared dictionary and
the repository-specific `typos.local.toml` overlay, then checks the tracked
Markdown. `make markdownlint` depends on this gate, and `make all` runs it with
the repository's release build.

`TYPOS_CONFIG_BUILDER_VERSION` in the `Makefile` pins the
`typos-config-builder` release the gate runs (currently `v0.1.3`). Raise it
together with the regenerated `typos.toml`, never on its own.

Do not edit `typos.toml` by hand. Add only narrow identifier, API, proper-name,
or immutable-fixture exceptions to the local overlay; ordinary prose belongs in
Oxford spelling. Because the dictionary is live, `typos.toml` must never be
drift checked in continuous integration.

The shared `typos-config-builder` CLI refreshes the estate dictionary into an
untracked local cache only when the authoritative copy is newer. A valid cache
remains usable when the network is unavailable.

## CodeScene coverage and the CV-005 contract

Main owns coverage publication (estate rule CV-005). A pull request measures
coverage for its own ratchet check only, in the `build-test` job of `ci.yml`
(`with-ratchet: 'true'`, `publish-artefact: 'false'`); it never contacts
CodeScene. `.github/workflows/coverage-main.yml` runs on every push to `main`
in the `codescene` environment, generates coverage again and uploads it with
`mode: upload`, guarded on `github.ref == 'refs/heads/main'` alone. The token
reaches the uploader only through its `access-token` input, never an `env`.
When the repository has no `CS_ACCESS_TOKEN` (no CodeScene project yet) the
uploader records a `CodeScene upload skipped` notice and a step-summary line
and the run still succeeds.

`make test-workflow-contracts` checks that shape. It runs the shared
`cv005-contracts check --repository .` from the commit named by
`CV005_CONTRACTS_REF` in the `Makefile` (a bump of that pin is a reviewed
change, never a silent upgrade), then the local wiring tests in
`tests/workflow_contracts`. It needs `uv`, Python 3.13 for the checker, and
`pytest` and `pyyaml`, which `uv run --with` supplies. Both runs go through
`UV` and `UV_ENV`, so a `uv` outside `PATH` can be injected. `make all` and CI
run the target; `.github/cv005.toml` holds the repository name and nothing
else. A Dependabot pull request merged by the automerge workflow fires no push
event, so that merge publishes nothing until the next push to `main`
(shared-actions #518 tracks the fix).
