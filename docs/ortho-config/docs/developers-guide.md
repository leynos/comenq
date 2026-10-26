# Developers guide

This guide documents how contributors work with tests in this repository. It
focuses on behavioural tests because they span multiple crates and have the
highest maintenance cost when patterns drift.

## Current testing strategy

The workspace runs one unified test workflow via Make targets:

- `make check-fmt`
- `make lint`
- `make test`

These are required quality gates for code changes. Behavioural coverage runs
inside the standard Rust test harness, not a bespoke test runner.

`make lint` also runs `interrogate` over every tracked and non-ignored untracked
`*.py` and `*.pyi` file. Git path discovery includes hidden directories and
does not restrict the scan to package and test directories, so Python sources
used by GitHub workflows and actions, scripts, tests, tools, and benchmarks are
covered as well.

The inventory is discovered inside the recipe and passed NUL-separated through
`xargs -0`, not through a `$(shell …)` variable. A `$(shell …)` expansion
splits its output on whitespace and cannot quote what it finds, so a file named
`local helper.py` would reach `interrogate` as two arguments; `xargs -0` execs
the command directly and keeps every path whole.

Coverage is judged by two checks, because `--fail-under 100` alone is not
enough. `interrogate` rounds the percentage to the precision of the threshold
before comparing it, so a run missing one docstring in 2000 reports `100.0%`
and passes. The recipe therefore also reads the `Miss` column of the verbose
summary, which is an exact count, and fails on any non-zero total. Because
`xargs` may split a large inventory into batches, every batch's `TOTAL` row is
summed rather than just the last one: the missing item need not fall in the
final batch.

Discovery writes to a temporary file instead of straight into a pipe. A
pipeline reports the status of its last command, so a failing `git ls-files`
would otherwise let an empty batch pass the gate; the recipe runs under
`set -e` and aborts on that failure. A genuinely empty inventory still passes
and does not invoke `interrogate` at all, which keeps it distinct from a run
that produced no summary.

The shared workspace Clippy policy denies `missing_docs_in_private_items`.
Document private modules with `//!` and private items with `///`, including
build support, examples, and shared fixtures. Explain internal contracts such
as ownership, precedence, error handling, and generated-code assumptions.
`make lint` applies the policy across workspace targets and features; keep new
members opted into the workspace lint table. An unavoidable generated-code
exception must use an item-scoped `#[expect(..., reason = "...")]` that names
the concrete limitation.

## Clap default inference

