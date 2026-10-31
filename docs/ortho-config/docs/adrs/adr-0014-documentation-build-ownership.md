# ADR-014: Documentation build and process ownership

Status: Accepted.

Date: 2026-10-19.

## Context and problem statement

Rust examples in the README and user's guide are compiled from their exact
published source bodies. A CLI-package integration test exercises the documented
`cargo-orthohelp` command. Cargo supplies that test with the already-built CLI
executable; the test snapshots it into a private run directory instead of
rebuilding the same executable. Rust example workspaces use isolated manifests
and working directories, but compiling a fresh Cargo workspace for each test
can rebuild a large dependency graph. Reusing compiler outputs introduces a
separate race: Cargo may finish one build before its caller has copied a
same-named executable, allowing another process to replace that executable
between build and copy.

The production `cargo-orthohelp` bridge had a related cache-boundary problem.
Its IR cache key is stable, but assigning a new dependency target beneath each
key forced another cold dependency build. Storing the generated manifest and
source directly in the key directory also shared mutable inputs between
invocations. The design needs to keep the IR cache identity stable while
putting generated files in a private invocation directory and building into the
selected package's Cargo-resolved target.

The runner must also retain the machine's configured Rust compiler admission,
warning flags, build parallelism, and shared Cargo package cache while
preventing application secrets and cache credentials from reaching Cargo. It
preserves the non-secret `SCCACHE_GHA_ENABLED` cache-mode setting but excludes
`ACTIONS_RUNTIME_TOKEN`. A timeout must bound quiet compilation as well as
noisy commands, and cleanup must remain within the subprocess tree owned by the
test.

Local validation previously reached 600-second timeouts in four documentation
build subprocesses. Admission-wrapper children were queued or active, the
configured capacity was full, and the managed sccache service was unreachable,
so compilation ran uncached. No Rust source diagnostic was reported, but the
available evidence does not establish how much elapsed time admission consumed.
A controlled fresh-target build of the pre-transplant candidate completed in 66
seconds; cache hit and miss state was unavailable, so this does not establish a
cold-cache duration. The transplanted candidate has no hosted run yet.

## Decision drivers

- Preserve Cargo and compiler ownership of fingerprints, lock files, and the
  configured shared package cache.
- Separate stable IR cache identity from private production bridge inputs and
  the selected package's Cargo target.
- Seed each generated bridge's private lockfile from the selected workspace
  when one exists.
- Reuse safe build outputs without sharing mutable manifests, example inputs,
  run directories, or executable snapshots between workspaces.
- Protect the complete build-and-snapshot interval across separate OS
  processes, including nextest workers.
- Discover executable locations from Cargo's machine-readable output instead
  of assuming a target directory layout.
- Preserve compiler admission, warning enforcement, configured job limits,
  and relevant Cargo configuration while excluding application secrets.
- Bound the whole documentation scenario, each child operation, output
  retention, and process-tree cleanup.
- Remain compatible with Rust 1.89 and supported Unix and Windows hosts.

## Options considered

### Rebuild all dependencies for every workspace

Give each temporary workspace a private Cargo target directory and compile the
entire dependency graph for every scenario. This preserves executable
ownership, but repeats expensive work and caused the observed long validation
run. It is not the selected approach.

### Share Cargo's target directory without a snapshot lock

Cargo serializes its own build operations, but its build lock does not cover
the caller's later executable copy. Another build of a binary with the same
name can overwrite the source after one Cargo process exits and before the
first caller copies it. Cargo JSON reports the path, but the path alone does
not protect the file. This option does not provide snapshot ownership.

### Protect build and snapshot with a process-local mutex

A mutex prevents concurrent threads in one test process from copying during
another build. It does not coordinate separate `cargo test` or nextest
processes. This option leaves the inter-process race open.

### Give each production IR key a separate Cargo target

Place a fresh compiler target beneath every `target/orthohelp/<hash>/` entry.
This treats the IR cache key as a compiler-cache identity too, so each source
fingerprint creates another cold dependency build. It repeats work without
being necessary for private generated inputs.

### Share the selected target and the IR-key input directory

Point every bridge at the selected package target while continuing to write its
generated manifest and source into the stable IR cache entry. The shared target
enables compiler reuse, but the key directory then becomes mutable input state
shared by concurrent invocations. This option does not isolate the generated
crate.

### Use a separate documentation target and an OS-level lock

Place documentation compiler output under
`<parent-target>/documentation-examples/v1/target` and lock it from before
Cargo starts until executables have been copied into the caller's private run
directory. This isolates the documentation build, but duplicates the parent
dependency graph. This option was initially selected, then superseded after
Cargo 1.89's profile-lock behaviour was checked on the supported toolchain.

## Decision outcome

For documentation tests, use the parent target directory resolved by Cargo and
place shared test-only ownership state under
`<parent-target>/documentation-examples/v2`. `BuildState` owns metadata
discovery, the OS-level lock, and recovery marker for both the Rust-example
build helper and the CLI integration test. This sidecar directory is not a
compiler target. Rust examples parse bounded Cargo JSON artefact records and
snapshot reported binaries into their private workspaces while holding the
lock. The CLI integration test instead snapshots the Cargo-provided
`CARGO_BIN_EXE_cargo-orthohelp` executable into its private run directory. Its
Cargo-dispatched command still compiles and runs the generated bridge through
the production bridge helper.

An observed Cargo 1.89 test run released the profile build lock before
executing tests, allowing the sidecar lock to coordinate snapshot ownership
without a parent/child Cargo lock cycle. A regression test now checks this
contract directly from the running test process, without launching another
Cargo workspace. It coordinates documentation snapshot owners and reports
contention as an inconclusive observation because another Cargo process may
hold the profile lock.

Tests that build through this shared sidecar are serialized before their
scenario deadlines start: direct `cargo test` uses a named in-process serial
lock, while nextest assigns the affected binaries to one test group. The
independent-workspace regression retains its concurrent workers inside that
test, so it continues to exercise cross-process artefact ownership.

The identifier-artefact integration test gives its fixture a private target
nested under Cargo's configured workspace target. Its package clean can then
force a fresh build without deleting test executables that the outer Cargo
workspace has already built. The fixture continues to use Cargo's default
shared package cache.

The build cache is test infrastructure. It may reuse compiler output through
Cargo's normal target and package-cache behaviour, but it must not relocate,
clear, or repair the shared package cache. A unique run directory and exact
documented source body remain owned by each workspace. Workspace setup copies
the selected repository's `Cargo.lock`, when present, into the private example
workspace to reuse dependency versions without editing the parent lockfile.
This helper is private to the documentation layout, separate from production
bridge seeding. The concurrent workspace-ownership tests must continue to
demonstrate that same-named fixture binaries run from their own snapshots,
including when independent OS processes build concurrently. The
`cargo-orthohelp` documentation test invokes the actual Cargo
external-subcommand path after taking its private executable snapshot; it does
not rebuild the CLI executable from a JSON artefact path.

For production bridge builds, keep `target/orthohelp/<hash>/` as the stable IR
cache entry, but create a temporary invocation directory beneath it for the
generated manifest, bridge source, copied workspace `Cargo.lock` when present,
and executable snapshot. Build into the selected package's Cargo-resolved
target directory to reuse compiler artefacts across IR cache keys. Cargo may
update the private lockfile copy for the generated root package, but must not
edit the selected workspace's lockfile. Run the JSON-reported executable from
its private snapshot; do not assume a `target/debug` path.

The generated manifest disables automatic binary discovery and names its
explicit bridge target from both the selected package root and unique
invocation directory. This keeps the artefact path private to the invocation,
even when an ordinary workspace target uses the same conventional bridge name,
while preserving the selected target directory and shared dependency cache.

A production bridge helper holds a stable `fs4` operating-system lock across
the Cargo JSON build, executable snapshot, and generated-output cleanup. It
serializes bridge invocations that share the target directory. Cargo's own
target lock does not cover the snapshot after Cargo exits, so the distinct
generated target name also prevents an ordinary workspace build of
`orthohelp_bridge` from replacing the reported executable in that interval.
After the private snapshot succeeds, the helper removes only Cargo-reported
files named for that invocation's generated binary. Missing files are already
clean; other cleanup failures are warnings and do not discard the valid
snapshot. Dependency outputs and shared Cargo caches remain owned by Cargo. On
MSVC an uplifted `<binary_name>.pdb` is part of that same Cargo-reported output
set, so the ownership rule covers it too. The helper is private to the
production bridge build path and is not a general application subprocess API.
The similar documentation-test helper remains test-only and cannot be imported
by the binary.

Production bridge builds retain the configured compiler and documentation
wrappers, applicable wrapper configuration, job limits, warning enforcement,
and coverage instrumentation. Rust flags are not sanitized to create a separate
fingerprint, and the default shared Cargo cache is unchanged. The Cargo JSON
reader retains at most 1 MiB of rendered compiler diagnostics and indicates
when output is truncated; it has no presentation sink. The `run_cargo` boundary
forwards that bounded output to stderr after draining the records and prints a
truncation notice when needed. The nested build asks for Cargo's plain `json`
message format so rustc's rendered diagnostics stay inside that bounded stream
and the inherited stderr cannot bypass the bound. The stable IR cache entry and
path are unchanged; publication remains atomic and skips replacement when the
existing file already contains identical content.