The `cli_default_as_absent` default path is split between parse-time metadata
and generated loader code. `ortho_config_macros/src/derive/parse/clap_attrs.rs`
retains the raw `default_value`, the leaf type and field shape, and parser
hints such as `value_parser`, `value_enum`, delimiters, and enum case handling.
Keep this metadata in the parse intermediate representation (IR); do not
reconstruct it from generated tokens later in the pipeline. The parser-faithful
approach is recorded in the [design decision log](design.md#9-decision-log).

The parse layer accepts scalar, `Option<T>`, and `Vec<T>` fields. It rejects
nested wrappers and map fields when their clap string default cannot be
replayed faithfully. Unsupported shapes should receive a focused compile-time
diagnostic that points to an explicit `#[ortho_config(default = ...)]`
alternative. Parser settings that change tokenization or accepted spelling must
either be retained in the IR and applied to the synthetic argument, or cause
the shape to be rejected during parsing.

`ortho_config_macros/src/derive/build/defaults.rs` owns the replay boundary.
`DefaultStructInit` keeps fallible default resolutions separate from the
infallible defaults struct fields: generated code builds a one-argument clap
command, parses the captured default, and then supplies the resolved value to
the defaults layer. A conversion failure is appended to the existing error
accumulator as `OrthoError::DefaultValueConversion`; it must not be handled by
`unwrap`, `expect`, or a panic in generated code. Explicit
`#[ortho_config(default = ...)]` remains higher precedence than inferred clap
metadata.

When changing this path, update parse-IR tests, generated-loader integration
tests, and compile-fail coverage for unsupported shapes. Include cases where a
file or environment value overrides the inferred default, while an explicit CLI
value still wins. Run the standard quality gates before requesting a CodeRabbit
review.

### Feature gates in generated code

Derive output is compiled in the caller's crate. A `#[cfg(feature = "...")]`
attribute emitted by the derive therefore tests the *caller's* features, which
an application rarely declares, rather than `ortho_config`'s. Before 0.10.0 the
`cli_default_as_absent` `CliValueExtractor` implementation was guarded this
way, and its `not(feature = "serde_json")` arm emitted a `compile_error!` in
every application without its own `serde_json` feature.

The derive now emits that implementation with no feature test. This is sound
only because `ortho_config` does not build without its `serde_json` feature:
although the feature is declared optional, the discovery, file-loading,
profile, and merge modules use `serde_json` unconditionally. A gate whose
disabled arm can never be compiled would be untestable dead code.

Do not emit `cfg` attributes for `ortho_config` features from the derive. If
generated code must depend on a feature that `ortho_config` can genuinely be
built without, select the code inside `ortho_config` instead, for example with
a hidden `#[macro_export]` macro defined once per feature state, and add a
compile-fail fixture that builds `ortho_config` without the feature. The same
applies to `cli_default_as_absent` if `serde_json` is ever made genuinely
optional. The decision is recorded in the
[design decision log](design.md#9-decision-log).

Two tests cover the current boundary:

- the generator unit test in
  `ortho_config_macros/src/derive/generate/ortho_impl.rs` requires the
  implementation and rejects any `cfg` or `compile_error!` in the expansion; and
- the documented `cli_default_as_absent` examples, such as
  `migration-cli-default-after`, compile and run in the generated documentation
  workspace. That crate declares no features and takes `ortho_config`'s default
  features, which `workspace_tests.rs` pins, so it stands for a downstream
  application.

### Scoped use of `googletest` and `pretty_assertions`

The `googletest` and `pretty_assertions` dev dependencies (decision D9 of
[execplan 9-1-1](execplans/9-1-1-profile-metadata.md)) are scoped to the
profile test modules and the profile-layer tests only. New tests elsewhere
should keep using the existing `assert_eq!`/`ensure!` conventions; do not
migrate existing tests to these crates. When a new assertion needs richer
matcher output, prefer `googletest::assert_that!` with matchers for shape
inspection and `pretty_assertions::assert_eq!` for scalar equality, and keep
the use local to the module that needs it.

### Nextest test-group serialization

`.config/nextest.toml` places nine documentation and cache users in the
single-threaded `orthohelp_cache` group: `rstest_bdd`, `powershell_windows`,
`nested_subcommand_end_to_end`, `documented_command`,
`documentation_examples_rust_tests`, `agent_context_contract`,
`policy_report_contract`, `golden_tests` and `identifier_artefact_e2e`. These
binaries can clear the shared BDD cache or build documented examples against
Cargo's shared target. A named process-local `Mutex` serializes scenarios
within one test process, including direct `cargo test`. It cannot coordinate
independent test processes, so filesystem leases also protect cache ownership
across them.

The `powershell_windows` cases invoke `cargo-orthohelp --format ps` for the
same package and ephemeral bridge directory (`target/orthohelp/<hash>/`). On
Windows, `cargo build` holds a read lock on `Cargo.toml`; a concurrent
truncate-and-rewrite from another invocation violates that lock. Keep the whole
group serialized unless the shared-state access has been eliminated.

The `boundary` binary has a group of its own, `boundary_packaging`. Its
packaged contracts materialize the release once per *process*, and nextest runs
each test in its own process, so without the group six of them run
`cargo package` and unpack `target/localization-boundary/packaging-target` side
by side. Cargo's package-cache lock does not cover that: it does not hold
readers out while a package root is removed and unpacked, so one process can
replace a root another is reading. `make boundary` and `make test` start one
process for the binary and need nothing extra, which is why the group exists
only for nextest.

That group also fixes the per-test allowance arithmetic. One packaged test runs
both `cargo package` invocations, each bounded at 240 s by `PACKAGE_DEADLINE`,
so the pair has to fit inside the 600 s base allowance with room for the root
checks around them. A deadline that did not fit could never fire: nextest would
stop the test first, and the error naming the invocation that hung would never
be printed. `boundary_packaging_test.py` reads the group, the manifest and the
deadline so neither half can drift from the other.

On Windows only, one further override gives each of four binaries every nextest
slot (`threads-required = "num-test-threads"`), so each runs alone:
`ortho_config::crate_path_trybuild`, `ortho_config::declarative_merge_trybuild`,
`ortho_config::compile_fail` and `cargo-orthohelp::compile_time`. Each drives
a cold child `cargo` build through trybuild in the first coverage pass. Run
side by side on Windows, those builds exhausted the 600 s per-test allowance:
every Windows failure in runs 36127709137, 36127710357, 36127710423,
36127710779 and 36127709460 was `compile_time`'s `must_use_compile_tests` at
600 s. That entry keeps its 600 s ceiling, and Linux keeps its parallel
execution. `windows_trybuild_isolation_test.py` holds the exact binary set, the
platform, the reservation, that entry's allowance, and the absence of any other
slot reservation.

Exclusivity and the raised allowance are two fixes for one failure, and the
file carries both. nextest resolves each override field from the **first**
matching entry that sets it, so the Windows entry above, being first, decides
the four binaries it names; the wider entry below is second and supplies the
rest of the class. The four therefore keep both 600 s *and* run alone, while
`boolean_cli_trybuild`, `env_source_trybuild`, `generated_lint_trybuild`,
`localization_trybuild`, `localized_parse_trybuild` and `subcommand_trybuild`
take the 960 s entry, and Linux takes 960 s for all ten. Merging the two would
have to choose between running alone and the larger allowance; leaving both
keeps each measured remedy at the binaries it was measured on.
`trybuild_tier_test.py` reads the widening and
`windows_trybuild_isolation_test.py` the reservation, so neither remedy can be
removed by editing the other's entry.

The reservation costs Windows wall time. On run 36195916343 the job took 53.5
min, against a 38 min median over the preceding pull-request runs, and
`must_use_compile_tests` took 158.7 s. A single-threaded test group for the
same four binaries was measured as an alternative on run 36264533224. It saved
no wall time (52.9 min), and `must_use_compile_tests`, still competing with the
rest of the suite, took 523.5 s of its 600 s allowance, so the reservation was
kept.

#### Packaging must resolve the way a clean runner resolves

The packaged contracts are the only place in this repository that exercises a
stranger's dependency graph, and the failure they are most likely to hide is
resolution that only works on a developer's machine. Two rules keep them honest.

**Patch every internal edge the invocation resolves, not just the ones it
packages.** `cargo package` builds every dependency table, dev-dependencies
included, so a path member that is only a dev-dependency still has to be
patched, or its rewritten version requirement is looked up in an index that may
never have seen it. Patches are keyed by *package* name, which is not always
the directory name: `ortho_config_test_helpers` lives in `test_helpers`.

**Do not remove `--verify` to get past a packaging failure.** It is the step
that unpacks each `.crate` into the `package/<name>-<version>/` root every
consumer and manifest contract reads. `--no-verify` writes the archives and
unpacks nothing, so the failure moves from packaging to path resolution. The
remedy for a verify-time failure is to remove the cause - a dev-dependency
cycle is the one that has actually bitten here, because verifying the macros
tarball pulled the patched library back in through its normal edge and
duplicated the package under verification.

A packaging failure on a *clean* registry is not reproducible by re-running the
command locally: the developer's `CARGO_HOME` already holds the index entries
the runner lacks. Reproduce it with an isolated `CARGO_HOME` that has the
internal crates' entries removed, and check the output for
`patch … was not used in the crate graph` - that warning means the patch under
test was never consulted, and the probe proved nothing.

## Subcommand dispatch changes

Cargo's external-subcommand contract is an entry-point concern, not a
configuration-loading concern. When the way a `cargo-*` binary accepts or
forwards the injected subcommand token is changed, update all of the following
in the same change:

- `docs/design.md` §4.17 and
  [ADR-0004](adrs/adr-0004-cargo-external-subcommand-entry-point.md).
- `docs/roadmap.md` if the work remains tracked there.
- Any user-facing guide or README that shows `cargo <name>` or
  `cargo-<name> <name>` invocation.
- Regression coverage for both `cargo <name> [OPTIONS]` and
  `cargo-<name> <name> [OPTIONS]` once the repository adds or revises those
  tests.

## Derive expansion ownership

`ortho_config_macros/src/components.rs` owns the intermediate expansion bundle
and loader-token assembly shared by generated implementation paths. Proc-macro
entry points and CLI field extraction metadata remain in `lib.rs`. Output
generators consume the assembled bundle rather than independently rebuilding
discovery settings or CLI key mappings.

Derive-side file-discovery token assembly belongs to
`ortho_config_macros/src/derive/load_impl/discovery.rs`. Legacy and configured
builder setup, including lookup-only source wiring, stay behind the parent
module's `build_file_discovery` entry point. The generated block returns
ordered layers and a separate discovery-failure signal. Policy resolution
remains in `derive/policy_impl.rs`, and environment merging remains in
`build_env_section`.

Field documentation resolution and duplicate-source checks remain in
`derive/generate/docs/fields/mod.rs`. Its private `emission` module accepts
resolved components and emits the public IR initializer without discovering
fields or applying policy. Only the parent field builder uses this boundary;
other generators continue through `build_fields_metadata`.

MAML parameter prose belongs to
`cargo-orthohelp/src/powershell/maml/paragraphs.rs`. Only the MAML renderer
consumes its ordered paragraphs; XML structure and escaping remain in
`render.rs`. Other output formats retain their own formatting contracts.

## CLI long-flag validation ownership

`LongFlagCandidate` in
`ortho_config_macros/src/derive/build/cli/cli_flags/validation.rs` owns one
borrowed field identifier and its raw `cli_long` spelling during validation.
Only `validate_cli_long` constructs it; other derive-builder code uses the
`cli` module facade. The context is limited to one spelling check and is not a
reusable representation of a validated flag.

The raw spelling carries no validity promise. Validation checks empty input,
then a leading dash or underscore, then the ASCII alphanumeric-or-dash grammar,
and finally clap's reserved `help` and `version` names. This order preserves
the most specific syntax diagnostic and its source-field span. Duplicate-name
checks remain in field resolution, where declaration-order reservations are
available. Keep this context limited to syntax and reserved-name validation;
short flags and duplicate tracking compose at their existing owners.

## Schema ownership

Documentation IR, agent context, and policy reports have separate owners. See
[ADR-0003](adrs/adr-0003-define-schema-ownership-for-agent-native-contracts.md)
for the accepted decision.

Add localized human-documentation fields to `ortho_config::docs` only when they
are required by generated documentation, localization, roff, PowerShell help,
or other human-facing reference material. Those fields are versioned by
`ORTHO_DOCS_IR_VERSION` and exposed through `OrthoConfigDocs`.
`OrthoConfigSubcommandDocs` is part of the same human-documentation IR contract
and uses the same versioning boundary for recursive subcommand metadata.

Adding, removing, or changing a field in `ortho_config::docs` requires a
matching change in `cargo-orthohelp::schema`, which mirrors the IR for the
standalone generator. The two definitions must stay byte-for-byte aligned;
`ORTHO_DOCS_IR_VERSION` is bumped in both, and the version-alignment test fails
if the pair drifts. Add fields with `#[serde(default)]` so documents written
before the field existed still read correctly, as ADR-0003 requires.

Add compact agent invocation fields to `ortho_config::agent_context` when
downstream applications need a reusable machine-readable command contract. Use
`ORTHO_AGENT_CONTEXT_SCHEMA_VERSION` for compatibility. Do not add Fluent
message identifiers, localized long prose, or renderer-specific output
structures to the agent-context schema.

Use `AGENT_CONTEXT_KIND_SUFFIX` and `agent_context_kind` as the single source
for the agent-context `kind` discriminator. Do not hand-format
`"<tool>.agent_context"` at call-sites. `kind` identifies the payload family;
compatibility detection stays on `ORTHO_AGENT_CONTEXT_SCHEMA_VERSION`.

Skill manifest descriptors are part of this agent-context contract: keep
`SkillManifest`, `SkillCommandRef`, and `AgentContext.skill_manifests` in
`ortho_config::agent_context`, and keep downstream manifest prose
application-owned.

Profile support (roadmap 9.1.1) adds two agent-context fields with an
omitted-when-absent rule: `ProfilesDeclaration.selection` and
`ProfilesDeclaration.list_command` are `Option` fields that serialize only when
present, so the unsupported `{ "supported": false }` case stays byte-identical
to the pre-profile schema. New optional fields must follow that rule and
provide a `ProfilesDeclaration::unsupported()`/`supported()` constructor, so
downstream construction of the declaration itself survives later field
additions. Retyping `AgentContext.profiles` is nonetheless a break for callers
that build `AgentContext` with a struct literal: they must either update the
literal's `profiles` value or construct it with one of those two constructors.
The constructors keep future *field* additions non-breaking, not this field's
retype. The derive emits the matching IR `DocMetadata.profiles` only for
opted-in structs; the `cargo-orthohelp` bridge maps it into the declaration.
Adding `DocMetadata.profiles` is a second break of the same kind, and it
reaches two copies of the type — `ortho_config::docs::DocMetadata` and the
`cargo_orthohelp::schema::DocMetadata` mirror. The field is
`#[serde(default)]`, so the wire contract stays additive and older JSON still
deserializes, but a defaulted field is still required in a struct literal:
every external literal, including the linked Weaver and Netsuke consumers, must
name `profiles: None`. The runtime "which profile is active" concern is
`SelectedProfile` and `ProfileLoadOutcome`, deliberately separate from the
static agent-context contract.

`ortho_l10n::try_normalize_segment` is the single source of truth for strict
runtime and derive-time Fluent identifier segments. Reuse it, or the
complete-identifier helpers re-exported beside it, from command localization,
derive output, and future lookup-id generation instead of duplicating ASCII
normalization rules. Keep the tolerant catalogue load path in
`ortho_l10n/src/fluent.rs` separate: it exists only to pre-normalize
hand-authored resource ids such as dotted catalogue keys before Fluent parses
them, and must not be used to validate generated command ids.

`#[derive(OrthoConfig)]` emits `OrthoConfigLocalization` for the deriving
configuration. Its `LOCALIZATION_BASE` is the dotted catalogue root from
`#[ortho_config(localization_base = "…")]` (or the normalized application-name
default), while the command-level constants and `ARG_IDS` use the normalized,
hyphen-joined Fluent ids. Each `ArgLocalizationIds` entry records the Clap
argument name and its `help`, `long_help`, and `value_name` ids. Argument ids
are validated for normalized collisions during expansion; flattened,
subcommand, and `skip_cli` fields are excluded from `ARG_IDS`. Set
`ORTHO_CONFIG_EMIT_IDENTIFIERS=1` for an opt-in standalone inventory; the
derive writes it through per-expansion fragments and merges them
deterministically. Mounted command-tree identifiers remain owned by the
path-aware documentation IR.

The rendered layout depends on the inventory size. Up to 1 MiB the whole
inventory is written to `${OUT_DIR}/ortho-config/cli-identifiers.json`. Above
that cap — measured on the serialized document, and never splitting a batch
below one entry, so a single oversized entry stays in one part — the output
becomes `cli-identifiers.index.json` plus the numbered
`cli-identifiers.<n>.json` parts it lists. Read the index rather than assuming
the single-file path when the cap may be exceeded. A render prunes every file
of the *other* layout before publishing, so a build that switches
representations never leaves a stale file where a reader would find it.

`OUT_DIR` is package-scoped, so every writer holds an exclusive
`cli-identifiers.lock` across the complete fragment-write, merge, prune, and
publication sequence; the unit tests in `artefact/artefact_lock_tests.rs` spawn
two real processes to prove the exclusion and the release, rather than relying
on an in-process mutex.

That lock serializes *writers among themselves*. It is cooperative, so it does
not exclude a process that never takes it: a reader — a build script listing
the directory, or any other tool — can observe an intermediate state between
the prune and the writes that follow it, and can read the fragments before a
render. Atomic per-file writes keep each file whole, but the directory as a
whole is not replaced atomically and the layout may change between renders. A
reader that must be consistent has to take the same lock or retry.

A second opt-in, `ORTHO_CONFIG_ARTEFACT_REPORT=1`, turns on the observation
channel that explains the same sequence when a build is slow or fails. It
writes three files below the artefact root, none of which is part of the
inventory and none of which the prune matches.
`cli-identifiers.lock-wait.<pid>.json` is written once a publication has been
blocked on the lock for five seconds, and refreshed every second while the wait
continues, so a build that appears hung can be diagnosed as waiting rather than
deadlocked. `cli-identifiers.build-report.<pid>.json` records the process, the
expansion, the stage the publication reached — `lock_wait`, `fragment_write`,
`merge`, `render`, `prune`, or `publish` — with its lock-wait and total
durations in whole milliseconds, a bounded `completed`/`failed` outcome, a
`written_at_ms` timestamp, and, on failure, a bounded `io`/`serialization`
error category. `cli-identifiers.observation-errors.<pid>.jsonl` is the
channel's own failure record: it appends one JSON Lines record per observation
that could not be written, carrying the operation, the same bounded category,
and the underlying error's text.

All three are per-process because `OUT_DIR` is package-scoped and several
compiler processes may publish into one directory; the error record is what
makes the best-effort contract safe, since a build with no report at all is
otherwise indistinguishable from one whose report was dropped. Every write is
best effort: the channel explains builds and must not be able to fail one, so
an unwritable directory leaves the publication's own result untouched — the
report write on the acquisition-failure path is attempted even though no lock
was taken, precisely so that the build which could not start is still explained.

**A marker's presence is not evidence of contention now.** The wait marker is a
snapshot, not a lock: a process killed mid-wait never retires its marker,
failed cleanup leaves one behind and records that in the error log, and process
ids are reused, so a live pid does not prove the writing process is still
running. Read `written_at_ms` against the build's own timeline; a marker older
than the build under investigation is a leftover, not a clue. This is why the
filename carries the pid *and* the body carries a timestamp — neither alone
establishes freshness.

**A report describes the last publication its process completed.** A report is
named for the process, and each publication replaces what that process reported
before, so a crate with several derives leaves the report of its last one
rather than a file per derive per build. A replacement that fails leaves the
*earlier* report in place — the write is atomic, so a failed rename never
truncates the destination — which is the other reason `written_at_ms` and the
recorded expansion are load-bearing rather than decorative.

`artefact/artefact_report.rs` owns the channel's vocabulary, serialization, and
the build report; `artefact/artefact_wait_marker.rs` owns the announcement and
its retirement; `artefact/artefact_channel_log.rs` owns the failure record;
`artefact/artefact_observation.rs` owns the stage and duration state one
publication accumulates; `artefact/publish.rs` is its only producer. A build
that did not opt in takes the plain blocking `lock`, so the common case pays no
polling cost for a channel it never enabled. Because a blocked publisher does
not hold the lock, the wait marker is written through `write_private_atomic`,
whose temporary derives from the already-unique destination; the shared
`atomic_write` temporary would let two blocked publishers collide.

`artefact/publish.rs` owns that whole write path. It takes the lock, writes the
fragment, merges, renders, prunes, and publishes — one function,
`publish::publish`, called by `emit` and by the lock tests alike, so the
exclusivity the tests observe is the exclusivity the compiler runs. The guard
is threaded by reference into each step rather than merely bound in `publish`:
a local that is never read can be dropped early by a later edit while every
test still passes, because a publisher that acquires and immediately releases
still *blocks* on acquisition and is therefore indistinguishable to an outside
observer. Passing `&LockGuard` makes releasing it early a borrow-check error
instead of a silent regression. `emit` is correspondingly the **only** reader
of the ambient environment (`OUT_DIR`, `CARGO_CRATE_NAME`, and the opt-in
flag); everything below it takes a `publish::Publication` of explicit values.
That boundary is what lets a test publish into a temporary root without
mutating the process environment, which matters because build scripts and tests
share one `OUT_DIR` per package and an unset variable would otherwise be
indistinguishable from a stale one.

`artefact/artefact_probe.rs` owns the stale-fragment policy:
`source_still_exists` is its only entry point, `probe_metadata` is the only
`std::fs::metadata` call site for it, and `classify_metadata` is the only copy
of the rule that `NotFound` alone means stale while every other metadata error
aborts the merge. `merge_fragments` is the single production caller; the
identity artefact's merge and the merge-level regression tests are its only
call sites. For OS-independent policy tests, `with_metadata_outcome` is a
test-only, thread-local injection that matches one exact probe path and is
compiled out of non-test builds. It exists because no filesystem layout
provokes the same metadata error portably: a regular file in a parent position
yields `NotADirectory` on Unix and `NotFound` on Windows. Which `ErrorKind` a
platform reports for a given layout is an implementation mapping, not a
portability guarantee, so a policy test built on a real layout would assert a
different kind per platform. The policy tests in
`artefact/artefact_probe_tests.rs` inject the outcome and drive the shared
classification, while the merge-level tests in
`artefact/artefact_fragment_tests.rs` inject through the same boundary and
assert the diagnostic retains the path and the injected error marker. It is
**not** a general-purpose filesystem abstraction and must not grow readers, an
enumeration capability, or call sites beyond this probe.

Normalized argument-ID collisions within one derived struct fail at compile
time. Hand-built and dynamic command trees retain the runtime panic contract.
Cargo does not include proc-macro environment reads in its rebuild fingerprint;
after changing the opt-in variable or source, refresh the artefact with
`cargo clean -p <package> && ORTHO_CONFIG_EMIT_IDENTIFIERS=1 cargo build -p <package>`.
Do not rely on a warm build to observe changed artefact output.

Use `LocalizedParse` for default-base localized clap parsing and
`parse_localized_command` when callers need to pass a command that has already
been localized with `LocalizeCmd::with_base`. Keep the two parse-error paths in
that helper distinct: errors from `try_get_matches_from_mut` already have the
command available, while `FromArgMatches::from_arg_matches` errors must be
enriched with `with_cmd(&command)` so missing-subcommand translations retain
`valid_subcommands`.

Compile-time coverage for this API belongs in `trybuild` pass/fail cases that
exercise the public trait bound (`LocalizedParse: clap::Parser`). Fluent-unsafe
command identifiers are still a documented runtime panic contract owned by
`message_id_for` and `LocalizeCmd::localize`, so keep that coverage in ordinary
runtime panic tests until derive-emitted identifiers move validation to compile
time.

`ortho_l10n` owns the shared fallible identifier normalizer. The runtime panic
and macro spanned diagnostics wrap that implementation; argument fragments use
the shared separator without imposing the root rule. See
[ADR-0015](adrs/adr-0015-localization-runtime-boundary.md). Generated docs IR
receives mounted paths through path-aware trait methods; handwritten trait
implementations use their provided fallback. The optional identifier artefact
uses per-expansion fragments and a deterministic merge below `OUT_DIR`; its
schema has standalone scope until a downstream consumer joins it to docs IR.

## CLI localization surface

Phase 11 promotes the localization helpers from the example into the public
crate surface. `LocalizeCmd`, `LocalizedParse`, and `parse_localized_command`
are implemented; `#[derive(OrthoConfig)]` also emits the public
`OrthoConfigLocalization` contract and supports opt-in identifier artefacts.
Their current contracts are documented above. `localized_default` is recognized
by the derive but rejected with a deferral diagnostic, so the derive-level
default-value feature remains future work.

That deferral is about the derive attribute, not about catalogues. `ortho_l10n`
now owns the runtime and its embedded English and Japanese resources;
`FluentLocalizer::embedded` loads them, and `FluentLocalizer::builder` layers a
consumer's catalogue over them. See
[ADR-0015](adrs/adr-0015-localization-runtime-boundary.md) for why the runtime
is a separate crate and why `ortho_config` re-exports its actual types instead
of adding bridge traits.

The remaining proposals belong at clear ownership boundaries: locale lifecycle,
Clap coverage, and translation interfaces in the core localizer; identifier
generation in `ortho_config_macros`; and aggregate translation reports in
`cargo-orthohelp`. These proposals are not available crate APIs yet:

- `BootLocalizer` / `BootHandle` define the boot-to-final typestate lifecycle.
- `FluentEmbedLocalizer` adapts an application-owned loader. No
  `i18n-embed-bridge` feature currently exists; the design proposes a dedicated
  optional feature for this adapter and dependency. `cargo-orthohelp`
  translation tooling is an integration point: the bridge adapts an
  application-owned loader and neither creates nor owns it.
- `ClapErrorCoverage` and the exhaustive error-kind count gate remain proposed.
  The proposed `ortho_config/build.rs` emits
  `ORTHO_CONFIG_CLAP_ERROR_KIND_COUNT` for the total variant count of the
  supported resolved `clap_builder` identity and verifies the compiled `clap`
  facade re-export. Only then is `const_assert_eq!` against the matrix length a
  mechanical gate. `ClapErrorCoverage` reports translated identifiers the
  supplied localizer cannot resolve. See
  [design §6.1.1](cli-localization-design.md#611-proposed-coverage-gate) and
  §6.2 for the contract.
- `MissingTranslationReporter` / `MissingTranslationEvent` and
  `cargo-orthohelp` JSON translation coverage reports remain roadmap work; the
  proposed reporter aggregates events under
  `target/orthohelp/missing-translations/<locale>.json`.

The proposed lifecycle treats each parse as a locale snapshot: parsing uses
`BootHandle<Boot>`, and finalization consumes it to produce `BootHandle<Final>`
after configuration is merged. Finalization does not swap the locale of a
running service. A long-lived service that supports locale changes must own a
swap primitive (the design's baseline is `arc_swap::ArcSwap<dyn Localizer>`)
and load the current snapshot at each request boundary. See
[the design](cli-localization-design.md), §§5.2–5.3 and §12. Translator
diagnostics and aggregation are specified in
[design §9](cli-localization-design.md#9-translator-diagnostics) and roadmap
§11.

Add agent-native warning and hard-failure report fields to
`cargo_orthohelp::policy` while `cargo-orthohelp` is the only emitter. Use
`ORTHO_POLICY_REPORT_SCHEMA_VERSION` for compatibility and keep rule
identifiers, finding codes, severities, and source locations machine-stable.
Extract the report model into `ortho_config` only after a new ADR approves
shared ownership.

The `--check-agent-native` lint runs without the bridge build and writes
exactly one JSON `PolicyReport` document atomically to `policy-report.json` in
the output directory, plus a one-line human-readable summary to stderr. It
evaluates the package's policy configuration; the bridge-driven behaviour rules
below are exported as library API because they need a compiled agent context
that a policy-only run deliberately does not build.

The behaviour rule IDs live under `agent-native.behaviour.*` and their machine
codes are stable across runs:

- `agent-native.behaviour.destructive-bypass` / `destructive_bypass_missing`;
- `agent-native.behaviour.prompt-bypass` / `prompt_bypass_missing`;
- `agent-native.behaviour.bypass-unknown` / `bypass_flag_unknown`;
- `agent-native.behaviour.undeclared` / `interaction_unknown` and
  `mutation_unknown`.

`cargo_orthohelp::policy::rules::behaviour::check_behaviour(context, mode)` is
the entry point, and it is total: `PolicyMode::Off` returns an empty report
without evaluating any rule. Each `PolicyResult.location` is `null` because
agent context carries no source spans; keep the `message` self-contained
(command path plus the exact annotation to add).

Use `rstest` for schema unit tests. Add `rstest-bdd` behavioural scenarios and
end-to-end tests when a change affects observable CLI behaviour, generated
artefacts, persisted output, integration contracts, stdout, stderr, or exit
codes. Do not add Kani, Verus, or property-test tooling unless the change
introduces a substantive invariant across a range of inputs, states, orderings,
or transitions.

When adding metadata fields, record the legacy default beside the field
definition and cover the absent-field case in tests. Defaults must be explicit:
do not infer JSON support, mutation effect, interaction mode, exit classes,
pagination, profile support, capability provenance, delivery support, feedback
support, or execution-ledger support from command names or missing data. Apply
defaults in OrthoConfig readers, generators, or transforms; do not rely on JSON
Schema validation to mutate payloads.

Keep generated human documentation compatible unless a roadmap item approves a
versioned migration. The `cargo-orthohelp` `ir`, `man`, `ps`, `agent-context`,
and `all` formats, their accepted spellings, generated paths, and process
success/failure contract are externally visible behaviours. Add policy and JSON
status surfaces beside those formats rather than changing them.

Keep schema ownership aligned with ADR-0003. Localized human-documentation data
belongs in `ortho_config::docs`, compact reusable agent context belongs in
`ortho_config::agent_context`, and policy reports stay in
`cargo_orthohelp::policy` until a later ADR extracts a shared report model. Do
not introduce crate dependency cycles to share convenience helpers; move shared
contracts downward instead.

### Generating agent-context output

`cargo-orthohelp --format agent-context` reads the same bridge `DocMetadata` as
the human documentation generators and writes `<out>/agent-context.json`.
`cargo-orthohelp --format all` writes the same file beside IR, man-page, and
PowerShell artefacts. Keep the transform projective: it may copy or derive
compact command metadata from the bridge IR, but it must not inspect rendered
roff, PowerShell help, or localized IR output.

`--format agent-context` is the generator format. Downstream applications that
emit their own runtime payload expose `context --json` as defined by
[ADR-0007](adrs/adr-0007-downstream-context-command-naming.md).

Agent-context output is not localized. The current transform may use the short
en-US command description as `AgentCommand.summary`, but it must not copy
localized long help, Fluent identifiers, roff fragments, or PowerShell wrapper
structures into `ortho_config::agent_context`.

Represent positional inputs by leaving `AgentInput.long` absent. The adapter
detects a positional input from existing CLI metadata when
`cli.long.is_none() && cli.short.is_none() && cli.takes_value`. Do not add a new
`AgentInput` kind field unless a later ADR or roadmap item changes the schema
ownership decision.

Treat `AgentInput.default` as display-only. It is normalized for stable
goldens, but it is not executable or machine-parseable.

`AgentCommand` carries the declared behaviour surface populated by the bridge
from the documentation IR `behaviour` block:

- `interaction_mode` — `InteractionMode::Unknown` when undeclared;
- `mutation_effect` — `MutationEffect::Unknown` when undeclared;
- `bypass_flag` — `Option<String>`, explicit `null` when absent;
- `dry_run_flag` — `Option<String>`, explicit `null` when absent.

The bridge maps IR `InteractionKind`/`MutationKind` onto the agent-context
enums, copying `bypass` and `dry_run` verbatim. It never infers these values
from command names, verbs, or flags: absence stays `unknown`/`null` (design doc
§8.1). The derive-side keys and grammar are recorded in
[ADR-0010](adrs/adr-0010-behavioural-metadata-attribute-surface.md).

Evolve the schema through the compatibility policy in
[agent-native-cli-design.md](agent-native-cli-design.md) §8.2. Bump
`ORTHO_AGENT_CONTEXT_SCHEMA_VERSION` for breaking changes such as field
renames, enum wire-string changes, null-versus-omitted changes, required-field
changes, or `deny_unknown_fields`. Additive optional fields stay within the
same version, and consumers must ignore unknown fields.

Pin every schema change with the `ortho_config` wire snapshot and
variant-exhaustive enum tests. Add end-to-end `cargo-orthohelp` goldens by
using the existing multi-root fixture pattern: add a root type to
`tests/fixtures/orthohelp_fixture` and select it with `--root-type` rather than
creating a new workspace crate. Keep the 6.2.2 nested agent-context assertions
separate from the 6.1.2 roff, PowerShell, and IR nested-renderer assertions. Do
not duplicate a per-field schema table here; use the rustdoc for
`ortho_config::agent_context`, the §3.2 JSON example, and the committed wire
snapshot as the canonical field references.

Run `coderabbit review --agent` after major milestones that change schemas,
documentation contracts, or externally visible behaviour. Clear its concerns
before moving to the next milestone.

### Public API

The following functions form the stable agent-context surface for 6.2.2.

`cargo_orthohelp::agent_context`:

```rust
/// Convert bridge documentation IR into an `AgentContext` payload.
///
/// `package` is used to populate `AgentContext.package`.  Pass `None` for
/// `localizer` to omit command summaries; pass an EN-US `Localizer` to
/// include them.
#[must_use]
pub fn bridge_ir_to_agent_context(
    meta: &DocMetadata,
    package: &str,
    localizer: Option<&dyn Localizer>,
) -> AgentContext
```

`cargo_orthohelp::output`:

```rust
/// Serialise `payload` as pretty-printed JSON and write it atomically to
/// `<out_dir>/agent-context.json`.
///
/// Returns the path of the written file on success.  Fails with
/// `OrthohelpError::Io` for filesystem errors and `OrthohelpError::IrJson`
/// for serialisation failures.
pub fn write_agent_context(
    out_dir: &Utf8Path,
    payload: &AgentContext,
) -> Result<Utf8PathBuf, OrthohelpError>
```

`cargo_orthohelp::cli::OutputFormat`:

```rust
/// Emit a compact, non-localized agent-context JSON manifest.
/// Writes `<out_dir>/agent-context.json`.
AgentContext,
```

### Internal default-display normalizer

`cargo_orthohelp::agent_context::normalize_default_display` is a crate-internal
helper of the agent-context bridge, not part of the stable surface listed
above. It rewrites a displayed default value so Rust path separators render
consistently, removing whitespace around `::` outside quoted strings, character
literals, and raw strings, whose contents it copies verbatim. Whitespace inside
a literal is data, so only the surrounding path separator is normalized.

Ownership stays with the agent-context bridge: the bridge's input projection
(`agent_context::input`) calls it when populating `AgentInput.default`, and
nothing outside `cargo-orthohelp` may depend on it. The normalization is
display-only. `AgentInput.default` is not executable or machine-parseable, so a
consumer needing the original rendering must read the documentation IR rather
than this output. Keep the helper `pub(crate)`; exposing it publicly would make
a display detail of one bridge a compatibility promise, and widening it belongs
with a documented agent-context API decision, not a refactor.

### Consumer dependency tiers

[Agent-native CLI assistance design](agent-native-cli-design.md) §2.2 is the
authoritative source for the hard and soft ship-time dependency tiers that
apply to Weaver, Netsuke, and other downstream consumers. When changing a
hard-dependency capability, update §2.2 and the cited roadmap item in the same
change. When changing a soft-dependency capability, also record which roadmap
item any temporary local consumer adapter shadows, so its eventual replacement
can be tracked.

Run `coderabbit review --agent` after major milestones that change schemas,
documentation contracts, or externally visible behaviour. Clear its concerns
before moving to the next milestone.

## Agent-native architecture boundary

Agent-native CLI assistance is contract modelling work inside OrthoConfig, not
a transfer of downstream application execution into this repository. The
canonical contract and boundary document is
[Agent-native CLI assistance design](agent-native-cli-design.md).

Contributors should keep reusable command-contract policy in OrthoConfig:

- command, option, output, and workflow metadata;
- documentation IR, agent-context schema, and related versioning policy;
- generated human documentation and compact agent-facing context;
- vocabulary, structured-output, and bounded-list lint policy; and
- optional shared primitives for profiles, delivery targets, feedback stores,
  skill manifests, and execution-ledger metadata.

Downstream applications own the execution side of those contracts. Weaver,
Netsuke, or another consumer remains responsible for command execution, domain
side effects, sandboxing, safety policy, long-running job semantics, provider
routing, build graph behaviour, and application-specific persistence. If
OrthoConfig executes downstream commands or owns downstream side effects, stop
and revisit the boundary in the agent-native design.

## Profile layering (roadmap 9.1.1)

Profile support is opt-in at the struct level through the derive attribute
`#[ortho_config(profiles)]`. A struct that does not opt in keeps the four-tier
merge order, gains no `--profile` flag, and treats a `[profile.*]` table in a
shared file as an ordinary key rather than extracting it. The accepted design is
[ADR-0012](adrs/adr-0012-profile-selection-and-layering.md); the user-facing
view is the
[named profiles section](users-guide.md#switch-between-named-profiles) of the
users' guide.

Opted-in structs merge five tiers, lowest to highest: built-in defaults,
configuration files, the selected profile, environment variables, and CLI
flags. An explicit flag beats the profile even when the flag value equals the
built-in default, so the generated CLI push consults clap's value-source
information rather than relying on a value comparison alone.

Extraction runs over the *resolved file chain* — the first successful discovery
candidate plus its `extends` chain — not over every candidate the discovery
walk considered. Every file layer in that chain has its reserved `profile` root
key stripped, and one profile layer is produced for each file that defines the
selected profile, in chain order (base first). The environment layer has the
selector stripped too, and the generated flag is excluded from the serialized
CLI layer, so the selector never merges as an ordinary configuration value.
Subcommand loading uses a separate pipeline and ignores profiles; a `cmds` key
inside a profile body is rejected rather than silently ignored.

The merge API and the post-load surface divide as follows:

- `MergeProvenance::Profile` tags a profile layer's provenance;
- `MergeLayer::profile(value, path)` constructs one, and
  `MergeComposer::push_profile` pushes it after the file layers and before the
  environment layer;
- `extract_profile_layers` is the extraction and validation entry point,
  returning an `ExtractionOutcome` with `file_layers` and `profile_layers`;
- `ProfileLoadOutcome` is the post-load runtime type that reports the active
  selection, with `config()`, `into_config()`, and `selection()` accessors; and
- the generated profile entry points named below return a `ProfileLoadOutcome`.

Keep the [ADR-0007](adrs/adr-0007-downstream-context-command-naming.md)
static/runtime split: profile *support* is agent-context metadata and is
static, while the *active selection* is runtime-only and reported by
`ProfileLoadOutcome`. Selected-profile output is deliberately not part of the
static agent-context contract, which documents the mechanism rather than the
moment.

### Generated profile entry points

Opting in with `#[ortho_config(profiles)]` adds methods that an opted-in struct
does not otherwise have. They divide into a process-backed pair and an injected
pair, and the split matters: the process-backed methods read the live
environment, so a test that calls them cannot be hermetic, whereas the injected
methods take their environment from the caller.

The process-backed pair, generated on both the CLI struct and the config struct:

- `load_with_profile_from_iter(iter)` composes the five tiers from an argument
  iterator plus the live process environment. Both surfaces return the
  configuration type rather than the type that owns the method:
  `ProfileLoadOutcome<Config>` on the CLI struct, which performs the load, and
  `ProfileLoadOutcome<Self>` on the config struct, whose delegate returns
  whatever the CLI loader produced; and
- `load_with_profile()` is its convenience form over `std::env::args_os()`.

The injected pair, which mirror the source-aware loaders described under the
[environment access boundary](#environment-access-boundary):

- `load_with_profile_from_iter_with_sources(iter, discovery_source, merge_source)`
  is the hermetic counterpart of `load_with_profile_from_iter`; and
- `load_with_profile_with_sources(discovery_source, merge_source)` is the
  hermetic counterpart of `load_with_profile`.

The two sources are deliberately separate types, and the separation is a safety
property rather than an ergonomic detail. `discovery_source` is a
`SharedEnvSource` (`Arc<dyn EnvSource>`), which resolves a named variable only;
`merge_source` is a `SharedScanEnvSource` (`Arc<dyn ScanEnvSource>`), which can
enumerate everything the source holds. Discovery needs to look up a handful of
names — the configuration-path selector, the XDG base directories, the Windows
application-data variables, and the home directory fallback — while the
environment merge layer has to scan an application prefix. Typing the
capabilities apart means a lookup-only discovery source physically cannot
enumerate the merge environment, so a caller cannot accidentally hand the
discovery walk a scanner that sweeps unrelated process secrets into a layer. A
single `MapEnv` can back both roles by coercing two `Arc` clones, one to each
type. The `EnvSource` boundary itself is documented under the
[environment access boundary](#environment-access-boundary).

Opted-in structs also gain the primitive form
`compose_layers_with_selection_from_iter_with_sources(iter, discovery_source,
merge_source)`,
which returns the `LayerComposition` together with the resolved
`Vec<SelectedProfile>` instead of merging immediately. It is the primitive the
injected pair is built on: `load_with_profile_from_iter_with_sources` calls it
and maps `into_merge_result` over the composition. It is generated on the CLI
struct only; the config struct receives delegating wrappers instead. Both
structs also gain the injected `compose_layers_from_iter_with_sources`, which
the config struct delegates to the CLI struct.

The injected methods, and the compose helpers they build on, carry
`#[allow(dead_code, reason = "Generated method may not be used in all builds")]`
because a generated method can be unused in a consumer that never calls it;
the process-backed `load_with_profile_from_iter` and `load_with_profile` are
not annotated, since a consumer that opts into profiles is expected to call one
of them. See [issue #584](https://github.com/leynos/ortho-config/issues/584)
for the tracked follow-up that would replace that blanket allow.

Of the injected pair, only `load_with_profile_from_iter_with_sources` carries
the hooks itself; `load_with_profile_with_sources` is a thin delegation that
reaches them through the `from_iter` form. Those hooks label the event
`operation = profile_load`, `source = injected`, where the process-backed pair
emit `source = process`; the
[environment merge telemetry](#environment-merge-telemetry) section lists the
full closed vocabulary. Because the delegation supplies its own argument vector
from `std::env::args_os()` while taking the environment sources from the caller,
`load_with_profile_with_sources` is injected-labelled on the strength of its
*environment* sources alone — the argument vector is never injected.

## Behavioural test layout

Behavioural suites live in crate-local integration test targets. The repository
layout guide records the current target and feature-file inventory.

The `ortho_config` BDD target is rooted at `tests/rstest_bdd.rs`; it declares
the shared `tests/fixtures/` module and the `tests/rstest_bdd_support/` module
with conventional `mod` declarations. `cargo-orthohelp` also uses a top-level
`tests/rstest_bdd.rs` entry point and keeps its BDD modules under
`tests/rstest_bdd_support/`. Keep the PowerShell golden-test fixture at
`cargo-orthohelp/tests/golden/powershell_fixture.rs` with the golden tests that
consume it.

Step definitions use `rstest-bdd` macros (`#[given]`, `#[when]`, `#[then]`) and
consume `rstest` fixtures. Scenario-local mutable state is modelled with
fixtures and `Slot<T>` values inside `#[derive(ScenarioState)]` structs.
Cross-scenario mutable sharing is forbidden; use `#[once]` only for expensive,
effectively read-only infrastructure.

Keep richer fixture families isolated. For example, `NestedDocsConfig` and
`NestedDocsContext` back `docs_ir_nested.feature`, and their steps live in a
fixture-specific module rather than expanding unrelated step files. Likewise,
`CargoContext` and its `#[fixture]` provider plus the `cargo` steps live in
`tests/rstest_bdd_support/behaviour/steps/cargo_steps.rs`, backing
`cargo_entry_point.feature` (scenarios: Cargo dispatch parses the inner
options; bare invocation without the injected token is rejected), also isolated
in a fixture-specific steps module. Future contributors should extend the
existing fixture families rather than duplicate them.

### Policy configuration test layout

Agent-native policy configuration (roadmap 7.1.1) follows the same layout. The
canonical vocabulary is a single source of truth in
`cargo-orthohelp/src/policy/vocabulary/mod.rs` (Decision D2); the agent-context
verb mapper currently imports its constants, so that consumer cannot drift. The
unimplemented 7.1.2 lint rules do not import them yet. Assertions in new policy
tests split by crate: `assert!`-style boolean checks in `cargo-orthohelp`, and
matcher- or collection-shaped checks in `ortho_config` agent-context tests
(Decision D8).

Fixtures: `tests/fixtures/orthohelp_policy_warn_fixture/` (mode `warn`, one
redundant exception, one scoped exception) and
`tests/fixtures/orthohelp_policy_deny_fixture/` (mode `deny`, one malformed
exception) back the policy golden and behavioural suites; `orthohelp_fixture`
keeps no policy table and serves the "off by default" scenario (Decision D10).
`tests/fixtures/orthohelp_policy_adoption_fixture/` omits `root_type` and the
`ortho_config` dependency so the check-first pipeline (Decision D11) stays
covered for packages still adopting the toolchain. The CLI-level policy
behaviour is exercised end-to-end by
`cargo-orthohelp/tests/features/orthohelp_policy.feature` with steps in
`cargo-orthohelp/tests/rstest_bdd_support/behaviour/steps_policy.rs`, and the
report wire format is snapshotted in
`cargo-orthohelp/tests/golden/policy_report_tests.rs`.

The policy boundary keeps `PolicyConfig` and `evaluate` as pure domain code.
`run_policy_check` performs the check early, before generation, and writes the
machine-readable report through `output::write_policy_report`, which uses an
atomic artefact write. `apply_policy_to_context` maps the configured policy to
agent context while deliberately excluding exception reasons. The CLI exposes
`--check-agent-native` and the `--policy-mode <off|warn|deny>` report override;
the latter requires the check flag. The fixture layout above remains the
canonical policy coverage, and `googletest` plus `pretty_assertions` are
`cargo-orthohelp` development dependencies used by those tests.

The stdout writers `output::write_agent_context_stdout` and
`output::write_policy_report_stdout` serialize one compact JSON document,
append a newline, then write and flush the locked stdout handle. Keep tracing
on stderr: stdout is a machine-readable channel and must not contain
diagnostics. The policy check writes and emits its report before deciding
whether generation may continue. With `--check-agent-native` alone, it returns
after the report; with an explicit generator format, a successful check
proceeds to generation. That generation writes `agent-context.json` for
`agent-context` and `all`, but suppresses AgentContext JSON on stdout whenever
policy checking is enabled. A deny-mode finding fails after the report is
retained and emitted, before any generator artefacts are created.

### Integration test targets

`ortho_config` also carries plain `rstest` integration suites that are distinct
from the behavioural (`rstest-bdd`) suites above. Each directory-style suite
needs its own `[[test]]` stanza in the crate manifest; a suite without a stanza
is not compiled or run by Cargo. The repository-layout guide records the
current suite and module inventory.

Run either suite on its own for focused debugging:

```bash
cargo test -p ortho_config --test <target>
```

### Public documentation examples

The root README, `docs/users-guide.md`, `docs/cargo-orthohelp-api-guide.md`, and
`docs/v0-10-0-migration-guide.md` are executable documentation. Every fenced
block in those files must have a unique `tested-example` marker on the
immediately preceding line. The documentation-example test support owns parsing
and lookup for these examples. It is test infrastructure only; production code,
other crates, and examples must not depend on it.

The loader is shared by the documentation integration-test targets. Each target
loads and parses the documents once, then borrows examples from its cached
registry. The cache is immutable and has no reset operation within an
integration-test process. Tests that need fresh input should call the pure
parser with owned text or use a separate integration-test target. Keep the
loader's scope limited to loading exact fence bodies, rejecting unmarked or
malformed fences, and querying stable identifiers. Put scenario policy in the
test target that consumes it: compile and run Rust, parse data formats, execute
documented commands, and compare observable output. Do not copy a fence into a
fixture because the copied text can pass after the published example has
drifted. Identifiers use a closed filename-safe grammar: start with a lowercase
ASCII letter, then use lowercase ASCII letters, digits, or single hyphens. The
parser and temporary workspace both enforce this boundary before constructing
paths.

The documentation-example workspace helper owns temporary Cargo package
assembly for Rust examples. Reuse it only from documentation tests that compile
an exact fence body. A workspace has one owner: operations that add, build,
run, or write require an exclusive mutable borrow. Concurrent scenarios must
create an independent workspace per thread rather than share one workspace. The
helper may add the dependencies needed to compile a published example, but it
must not rewrite that source. Workspace setup copies the repository's
`Cargo.lock`, when present, into the private example workspace to reuse its
dependency versions without editing the parent lockfile. This helper belongs to
the documentation layout and is separate from production bridge seeding. Keep
the expected identifier registry closed so adding an example without choosing
its behavioural contract fails a test. The documentation-example Cargo runner
owns isolated Cargo process setup for these documentation tests; reuse it only
when executing a documented Cargo workflow with caller-owned temporary state.
The build-tool environment is a closed allow-list, separate from the narrower
environment used by documented applications. It retains the configured Cargo
home and shared package cache, Rust toolchain, compiler and documentation
wrappers, applicable wrapper settings, build-job limits, warning flags, and
platform paths. It also forwards `RUSTFLAGS`, `CARGO_ENCODED_RUSTFLAGS`,
`CARGO_LLVM_COV`, and the LLVM profile-output controls unchanged, so a nested
build reuses the parent's compiler flags and dependency fingerprints instead of
compiling a second, instrumentation-free copy. Those variables are therefore
not sanitized: rewriting them would split the fingerprint and defeat the shared
cache.

The allow-list is closed: unlisted application variables do not cross it.
Target-specific forwarding is limited to
`CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER`. Proxy and registry settings come
from Cargo's project and home configuration, including configured credentials;
the documentation-test environment forwards no proxy or registry environment
variable. Registry tokens, `SSH_AUTH_SOCK`, `ACTIONS_RUNTIME_TOKEN`, cache
credentials, application secrets, and other unlisted variables do not cross
this boundary. The non-secret `SCCACHE_GHA_ENABLED` cache-mode setting is
preserved. Documented binaries receive only the non-sensitive Windows runtime
variables, LLVM profile-output controls, deterministic home directories, and
deliberate scenario overrides named by the workspace policy. Do not clear or
relocate the shared Cargo cache to make a test pass.

Documentation Rust examples use Cargo's resolved parent target for compiler
artefacts. Their separate ownership state and OS-level lock live under
`<parent-target>/documentation-examples/v2`; this is not another Cargo target.
The shared test-only `BuildState` module handles metadata discovery, lock
ownership, and the recovery marker for both Rust-example builds and the CLI
documentation integration test. The Rust-example build helper additionally
parses compiler-reported Cargo JSON paths and snapshots those executables into
private workspaces. This protects same-named binaries across separate test
processes; a process-local mutex is insufficient. Cargo 1.89 released its
profile build lock before running tests in the observed workflow. A regression
test checks this lock-order contract; it can report an inconclusive result when
another Cargo process owns the profile lock.

The CLI integration test lives in the `cargo-orthohelp` package and snapshots
the executable path Cargo supplies through `CARGO_BIN_EXE_cargo-orthohelp` into
a private run directory. It does not rebuild that CLI executable from Cargo
JSON while another test might be using it. The test then runs the documented
Cargo external-subcommand flow; its generated bridge is built and snapshotted
by the production bridge helper. Keep manifests, source inputs, run
directories, and copied executables private to their owner. A successful
operation requires confirmation that the owned process tree has completed and
its output streams have drained; root exit and pipe EOF alone do not establish
that descendants have stopped. If that completion cannot be confirmed within
the cleanup deadline, fail the operation and retain any cache ownership marker
for owner recovery.

A direct Cargo build error from the subprocess runner retains its marker even
when OS-tree cleanup completes: an externally admitted compiler may outlive the
cancelled Cargo process, so its lifetime is unconfirmed. A fully observed Cargo
nonzero exit and preparation or snapshot errors keep the existing cleanup path;
remove the marker only after owned-process completion is confirmed. An opaque
nested CLI callback error retains its marker because internal capture pipes
hide compiler writer lifetime from the outer runner. Never clear an unconfirmed
marker automatically or terminate an unrelated process.

During `cargo llvm-cov`, `show-env` exports its selected target as
`CARGO_LLVM_COV_TARGET_DIR`, not `CARGO_TARGET_DIR`. Metadata discovery
promotes that value to `CARGO_TARGET_DIR`, so it resolves the instrumented
parent target instead of silently selecting a cold default target. Otherwise
metadata follows the caller's Cargo target configuration. Nested Cargo builds
preserve coverage flags and LLVM profile-output variables, while omitting the
coverage CLI from `RUSTC_WRAPPER`; configured Cargo wrappers and warning policy
remain active. Regression cases cover target precedence, wrapper selection, and
secret exclusion.

Keep fallible host-tool discovery and environment preparation at the runner
boundary; command construction consumes prepared values without starting
subprocesses or writing files. The process runner is test infrastructure for
these documentation tests only: route Cargo, host-tool, and documented-binary
commands through it, while command construction and exit-status policy remain
with their existing owners. Its build, execution, discovery, and cleanup
policies use one monotonic overall scenario deadline, keep per-stream capture
to 1 MiB while continuing to drain output, and report truncation and
unavailable evidence on failure. Output activity does not extend a deadline.
The runner must prove that owned descendants have completed before it reports
success, on every supported platform. The mechanism is platform-specific; the
contract is not. This ownership applies only to test subprocesses and must not
terminate unrelated processes or shared cache services.

Documentation example scenarios start a fixed 956-second monotonic envelope
inside their 960-second nextest override, reserving two two-second cleanup
allowances. Individual build, execution, and discovery operations have 956, 60,
and 30-second caps, each further limited by the scenario time remaining. The
`cargo-orthohelp` BDD target uses the same 956-second operation envelope. A
local direct-Cargo run reached its 595.791-second build deadline with only
repeated Cargo package-cache lock messages and no compiler-progress evidence.
The 956-second envelope is 1.60 times that observed lower bound. Its deadline
counts cache and compiler-admission waits without estimating or subtracting
them, and preserves two seconds for child cleanup and two for test reporting.
Output does not reset either deadline. The workflow contract checks this
ordering against nextest's 1,800-second whole-run limit, the 2,700-second Cargo
watchdog, and the enclosing job ceiling. See
[Test timeouts](#test-timeouts-the-tiers-this-repository-sets) and
`documentation_budget_test.py`.

Shared Cargo-runner and process-runner regressions live in the owning
`documentation_examples_rust_tests` target. Other callers use runtime-only
wrappers, so the same child-process probes do not run again in each integration
binary. Build-heavy Rust documentation tests share a named in-process serial
lock, and the matching nextest binaries share one test group. This prevents a
test from spending its scenario deadline waiting behind another test that owns
the same build-and-snapshot lock. The independent-workspace regression still
performs its two workspace operations concurrently inside one test.

Nested Cargo commands discover the configured target directory with
`cargo metadata` and reuse that target and Cargo's default shared package
cache. Documentation workspaces keep their source, manifest, run files, and
executable snapshots in private directories; a sidecar `fs4` lock protects
build-and-snapshot ownership across processes. The BDD scenarios use a separate
filesystem lease under the selected target while reading and updating their
shared bridge cache. The identifier-artefact test owns a private target nested
under the configured workspace target, so `cargo clean -p` cannot remove
workspace test executables. It still uses Cargo's default shared package cache.
Cargo retains compiler wrappers, job limits, warning enforcement, coverage
settings, and allow-listed native build inputs. Application secrets and
arbitrary application variables are excluded. Run-file paths must contain
normal relative components only.

The contracts are split across integration-test targets by concern, following
the layout used in Netsuke:

- `documentation_examples_tests.rs` owns the closed identifier registry, the
  packaging contract for the README, and static contracts for data fences such
  as configuration files, JSON documents, and command spellings;
- `documentation_installation_tests.rs` owns installation and upgrade
  manifests. It sweeps every TOML fence that depends on `ortho_config` and
  requires the version to equal `CARGO_PKG_VERSION`, so `lading bump`, which
  rewrites the README and user's guide, keeps the contract and the
  documentation in step. The `[bump.documentation]` globs in `lading.toml` also
  name `ortho_l10n/README.md`, so its install line moves with each bump; that
  README is a doctest target of `ortho_l10n` rather than part of this sweep.
  Migration guides are excluded from the sweep because their versions describe
  a fixed upgrade path;
- `documentation_examples_rust_tests.rs` compiles every Rust fence into one
  temporary workspace and runs it. Examples whose contract is their output for
  fixed arguments belong in the table in
  `documentation_examples/simple_runs.rs`; examples that need fixture files,
  environment overrides, or transcripts belong in a feature-named flow module
  such as `guide_flows.rs`, `migration_flows.rs`, or `cargo_flows.rs`; and
- `cargo-orthohelp/tests/documented_command.rs` runs the documented
  `cargo orthohelp` command flows against workspace packages, using the
  executable Cargo has already built for the package, and checks the artefacts
  and streams they produce. The command text itself is pinned by the static
  contracts, so the executed flags and the published flags cannot drift apart.

A console transcript is tested in two halves: assert the exact published text,
then run the binary under the conditions each command line names and compare
the output with the lines that follow it. Placeholder package names in command
examples, such as `my-cli`, are mapped onto workspace fixture packages whose
manifests carry the documented configuration.

The `ortho_config` crate publishes the repository README itself: its manifest
sets `readme = "../README.md"`, and `cargo package` copies that file into the
crate root. Do not add `ortho_config/README.md`; it would collide with the
copied file and reintroduce a second README to keep in step. Because crates.io
and docs.rs render the README away from the repository, every link in it must
be absolute, which `repository_readme_links_are_absolute` enforces.

### Production bridge build ownership

The production bridge used by `cargo-orthohelp` has a separate build helper
from the documentation-test workspace helper. The stable IR cache entry stays
under `target/orthohelp/<hash>/`; each invocation writes its generated
manifest, bridge source, a private copy of the selected workspace's
`Cargo.lock` when present, and its executable snapshot into a temporary
directory under that entry. Cargo builds into the selected package's
Cargo-resolved target. This reuses compiler output across cache keys without
sharing mutable bridge inputs or running a binary from an artefact path that
another build can overwrite.

The production helper is scoped to bridge compilation. It uses an `fs4`
operating-system file lock in Cargo's selected target directory across the
JSON-reported build, its copy into the private run directory, and cleanup of
the generated binary outputs. The generated manifest selects a target name
derived from the package root and private invocation directory, so an ordinary
workspace binary named `orthohelp_bridge` cannot replace the reported
executable after Cargo releases its own target lock. Once the private snapshot
succeeds, the helper removes only Cargo-reported files whose names belong to
that generated binary. Missing files are accepted; other cleanup failures are
logged without invalidating the snapshot. Dependencies and the shared Cargo
cache remain under Cargo's ownership. The generated IR cache path and its
atomic, idempotent publication remain unchanged. Cargo's configured wrappers,
job limits, warning flags, and coverage settings remain active. The helper does
not rewrite Rust flags or relocate the default shared Cargo cache. Its Cargo
JSON reader retains at most 1 MiB of rendered diagnostics; `run_cargo` forwards
that bounded output to the supervising CLI after draining the stream and marks
truncation explicitly. The nested build asks for Cargo's plain `json` message
format, so rustc's rendered diagnostics travel in that bounded stream rather
than through inherited stderr. The build environment is an explicit allow-list,
and three families of Cargo setting are also recognized structurally rather
than by name: target-specific `LINKER` and `RUSTFLAGS` settings, and registry
`INDEX` locations. Proxy endpoints and registry indexes cross only when their
values contain no user information, query, or fragment, because a URL can carry
credentials that a name check cannot see. An omitted value is reported by
variable name, never by value. See the
[cargo-orthohelp bridge design][bridge-build-ownership] for the ownership and
data-flow details.

[bridge-build-ownership]:
  cargo-orthohelp-design.md#633-bridge-build-ownership-and-recovery

The lock is not a recovery signal. The fail-closed marker is
`<selected-target>/orthohelp-bridge-build.in-progress`. If an interruption
leaves it behind, confirm no Cargo, rustc, or wrapper-managed build process is
still using that target before removing only the marker file and rerunning the
command. Let healthy compiler admission and Cargo lock waits complete; do not
infer that a build has stopped from a queued wrapper alone. The bridge helper
must not clear an unconfirmed marker, kill unrelated processes, remove the
shared cache, or restart a cache service. The existing nextest serialization
groups remain until their shared-state assumptions are independently disproven.

#### Windows nested Cargo builds

The bridge performs a nested Cargo build, so a Windows developer or CI runner
needs a discoverable MSVC toolchain and Windows SDK before any gate that
compiles Rust. The hosted `build-test (windows)` lane performs this setup in
its "Configure MSVC for nested Cargo builds" step; reproduce the equivalent
environment locally.

Prerequisites are Visual Studio with the **Desktop development with C++**
workload and the x64 compiler tools. Discovery uses `vswhere`, which hosted
runners ship at
`${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe`,
filtered on `Microsoft.VisualStudio.Component.VC.Tools.x86.x64`.

`VsDevCmd.bat -arch=x64 -host_arch=x64` configures the x64 toolchain by setting
environment variables in the process that runs it; it does not print those
values for a parent PowerShell or Bash shell to export. Keep that process in
the ancestry of the build instead. The simplest route is to open an **x64
Native Tools Command Prompt** for the installed Visual Studio and run the gates
from it, so every gate inherits the `INCLUDE`, `LIB`, `LIBPATH`, and SDK
variables the script set.

A shell that cannot inherit them, such as the Bash coverage step the hosted
lane runs, has to read them back from a child `cmd.exe` that calls the script
and then lists its environment:

```powershell
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$installation = & $vswhere -latest -products '*' `
  -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
  -property installationPath | Select-Object -First 1
$env:MSVC_VSDEVCMD = Join-Path $installation 'Common7\Tools\VsDevCmd.bat'
$environment = & $env:COMSPEC /d /s /c `
  'call "%MSVC_VSDEVCMD%" -arch=x64 -host_arch=x64 >nul && set'
```

`>nul` hides the script's banner so only the resulting `NAME=value` lines
remain, and `set` with no arguments prints every variable the child now holds.
Import only the toolchain names the gates need — `INCLUDE`, `LIB`, `LIBPATH`,
`VCINSTALLDIR`, `VSINSTALLDIR`, `VSCMD_ARG_TGT_ARCH`, `WindowsSdkDir`, and
`WindowsSDKVersion` — rather than the whole environment, which also carries
application credentials.

Two checks make the setup fail closed rather than produce a confusing linker
error later. `LIB` must be present, and at least one directory on it must
contain `kernel32.lib`, which proves the Windows SDK search path resolved. The
x64 `link.exe` found under `VC\Tools\MSVC\**\bin\Hostx64\x64` is then placed on
`PATH`, and its full path is exported as
`CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER` so the nested build selects it
rather than whatever `PATH` resolves first.

That ordering matters: the coverage action runs under Bash, and Git's
`/usr/bin` precedes Visual Studio on `PATH`. Configure MSVC before coverage or
any other step that builds Rust, or the nested bridge build links against the
wrong tools. The setup is a CI and local-development requirement only; it does
not change the production allow-list beyond the linker variable already
documented above.

### Python docstring examples

The helper scripts under `scripts/` carry NumPy-style `Examples` sections, and
those examples are executed rather than read. `PYTEST_FLAGS` in the Makefile
hands `--doctest-modules` the modules to collect, and `make test` runs them
beside the script tests.

The list is written by hand, which is the mechanism that fails, and it fails in
both directions. A module that gains an example is collected only if somebody
remembers to name it, and the spelling helper had gained one that nothing ran,
so it could have gone untrue without a gate noticing. A module the list still
names after a deletion is the other direction, and the same helper supplied it:
it was removed with the legacy spelling generator while the list still named
it, which ends the whole lane rather than quietly collecting less.

`scripts/tests/test_doctest_collection.py` is the guard. It sweeps `scripts/`
for modules containing `>>>` and fails when one is neither named by
`PYTEST_FLAGS` nor covered by a directory the list names, which is how
`scripts/tests` covers the test modules. A second contract fails when the list
names a path that no longer exists, because that ends the whole lane rather
than quietly collecting less. A third pins the sweep itself: both of the others
are satisfied by a discovery that returns nothing, so the sweep must find
`scripts/bump_version.py`, which carries fifty-odd examples.

Three mutations are caught: a module dropped from the list, a named path
misspelled, and a sweep narrowed to a suffix no file uses.

### Shared test-support helpers

The shared test-support modules own the `ToAnyhow` trait, which converts an
`OrthoResult<T>` (`Result<T, Arc<OrthoError>>`) into an `anyhow::Result<T>`,
preserving the original error as the `anyhow` source. It is the single
conversion point for integration-test targets that report failures through
`anyhow`; before it existed, each suite grew its own free function or inline
`map_err(|err| anyhow!(err))` call, and those copies agreed on behaviour by
accident rather than by construction. New local `to_anyhow`-style conversions
are not permitted — depend on this module instead.

The shared discovery-builder helper owns `discovery_with`, a pure builder that
returns an independent `ConfigDiscovery` wired to the caller's `MapEnv` only;
it reads no other environment variables and touches no filesystem, and it
resolves its sole project root to a fixed test root. It is owned by the
injected-source discovery suites — candidates, properties, telemetry, and
metrics — which need a `ConfigDiscovery` configured identically so that their
results remain comparable. Any test needing a deterministic discovery over an
injected environment may reuse it.

The shared isolated-environment helper owns `with_host_overrides`. It serves
suites that load through the *derived* entry points, where the derive macro
builds its own `ConfigDiscovery` and no attribute can select a project root.
`with_host_overrides` points the XDG search bases at the caller's fixture root,
which ranks them above the platform default `/etc/xdg`; an empty
`XDG_CONFIG_DIRS` still falls back to that default, so leaving the variable
unset is not isolation. It is owned by the CLI integration and
error-aggregation suites.

One hole stays open and is inherited from the injected-source migration: the
derived loader's project rung resolves its root from `std::env::current_dir`,
so a `.config.toml` beside the test binary's working directory would still be
discovered. The exposure is bounded by the rung's position — projects are
searched *last*, so the ambient file is reached only once every earlier
candidate is absent or has failed to load. The bound is weaker than it looks:
discovery records a failure and keeps walking, so a malformed file, an invalid
selector target, or a failed required candidate all fall through to the project
rung rather than ending the search. No injection point can redirect it:
`project_file_name` names a file, while `project_roots`, `add_project_root`, and
`clear_project_roots` are reachable only when a test builds the
`ConfigDiscovery` itself, as the discovery-builder suite above does. Closing
the gap needs a derive-level discovery attribute for project roots.

The common behavioural-step helper module owns `set_scalar_once`,
`set_nonblank_scalar_once`, and the `SlotTakeOrExt` extension trait's
`take_or`. The first two centralize the repeated "guard, validate, and populate
a scalar slot" shape used to fill scenario `Slot` state exactly once; `take_or`
centralizes the repeated "take the slot's value or fail with a descriptive
error" shape. Both are scoped to behavioural step modules; step modules must
use these helpers rather than re-implementing slot-guard boilerplate.

The common behavioural-step helper module also owns `config_fixture`,
`shared_sources`, and `isolated_home_env`. `config_fixture` writes `contents`
into `file_name` inside a fresh temporary directory and returns the
`tempfile::TempDir` guard together with the absolute file path; passing `None`
for `contents` derives the path without writing, which preserves scenarios that
deliberately leave a selected file absent, such as those in
`merge_composer_steps`, `cli_default_as_absent_steps`, and `subcommand_steps`.
`shared_sources` splits one owned `MapEnv` into the `SharedEnvSource` and
`SharedScanEnvSource` handles that loader APIs require for their separate
discovery and merge roles, both reading that single environment.
`isolated_home_env` returns a `MapEnv` pinning `HOME`, `XDG_CONFIG_HOME`, and
`XDG_CONFIG_DIRS` under a supplied root, so subcommand scenarios do not inherit
the machine's home directory; it is gated on the `serde_json` feature because
only the subcommand step modules use it. These helpers are owned by the
behavioural step modules under `behaviour/steps/`, which must use them rather
than re-implementing temporary-directory, config-file, or
environment-source-pair boilerplate. Callers must keep the `tempfile::TempDir`
guard alive until the loader has read the fixture, because dropping it deletes
the directory.

The shared helper crate's coverage module, `test_helpers/src/coverage.rs`, owns
`resolve_profile_path`. Tests that launch a child process clear its
environment, which also drops `LLVM_PROFILE_FILE`; an instrumented child would
then write its profile to a default location, either inside a fixture directory
the test deletes or inside the working tree where no collection step expects it.
`resolve_profile_path` returns an absolute profile path unchanged and joins a
relative one to the parent's working directory, which is the only stable anchor
once the child runs in a fixture directory. It is owned by the child-process
probes in `hello_world`'s layer tests and the `subcommand` integration target:
any probe that calls `.env_clear()` and forwards coverage must read the
parent's path through `ProcessEnv` and pass the resolved value to the child
rather than restating the anchoring rule. Both crates already depend on
`ortho_config_test_helpers` under the `test_helpers` import name.

The subcommand fixture module, `ortho_config/tests/subcommand/fixtures.rs`,
owns the `isolated_root` fixture and the `close_discovery` constructor. The
fixture returns a temporary root paired with a discovery source closed against
the ambient environment, and `close_discovery` pins the Unix global rung to
that root so the search cannot walk out into the developer's own
`XDG_CONFIG_DIRS`. It is owned by the `subcommand` integration target, whose
child modules reach it as `super::fixtures`; suites in that target must take
their discovery source from here rather than repeating the `#[cfg]`-guarded
construction inline, because a hand-rolled copy that omits one platform branch
silently reads the ambient environment on that platform instead of failing.
File contents and merge-source values stay with their tests — those are the
behaviour under test. Other integration targets keep their own setup: in
particular, `basic.rs` retains a private `write_config` because its cases need
nested fixture paths with parents created first, and the discovery suites use
`discovery_with`, which owns no temporary root.

The decoding fixture module,
`cargo-orthohelp/tests/fixtures/powershell_decoding.rs`, owns the
byte-order-mark aware decoders `decode_utf16` and
`decode_probable_utf16le_without_bom`, their `Endianness` selector, and the
`to_u16` code-unit join. It is owned by the `powershell_windows` integration
target, which reaches it as `super::fixtures::powershell_decoding` and whose
`decode_help_output` composes the two decoders in the order it wants to try
them.

The module lives in the shared fixture tree rather than beside that test
because of where the test can run. `powershell_windows` is gated on
`cfg(windows)`, and it declines to exercise the decoders at all when neither
`powershell.exe` nor `pwsh` is on the machine, so a copy of the decoders inside
it would be untested everywhere except a Windows host with a shell installed.
As a module of `fixtures`, it rides along in every integration target that
declares `mod fixtures` unconditionally — `agent_context_contract`,
`cli_dispatch`, `golden_tests`, `nested_subcommand_end_to_end`,
`policy_report_contract`, and `rstest_bdd` — so on Linux, where
`powershell_windows` compiles to nothing at all, the decoders are still
compiled and still tested, with no shell involved. Suites must reach the
decoders here rather than re-deriving byte-order detection, because the
zero-high-byte threshold, the two different trailing-byte policies, and the
rule that a byte-order mark must be the mark its order implies are the
behaviour under test, and a second copy would drift in exactly the cases that
are hard to see.

## Isolated filesystem tests

Unit tests under `ortho_config/src/file` stage their configuration trees in a
per-test temporary root rather than changing the process working directory or
using Figment Jail. The generic fixture shape is documented under
[temporary-file fixtures][temporary-fixture-guide]; the rules below are
specific to these tests.

[temporary-fixture-guide]: rust-testing-with-rstest-fixtures.md#a-fixtures-for-temporary-files-and-directories

`ortho_config/src/file/tests/mod.rs` owns the convention and consolidates the
ambient-filesystem ceremony into two entry points, imported by submodules as
`use super::{TestRoot, test_root};` and `use super::with_fresh_graph;`:

- `test_root` is a fallible `rstest` fixture returning `Result<TestRoot>`. A
  test takes it as `test_root: Result<TestRoot>` and binds the whole value with
  `let root = test_root?;`. Renaming the binding keeps the lint configuration's
  denied `shadow_*` lints satisfied; the guard still outlives the test body.
- `with_fresh_graph` is a callback helper for tests that need the loader's
  cycle-detection state. Its callback receives the capability, the canonical
  root, the current `config.toml` path, and the mutable visited and stack sets.

`TestRoot` pairs a private `TempDir` guard, the canonical `root: PathBuf`, and a
`dir: cap_std::fs::Dir` capability opened with `open_ambient_dir` over the
same directory; the fixture is the only place in the test module that opens a
capability. The guard is held rather than exposed so it outlives the test;
dropping it deletes the directory that `root` points into.

Fixture files are written through the capability
(`dir.write("config.yaml", b"...")`) and passed to production helpers as
canonical absolute paths. `load_config_file` canonicalizes the file it is
given, and `process_extends` canonicalizes `current_path`'s parent before
joining a relative `extends` base to it, so a test that speaks in
`Dir`-relative names still observes absolute, symlink-resolved paths in merge
results and error messages.

`resolve_base_path` canonicalizes the base, not `current_path`; the
missing-file error interpolates `current_path` as supplied. Pass a canonical
`current_path` when a test asserts absolute paths in that error; a relative
value appears as supplied.

Do not destructure `TestRoot` with `..`: the `TempDir` guard drops at the end
of the `let` statement, deleting the root before the test body runs, and the
test then fails with `No such file or directory`. Bind the whole value instead.

Tests must not reintroduce Figment Jail, `std::env::set_current_dir`, or writes
outside the temporary root. This work is part of the migration tracked by
[#452](https://github.com/leynos/ortho-config/issues/452).

## Snapshot tests

Use `insta` for renderer golden coverage that would be noisy as handwritten
string assertions. Place snapshots beside the integration test that owns them,
and redact dates, absolute paths, and other environment-specific substrings with
`insta::with_settings!` filters before committing baselines.

Review snapshot changes with `cargo insta review`. For non-interactive baseline
creation in a controlled milestone, use `INSTA_UPDATE=always` and then verify
that no `.pending-snap` or `.snap.new` files remain before running the normal
quality gates.

## `rstest-bdd` migration strategy

Status: adopted at v0.6.0. See `docs/execplans/migrate-rstest-bdd-v0-6-0.md`
for the current migration and `docs/execplans/adopt-rstest-bdd-v0-5-0.md` for
the previous one. The imported upstream guides are
`docs/rstest-bdd-users-guide.md`, `docs/rstest-bdd-v0-6-0-migration-guide.md`
and `docs/rstest-bdd-v0-5-0-migration-guide.md`; their exact upstream revision
is pinned in `docs/rstest-bdd-doc-provenance.md`.

Migration guidance for contributors:

- Workspace pins are `rstest-bdd = "0.6.0"` and
  `rstest-bdd-macros = "0.6.0"`. The workspace floor stays at Rust 1.89.0 and
  the pinned toolchain did not move, but the reason is not simply that upstream
  requires 1.88: `rstest-bdd-macros` 0.6.0 depends unconditionally on
  `rstest-bdd-harness`, which pulls `cargo_metadata` 0.23.1 and hence
  `cargo-platform ^0.3.0`, whose floating resolution to 0.3.3 declares a 1.91
  floor. `Cargo.lock` therefore holds `cargo-platform` at 0.3.2 (declared
  1.88). Do not drop that constraint when refreshing the lockfile, and re-check
  the whole graph before raising the floor.
- To re-check the floor after any dependency change, use `Cargo.lock` to
  enumerate resolved registry packages, then read each package's `rust-version`
  from its registry manifest or full `cargo metadata` output. Compare those
  values with the workspace's declared floor. The offending package is not
  necessarily one this workspace names directly; here it was a third-level
  transitive dependency reached through a crate that upstream does not require
  for a plain-BDD consumer.
- The MSRV breakage surfaced only as a `trybuild` failure in
  `cargo test -p ortho_config --test compile_fail`, because the generated
  trybuild sandbox resolves its own lockfile. A green workspace build is not
  evidence that the floor holds.
- A step returning an alias of `Result<T, E>` now fails the scenario when it
  returns `Err` and injects `T` when it returns `Ok`. `E` must implement
  `Display`. This replaced the v0.5.0 advice to avoid return type aliases;
  aliases are now classified correctly and are safe to use.
- A unit payload never overrides a fixture, so `Ok(())` from a
  `Result<()>` step is safe even where a `Slot<()>` flag fixture exists.
- Scenario functions must return `()` or an explicit fallible result;
  `#[scenario]` return classification is unchanged from v0.5.0.
- Prefer `scenarios!(..., fixtures = [...], tags = ...)` for large feature
  bindings to reduce handwritten wrapper boilerplate.
- Prefer descriptive placeholder names over generic `{string}` placeholders so
  step signatures remain explicit and compile-time checked.
- Implicit fixture names strip exactly one leading underscore, so `world` and
  `_world` both bind the fixture named `world`. Use an explicit `#[from(...)]`
  when a genuinely underscore-named key is intended.
- Remove file-wide lint suppressions used only for historical generated-fixture
  warnings; retain only narrow, item-level `#[expect(...)]` annotations when
  still required.
- Keep scenario isolation as the default and reserve `#[once]` for shared
  infrastructure only.
- If a sync step needs async bridging, use
  `rstest_bdd::async_step::sync_to_async`.
- Keep tag names filter-friendly (`@name_part` style). Avoid dots in tag names
  used with `tags = "..."` expressions.
- Feature-file paths passed to `scenarios!` are manifest-relative and stay
  that way; diagnostics, the JSON reporter, and the JUnit `classname` attribute
  all render them relative to the manifest, so no path handling changes are
  needed in this workspace.

## Adding or changing behavioural tests

When adding scenarios or steps:

1. Add or edit the `.feature` file first.
2. Implement or update step definitions under the matching
   `tests/rstest_bdd_support/` module tree in `ortho_config` or
   `cargo-orthohelp`.
3. Bind scenarios using `scenarios!` where possible; use explicit `#[scenario]`
   only when a feature needs bespoke fixtures or per-scenario control.
4. Keep assertions user-observable (`Then` steps) and avoid asserting private
   internals unless the behaviour cannot be observed externally.
5. Run the full required quality gates before finalizing.

## Observability

OrthoConfig and `cargo-orthohelp` follow a single observability convention so
that downstream applications can attach the subscribers and exporters they
prefer without contending with this workspace for global state.

- Use the `tracing` crate for all diagnostic output. Prefer structured
  `tracing::{trace, debug, info, warn, error}` events and spans over `println!`,
  `eprintln!`, or direct `log` macros. Attach fields for identifiers, state,
  and error context so subscribers can filter and correlate events without
  parsing message text.
- Wrap meaningful units of work in spans. Use `#[tracing::instrument]` or
  explicit spans around request handling, command execution, retries, and
  background jobs. Do not hold a `Span::enter()` guard across `.await`; use
  `Instrument::instrument` or scoped synchronous spans instead.
- Use the `metrics` crate where usage, uptake, failure, or mitigation metrics
  are required. Choose `counter!` for cumulative events, `gauge!` for values
  that rise and fall, and `histogram!` for distributions such as latency or
  payload size.
- Describe emitted metrics with `describe_counter!`, `describe_gauge!`, or
  `describe_histogram!` whenever the unit or purpose is not obvious from the
  metric name. Keep metric names stable and labels low-cardinality. Do not put
  user input, request identifiers, unbounded path parameters, or raw error
  strings into labels.
- Respect the library and application boundary. Libraries in this workspace,
  including `ortho_config` and `cargo-orthohelp`'s reusable modules, may emit
  `tracing` events and `metrics` instrumentation, but must not install global
  subscribers or recorders. Applications and binaries should initialize their
  chosen exporters and subscribers once, as early as practical in startup.

Use `tracing` and `metrics` together where it aids diagnosis: spans give the
contextual envelope, events describe what happened inside, and metrics
aggregate the same activity for monitoring. New observability primitives, such
as additional metric families or span fields used across crates, should be
mentioned in the relevant design or component architecture document, so the
contract stays discoverable.

### Environment merge telemetry

The environment merge boundary emits a `merge.layer` tracing event at the
decision and terminal points of source-aware and profile-aware work. Events use
only these bounded fields:

- `operation`: `csv_env`, `derived_load`, `profile_load`, or `subcommand_load`;
- `source`: `process` or `injected`;
- `outcome`: `attempt`, `success`, or `failure`; and
- `category`: `none`, `opaque_key_transform`, `invalid_nesting`, `cli`,
  `default_value_conversion`, `file`, `cyclic_extends`, `gathering`, `merge`,
  `validation`, `profile`, or `aggregate`.

`CsvEnv` emits process-backed events, or injected events when `with_source`
supplies a scanning source. The source label tracks the environment the entry
point actually reads, and for the generated loaders that means the
`*_with_sources` suffix rather than the operation name. Derive-generated and
subcommand source-aware loads (`*_with_sources`) take injected sources and emit
`source = injected`; so do the profile-aware injected pair
(`load_with_profile_from_iter_with_sources` and
`load_with_profile_with_sources`), which are covered in
[generated profile entry points](#generated-profile-entry-points). The
profile-aware entry points without that suffix (`load_with_profile_from_iter`
and `load_with_profile`) accept no injected source and read the live process,
so they emit `source = process`. A profile-enabled struct's
`load_from_iter_with_sources` emits `source = injected` under
`operation = profile_load`, not `derived_load`, because it resolves a selection
on that boundary; the operation label distinguishes what the call does, and the
source label distinguishes where its environment came from. The events never
contain environment values, keys, paths, configuration data, caller-supplied
prefixes, or raw error text. Error categories are reduced to the closed
vocabulary before emission so subscribers can aggregate failures without
receiving sensitive input.

Capture tests must cover successful and failing paths for each emitting
operation. They assert the operation, source, outcome, and category fields, and
verify that captured events contain neither injected values nor keys or paths
from the test inputs. The library does not install a global subscriber;
applications attach their own capture or export layer at the boundary.

With the optional `metrics` feature enabled, the same merge boundaries also
increment `ortho_config.merge.attempts` and `ortho_config.merge.outcomes`. Both
counter families use only the bounded `operation`, `source`, `outcome`, and
`category` labels described above; the attempt counter uses `attempt` and
`none` for its outcome and category. No metrics recorder is installed by the
library.

### Discovery telemetry

Configuration discovery emits its events from two co-located modules, both of
which are written so that a leak is a compile-time impossibility rather than a
review responsibility:

- `discovery/telemetry.rs` owns the closed label vocabulary, the three counter
  families, and six event callsites whose field lists are fixed;
- `discovery/telemetry_events.rs` owns the remaining event callsites, one per
  field subset, because a `tracing` event's field *names* are fixed at the
  macro call even when a value is absent.

The split is described under [emitter structure](#emitter-structure) below.
Nothing outside these two modules emits a `discovery.*` event, and nothing
outside `telemetry.rs` calls the second module: the entrypoints that choose a
field subset are dispatchers, so a caller states which facts it has and the
dispatcher picks the callsite whose static metadata matches.

| Event                       | Fields                                                                                                                                                                                                                                                                                                                                            | Emitted from                |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------- |
| `discovery.source_selected` | `source`: `process`, `injected`                                                                                                                                                                                                                                                                                                                   | `builder::build`            |
| `discovery.selector`        | `state`: `not_configured`, `unset`, `empty`, `accepted`                                                                                                                                                                                                                                                                                           | `candidates::push_selector` |
| `discovery.xdg`             | `config_home` and `dirs`: `absent`, `empty`, `present`; `resolution`: `default`, `list`                                                                                                                                                                                                                                                           | `candidates::push_xdg`      |
| `discovery.home`            | `source`: `home`, `userprofile`, `fallback`, `none`                                                                                                                                                                                                                                                                                               | `candidates::push_home`     |
| `discovery.attempt`         | `operation`: `discover_first`, `compose_layers`, `policy_resolve`                                                                                                                                                                                                                                                                                 | `load`, `policy`            |
| `discovery.candidate`       | `operation`; `outcome`: `optional_failure`, `required_failure`; `required`; `source`: `required_explicit`, `explicit`, `selector`, `xdg`, `windows`, `home`, `project`; `category`: `file`, `cyclic_extends`, `gathering`, `validation`, `other`; `scope` (scoped walks only): `system`, `user`, `project`                                        | `load`                      |
| `discovery.project_root`    | `state`: `cwd_unavailable`                                                                                                                                                                                                                                                                                                                        | `builder::build`            |
| `discovery.load`            | `operation`: `discover_first`, `compose_layers`, `policy_resolve`; `outcome`: `success`, `not_found`, and under `policy_resolve` also `required_failure`, `optional_failure`; `source` (success only): `required_explicit`, `explicit`, `selector`, `xdg`, `windows`, `home`, `project`; `scope` (scoped walks only): `system`, `user`, `project` | `load`, `policy`            |
| `discovery.policy`          | `operation`: `policy_resolve`; `selector_class` (optional): `cli`, `environment`                                                                                                                                                                                                                                                                  | `policy`                    |

Table: Structured events emitted during configuration discovery, with the
closed field vocabulary each carries and the module that emits it.

Behind the optional `metrics` feature, discovery also increments three counter
families using the same bounded labels:

| Metric                                      | Labels                            |
| ------------------------------------------- | --------------------------------- |
| `ortho_config.discovery.attempts`           | `operation`                       |
| `ortho_config.discovery.outcomes`           | `operation`, `outcome`            |
| `ortho_config.discovery.candidate_failures` | `operation`, `source`, `category` |

Table: `metrics`-feature counter families and their bounded labels, mirroring
the `tracing` event fields above.

Rules for extending this set:

- **Every field is a `&'static str` from a constant declared in
  `telemetry.rs`, or a `bool`.** Discovery's inputs and outputs — variable
  values, resolved paths, file contents — are exactly what must not reach a
  log, and callers cannot pass one through a function that accepts only the
  closed vocabulary. The same property keeps the `metrics` labels bounded; an
  unbounded label here becomes an unbounded time-series in a consumer's
  monitoring system. Declare the constant in `telemetry.rs` even when its only
  callsite is in `telemetry_events.rs`: one module owning the whole vocabulary
  is what makes the closed set readable in one place.
- **Add an event spelling, not a parameter.** A new combination of optional
  fields is a new function in `telemetry_events.rs` plus a new arm in its
  dispatcher, never a wider signature on an existing emitter.
- **The discovery telemetry suite enforces it.** The suite feeds discovery
  distinctive values and asserts no captured field contains any of them. Adding
  a field that formats a path fails that test rather than shipping.
- **A separate suite pins field *names*, not just values.**
  `ortho_config/tests/discovery_telemetry_metadata.rs` reads `Metadata::fields`
  for events whose optional fields are absent and asserts the name is missing —
  the check a value-level assertion cannot make, because a `None` value already
  renders as nothing.
- **Counters go behind the optional `metrics` feature**, per the library
  and application boundary above. The `tracing` events are unconditional.

#### Emitter structure

`tracing` builds an event's static `Metadata` from the macro's *field list*.
Recording a `None` for an `Option` field records no value, but the field's name
stays in the metadata either way, so a subscriber filtering on
`Metadata::fields` would see a field the design document promises is absent.
"Omitted" therefore has to mean omitted from the metadata, and the only way to
write that is one macro call per field combination. `telemetry_events.rs` holds
those callsites:

- `policy_resolution_with_selector` and `policy_resolution_without_selector`
  for `discovery.policy`;
- `candidate_failure_scoped` and `candidate_failure_unscoped` for
  `discovery.candidate`, whose `scope` is present only on a scoped walk;
- four `load_outcome_*` emitters for `discovery.load`, where the independent
  `source` and `scope` options combine into four spellings.

Three small declarative macros provide the bodies, so each function expands to
a single statement. The bodies would otherwise be near-identical, because the
difference between them *is* the field list, and hand-writing them drew a
measured `CodeScene` duplicacy finding. Keep the macros private to the module:
the contract is the set of functions and the metadata each produces, not the
expansion.

Two rules keep this structure from decaying:

- **One `tracing` emission per function.** When a dependent crate enables
  `tracing`'s `log` bridge, Cargo's feature unification expands every event
  into a branch ladder, and those branches count against the workspace's
  `cognitive-complexity-threshold` in `clippy.toml` (9). In this workspace the
  enabling crate is `rstest-bdd`, a dev-dependency, so the ladder appears in
  every test build even though the shipped libraries never enable it. A
  function that reports two facts therefore delegates each to a small
  `fn log_<fact>` helper, as `bridge/telemetry.rs` and the atomic-write helpers
  in `output/mod.rs` do; their module docs record the reason.
- **Split by size, not by taste.** The vocabulary, the entrypoints and their
  documentation together reach the 400-line module cap, which is why the
  emitters and the counters live in sibling modules. Moving code back would
  fail that cap before it failed a test.

When adding an event, add its row below, its constants to `telemetry.rs`, its
emitter or dispatcher arm to the module that owns the relevant shape, and a
metadata-level assertion to `discovery_telemetry_metadata.rs` if it carries an
optional field.

One trap worth recording: `tracing`'s global maximum level is recomputed as
dispatchers register and drop, so concurrent tests each installing a
thread-local subscriber can drop the maximum out from under one another and
silently lose events. `pin_global_max_level` in the shared test support holds
one dispatcher for the lifetime of the binary. This was an observed
intermittent failure, not a precaution.

### `cargo-orthohelp` bridge and output telemetry

`cargo-orthohelp`'s diagnostics follow the same one-emission rule but a
different boundary rule: every event here is diagnostic only. None decides
anything, none is a contract a consumer may filter against, and no control flow
returns through an emitter. The modules are grouped by the fact they report:

- `bridge/telemetry.rs` — the ephemeral bridge build (`succeeded`, `failed`
  with the child's standard error) and the IR cache (`unchanged`, `write`).
  `bridge/mod.rs` additionally logs a cache hit when it returns cached IR.
- `output/mod.rs` — the atomic JSON write: the start, the completion with its
  byte count, the stdout path, and one failure event per fallible step
  (prepare, open, write, flush, sync, rename, and parent-directory sync).
- `output/agent_context.rs` — the agent-context document's own
  start/completion/failure trio.

These carry real paths, artefact names, byte counts, and error renderings, so
they are not held to discovery's closed-vocabulary rule. Treat that as a
boundary, not a licence: the bridge's `failed` event includes the child's
standard error because a bridge build failure is otherwise opaque to the user,
and that text is the only way to diagnose one. Do not extend the same treatment
to `discovery`, whose inputs are configuration values and are exactly what the
closed vocabulary exists to keep out.

## Digest rendering

`cargo-orthohelp` hashes cache inputs with SHA-256 and renders the digest as 64
lowercase hexadecimal digits. From `sha2` 0.11 the `finalize` and `digest`
methods return `hybrid_array::Array<u8, _>`, which dereferences to `[u8]` but
does not implement `core::fmt::LowerHex`, so the `{:x}` formatting the cache
previously used no longer compiles. `sha2` 0.11 also no longer implements
`std::io::Write`, so `std::io::copy` cannot stream into a hasher.

- Render digest bytes through the crate-internal `to_lower_hex` helper. It is
  scoped to `cargo-orthohelp`, owns no state, and may be called from any
  crate-internal site that needs lowercase hex. Do not reintroduce `{:x}` or
  per-byte `format!` calls.
- The helper maps each nibble arithmetically rather than indexing a digit
  table, because the workspace denies `clippy::indexing_slicing`. Keep it that
  way instead of adding a scoped suppression.
- Keep the helper private. The crate exposes no hex-encoding contract to
  downstream consumers, and publishing one would commit it to supporting that
  contract. Add a dedicated dependency such as `hex` only if a public surface
  genuinely needs one.
- Pass the digest by reference, as in `to_lower_hex(&hasher.finalize())`, so
  `Array` coerces to `&[u8]` through `Deref`.
- Feed a hasher with `update` from a bounded buffered read loop, or through a
  small `std::io::Write` adapter newtype that forwards to `update`. Do not
  reach for `std::io::copy` with a hasher as the sink.
- The sibling RustCrypto crates move in lockstep and carry the same break:
  `sha1`, `sha3`, and `md-5` go to 0.11, and `hmac`, `hkdf`, and `pbkdf2` go to
  0.13. Apply the same rules when introducing any of them.

The rendered form is byte-for-byte identical to the previous `{:x}` output, so
the change does not invalidate cache directories. A bounded exhaustive unit
test covers the whole `u8` range, and `cache.rs` pins the canonical SHA-256
digest of `b"abc"` so a self-consistent but wrongly ordered or zero-truncated
encoder cannot pass.

## Scoped and policy-based configuration discovery

Automatic discovery historically resolved one file: the candidate list was
scanned most-preferred first and the scan stopped at the first success. The
build tool that motivated the change needed an ordered, fail-closed explicit
selector chain and a stack of every file that exists, with project keys
overriding user keys while user-only keys survive. The resulting runtime types
live in `ortho_config/src/discovery/`, and RFC 0002
(`docs/rfcs/rfc-0002-config-layer-resolution-policy.md`) is their design
record: it fixes the semantics tables, the suppression gate, the drain
precedence, and the stability surface that must settle before the first release
shipping them. Read it before changing any behaviour here.

Candidate provenance is owned by candidate generation. The co-located
`ortho_config/src/discovery/candidates/provenance.rs` groups the bounded
telemetry source/scope pair and XDG decision labels. Candidate call sites
assign these values where the source is known; keep raw paths and environment
values out of them. Explicit paths and selectors remain outside
`CandidateOrigin` because they have no automatic scope. The accumulator keeps
the first source and list position while merging later scopes for duplicate
paths.

This section is about file-layer *resolution*. The agent-native,
`cargo-orthohelp` policy configuration described in
[Policy configuration test layout](#policy-configuration-test-layout) is a
different feature that shares only the word "policy".

### Type roles and how they compose

Four layers of responsibility, each thin enough to test on its own:

- `ConfigDiscovery` and `ConfigDiscoveryBuilder` own the candidate search space:
  the file names, the project roots, and the injected `EnvSource`. See
  [Environment access boundary](#environment-access-boundary).
- `ConfigFilePolicy` sits above that builder and adds the explicit selector
  chain, the two mode enums, and the scope order. `from_builder` is the
  constructor; the builder methods are chainable.
- `FileLayerOutcome` is the replayable result of `resolve_layers`. It is a
  struct rather than an enum because a successful chain can coexist with an
  earlier ignorable probe, and because a fatal selected-file failure must still
  carry the winning selector.
- `ResolvedSelection` records which selector won and where it pointed, on both
  the success and the failure path, so an error can name the selector that
  chose the offending file.

`resolve_layers` reads the environment once, at resolve time, and never returns
`Err`; failures are carried in the outcome's classified error buckets and
realized only when it is drained. Two exhaustive lists in RFC 0002 describe the
surface and are worth checking against any change: the "Reusable file-layer
resolver" section for the accessors, and "File-layer error policy" for the
four-case mapping. That document also records what has deliberately not changed:
`compose_layers`, `compose_layer`, `load_first*`, `DiscoveryLayersOutcome`,
`LayerComposition`, `MergeComposer`, and `MergeLayer` keep their signatures and
behaviour, because the new types are additive.

### First-wins and scope stacking are two code paths

`AutomaticMode` decides what automatic discovery does once it runs, and the two
variants do not share a traversal.

- `AutomaticMode::FirstWins` is the default. `compose_scoped_layers` preserves
  `ConfigDiscovery::compose_layers` exactly, by calling it directly and
  returning no origins. `compose_layers` walks the flat candidate list that
  interleaves explicit, environment, and platform entries, and stops at the
  first candidate whose `extends` chain loads.
- `AutomaticMode::StackScopes` resolves every requested scope in `scope_order`
  and appends the layers of every applicable candidate that loads, so a project
  file layers over a user file instead of replacing it.

Scope is not a concept under `FirstWins`, so a scope request is never
dereferenced there and no scope can be an origin. Within `StackScopes`, every
candidate in a scope is attempted rather than only the first that loads: that
is the mode, and it is also what makes the diagnostics complete, since a
candidate's defect is only discoverable by opening it. A failed lower-preferred
candidate therefore does not prevent the layers that did load from being
returned.

The legacy `compose_layers` is retained unchanged and the scoped engine groups
the platform generators only; explicit and environment selection belong to the
selector chain. They are two tested code paths, not one path wearing two
signatures, so a fix to one is not automatically a fix to the other.

### Selector precedence, and why it suppresses

`ConfigFilePolicy::selectors` takes an ordered chain. `resolve_layers` walks it
with `find_map` and the first selector that resolves a path wins. Resolution
reads a CLI-supplied `PathBuf` directly, and for `ConfigPathSelector::env`
reads the named variable through the discovery `EnvSource`, treating an unset
or empty value as "no selection".

A winning selector suppresses every later selector and all automatic probing.
That is the property `add_required_path` cannot deliver: the gate lives at
candidate generation, so a required path can no longer mechanically coexist
with later discovered layers. `ExplicitMode` decides what happens once a rung
wins:

- `ExplicitMode::RequiredExclusive` is the default. A missing or malformed
  selected file produces a single terminal `selected_error` carrying the winning
  `ResolvedSelection`; automatic discovery never runs, and there is no
  fallback.
- `ExplicitMode::Optional` is exclusive but tolerant. An absent selected path
  yields no file layers while still suppressing automatic discovery; a
  malformed file is still an error.

Draining follows a fixed precedence, and the change is worth knowing because it
alters which failures surface: a `selected_error`, when present, is the sole
surfaced error and the layers are empty; otherwise `reportable_errors` always
surface, and `ignorable_errors` surface only when no layer loaded. The
reportable bucket is the genuinely new expressiveness. Previously a malformed
*automatic* file landed in `optional_errors` and was dropped whenever a later
candidate succeeded, so a broken project file could vanish behind a valid user
file.

### Scope order, and the two opposite orderings

Two orderings meet in `discovery/scoped.rs` and they run in opposite directions.
`docs/design.md` states the point authoritatively and should be read in full;
the short form is:

- The candidate list is a **preference** order, most-preferred first, because
  index 0 is the location the historic first-wins scan selects.
- A composed layer list is a **precedence** order, because a `MergeComposer`
  applies layers in the order given and the last one wins.

A scope therefore walks its candidates in reverse preference order, applying
the least-preferred location first and the most-preferred last, so the historic
winner still wins while every lower-preferred location contributes a base layer
for the keys it alone sets. This keeps "later applied wins" as the single rule
for the whole system. Emitting candidates in preference order instead would let
a fallback such as `~/.demo.toml` silently override
`$XDG_CONFIG_HOME/demo/config.toml`, inverting established behaviour the moment
a second location starts loading.

The reversal happens in one named place, `ConfigDiscovery::scope_candidates`,
so that no call site has to hold two opposite orderings in mind: everything
downstream of it merely appends in application order. A maintainer changing the
traversal should keep that property — a second reversal, or a call site that
"fixes up" the order it receives, would restore the inversion this design
exists to prevent.

`scope_order` is a *request*, not a record. The default is
`[System, User, Project]`, and later scopes override earlier ones, so project
layers naturally override user layers. A scope contributes nothing when none of
its candidates loads, or when every layer it produced was already contributed
by an earlier scope.

Two invariants make that stacking deterministic, because the loader
de-duplicates and detects `extends` cycles only within a single chain:

- De-duplication is by **canonical** path across the whole composition, keeping
  the earliest position in application order, which within a scope is the
  lowest-precedence one. Keying on the canonical path the loader already stored
  collapses aliases and symlinks without changing public layer metadata, and it
  stops a file reachable from two places from contributing two layers and
  silently doubling append-strategy vectors. A parent reached by two children
  applies once, at the position its first child gave it; both children still
  override it.
- The `extends` chain of each file remains parent-first, so a parent is still
  applied before the child that overrides it. `extends` resolution is
  chain-local, with each file resolving its own parents against its own visited
  set, and a cross-scope cycle is reported with the same cyclic-extends error
  as a within-chain cycle.

### Inspecting and replaying a `FileLayerOutcome`

The load-bearing requirement is that the layers are resolved once and reused:
the same outcome is peeked for an early scalar read, then replayed into the
merge. The accessors split by purpose rather than by field.

- Early inspection, before any merge runs. `selection()` returns the winning
  `ResolvedSelection`; `selected_error()` returns the fatal selected-file error;
  `reportable_errors()` returns the non-fatal errors that should always be
  reported; and `merged_file_value()` folds the loaded layers into one JSON
  object for a scalar-only peek, such as a diagnostics flag that governs how
  later errors are rendered.
- Provenance for an operator-facing "what loaded" trace. `origins()` returns the
  scopes that actually contributed, and it does **not** echo `scope_order`: a
  requested scope whose candidates were all absent, or whose every layer was
  already contributed by an earlier scope, is not an origin. Under `FirstWins`
  it is always empty, and a failed explicit selection reports none either,
  because selection is not automatic discovery. That is why the report is
  computed inside the scoped engine rather than reconstructed by the policy.
- Replay, which reads no files because the layers are already resolved.
  `push_into` drains the layers into an existing `MergeComposer`;
  `into_layers_and_errors` drains them plus the classified errors into a
  caller-owned buffer, which is the shape the generated loader already feeds to
  `LayerComposition::new`; and `into_result` is the convenience form that
  aggregates into an `OrthoResult` instead. Both drains call one private
  helper, so the order above has a single source of truth rather than two
  restatements.

`merged_file_value` is deliberately narrow. The fold covers scalar keys only: a
collection-typed key reflects last-file-wins rather than the field's append or
keyed-merge strategy, so only scalars should be read from it. Widening it later
would be safe; broadening the claim and then narrowing it would not.

### The derive attribute plumbing

A struct opts into this path by setting any policy key alongside the existing
discovery keys:

```rust
#[ortho_config(discovery(
    app_name = "demo",
    env_vars = ["DEMO_CONFIG"],
    explicit_mode = "required_exclusive",
    automatic_mode = "stack_scopes",
    scope_order = ["user", "project"],
    project_file_name = "demo.toml",
    project_root_from = "directory",
))]
struct DemoConfig { /* ... */ }
```

`ortho_config_macros/src/derive/parse/discovery_attrs.rs` parses the keys, and
`uses_policy` is what selects the emitter: a struct setting only the
pre-existing keys keeps generating the legacy loader verbatim, while an opt-in
struct routes through `build_policy_based_loading` in
`ortho_config_macros/src/derive/policy_impl.rs`, which assembles a
`ConfigFilePolicy`, calls `resolve_layers`, and drains it with
`into_layers_and_errors`.

- `env_vars` is an ordered alias chain, resolved into `ConfigPathSelector::env`
  rungs in the order written. It is mutually exclusive with the single
  `env_var`, which is a compile error because the combined precedence would be
  ambiguous. A struct that sets only `automatic_mode` still honours `env_var`;
  the emitter prefers `env_vars` when it is non-empty, so opting into the
  policy path cannot silently drop the override a non-policy struct already
  honours.
- `explicit_mode` and `automatic_mode` accept the string forms of the runtime
  enum variants and are validated at compile time. Their defaults reproduce the
  existing behaviour: `first_wins` for automatic mode, and `required_exclusive`
  for explicit mode. An unrecognized string is a compile error naming the
  accepted set.
- `scope_order` lists scopes by name — `system`, `user`, or `project` — and is
  validated at compile time.
- `project_root_from` names a CLI field to read at run time, which is the part
  static attributes cannot otherwise reach: the generated code checks the
  parsed CLI value and calls `project_root` when it is present. The macro
  rejects the attribute unless the field exists, is not `skip_cli`, and is a
  `PathBuf` or `Option<PathBuf>`. The value is the Rust field identifier, not
  the serde rename, because the wiring is code-level.
- An injected environment source still reaches the builder on this path, so a
  `load_from_iter_with_sources` caller's `MapEnv` is honoured for both the
  selector rungs and automatic discovery. Without that step, discovery would
  read the real process environment instead.

The derive grammar and the runtime enum strings become a public grammar once
shipped, which is why RFC 0002 stages the derive extension last. The static
grammar also cannot express every policy — a root derived from two fields, or
conditional scopes — so the runtime `ConfigFilePolicy` remains the escape hatch
for an application that builds one by hand and bypasses the derive.

### Testing these diagnostics

Each refusal above has two tests, and they are not duplicates of one another.
The unit tests in
`ortho_config_macros/src/derive/parse/tests/discovery_validation.rs` call the
parser directly and assert the *message string* and the variant it belongs to;
that is the place a message is changed. The trybuild fixtures under
`ortho_config/tests/ui/` compile a small derive and pin the *rendered form* —
the message together with the span the compiler points at — which is the only
way to catch a diagnostic that still says the right thing in the wrong place.

Five fixtures cover the policy and discovery vocabulary:

- `policy_explicit_mode_invalid_value` and
  `policy_automatic_mode_invalid_value` for the two mode enums, each using a
  near-miss spelling so the fixture exercises the rejection rather than a typo;
- `policy_scope_order_invalid_value` for a scope name outside the three;
- `policy_project_root_from_skipped` and
  `policy_project_root_from_wrong_type` for the two `project_root_from`
  refusals the field lookup reaches. The third — a name matching no field at
  all — is covered by a parser unit test, which is why no fixture mirrors it.

Each `.stderr` is compiler output rather than a handwritten guess: emit it with
`TRYBUILD=overwrite cargo test -p ortho_config --test compile_fail`, then
re-run the same command with `TRYBUILD` unset to confirm. A snapshot that
drifts fails under the default, which is what makes the pair a check rather
than a record; a fixture whose code compiles clean is reported by trybuild as a
case that failed to fail, so add the `.rs` and `.stderr` together.

## Environment access boundary

`EnvSource` is the crate's injectable environment. `ProcessEnv` is the default
and preserves the historical behaviour; `MapEnv` models a closed set of
variables for tests.

### Ownership and permitted call sites

- `EnvSource` is owned by the discovery subsystem. `ConfigDiscovery` holds one
  as `Arc<dyn EnvSource>`, supplied through
  `ConfigDiscoveryBuilder::env_source` and defaulting to `ProcessEnv`.
- `ScanEnvSource` is owned by the merge boundary. `CsvEnv` accepts one through
  `with_source`, and the derived loader receives it through
  `OrthoConfig::load_from_iter_with_sources`.
- The discovery candidate reader and subcommand file discovery are the only
  production readers of `EnvSource`, and neither calls `std::env::var_os`
  directly: both go through the `EnvSource` implementation they are handed.
- The process-backed loaders are the legacy/default wrappers. They route their
  lookups through `ProcessEnv`, whose `get` is the crate's only
  `std::env::var_os` call.
- `GlobalConfigSources` is the `hello_world` application's narrow composition
  context. Its global and greeting-default loaders pass lookup and scan sources
  to the appropriate boundaries. Greeting-default loading also requires an
  explicit file base for `SubcommandFileContext`, so its source-aware path does
  not inspect the process working directory. It supplies clap match metadata
  with no explicit options so a clap default does not override an injected file
  or environment value. The existing no-argument loaders remain process-backed.
  File-only greeting overrides take a lookup source without acquiring the scan
  capability. This context belongs only at the example's configuration-loading
  call sites, not in command construction.
- `message::build_take_leave_plan_with_greet_loader` is a private composition
  seam owned by the `hello_world` message module. The public
  `build_take_leave_plan` supplies the process-backed greeting loader; message
  tests may supply explicit defaults or load them through
  `load_greet_defaults_with_sources` and `GlobalConfigSources`. Keep this seam
  within greeting/farewell composition and its tests; source selection remains
  with the configuration-loading caller.
- `test_support::sample_config` is the `rstest` fixture that loads the shipped
  sample files through closed sources. It is the single place that copies
  `config/baseline.toml` and `config/overrides.toml` into a fixture root and
  resolves the sample greeting defaults, so message tests inject it rather than
  repeating that setup. It belongs to the example's test support surface, not
  to production composition.
- Example tests may pass a `ConfigDiscovery` with explicit project roots to the
  crate-private override loader. Source injection alone does not disable
  project-root or current-directory candidates. Tests needing no ambient
  candidates must configure the discovery builder and set `XDG_CONFIG_DIRS` to
  a fixture directory when applicable, since an unset value selects the
  platform's system fallback.
- The `hello_world` override-test setup functions each stage one explicit,
  XDG, or local-app-data fixture and return its closed `MapEnv`. They belong
  only to the override scenario table; other tests use `ConfigFixture` and
  select their own source to keep setup and precedence assertions visible.
- It is **not** a general environment service. Adding readers elsewhere in the
  crate requires a decision about scope, not a call site.
- Subcommand file loading may use `SubcommandFileContext` for named lookup of
  `HOME`, `USERPROFILE`, and XDG configuration keys while resolving candidate
  files from its explicit base. On non-Unix and non-Redox targets it may also
  use the source's native platform configuration-directory fallback. It must
  not enumerate variables or become a general-purpose environment service.
- `first_existing_xdg_candidate` is private to subcommand path
  discovery and is called only by `push_xdg_candidates`. It skips `NotFound`
  and propagates other metadata errors with the candidate path.
- The derived `SelectedSubcommandMerge::load_and_merge_selected_with_sources`
  uses that file context and a separate `SharedScanEnvSource` to merge the
  selected enum variant. Its file base and environment values are supplied by
  the caller; the existing `load_and_merge_selected` remains process-backed.
  `SelectedSubcommandSources` groups the two capabilities only for
  `load_globals_and_merge_selected_subcommand_with_sources`; its global loader
  remains caller-owned. Manual trait implementations must override the injected
  method or receive `SourceInjectionUnsupported`, without falling back to the
  process environment.

### Composition rules

- **Lookup by name only.** There is deliberately no enumeration method. RFC 0001
  makes "the crate never scans the whole process environment" a safety property
  of environment access: a process holding unrelated secrets must never have
  them enumerated, copied, or logged. An enumeration method here would void
  that guarantee for every holder of an `EnvSource`, however careful individual
  callers were.
- **The merge layer uses a different abstraction.** `CsvEnv` legitimately scans
  a prefix because `figment::providers::Env` does, so its injectable path takes
  `ScanEnvSource`, a separate explicitly named type. The scanning capability is
  visible in the signature rather than latent in a trait whose other users must
  not scan. Callers can use one `Arc<MapEnv>` by coercing cloned `Arc`s to
  `SharedEnvSource` and `SharedScanEnvSource`;
  [#412](https://github.com/leynos/ortho-config/issues/412) completed this path.
- **Injected merge providers are declarative.** Injected `CsvEnv` loading
  replays only the declarative prefix, case, split, and CSV options. Calling
  `CsvEnv::with_source` after `map()` or `filter_map()` is rejected because
  arbitrary closures cannot be replayed against an injected `ScanEnvSource`.
- **Owned returns, deliberately.** An `impl Iterator` return would be
  return-position `impl Trait` in traits (RPITIT) and make the trait unusable
  behind a trait object, forcing `ConfigDiscovery<S>` generics to leak through
  the derive-generated `load()` and every call site. Configuration resolves
  once per process, so the allocation is immaterial.
- **`home_fallback` defaults to `None`.** `ProcessEnv` overrides it by
  default, and custom sources may too. See the users' guide for why an injected
  source must be able to suppress the platform lookup.
- **`config_dir_fallback` defaults to `None`.** Only non-Unix and non-Redox
  subcommand discovery uses it. `ProcessEnv` retains `directories::BaseDirs`
  native-path behaviour; `MapEnv` stays closed, and custom sources may supply
  an explicit native configuration root for deterministic tests.

### Process-backed environment test probes

Tests that compare `CsvEnv` process behaviour with an injected `MapEnv` must
run the process-backed provider in a child with a cleared environment and only
the explicit probe inputs. Keep the probe ignored during ordinary test runs and
require a dedicated opt-in environment marker before it reads or emits process
data, so accidental direct invocation cannot expose ambient variables. Run one
exact probe at a time with `--show-output --test-threads=1`, validate exactly
one dedicated output marker, and compare the child's result in the parent with
the injected provider's result. Prefix child inputs with a test-specific key so
runtime variables such as LLVM coverage settings cannot enter the comparison.
Coverage settings are the one deliberate exception: `.env_clear()` also drops
`LLVM_PROFILE_FILE`, so a probe running under instrumentation forwards it
through `test_helpers::coverage::resolve_profile_path` to keep the child's
profile next to its parent's.

Telemetry probes follow the same child-process boundary for the real process
source. Exercise injected sources separately in the parent, and assert both
that the event vocabulary stays bounded and that input sentinels never appear
in captured fields. This keeps process collection, injected-source coverage,
and redaction checks explicit without copying the parent's environment into a
probe.

## Dependency management

Cargo dependencies in this workspace follow strict version pinning rules so
that builds remain stable and reproducible across contributors and continuous
integration (CI) environments.

- Use SemVer-compatible caret requirements for every dependency declared in
  `Cargo.toml`, for example, `some-crate = "1.2.3"`. This is Cargo's default
  and accepts non-breaking minor and patch updates while rejecting breaking
  changes from a new major version.
- Do not use wildcard (`*`) or open-ended inequality (`>=`) version
  requirements. They admit unpredictable upstream changes into the build and
  are forbidden in this workspace.
- Reserve tilde (`~`) requirements for the narrow case where a dependency must
  be locked to patch-level updates for a specific, documented reason. Record
  the rationale alongside the dependency entry or in the related design
  document, so a later reader can re-evaluate the constraint.
- Keep dependencies current. When upgrading a crate, run the full quality
  gates (`make check-fmt`, `make lint`, `make test`) and, where the upgrade
  changes behaviour or public API, update the relevant design document, ADR, or
  migration guide.
- Capture substantive dependency choices, such as adopting or replacing a
  crate, in an ADR following the documentation style guide. Reference the ADR
  from the design document and from this guide where future contributors should
  be aware of the decision.

### Workflow pins and Dependabot

Dependabot owns scheduled dependency updates through `.github/dependabot.yml`:

- Root GitHub Actions updates run weekly and use the `dependencies` and
  `github-actions` labels.
- Root Cargo workspace updates run daily and use the `dependencies` and
  `cargo` labels.
- Python test requirements in `scripts/` update daily and use the
  `dependencies` and `python` labels.
- The root `rust-toolchain.toml` updates weekly through the independent
  `rust-toolchain` route, using the `dependencies` and `rust-toolchain` labels.

GitHub Actions updates include calls into `leynos/shared-actions`. Contract
tests that assert a caller's exact commit SHA create a lockstep dependency:
every time Dependabot opens a bump PR, the test fails until a human edits the
pinned constant to match. That defeats the purpose of automated dependency
updates and turns a routine bump into a manual chore.

Contract tests may still verify the *shape* of a reusable-workflow caller. They
must not verify the specific SHA value.

- Do assert the workflow references the correct reusable workflow path.
- Do assert the ref is pinned to a full 40-character commit SHA, not a
  mutable branch such as `main` or `rolling`.
- Do assert the expected `on:` triggers, least-privilege `permissions:`, and
  the inputs the caller relies on.
- Do not hard-code the current SHA value as an expected string. Match it with
  a pattern instead.
- Do not fail a test purely because Dependabot bumped the pinned SHA.

```python
import re

SHA_RE = re.compile(r"^[0-9a-f]{40}$")

def test_uses_pinned_full_sha(caller_step):
    ref = caller_step["uses"].split("@")[-1]
    assert SHA_RE.match(ref), f"expected a 40-hex commit SHA, got {ref!r}"
```

If a workflow's behaviour genuinely depends on a feature only present from a
particular commit onwards, express that as a comment or a changelog note, not
as a test assertion on the SHA string.

One narrow exception exists, and it is worth stating because it looks like a
violation. `sccache_wiring_test.py` names `32c8ea64` in a set of pins that
install sccache while exporting neither half of its wiring, and fails if the
workflows return to one of them. That is not a lockstep on the current value:
it never fails a forward bump, it hard-codes nothing that Dependabot will
change, and it exists because reverting the pin is the one edit that would undo
this wiring while leaving every other assertion here green.

### sccache: the wrapper and the backend

sccache needs two halves to do anything. The rustc wrapper makes the compiler
run through it; the backend gives the cache somewhere to live. Either half
alone is worse than neither, because the job pays the install and reports
nothing amiss.

This repository ran with neither. The shared Rust setup action installs sccache
whenever `use-sccache` is true, which is its default, and at pin `32c8ea64` it
exported no wrapper and selected no backend. Every Rust job installed sccache
and compiled uncached, which is visible only as a slow build.

The two halves are now set in different places, deliberately:

| Half                  | Where it is set                        | Why there                                                                                         |
| --------------------- | -------------------------------------- | ------------------------------------------------------------------------------------------------- |
| `RUSTC_WRAPPER`       | the shared action, not this repository | it must be the absolute path of the sccache the action installed                                  |
| `SCCACHE_GHA_ENABLED` | job-level `env:` in this repository    | it is a choice about where the cache lives, and the action stands aside when a caller has made it |

*Table 1: Which half of the sccache wiring is set where, and why.*

`RUSTC_WRAPPER` is deliberately absent from every workflow here. A bare
`RUSTC_WRAPPER: sccache` resolves through `PATH`, and an invocation that
resolves nothing compiles uncached without entering the hit-rate denominator,
so the statistic a reader would check cannot show the failure; whitaker #409
measured 69 such invocations after switching to the action's absolute path. The
action also stands aside when the caller has already set the variable, so a
value here would silently replace the path with the bare name.

Three jobs run the shared Rust setup and select the backend: `build-test` and
`binstall-packaging` in `ci.yml`, and `coverage-upload` in `coverage-main.yml`.

Two Rust lanes are outside this, each for its own reason:

- `mutation-testing.yml` calls a reusable workflow. Caller job `env:` does not
  propagate into one, and that workflow exposes no sccache input, so there is
  nothing to set from here.
- `verify-published-assets` in `release.yml` runs the setup but compiles
  nothing: it dry-runs `cargo binstall` against already published archives.
  Note that it is *not* excluded by the action's own
  `github.event_name != 'release'` guard, which a reader might assume. This
  workflow triggers on a tag push and on `workflow_dispatch`, never on the
  `release` event, so that guard never fires here.

`rust-build-release` and `mutation-cargo` are exempt from the one-SHA
assertion. Every reference into the shared-actions tree, those two included,
sits at `6cec89bac47a` today, where the repin that brought in setup-rust's
fail-open sccache start-up left them. The exemption remains an allowance for
them to diverge without the contract reporting a partial repin, as they did
after the CodeScene coverage adoption's repin: neither serves a lane this
wiring governs, `rust-build-release` is the release build the action excludes
from sccache, and `mutation-cargo` is a reusable workflow, which caller job
environments cannot reach. Moving either is a separate change that needs its
own evidence.

The allowance is proved in both directions rather than assumed. Moving
`rust-build-release` to a SHA of its own leaves the contract passing, which is
the exemption working; adding a sibling whose path merely begins with an exempt
one, such as a `rust-build-release-extra`, fails it, because the exemption is
keyed by the whole path inside the shared-actions tree and matched by equality
rather than by prefix.

`sccache_wiring_test.py` holds four things: that every reference into the
shared-actions tree sits at one SHA, so a partial repin cannot pass while the
guide claims otherwise; that the pin is not one of the revisions known to
export neither half; that no workflow sets `RUSTC_WRAPPER`; and that every job
discovered to run the shared Rust setup selects a backend, with the exclusions
named individually and with their reasons rather than as a blanket allowance.
The discovery is itself pinned, because the other assertions are all satisfied
by a sweep that finds no jobs.

Two of its sweeps are open rather than enumerated, and both had to be widened
after review found them closed. References are matched by the
`leynos/shared-actions/` prefix with `rust-build-release` and `mutation-cargo`
named as the exceptions, rather than by listing the four paths in use today: a
governed reference added later under an unlisted path would otherwise sit at
any SHA it liked while the contract reported agreement. And workflows are read
from both `*.yml` and `*.yaml`, because GitHub runs either, so a sweep over one
suffix claims repository-wide coverage while ignoring half the places a Rust
job can be declared.

Each caching job also ends with a `Report sccache statistics` step, and the
contract requires it. The wiring is invisible from the outside: a job with a
wrapper and a job without one both succeed, and only sccache's own compile
request and hit counts tell them apart. The step invokes the binary through
`SCCACHE_PATH` rather than by name, for the same reason the wrapper is not set
by name, and it runs on failure too, because the statistics of a failed build
are often what explain it.

Read the second run of a branch, not the first. The first stores into an empty
cache, so its hit rate says nothing about whether the wiring works. Compare
compile requests as well as hit rates: an invocation that resolved no wrapper
never enters the denominator, so a wrapper defect raises the request count
rather than lowering the rate.

Nine mutations are caught: one reference left at the old pin, the backend
removed from a job, `RUSTC_WRAPPER` set by name, the sweep narrowed so it finds
nothing, an excluded job quietly gaining a backend, the statistics step
removed, the statistics reported through a bare `sccache` rather than
`SCCACHE_PATH`, a governed reference added under a new path at a different SHA,
and a Rust job declared in a `.yaml` workflow without a backend. The last two
passed before the sweeps were widened, which is how they were shown to be real
rather than theoretical.

### Workflow contract gate

`make test-workflow-contracts` runs two stages against the checked-in workflow
and Make sources. The first is the shared CV-005 contract library,
`cv005-contracts check --repository .`, run through `uv tool run` on Python
3.13 from the full commit named by `CV005_CONTRACTS_REF`; it needs `uv` and
network access to fetch that commit. The second is the repository's own
contracts in `tests/workflow_contracts`. The target is separate from
`make test` because it needs neither a Rust toolchain nor the workspace's
Python test requirements. The second stage is this command:

```bash
uv run --with 'pytest>=8,<10' --with 'pyyaml>=6,<7' \
    --with 'hypothesis>=6,<7' pytest \
    tests/workflow_contracts --doctest-modules -q
```

Three things about that command are deliberate. The requirements are named
inline rather than taken from `scripts/requirements-test.txt`, so the gate
stays runnable on a checkout with no virtual environment. Each carries an upper
bound, because the target has no lockfile and a future major release of any of
them could change collection or doctest behaviour with no edit to this
repository. And `--doctest-modules` collects the examples in the support
modules, so an example that stops matching the helper it documents fails the
gate rather than ageing quietly.

`tests/workflow_contracts/makefile_support.py` holds the helpers the contracts
share for reading a Makefile: recognizing a recipe line, and extracting the
subcommand a target hands to a tool. It is support code, not a contract, so it
contains no test functions; its own executable examples are what
`--doctest-modules` collects. Contracts that read a workflow parse the YAML
document rather than matching its text, so a re-indentation cannot change a
verdict.

### Whitaker in continuous integration

In continuous integration (CI), the Linux lint leg installs Whitaker through
the shared `leynos/shared-actions/.github/actions/install-whitaker` action,
pinned to a full commit hash (SHA, the Secure Hash Algorithm identifier of a
Git commit), in a step named `Install Whitaker` that is guarded to
`matrix.platform == 'linux'`. The action owns the install: it takes a
digest-verified, prebuilt `whitaker-installer` (0.2.9 or later), passes
`--no-source-fallback`, preinstalls the prebuilt `dylint-link`, authenticates
with `github.token`, and caches the installer itself. The repository therefore
carries no resolve, cache or install script for it. Whitaker's lint suite is a
rolling release, so the suite is not pinned here; the pinned action revision is
what stays fixed.

The concordat QG-002 rule accepts a revision of that action only if it is a
shared-actions commit, from the revision that first carried the install rules
onwards, that leaves the action directory content-identical, so the pin moves
with the other shared-actions references (the one-SHA contract in
`sccache_wiring_test.py`, which exempts `.github/actions/rust-build-release` and
`.github/workflows/mutation-cargo.yml`; those two may sit at their own full
commit hashes, and the Whitaker pin follows the rule for every non-exempt
reference). The workflow contract `whitaker_install_test.py` asserts exactly one
`Install Whitaker` step, using the action at a full commit hash, with no
script of its own, on the Linux leg only.

### The advisory Clippy-on-beta lane

`clippy-beta` is the early-warning job for lints that have landed on the Rust
beta channel but not yet on the pinned toolchain. It is advisory at the *job*
level, through `continue-on-error: true` next to `runs-on` rather than inside
the lint step. Job scope is what keeps a new beta lint from blocking a merge
while still leaving each leg's own result visible; the pinned-toolchain
`build-test` job remains the required one, and `clippy-beta`'s own
`fail-fast: false` is what stops a failing Linux leg from cancelling the
Windows one before it reports.

The job runs on `ubuntu-latest` and `windows-latest`, and the Windows leg earns
its machine. `make lint-clippy` compiles materially different source on the two
platforms: `cargo-orthohelp/tests/powershell_windows.rs` is gated behind
`cfg(windows)` and is not compiled anywhere else, and the same is true of the
MSVC branch of `ortho_config/tests/documentation_examples/cargo_runner.rs`. On
2026-09-28 the job failed on four lints in the first of those — a `Result`
predicate and three constant-size chunk iterators — none of which a Linux-only
leg can see.

The lane's strictness is not written into the workflow. Both this job and the
pinned `build-test` job call `make lint-clippy`, whose flags come from the
Makefile's `CLIPPY_FLAGS`
(`--workspace --all-targets --all-features -- -D warnings`). CI states the
strictness once, in the Makefile, and
`tests/workflow_contracts/clippy_beta_test.py` asserts both ends: the job's
platforms and advisory scope, and the target it calls.

## Publish dry run

`make publish-check` runs `lading publish` over the workspace. lading copies
the workspace, then packages and dry-run publishes each crate in the order
`lading.toml` declares. That per-crate packaging is the point: `cargo package`
builds each crate from its own packaged sources, so it is the only thing here
that sees what a published crate exports. A symbol that is public within the
workspace but missing from a crate root compiles under the workspace test run
and under Clippy, and fails only in this step. Issue #414 is this repository's
own instance: `OrthoConfigSubcommandDocs` is exported by the workspace
`ortho_config_macros` but not by its published release of the same version, so
tarball verification resolves the re-export against the published crate and
fails with "no `OrthoConfigSubcommandDocs` in the root". Nothing else in CI
sees that.

The order is a claim about the workspace graph, so it is checked against the
graph rather than restated beside it.
`tests/workflow_contracts/publish_order_test.py` derives the published set from
the manifests (a member publishes unless it sets `publish = false`) and the
edges from the `path` dependencies they declare. It asserts that the order
names exactly the publishable members, and that a crate appears after
everything it depends on. A wrong order still builds locally, because Cargo
resolves the path; it fails only when a publisher asks crates.io for a version
that is not there yet.

Dev dependencies are real edges here. The workspace has no dev-dependency cycle
today: the macros crate once dev-depended back on `ortho_config`, which made
the pair mutually dependent, and that edge has since been removed. The contract
keeps a narrow exemption for mutually dependent pairs, so a future cycle cannot
fail the ordering check for a reason no order can fix. Every other edge is
enforced, including `ortho_config`'s dev dependency on
`ortho_config_test_helpers`, which is why the helper crate is published before
it. That edge has already been broken once.

Before it packages, lading runs a pre-flight:
`cargo check --workspace --all-targets`, then `cargo test`, both into a
throwaway target directory. `lading.toml` sets `preflight.unit_tests_only`,
which narrows the second of those to the library and binary unit tests.

In CI the pre-flight is skipped outright. The `Publish dry run` step sets
`LADING_SKIP_PREFLIGHT`, because both `Test and Measure Coverage` steps are
unconditional and run ahead of it, so the pre-flight would be a second
execution of a workspace this job has already passed and failed its lane on.
The skip drops the auxiliary builds and the cargo check and test pair, and
nothing else. The `Cargo.lock` freshness guard still runs, and the working-tree
guard is unaffected: it is opt-in through `--forbid-dirty` either way, and
`PUBLISH_CHECK_FLAGS` is empty, so neither a skipped nor an executed pre-flight
enforces a clean tree here. The packaging still runs.

The variable is set on the step rather than in `lading.toml`, because a
configuration file cannot tell a CI run from a local one. On a workstation
nothing has run the tests first, so `make publish-check` still runs the full
pre-flight. `tests/workflow_contracts/publish_preflight_scope_test.py` pins
both halves of that arrangement, and pins the packaging from both ends: the
step's command as tokens, and the Make target's recipe handing lading the
`publish` subcommand. Either half alone is defeatable.

lading itself is pinned. `LADING_REF` in the Makefile holds the full commit SHA
of the v0.3.1 release commit, which is the first release carrying the skip. It
names a SHA rather than the tag because a tag can be repointed. Without any pin
at all, `uvx --from git+...` tracks lading's default branch and would change
what the release gate runs with no edit to this repository.

## Releasing `cargo-orthohelp` binaries

`cargo-orthohelp` is installed by downstream continuous integration (CI) under
a no-source-build policy, so every release must carry prebuilt archives that
`cargo binstall` can resolve. `.github/workflows/release.yml` publishes them
for five targets, each built on a runner of its own architecture and operating
system rather than cross-compiled:

*Table: `cargo-orthohelp` release targets and native runners.*

| Target                      | Runner             |
| --------------------------- | ------------------ |
| `x86_64-unknown-linux-gnu`  | `ubuntu-24.04`     |
| `aarch64-unknown-linux-gnu` | `ubuntu-24.04-arm` |
| `x86_64-apple-darwin`       | `macos-15-intel`   |
| `aarch64-apple-darwin`      | `macos-latest`     |
| `x86_64-pc-windows-msvc`    | `windows-latest`   |

Table: Release targets for `cargo-orthohelp` and the runner each archive is
built on, one runner per target architecture and operating system.

### Archive layout

`scripts/release_archive.py` builds one target and writes
`dist/cargo-orthohelp-<target>-v<version>.tgz`, holding exactly one member,
`cargo-orthohelp-<target>-v<version>/cargo-orthohelp` (with `.exe` on Windows),
plus a `sha256sum`-compatible `.sha256` sidecar. Member metadata and the gzip
timestamp are fixed, so rebuilding a tag reproduces the same bytes.

That layout is a contract with the `[package.metadata.binstall]` templates in
`cargo-orthohelp/Cargo.toml`: `pkg-url` renders the archive name and `bin-dir`
renders the member path. Change one and the other must change with it.
`scripts/tests/test_release_archive.py` renders the real templates against the
staged archive, and `tests/workflow_contracts/release_workflow_test.py` pins
the workflow shape, so a mismatch fails on the pull request.
`scripts/verify_release_archives.py` applies the same checks to a directory of
archives; the workflow runs it on the staged output and again on the downloaded
draft assets.

### Release flow

The workflow creates a draft release, builds and uploads every target's archive
and sidecar, audits the draft, publishes it, then resolves the real asset URLs
with `cargo binstall --dry-run`. Two details are load-bearing and have broken
releases elsewhere in the estate:

- The jobs that call `gh` without an `actions/checkout` step set `GH_REPO`.
  Otherwise `gh` infers the repository from a git remote and fails with
  `not a git repository`, skipping every downstream job.
- The auditing job requests `contents: write` even though it only reads.
  Draft releases are invisible to read-scoped tokens.

A tag with a suffix, such as `v1.2.3-beta.1`, is published as a prerelease. The
workflow serializes on the effective tag, so a tag push and a manual dispatch
for the same tag cannot both pass the "does this release exist" check before
either creates it.

Build jobs check the release tag out, so the packaging script comes from the
tagged tree. Tags cut before this workflow landed carry no packaging script and
cannot be republished; cut a new tag instead.

`cargo binstall` reads `[package.metadata.binstall]` from the crates.io
release, not from this repository, so a version must be published to crates.io
with that metadata before `cargo binstall cargo-orthohelp@<version>` resolves
the archives. 0.9.1 is the first such version.

`cargo-orthohelp` carries its own version rather than inheriting the workspace
one, so a tooling-only release is a patch bump of that crate alone. Edit
`cargo-orthohelp/Cargo.toml`, refresh the lock with
`cargo update -p cargo-orthohelp`, and tag `v<version>`. Use
`scripts/bump_version.py` only when the whole workspace is being released.

That script discovers the internal dependency requirements instead of naming
them. For each member manifest it bumps every dependency declared with a
`path`, across the normal, dev, and build tables. A hand-kept list of crates
went stale as soon as `ortho_l10n` joined the graph: `ortho_config` kept
requiring a version the release would never publish, and the resulting failure
would have surfaced only in the publish dry run. External requirements are left
alone, so a dependency on a third-party crate is never rewritten. Selection
works on the table and the name together rather than on the name alone, so a
crate whose name is also an external dependency's is rewritten only in the
table where it is declared with a `path`.

### Verifying the packaging locally

`make test` covers the packager and the auditor; `make test-workflow-contracts`
covers the workflow shape. To exercise the real build:

```bash
uv run --script scripts/release_archive.py "$(rustc -vV | sed -n 's|host: ||p')"
uv run --script scripts/verify_release_archives.py --dist-dir dist \
  --target "$(rustc -vV | sed -n 's|host: ||p')"
```

The pull-request job `binstall-packaging` runs those two commands plus a smoke
test of the extracted binary on Linux, macOS, and Windows.

## Spelling gate

`make markdownlint` enforces en-GB-oxendict (Oxford) spelling over the
repository's Markdown prose with [`typos`](https://github.com/crate-ci/typos),
as required by the [documentation style guide](documentation-style-guide.md).
Run the gate on its own with `make spellcheck`; `make spelling` is an alias for
it, because the shared `AGENTS.md` spelling block names that command. The whole
gate is the shared
[`typos-config-builder`](https://github.com/leynos/typos-config-builder), run
through `uv tool run` and pinned by the Makefile `TYPOS_CONFIG_BUILDER_VERSION`
variable, so local runs and CI use the same version of both the builder and the
`typos` binary it pins:

```bash
make spellcheck
```

The gate renders `typos.toml`, runs `typos` over the tracked Markdown files,
and then enforces the shared `[phrases.corrections]` policy, which rejects the
hyphenated form of "handwritten". Any stage that reports findings fails the
gate.

The generated configuration lives in the repository-root `typos.toml` and works
in three layers:

1. The `en-gb` locale corrects American spellings (`color` to `colour`,
   `behavior` to `behaviour`, `analyzed` to `analysed`).
2. The shared estate dictionary supplies generated `extend-words` entries that
   restore Oxford spelling, which the locale alone would not enforce: identity
   entries accept `-ize` inflections that the locale would otherwise "correct"
   to `-ise`, and `-ise` entries are corrected to `-ize`. Stems taking `-yse`
   (`analyse`, `paralyse`) are left to the locale, which already enforces them.
3. `typos.local.toml` adds only repository-specific names, quotations,
   deliberate fixtures, and exclusions that do not belong in the shared base.

`typos.toml` is a generated file. Never edit its entries by hand. The gate
regenerates it on every run from the live shared dictionary, which it refreshes
into untracked `.typos-oxendict-base.toml`, merged with the `typos.local.toml`
overlay. A word added to the shared dictionary therefore reaches this
repository on its next run, with no change here. Because the dictionary is live,
`typos.toml` must never be drift checked in continuous integration; any hand
edit is overwritten on the next run. When a run changes the tracked
`typos.toml`, commit the regenerated file; if the change is unrelated to the
work in hand, commit it in a separate base pull request and stack the branch on
it. `TYPOS_CONFIG_BUILDER_VERSION` in the `Makefile` pins the release the gate
runs (currently `v0.1.3`); raise it together with the regenerated `typos.toml`,
never on its own.

Generic Oxford stems and corrections belong in the shared dictionary maintained
by `leynos/agent-helper-scripts`. Keep local entries narrow: this repository's
overlay preserves its library names, non-English fixtures, tool and standards
names, and ExecPlan headings. Quoted APIs keep US spelling per the
documentation style guide. Inline code is spellchecked, so add a narrowly
backtick-bound pattern to `typos.local.toml` for an upstream API or identifier
rather than adding a word-level exception. Fenced code blocks remain ignored.

The gate passes `--force-exclude` so the `typos.toml` excludes also apply to
explicitly passed paths, for example Markdown that appears inside `target`
build output. To fix findings mechanically, run `typos` directly against the
generated configuration with `--write-changes`:

```bash
uv tool run typos --config typos.toml --force-exclude --write-changes <files>
```

Review automated rewrites before committing; spelling corrections must not
touch code samples, API names, or quoted material.

Bumping `TYPOS_CONFIG_BUILDER_VERSION` is only needed for builder code changes;
dictionary changes need no bump.

## Cancelling superseded pull-request runs

Every push to a pull request starts a fresh run of each gate. The run already
in flight is answering a question about a commit nobody will merge, and left
alone it holds a runner until it finishes, so the branch pays twice for one
answer. Every workflow a pull request can start therefore carries a concurrency
block:

```yaml
concurrency:
  group: ${{ github.workflow }}-${{ github.event.pull_request.number || github.ref }}
  cancel-in-progress: ${{ github.event_name == 'pull_request' }}
```

Two halves matter, and each fails in a way nothing else would notice.

- **The group keys on the pull request.** A group built from
  `github.run_id` is unique to one run, so it matches no predecessor and
  cancels nothing while reading exactly like a concurrency control. A constant
  group is the opposite failure: every open pull request shares one queue, and
  the first push anywhere cancels the gates running everywhere else.
- **Cancellation is conditioned on the event.** A literal
  `cancel-in-progress: true` reads as the stricter setting and is a regression.
  A push to `main`, a schedule, and a dispatch have no successor waiting, and
  the run on `main` writes the warm cache and records the coverage that no
  later run repeats.

`pull_request_target` workflows are out of scope. They run against the base
repository to carry a token, and the ones here automate pull-request
housekeeping rather than building, so cancelling one mid-flight is a hazard
with no minutes to win.

### The cancellation contract

`tests/workflow_contracts/concurrency_test.py` reads `.github/workflows` and
asserts, for every workflow declaring a `pull_request` trigger, that it
declares a concurrency group, that the group names no per-run expression, that
the group names something that varies per pull request, and that
`cancel-in-progress` is exactly the expression above. A further test asserts
that discovery still finds the workflows it is expected to, so a broken read
cannot empty the list and turn the rest into a vacuous pass. Run it with
`make test-workflow-contracts`.

## Development build footprint

The Concordat `rust-dev-build-footprint` rule keeps default development builds
small. The workspace `Cargo.toml` sets `incremental = false` and `debug = 0` in
`[profile.dev]`, and the test profile inherits both, so builds keep no
incremental compilation state and no debug information. The trade-off is that a
debugger sees no symbols or line tables in a default development build and
every rebuild recompiles whole crates. To debug, run with
`CARGO_PROFILE_DEV_DEBUG=true`; to rebuild incrementally, set
`CARGO_INCREMENTAL=1`. The pin is the stable `1.99` channel, so the rule's
unstable metadata key does not apply.
`tests/workflow_contracts/build_footprint_test.py` holds both settings.

## Test timeouts: the tiers this repository sets

Four independent timers can end a test run, and the canonical statement of how
they must be ordered lives in the `generate-coverage` README in
[`leynos/shared-actions`][shared-actions-coverage]. All four are set here now;
until this branch, tier one covered two trybuild binaries and nothing else, and
tier two did not exist at all.

| Tier                     | What it bounds                     | Where it is set                            | Current value                                                                           |
| ------------------------ | ---------------------------------- | ------------------------------------------ | --------------------------------------------------------------------------------------- |
| Per-test `slow-timeout`  | one test                           | `.config/nextest.toml`                     | 600 s (60 s x 10) base; 960 s (120 s x 8) for trybuild, BDD and documentation scenarios |
| nextest `global-timeout` | the whole test run                 | `.config/nextest.toml`                     | 1,800 s (30 m)                                                                          |
| Cargo watchdog           | one `cargo` invocation, wall clock | `RUN_RUST_CARGO_WAIT_TIMEOUT` at job level | 2,700 s (45 m)                                                                          |
| Job `timeout-minutes`    | the whole job                      | job level                                  | 165 m in `ci.yml`, 120 m in `coverage-main.yml`                                         |

*Table: the timers that can end a run, innermost first.*

### Documentation subprocess budgets on shared hosts

Executable documentation examples use the fixed 956-second scenario envelope
described above. Build, execution, and discovery are capped at 956, 60, and 30
seconds respectively; each operation also inherits the scenario's remaining
time. `cargo-orthohelp` BDD scenarios use the same 956-second envelope. These
scenarios run under a 960-second nextest allowance, count Cargo lock and
compiler admission time, and have no environment override; controlled
regression tests inject short-lived policies directly so they cannot change
parallel test processes.

The earlier separate-target implementation of the documented CLI flow took 56.5
s and 120.7 s on hosted runners, 588.0 s and 593.6 s on a shared host, and
600.5 s on a saturated shared host, where the old 600-second limit ended the
run. Those wall times did not include correlated admission-delay or reliable
cache hit/miss evidence. This change reuses Cargo's configured target and
shared cache, removing the duplicate cold dependency build. The finite scenario
ceiling remains tied to nextest's per-test allowance rather than being
increased to absorb unmeasured queue time.

### The outermost tier was missing

Neither coverage job declared `timeout-minutes` before this was written, so
both inherited GitHub's six-hour default. That is not a budget anyone chose,
and the Windows leg of `ci.yml` already runs for 86 minutes, so a hang there
cost six hours of a paid runner before anything stopped it.

### Two watchdogs per job, not one

Each coverage job runs the action twice, once with `serde_saphyr` and once
without, and each invocation gets its own watchdog. So the job must be able to
contain both budgets before it contains anything else, and the requirement is
the watchdog multiplied by the number of coverage steps in that job, plus the
work outside them. The canonical rule does not spell this out because most
callers invoke the action once.

### The per-test budget is a product, not a period

`terminate-after` counts warning periods, so the budget a test gets is `period`
multiplied by it. The longest override here is 120 s with a multiplier of
eight, so reading the period alone would report 120 s where the real figure is
960 s. The contract asserts that reading outright rather than leaving it
implied.

Two tiers can also name the *same* product while meaning different things,
which is worth knowing before sizing one by the other. The trybuild override
was 120 s x 5 and the default profile is 60 s x 10, so both allowed 600 s and
the override changed only how often the warning appeared. A binary added to the
override for its larger-looking period therefore gained nothing at all.

### Tier one covered two binaries, and tier two did not exist

The only `slow-timeout` in `.config/nextest.toml` was the trybuild override.
Two binaries were bounded at 600 s and every other test in the suite ran with
nothing under the job ceiling to stop it, because nextest's built-in 60 s
`slow-timeout` warns and never terminates: it names no `terminate-after`. The
sampled Windows run below reported 22 tests as slow and terminated none of
them. No `global-timeout` was set either, so a hung run was bounded only by the
cargo watchdog, which names `cargo` rather than the test still running.

Both are set now, and both were sized from this repository's own run history
rather than chosen. Three `build-test` jobs were read line by line, across
successful and failed runs and both platforms:

| Sample                                | Slowest single test | Longest nextest run | Run         |
| ------------------------------------- | ------------------- | ------------------- | ----------- |
| `build-test (windows-latest)`         | 364.8 s             | 1,023.6 s           | 34119577952 |
| `build-test (ubuntu-latest)`          | 301.8 s             | 620.0 s             | 34120845807 |
| `build-test (windows-latest)`, failed | no test output      | none                | 34124529877 |

*Table: per-test and whole-run durations, read from the nextest output of each
job. Each job runs the suite twice, so the two successful samples carry four
runs between them: 1,023.6 s and 648.2 s on Windows, 620.0 s and 417.7 s on
Linux, over 2,228 and 2,244 timed test results.*

The slowest test outside the trybuild override was
`cargo-orthohelp::compile_time must_use_compile_tests` at 364.8 s. The base
allowance is 600 s, ten warning periods of 60 s, about 1.6 times that worst
case: enough that a legitimately slow test finishes, small enough that a hang
is caught well inside the whole-run budget.

### The trybuild allowance was raised after it killed two tests

The trybuild class outgrew its 600 s. It is budgeted apart from the base
allowance because each of these binaries spawns a child `cargo` in a separate
target directory, so none shares cargo-llvm-cov's warmed cache and each pays a
cold dependency build. They also share one package cache and so block on its
lock while their neighbours build, which the logs show directly: run
36070786646 recorded four `Blocking waiting for file lock on package cache`
lines, and run 36068184976 recorded none.

Two measurements set the new figure:

| What                             | Duration | Of its budget   | Run         |
| -------------------------------- | -------- | --------------- | ----------- |
| `crate_path_trybuild`, passed    | 572.5 s  | 0.95 of 600 s   | 36068184976 |
| `must_use_compile_tests`, killed | 600.2 s  | the whole of it | 36070786646 |

*Table: the worst trybuild durations observed. The first passed with only 27 s
to spare, so the old allowance was already too small rather than merely
unlucky; the second was still cold-compiling its first dependencies
(`proc-macro2`, `unicode-ident`) when it was stopped, so it was slow rather
than hung.*

The allowance is now 960 s, eight warning periods of 120 s, which is 1.68 times
the 572.5 s worst observed — the same headroom the base allowance was sized
with against its own 364.8 s. The filter names all ten trybuild binaries rather
than the two it used to; eight end in `trybuild` and `compile_fail` and
`compile_time` do not, so no single glob covers the class and the names are
enumerated instead. Ten is the count `trybuild_tier_test.py` enforces, so a
trybuild binary that arrives after the enumeration is written is reported by
the contract rather than silently left on the base allowance:
`subcommand_trybuild` arrived that way on `main`, and `localization_trybuild`
and `boolean_cli_trybuild` arrived that way from branches, and the contract is
what caught each.

The 960 s allowance is lower than the 1,800 s whole-run budget, which the
ordering contract asserts. That comparison does not by itself guarantee a test
using its full allowance finishes before the run deadline, since a test that
starts late spends part of the budget waiting to be scheduled; it establishes
that the allowance cannot exhaust the run on its own.

The whole-run budget is 30 minutes, about 1.76 times the 1,023.6 s worst run.
It has to fit inside the watchdog with nextest's termination procedure and a
cold build counted, which is 1,800 s plus 70 s plus 600 s, or 2,470 s. The
watchdog was 1,800 s and could not have carried it, so it rises to 2,700 s.
That is what moves the ceilings: the requirement is the watchdogs a job
contains plus the work outside them plus the margin, and each job runs the
action twice.

[Issue 483](https://github.com/leynos/ortho-config/issues/483) asked for these
measurements and named the constraint they had to satisfy, that each coverage
job invokes the action twice so raising the watchdog costs twice as much
ceiling here. It is answered above.

### Documentation subprocess and cache ownership

The documentation process runner applies a hard overall deadline to each
subprocess. Build, execution, and discovery budgets are 956, 60, and 30
seconds, with two seconds reserved for bounded cleanup. A fixed scenario
deadline is never extended by output. The runner drains both streams while
retaining at most 1 MiB per stream, reports truncation and available evidence
on timeout or wait failure, and limits reader completion during cleanup.

These operations do not create a new Cargo target for each example workspace.
They resolve the caller's configured target directory, including the target
selected by `cargo llvm-cov`, and reuse Cargo's default shared package cache. An
`fs4` sidecar lease under `target/documentation-examples/v2` serializes
documentation builds through their JSON artefact snapshot; Cargo remains the
owner of compiler output and package-cache locking. Each example's generated
manifest, run directory, inputs, and executable snapshots remain private. BDD
scenarios hold their own cross-process lease while exercising the shared bridge
cache. These helpers are confined to documentation tests and do not change
application environment policy or terminate processes outside the owned
subprocess tree.

### What the ceilings are sized against

The allowance for work outside the watchdogs is per lane, because the two
differ by an order of magnitude and holding the trunk lane to the pull-request
lane's figure would demand a ceiling its own runs cannot justify.

| Lane                                   | Coverage steps  | Worst whole job | Outside those steps | Run         |
| -------------------------------------- | --------------- | --------------- | ------------------- | ----------- |
| `ci.yml` `build-test` (windows-latest) | 1,323 s + 950 s | 5,530 s         | 3,257 s             | 33447440225 |
| `coverage-main.yml` `coverage-upload`  | 563 s + 495 s   | 1,342 s         | 284 s               | 31908409573 |

*Table: measured coverage-step and whole-job durations. The last column is the
job's duration less its two watchdog-bounded coverage steps, so it is the work
the job timer covers and the watchdogs do not.*

The sample is 103 `ci.yml` coverage jobs, 100 successful and the rest failed or
cancelled, and 31 runs of `coverage-main.yml`, 29 successful and 2 failed. Runs
of every conclusion are read, not only successful ones: a run cancelled at its
ceiling is the case the sizing exists to prevent. No run in either sample was
ended by any of the four timers, the worst `ci.yml` job reaching 5,530 s.

On the Windows leg the work outside the coverage steps is dominated by cache
saving. So `ci.yml` is allowed 60 minutes and `coverage-main.yml` 15. With the
watchdog at 2,700 s and two coverage steps per job, the requirements are 165
and 120 minutes: 5,400 s of watchdog, plus the lane's allowance, plus a 900 s
margin. The margin is a term of the requirement rather than slack above it,
because a ceiling equal to the sum it contains cancels the job at the moment
the watchdog would have reported the overrun, and the report is the only thing
that makes an overrun actionable. The ceilings equal their requirements.

A lane in a workflow the contract has not measured is held to the larger
allowance until someone measures it and records a run id.

The first version of this section recorded 2,717 s for `ci.yml` and two
different figures for `coverage-main.yml`, 68 s here and 138 s in the contract.
Both came from counting the coverage steps differently: the numbers above
subtract the two `generate-coverage` steps and nothing else, which is exactly
what the watchdogs bound.

None of those runs was genuinely cold. One run is the coldest seen so far, not
a measurement of the cold case.

### The contract

`tests/workflow_contracts/timeout_ordering_test.py` asserts this by value over
every job invoking the coverage action, in both the `.yml` and `.yaml`
extensions. Jobs are its unit rather than steps, because the ceiling belongs to
a job and has to contain every watchdog inside it; counting the steps is what
makes the two invocations visible to the arithmetic. It reads a step's own
environment before the job's, as GitHub resolves it, and it fails on a
coverage-invoking job that declares no ceiling at all. The readings it rests on
live in `nextest_budgets.py`, `nextest_document.py`, `nextest_durations.py`,
`nextest_errors.py`, `timeout_budgets.py` and `coverage_lanes.py`, and are
driven with controlled values in `timeout_reading_test.py`.

Run them with `make test-workflow-contracts`. The target provisions `pytest`,
`pyyaml` and `hypothesis` through `uv run --with` rather than from the
project's own dependencies, so the contracts need no virtual environment of
their own and nothing they need reaches the published package. They are Python
because what they read is YAML and TOML; `make test` does not run them.

Each reading takes what it reads rather than fetching it. `coverage_jobs_of`
queries supplied workflow documents and reaches no filesystem and no parser,
`workflow_documents` is the acquisition that reads a directory, and
`coverage_jobs_in` is the two together, defaulting its directory to the
repository's own so the contract can call it with no argument at all while the
query below it can never reach a file. The pair is named for what each takes,
because a call site has to say which it is doing. The budget readings are
driven with Hypothesis as well as with named cases, in
`timeout_budget_properties_test.py`: the unit table and the duration grammar
are where a single wrong entry would leave every comparison downstream an
inequality between two plausible numbers, which a fixed case only catches when
it happens to be the case somebody wrote.

The nextest configuration is parsed with `tomllib` rather than matched as text.
A text match finds a key inside a comment, inside a `filter` string, or in a
table nextest never consults, and reports a budget the runner does not use.
`.config/nextest.toml` sets `global-timeout = "30m"`, and the ordering
assertion requires that value to sit above the largest per-test allowance and
inside the watchdog; it skips only when no whole-run budget is set at all. A
scraping reader would read a commented-out or filtered budget as one in force,
and comparing against a budget nobody had written is the failure this avoids.
`terminate-after` is optional, and a `slow-timeout` without it marks a test
slow and never stops it, so the reading refuses that form rather than reporting
one period as the budget. Every table in `.config/nextest.toml` sets it
explicitly, so no value here changes.

Durations are read with the grammar `humantime` accepts, which is what nextest
deserializes them with: a sequence of components each carrying a unit, written
`60s`, `2h 37m` or `2h37m`, with the long unit spellings. The reader used to
take one value and one of four short units, so `2h 37m`, `300 sec` and `30d`
were each refused as malformed while nextest loads all three, and two of them
sat in the contract's refusal list asserting the reader's own limitation as
though it were the file's fault. The grammar was measured against humantime
2.3.0, which is what the lockfile of the pinned cargo-nextest release resolves,
by compiling that parser and running the cases through it. Naming the version
matters: an earlier note here cited 2.4.0, which is the newest release rather
than the one the shared coverage action installs, `cargo-nextest@0.9.120`. A
value may carry a fractional part, and whitespace is tolerated around the
point, so `1.5m` and `1 . 5 m` are both ninety seconds. Whitespace inside the
number is ignored too, so `1 0s` is ten seconds and `1 2 . 3 4 s` is 12.34. The
short spellings `wk`, `wks`, `yr` and `yrs` are units alongside the longer
ones. The bare `0` is the one duration humantime reads without a unit, and it
is the exact text: its parser special-cases `0` before reading a character, so
`" 0 "` is refused and a reader that stripped whitespace first would accept a
duration nextest rejects. Case is significant, so `m` is minutes and `M` is
months.

The watchdog is resolved at the innermost scope that declares it, blank
included. GitHub takes the most specific declaration of an environment
variable, and an empty string is a declaration: a step setting
`RUN_RUST_CARGO_WAIT_TIMEOUT` to `""` hands that step's process an empty value,
not the job's. A reader that skips blanks and carries on outward credits the
lane with a budget nothing enforces, and the ordering assertion then passes
over a ceiling that does not exist. A case covers it, and restoring the
skip-and-continue reading fails that case alone.

Whitespace is Rust's, not Python's, and the class is written out for the same
reason the digit class below is. Rust's `char::is_whitespace` is the Unicode
White_Space property; Python's `\s` is that property plus U+001C to U+001F, the
file, group, record and unit separators, and `str.strip` and `str.split` carry
the same excess. Measured over the whole of Unicode, that is the only
disagreement, and it runs one way: Rust matches nothing Python does not. So a
reader spelling its whitespace `\s` skips a separator wherever it skips a
space, and reads `1\x1cs` as one second from a configuration nextest refuses at
startup.

Five sites carry the class, not one: the digit run, the fraction, the unit, the
outer trim, and the join that collapses a spaced number's digits. The join
cannot change an answer while the pattern refuses a separator, so no input
through the reader distinguishes a correct join from `str.split`; it is written
correctly anyway and tested through the widest whitespace the class allows,
because a later widening of the pattern would otherwise turn a refusal into a
silently different number.

Two contracts hold it. Four refusal cases name the separators, including one
between a digit and its unit, which is the shape nobody would notice in a file.
The other pins the class in both directions: Python's whitespace must exceed
the reader's by exactly those four, and the reader's must exceed Python's by
nothing. The second direction is the one the refusal cases cannot see, because
a class that had lost a genuine space would make the reader refuse
configurations nextest loads.

A digit is `0` to `9` and nothing else. Python's `\d` matches every Unicode
decimal digit and `int` reads them, so a reader written with it returns three
hundred seconds for `\u0663\u0660\u0660s` and for the mixed `3\u0660\u0660s`,
both of which humantime refuses: its parser compares against `'0'..='9'`,
reporting "expected number at 0" for the run that opens with such a digit and
"invalid character at 1" for the run that does not. The mixed spelling is the
sharper case because a reader that checked only its first character would still
accept it. That is the wrong direction for a contract, which would then certify
a configuration nextest cannot load.

The arithmetic is exact and in integers, because humantime's is: its parser
works in checked `u64` throughout and reports every failure as an overflow.
Reading a value through a float instead rounds what humantime refuses into
something plausible and certifies a configuration nextest cannot load.

Which integer depends on the unit, and this is the part a reader working in
nanoseconds alone gets wrong. A fraction of an hour or anything longer is
converted into whole *seconds*, so `0.000001h` is refused although its value is
a whole 3,600,000 ns, while `0.25h` is fifteen minutes. A fraction of a minute
or anything shorter is converted into whole nanoseconds, so `1.999999999s` is
accepted and `0.0000000015s` is not. A fraction of a nanosecond is refused
outright, whatever it spells, so even `1.0ns` will not load. The unit tables
are therefore split by which of the two a unit is measured in.

Four ceilings come with it, and they are different. A numeric literal must fit
the `u64` humantime reads it into, so `1000000000000000000000ns` is refused
even though its value in seconds is small. A fraction's own arithmetic is
checked, so `0.1000000000000000000s` overflows on the multiplication and
`1.00000000000000000000s` on the denominator, although both would fit as
durations. The accumulated seconds must fit the `u64` they are summed into, so
`18446744073709551615s` loads and one second more does not.

And the nanosecond remainder has a ceiling of its own, which is the one that
catches a reader summing into an unbounded integer. `add_current` opens with
`(out.subsec_nanos() as u64).add(nsec)?`, before any carry, so the remainder
held so far plus the component's nanoseconds must fit a `u64` by themselves.
Two values of `u64::MAX` nanoseconds carry the first to 18,446,744,073 seconds
and then overflow on the second. The duration they name, about 36.9 billion
seconds or some 1,169 years, is nowhere near the seconds ceiling; it is the
remainder that overflows, and it overflows first. That is exactly why checking
only the accumulated seconds afterwards reports a duration for text nextest
will not start under.

The reading was checked against the parser rather than against its
documentation: 4,016 generated durations, spanning every unit spelling,
fractions of up to twenty-one digits, values around the `u64` boundary and
humantime's tolerated whitespace, were run through both this reader and
humantime 2.3.0 compiled from the pinned release, and the two agreed on every
one.

It pins the condition each lane carries, which is none today. A skipped step
runs no `cargo`, so its watchdog never arms and the tiers say nothing about it:
`if: false` on the step or on its job would leave a lane that looks bounded and
is not. Adding a condition has to change the contract and this section with it,
and the lane coordinates are compared both ways, so a coverage lane appearing
without an entry fails rather than passing unexamined.

A document whose shape the reading does not expect fails on the assertion it
belongs to rather than with a Python fault several frames away. Each malformed
shape had its own way of raising during derivation: a `jobs` value that is a
scalar reaches `.items()`, and a non-mapping `env` or an unreadable
`timeout-minutes` reached arithmetic they could not survive. Each would have
failed the contract on a workflow that has nothing to do with coverage.

The two outcomes are not the same. A `jobs` value that is not a mapping yields
no lane at all: nothing in that document is a coverage job, so the document
contributes nothing to the assertions. A malformed value inside a job that does
run coverage keeps the lane because the lane is real. It reads the affected
budget as unset: an `env` that is not a mapping leaves the watchdog unset, and a
`timeout-minutes` that is not a positive whole number of minutes leaves the
ceiling unset. Both then fail the assertion that a coverage lane must declare
the tier in question, which is what a maintainer can act on.

It also pins how many coverage steps each job runs. The ceiling's requirement
is the sum of the watchdogs found, so deleting one of a job's two coverage
steps lowers that requirement by 2,700 s and every timing assertion still
passes while the lane measures half of what it did.

`tests/workflow_contracts/timeout_budget_properties_test.py` holds the readings
themselves, driven with synthetic workflows and synthetic nextest
configurations rather than the repository's own. Every ceiling here sits well
above its requirement, so a missing term in the derivation changes nothing
observable in this tree; against controlled numbers it does not. The lane
reading requires its documents, so `coverage_jobs_of(documents)` reaches no
filesystem and no parser; `workflow_documents(directory)` is where the
filesystem access and the YAML parsing happen, defaulting its directory to the
repository's own; and `coverage_jobs_in(directory)` composes the two. The
boundary is one named function rather than a default inside the derivations.

The `binstall-packaging` job also declares no ceiling. It invokes no coverage
step, so it is outside this contract, and bounding it is separate work.

[shared-actions-coverage]: https://github.com/leynos/shared-actions/blob/main/.github/actions/generate-coverage/README.md

## Runner placement

The Linux leg of `ci.yml`'s `build-test` runs on Ubicloud's
`ubicloud-standard-4`. On GitHub-hosted runners, its queue wait over the ten
runs before the move had a 10-min median and reached 29 min, against a 27-min
median wall. That made it the worst hosted-queue case in the estate.

The size is set for disk, not for wall time. `ubicloud-standard-2` is the
estate's starting shape, but on it this leg ran out of disk in both measured
runs (36308332712 and 36354295154). The second run began lint with 6,869 MB
free. After lint 5,297 MB remained, and the discard below freed about 1 GB. The
first coverage pass then took it down to 791 MB and failed with "No space left
on device". `ubicloud-standard-4` offers 150 GB, against 75 GB on `-2`.

A pull request from a fork cannot obtain an Ubicloud runner, so the leg falls
back to `ubuntu-latest` for forks. So does a pull request Dependabot opened:
hosted minutes cost nothing on a public repository, and Dependabot opens many
pull requests. The expression tests the pull request's author, not
`github.actor`, because the actor is whoever triggered the run, so a person
pushing to a Dependabot branch would flip the runner. A Dependabot pull request
therefore builds cold on the hosted pool:

```yaml
runs-on: >-
  ${{ (github.event.pull_request.head.repo.fork
  || github.event.pull_request.user.login == 'dependabot[bot]')
  && matrix.fork-runner || matrix.runner }}
```

Each matrix entry names its `platform`, its `runner` and its `fork-runner`. The
Windows leg names `windows-latest` for both. The job is named
`build-test (${{ matrix.platform }})`, so the required check contexts are
`build-test (linux)` and `build-test (windows)` whichever runner serves the
event. A name built from the runner would exist on only one of a fork's and a
branch's pull requests. The steps choose their platform through
`matrix.platform`, never through a runner label.

The Ubicloud leg exports the cache proxy's credentials before Setup Rust, which
starts the sccache server. That step is guarded on
`runner.environment == 'self-hosted'` because on a hosted runner there is no
proxy and the action fails closed. The Windows leg and the three packaging legs
stay GitHub-hosted.

Before coverage starts, the Linux leg runs `scripts/discard_build_trees.py`,
which removes the rustdoc and Clippy output (`target/debug`, `target/doc`) and
Whitaker's (`target/dylint`), prints each tree's size, and reports a tree that
was not there. It accepts only a single relative name for a direct child of the
target directory, and refuses one that is nested, absolute, escapes the target
or reaches its tree through a symlink, so a link such as
`debug -> llvm-cov-target` cannot delete another tree. A missing target
directory or a filesystem failure while resolving, examining, measuring or
removing a tree ends the step with a one-line diagnostic on standard error and
status 1. It keeps the registry and sccache. Trybuild's child builds land under
the coverage target directory, which trybuild takes from `cargo metadata`, so
they cannot be redirected independently. Removing them between the two coverage
passes would throw away the warm builds the second pass reuses. The leg prints
`df -BM /` at each boundary: before lint, before and after the discard, and
after each coverage pass.

[ADR-0009](adrs/adr-0009-linux-build-test-runner-placement.md) records the
decision.

`runner_placement_test.py` holds all of this. It reads the fallback by position
(the condition is exactly the fork test or the Dependabot author test), checks
each leg's runners, and checks that no fork reaches Ubicloud, that the names
are runner-independent, and that the job has a ceiling. It also checks the
credentials step's guard and position, that the packaging legs stay hosted, and
that the Linux leg discards the lint build trees before coverage.

## CodeScene coverage belongs to main

`coverage-main.yml` is the only workflow in this repository that runs a
CodeScene action. It runs on pushes to main, generates ratcheted coverage, and
uploads with `mode: upload`. No workflow serving pull requests names a
CodeScene action, invokes `cs-coverage`, or puts `CS_ACCESS_TOKEN` in reach of
any process.

This is the estate rule `main-owned-codescene-coverage`, and it is a policy
rather than a gap. A pull request from a fork cannot read the repository's
secrets, so the changed-line check on that lane was a silent skip for exactly
the contributions least likely to have been measured already. On a branch it
put a second tool on the critical path, and when this CodeScene project stopped
returning a gates configuration that tool failed every pull request here over a
defect in none of them.

What a pull-request lane keeps is the ratchet. `ci.yml` runs
`generate-coverage` with `with-ratchet` on the Linux leg
(`with-ratchet: ${{ matrix.ratchet }}`, true for the Linux matrix entry alone),
comparing against the baseline `coverage-main.yml` writes, which applies the
same "do not go backwards" gate from this repository's own history with no
token and no second tool.

`build-test` takes the default depth-1 checkout. It used to fetch full history
only so `cs-coverage check` could diff against the pull request's merge base,
and that gate has gone. At the pinned revision `generate-coverage` runs no git
command. The job's other git readers are `git ls-files` (mdtablefix `--git`,
and lading's lockfile guard) and `git status` (lading's working-tree guard),
and neither reads history. A future step that diffs against the merge base,
walks the log or describes a tag has to restore `fetch-depth` for itself.
`test_the_pull_request_lane_checks_out_shallowly` fails until that change also
updates the contract.

**The two lanes must be built the same way.** Both run two feature legs, paired
by the report each writes. The ratchet compares this commit's report against
that baseline, so the inputs deciding *what* is measured must agree across the
pair: the output path, the format and the feature selection.

That was wrong before this adoption. The pull-request leg compiled `metrics`
and the publisher's did not, so every line of the metrics facade read as newly
uncovered against a baseline that had never compiled it. The failure is silent,
because the ratchet reports a number either way: nothing distinguishes a fall
in coverage from two runs having built different code. The fix is on the
publisher rather than the lane, since the comment on that leg records why
`metrics` is there at all, namely that it is off by default and the facade
would otherwise never be compiled in CI.

**The publisher is guarded on the ref, and runs one at a time.** It declares no
`workflow_dispatch` today, so `github.ref` is always main and the guard changes
nothing now; it is there because a dispatch can be aimed at any branch and the
upload carries no ref, so adding one later would let a run from a feature
branch publish that branch's coverage as the trunk's. The contract holds the
guard as one `&&` term and refuses any `||`, because a substring match passes
`... && ref == main || dispatch`, which makes every conjunct optional. The
concurrency group queues a superseded run rather than cancelling it: two runs
racing would decide the baseline by which finished last, and a cancelled run
abandons both its upload and its baseline write, while a queued one publishes
later and the later push still wins. The group is exactly
`coverage-main-${{ github.ref }}`, keyed on the ref alone. A static group would
let a dispatch aimed at another branch, if one were ever allowed, displace a
pending main run and then skip its upload on the ref guard. A group keyed on
the event as well would let an older run upload after a newer one.

**The token reaches the upload as an input, and no `env` block holds it.** A
`codescene-token` step with no `if:` and no `env` runs exactly
`echo "available=${{ secrets.CS_ACCESS_TOKEN != '' }}" >> "$GITHUB_OUTPUT"`.
The expression is evaluated before the shell starts, so the step writes `true`
or `false` and no process ever holds the token. The upload is guarded on
exactly that output and the main ref, and passes the secret directly as
`access-token`. The upload action is composite and hands its step `env` to its
nested artefact-upload and cache steps, so the contract refuses the token in
every `env` block of the publisher. The old guard, `env.CS_ACCESS_TOKEN != ''`
on a step-level binding, also passed with the binding deleted, after which the
upload would skip forever. The contract therefore asserts the check step and
its command positively, and the shared CV-005 library names each of those
failures.

**The upload job runs in the `codescene` environment.** `coverage-upload`
declares `environment: codescene`, whose deployment policy admits `main` alone.
The token belongs there, but moving it is a pending owner step: until it moves,
`CS_ACCESS_TOKEN` is a repository secret, and the pull-request clauses above,
not the environment, keep it out of what a pull request can start. Once it
moves, the policy refuses every run whose ref is not `main`. It does not
inspect what a job executes: a `main` run that checked out and ran pull-request
code would still hold the secret, so no such job may exist. The publisher
declares no `workflow_dispatch`, so no dispatch is refused today; one added
later and aimed at a branch would be refused for the whole job. The shared
library holds the placement: every job invoking the uploader declares the
environment, no other job does, and no job a pull request can reach does. Names
are compared without case, as GitHub compares them, and a name computed by an
expression is refused because its placement cannot be proved. The pull-request
surface there also seeds from review and comment events, the merge queue,
`workflow_run` chains and pushes not confined to `main` or to tags, and it
follows `$/` calls as well as `./`, because an environment declaration is safe
only where no branch can start the job.

**No checksum input.** `installer-checksum` is rejected outright when non-empty
from the pinned uploader, and `archive-checksum` is not a rename of it: it
digests the action's CLI manifest archive, while the `CODESCENE_CLI_SHA256`
repository variable holds the installer script's digest. Carrying the old value
across under the new name fails every run. The action pins the CLI through its
own manifest now, which is what that variable stood in for; the variable is
unreferenced and can be removed from the repository's settings.

`make test-workflow-contracts` holds the shape by running
`cv005-contracts check`, the shared contract library in
`leynos/shared-actions`, from the full commit named by `CV005_CONTRACTS_REF` in
the Makefile; a fix to the rules is a pin bump. The library reads workflows
with a loader that refuses duplicate keys, follows the pull-request lane as a
closure through local calls and composite actions (a workflow declaring only
`workflow_call` runs on a pull request when a pull-request workflow calls it,
and `secrets: inherit` hands it the token), refuses the CodeScene action, the
CLI, the token and the `codescene.io` host anywhere a pull request reaches, and
drives every clause against breaching fixtures in its own suite, so this
repository keeps no copy of the readers. `workflow_reading.py` stays for the
runner-placement contract.

`.github/cv005.toml` holds the repository's parameters. Its two `[[pairing]]`
tables map each pull-request feature leg to the publisher leg that writes its
report; the broad leg ratchets in the Linux cell of the matrix, through
`with-ratchet: ${{ matrix.ratchet }}`, which the library reads in. Its
`[selection]` pins the publisher's exact inputs, so a change made to both lanes
at once is still a reviewed change. `coverage_lane_facts_test.py` keeps the two
facts the shared rules do not know: only the Linux leg ratchets, and the
pull-request checkout is shallow.

**The ratchet has to stay switched on.** Pairing the legs' selections proves
the comparison is fair, not that it happens. The library counts the broad leg
as the job's one ratchet in its declared Linux cell and refuses a job that
ratchets in none or in two, and `coverage_lane_facts_test.py` holds the matrix
`ratchet` value to true for Linux alone.

**Workflows are loaded strictly.** `workflow_reading.py`, kept for the
runner-placement contract, refuses a mapping that declares one key twice, since
PyYAML otherwise keeps the last value silently and a job declaring `runs-on`
twice would read as the half GitHub may not run. The library's loader does the
same for the CV-005 shape and exits 2 on a file it cannot read, rather than
passing.

## Command checklist

Run from repository root:

```bash
set -o pipefail; make check-fmt 2>&1 | tee /tmp/make-check-fmt.log
set -o pipefail; make lint 2>&1 | tee /tmp/make-lint.log
set -o pipefail; make markdownlint 2>&1 | tee /tmp/make-markdownlint.log
set -o pipefail; make test 2>&1 | tee /tmp/make-test.log
```

For targeted behavioural debugging:

```bash
cargo test -p ortho_config --tests
cargo test -p hello_world --tests --all-features
```

For a single `ortho_config` integration target, for example the subcommand or
clap-parsing suites described in
[Integration test targets](#integration-test-targets):

```bash
cargo test -p ortho_config --test subcommand
cargo test -p ortho_config --test clap_integration
```