The production bridge emits tracing spans and outcome events for cache lookup,
target-lock acquisition, Cargo build, snapshot, execution, and IR publication.
Events contain stable operation and outcome values plus elapsed time; they omit
paths, cache keys, error strings, and child output. `cargo-orthohelp` is a
one-shot CLI that installs a tracing subscriber but no metrics recorder, so it
reports per-invocation evidence through tracing instead of emitting metrics
that the default invocation would silently discard.

Build-tool environment preparation uses a narrow, explicit allow-list separate
from application environment isolation. It preserves the configured Cargo home,
shared package cache, Cargo configuration, toolchain selectors, `RUSTC_WRAPPER`,
`RUSTC_WORKSPACE_WRAPPER`, `RUSTDOC`, applicable wrapper settings, warning
flags, job limits, and platform requirements. It excludes unlisted application
variables and secrets. A runner-supplied target-specific MSVC linker and SDK
search paths are preserved and used directly; `vswhere` and `vcvars64.bat`
provide the bounded fallback when those paths are incomplete. MSVC discovery
command files belong to each caller's private workspace, separately from the
shared compiler target and ownership state. Cargo continues to read
configuration from the caller's working directory and configured Cargo home.
`cargo llvm-cov show-env` supplies `CARGO_LLVM_COV_TARGET_DIR` rather than
`CARGO_TARGET_DIR`; metadata preparation promotes that tool-selected target for
Cargo metadata, so documentation builds reuse the instrumented parent target.
Nested Cargo builds retain coverage flags and LLVM profile variables but omit
`cargo-llvm-cov` from `RUSTC_WRAPPER`: that variable names the coverage CLI,
which cannot serve as a compiler wrapper for a second Cargo process. Other
configured wrappers remain available through Cargo configuration and the
explicit wrapper settings. Their separate ownership state protects each
build-and-snapshot interval. Documented application binaries continue to
receive their smaller runtime allow-list. The bridge selector remains private to
`BuildEnvironment` and accepts only named toolchain inputs, Cargo target
linker or rustflags settings, and registry index URLs. Windows matching is
ASCII case-insensitive while child processes receive the original environment
key. Credential-bearing proxy and registry values are omitted with a warning
that names the variable but never its value.

One monotonic scenario deadline covers discovery, lock wait, build, snapshot,
and execution; operation-specific build, execution, discovery, and cleanup
budgets can only reduce the remaining time. Output never resets a deadline. The
runner checks expiry before polling and before accepting completion; a
completed poll observed after the deadline remains a timeout. Each output
stream retains at most 1 MiB while continuing to drain. Success requires both
process-root completion and confirmation that owned descendants have stopped
and output streams have drained; root exit and EOF alone are not proof that the
tree is complete. If tree completion cannot be confirmed within the cleanup
bound, fail the operation and retain any cache ownership marker for owner
recovery. Failure reports preserve captured output and truncation state, retain
the primary error if cleanup also fails, and bound cleanup and reader
completion. Supervision applies only to documentation-test subprocesses; it
does not terminate unrelated processes or shared cache services.

Linux checks process-group members through `/proc`, ignoring only terminated
zombies. Windows queries the private job's active-process count. Other Unix
hosts conservatively probe group existence after reaping the root; targets
without non-reaping root observation return an unsupported-operation error.
These checks never signal a Unix group after reaping its leader. Windows
runtime validation remains separate from a cross-platform compilation check.

Readers own their pipe handles and support cancellation independently of EOF.
Unix uses nonblocking reads through test-only `nix`; Windows checks available
bytes with `PeekNamedPipe` before reading. The Windows call has a narrow safety
boundary around the reader-owned handle. Cleanup requests cancellation before
its join allowance expires. A writer that escapes tree ownership cannot keep a
reader blocked indefinitely, and unconfirmed writer or cleanup state remains
explicit in the error.

Each build helper's command factory is private to its owning boundary:
documentation-test support or the production bridge. Regression tests inject
controlled compiler-protocol fixtures through the documentation helper's lock,
JSON observer, and snapshot path. They do not need a live compiler admission
service. Real documented examples still compile their published source bodies,
and the existing concurrent workspace assertions remain.

An abruptly interrupted build or incomplete cleanup leaves an ownership marker.
Every opaque nested CLI callback error retains the marker, including nonzero
exits and preparation failures, because internal capture pipes prevent the
outer runner from confirming nested compiler completion.

A direct Cargo build error from the subprocess runner retains its marker even
if OS-tree cleanup completes: an externally admitted compiler may outlive the
cancelled Cargo process. A fully observed Cargo nonzero exit, and preparation
or snapshot errors after the child lifetime is known, use the existing cleanup
path and remove the marker only after owned-process completion is confirmed. If
the runner cannot confirm that completion within its bound, recovery is
explicit: verify the cache owner's compiler processes have drained, then remove
the marker through the documented recovery path. Do not retry failed or
interrupted builds automatically or infer that a cache service is safe to
restart.

For the production bridge, a build or snapshot error that cannot confirm the
Cargo writer's lifetime also retains a fail-closed marker. A later invocation
must not remove it automatically. Recovery requires confirming that the bridge
Cargo and compiler processes have exited before removing the marker. The bridge
helper may not terminate unrelated processes or restart shared cache services.

## Timeout policy and validation status

The 600-second nextest per-test ceiling remains the default. The BDD target and
the serialized documentation-scenario group have 960-second overrides. The
1,800-second whole-run ceiling, 2,700-second Cargo watchdog, and workflow job
ceilings remain in force. Documentation scenarios use a 956-second operation
envelope, preserving two 2-second allowances within the 960-second per-test
ceiling for child cleanup and test reporting. A subprocess receives the smaller
of its operation budget and the scenario's remaining time. The 60-second
execution and 30-second discovery policies remain independent of that build
budget.

The BDD scenario's 956-second envelope starts after its in-process ownership
gate. A direct-Cargo run serialized those scenarios before starting each
deadline, but one nested bridge build then spent 593.838 seconds waiting on
Cargo's shared package-cache lock and produced no compiler progress. A later
`documented_command` run reached its 595.791-second effective deadline with
only repeated package-cache lock messages, no structured compiler progress, and
no correlated admission telemetry. The 956-second envelope is 1.60 times that
observed lower bound. It counts Cargo-cache and compiler-admission waits
without estimating or subtracting them; the filesystem lease still bounds
ownership across separate test processes.

The attempted corrected cold build exceeded 600 seconds while admission slots
were held and sccache was unavailable. The run did not produce a successful
cold-cache duration. Queued children and full capacity confirm contention was
present; they do not quantify the time it added to that build. A later
fresh-target probe completed in 66 seconds, but cache hit and miss state was
unavailable. The 956-second documentation envelope is sized from the observed
package-cache wait, not inferred compiler-admission delay; the bound continues
to include all queue time. Cache-service availability remains an
environment-owner prerequisite when it causes uncached compilation.

Validation exercises focused runner, workspace, documented-example, and
Cargo-subcommand tests, followed by the repository's sequential gates and
workflow contracts. Regression coverage includes allow-list preservation and
exclusion of `ACTIONS_RUNTIME_TOKEN` and an application secret, policy
selection, bounded timeout output, primary-error preservation, descendant-held
pipes, same-named executable ownership, and separate-process concurrency. The
workflow contract preserves ordering under the per-test, whole-run, watchdog,
and job limits. A successful cold-cache duration remains unestablished: the
attempted cold build exceeded its operation deadline while sccache was
unavailable and admission slots were held, and a later fresh-target run did not
report cache hit or miss state. Keep the build budget unchanged until a
controlled run provides usable cold-build evidence.

## Consequences

- Documentation-test workspaces reuse compiler outputs through Cargo while
  keeping source manifests and executed binaries private.
- Separate locks protect documentation-test snapshots and production bridge
  snapshots. The production bridge lock also owns cleanup of that invocation's
  Cargo-reported binary outputs, but it does not own IR cache contents beyond
  atomic publication.
- Test-only subprocess supervision does not enter the public crate dependency
  surface.
- Production use of `fs4` is limited to bridge build-and-snapshot ownership.
- Build timeouts remain bounded and evidence-rich while measured cold-build
  data is unavailable.
- Cache recovery requires ownership verification and cannot kill processes or
  restart shared services.
- Existing nextest serialization groups remain until their shared-state
  assumptions are independently disproven.

## References

- [Executable documentation-test design][design-infrastructure]
- [Documentation-example implementation guidance][developer-guidance]
- [Test timeout ordering][timeout-guidance]
- [Descriptor locking with `fs4`][file-lock]

[design-infrastructure]:
  ../design.md#419-executable-documentation-test-build-ownership
[developer-guidance]: ../developers-guide.md#public-documentation-examples
[timeout-guidance]:
  ../developers-guide.md#test-timeouts-the-tiers-this-repository-sets
[file-lock]: https://docs.rs/fs4/1.1.0/fs4/trait.FileExt.html
