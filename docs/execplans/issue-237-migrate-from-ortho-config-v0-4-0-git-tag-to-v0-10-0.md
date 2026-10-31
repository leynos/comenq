# Migrate `comenqd` configuration to `ortho_config` v0.10.0

This ExecPlan (execution plan) is a living document. The sections `Constraints`,
`Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`, `Decision log`,
`Outcomes & retrospective`, `Conformance basis`, and `Verification plan` must
be kept up to date as work proceeds.

Status: COMPLETE

## Purpose / big picture

This repository loads the daemon's configuration through `ortho_config`
declared as a Git tag (`v0.4.0`, whose crate version resolves as `0.3.0`). That
tag is not a published release: it is a moving target pinned by a mutable
reference, it duplicates a dependency graph that upstream now maintains and
re-exports, and it keeps the workspace on an API generation two major versions
behind the published crate.

After this change the workspace builds against the crates.io release
`ortho_config` 0.10.0, with the same observable behaviour for an operator: the
same `/etc/comenqd/config.toml` path, the same `COMENQD_*` environment overlay,
the same command-line overrides, the same token-resolution rules, the same
error stream and exit status for a bad configuration, and the same test
coverage. Someone can see this working by running

```console
make check-fmt && make lint && make typecheck && make test
```

and observing every gate pass, and by running the daemon against a deliberately
broken configuration and observing the same diagnostic and non-zero exit status
as before the migration.

The migration is not a version-string edit. The two API generations differ in
ways that touch this repository's source: `load_config_file` now returns
`OrthoResult<Option<Figment>>` where `OrthoResult<T>` is
`Result<T, Arc<OrthoError>>`; `OrthoError` gained variants and is still
`#[non_exhaustive]`; the error-conversion helpers became traits; and the crate
now re-exports `figment`, `serde_json`, `toml`, `uncased`, `xdg`, and
`serde_saphyr`. Each of those facts is addressed explicitly below.

## Constraints

Hard invariants that must hold throughout implementation.

- **Configuration precedence must not change.** The effective order is
  built-in `serde` defaults < `/etc/comenqd/config.toml` (or the `--config`
  path) < `COMENQD_*` environment variables < command-line flags. The
  environment layer keeps the exact semantics of
  `figment::providers::Env::prefixed("COMENQD_").split("__")`: a case-
  insensitive `COMENQD_` prefix strip, `__` mapped to the Figment key
  separator, and `figment`'s own `Value::from_str` scalar parsing.
- **The configuration file is mandatory.** `Config::from_file`,
  `Config::load`, and `Config::load_with_tokens` treat an absent file as an
  error and never fall back to defaults. OrthoConfig v0.10.0's discovery treats
  an absent candidate as `Ok(None)`; that difference is why the hand-built
  loader stays.
- **Token resolution must be byte-identical in behaviour.** Selection order is
  CLI `--github-token` > `--github-token-file` > `github_token_files` pool >
  `github_token_file` > inline `github_token`; a CLI token clears both file
  settings; a CLI token file clears the pool; the 64 KiB bound, the
  leading-`${VAR}` expansion, duplicate-token rejection (which must not
  disclose token values), and the exact messages for an empty token, an
  unterminated placeholder and an unset placeholder all stay as they are. The
  single asserted message substring, `unterminated environment placeholder`, is
  produced by this repository, not by OrthoConfig.
- **The public shape of `comenqd::config::Config` must not change.**
  `tests/public_api.rs` constructs it with all nine fields as a struct literal
  and is compiler-checked; adding, removing, or renaming a field is out of
  scope.
- **Exit-status and stream contracts must not change.** `main.rs` uses
  `color_eyre`; a configuration failure must still propagate as an `Err` from
  `main`, printing to standard error and exiting non-zero. Errors are never
  written to standard output. `--help` and `--version` handling stays with
  `clap::Parser::parse()`, which exits 0 without returning.
- **Imported upstream documents stay byte-for-byte identical.** The twelve
  files listed in `docs/third-party-docs-provenance.md` are copies, stored
  beneath `docs/ortho-config/` at their upstream relative paths so their
  relative links resolve without editing them. Local migration notes go in this
  plan and in this repository's own documents; files under `docs/ortho-config/`
  must never be edited.
- **`ortho_config` default features stay enabled.**
  `default-features = false` does not build in v0.10.0
  (leynos/ortho-config#627), and `serde_json` must remain available.
- **No suppressions.** No new `#[allow]`, no `--ignore-rust-version`, no
  warning suppression, and no claim that an unavailable check passed. The
  repository denies `allow_attributes` and `allow_attributes_without_reason` in
  `[workspace.lints.clippy]`, and lints are silenced only as a last resort with
  a tightly scoped, reasoned `#[expect]`.
- **Compile admission is never bypassed.** `RUSTC_WRAPPER` and `RUSTDOC` are
  not unset or overridden, `rustc` is never invoked directly to skip the queue,
  and the `build-limits` supervisor and the managed sccache service are never
  signalled, stopped, restarted, or "recovered".
- **Other agents' processes are not killed, and the shared Cargo cache is not
  isolated.** Wait for the package-cache lock rather than working around it.
- **Gates run sequentially for this worktree**, and sub-agents do not run full
  repository gates; `scrutineer` is the exclusive gate-runner for this
  candidate. Formatting, format checking, lint, and tests are not overlapped.
- **`/tmp` is scratch, not a build target.** Logs and checkouts go there; build
  output stays in the worktree's `target/`.
- **Work is confined to this worktree.** No `cd` to the original repository
  root, and never a bare `git stash` / `git stash pop`, because the stash stack
  is shared with other sessions.
- **No merge, no release, no publishing.** The deliverable is a branch and a
  draft pull request.

If satisfying an objective requires violating one of these, stop and escalate
instead of working around it.

## Tolerances (exception triggers)

Thresholds that trigger escalation when breached.

- Scope: if the implementation needs to change more than 20 files or more than
  600 net lines, stop and escalate. The expected footprint is 8–10 files and
  well under 300 net lines.
- Interface: if a `pub` item's signature must change beyond swapping
  `Result<T, ortho_config::OrthoError>` for `ortho_config::OrthoResult<T>`,
  stop and escalate. That return-type change is required by the dependency, not
  chosen, and is recorded in the decision log.
- Dependencies: if a new external crate must be added, stop and escalate. The
  expected change is removing nothing, adding nothing, and repointing one
  existing requirement. Adopting `ortho_l10n` is explicitly not wanted: no code
  here uses the localization runtime without the facade.
- MSRV: if the build fails on the pinned `nightly-2025-07-22` for a reason
  attributable to the v0.10.0 floor, a dated-pin bump is authorized by the
  issue; record the old and new identities and the reason. If the pin must
  move, do not exceed a nightly the reproducer actually proves necessary.
- Iterations: if a single gate still fails after three fix-and-rerun attempts,
  stop and escalate with the captured log.
- Behaviour: if any baseline test must be deleted, weakened, or marked
  ignored to make the migration pass, stop and escalate. Adding a regression
  test is always in scope.
- Ambiguity: if the migration guide admits two readings that would produce
  materially different behaviour, stop and present the options.

## Risks

- Risk: `ortho_config` 0.10.0 pulls a substantially different transitive graph
  (thiserror 2, cap-std, camino, dirs, xdg, uncased, tracing, clap with the
  `string` feature, and `ortho_config_macros`), and that graph may raise
  effective minimum versions or add a `link`-conflicting crate. Severity:
  medium. Likelihood: medium. Mitigation: a targeted
  `cargo update -p ortho_config` once the manifest no longer names the Git
  source (see EP-M1 — `--precise 0.10.0` fails while the lockfile still does),
  then `cargo tree -i ortho_config` and `cargo tree -i figment` compared with
  the baseline trees recorded below. Any unrelated churn in `Cargo.lock` is
  explained commit-by-commit rather than accepted silently.
- Risk: two `figment` versions could end up in the graph, one from
  `ortho_config` and one direct, producing a type-identity mismatch on
  `Figment`, `figment::Error`, and `figment::providers::Env`. Severity: high.
  Likelihood: low, because both require `^0.10`. Mitigation: after the update,
  `cargo tree -i figment` must show exactly one `figment 0.10.19` node
  reachable from both, and the code imports `Env` through
  `ortho_config::figment` so that identity cannot drift.
- Risk: dropping the `#[expect(clippy::result_large_err)]` attributes could
  leave an unused `#[expect]`, and `unfulfilled_lint_expectations` is denied by
  the workspace lints. Severity: low. Likelihood: high if done carelessly.
  Mitigation: remove the attributes in the same edit that changes the return
  type; `make lint` proves the outcome.
- Risk: changing `Result<_, OrthoError>` to `OrthoResult<_>` breaks the `??`
  in `main.rs` or `.map_err(ortho_config::OrthoError::from)` call sites, because
  `Arc<OrthoError>` must still satisfy `Into<color_eyre::Report>`. Severity:
  low. Likelihood: low. Mitigation: the existing `??` already compiles against
  `Arc` today through `OrthoError`'s `Error` impl; the follow-up
  `make typecheck` proves it, and the planned `main.rs` test invokes the binary
  rather than reasoning about it.
- Risk: the behavioural `config.feature` suite mutates the process environment
  (`EnvVarGuard`) and is `@serial`. Adding process-environment mutation is
  forbidden by the issue, so new tier tests must not widen that surface.
  Severity: medium. Likelihood: medium. Mitigation: new coverage is written as
  in-process unit tests that pass explicit `CliArgs`, and every case whose
  subject is an environment value runs in an isolated child process with a
  controlled environment, which §5 names as the sanctioned alternative. That is
  not a matter of taste: `figment`'s `Env::iter` calls `std::env::vars_os()`
  directly, `Env::new()` is private, and no constructor or builder method
  accepts an environment source, so the merge-layer environment tier cannot be
  exercised in-process without mutating the test process. `ortho_config`'s
  `MapEnv` and `_with_sources` entry points serve *discovery*, not this
  repository's hand-built merge, so they are not a substitute. The earlier
  draft of this plan claimed the in-process `EnvVarGuard` route; that was wrong
  and the implementation was reworked.
- Risk: `figment`'s `Env` provider and `ortho_config`'s `CsvEnv` parse the same
  string differently, so a silent swap would change what
  `COMENQD_COOLDOWN_PERIOD_SECONDS=960.0` or `=960` means. Severity: high.
  Likelihood: low after this decision is recorded. Mitigation: keep `figment`'s
  `Env`, with the two concrete behavioural differences recorded in the decision
  log, and add a test pinning integer and boolean environment parsing.

## Progress

- [x] (2026-10-26T12:00Z) Resolved the upstream tag independently: annotated
      tag object `996e5a1c9f22ace4f55bf5dcc196d2c0524262bf`, peeled commit
      `38812c563e3953a490e3efc202609f37e21a0517`; both matched issue #237
      exactly.
- [x] (2026-10-26T12:05Z) Imported the five authoritative documents plus their
      six relative-link targets and one directory listing, from the peeled
      commit only, and verified every SHA-256 byte-for-byte.
- [x] (2026-10-26T12:10Z) Wrote `docs/third-party-docs-provenance.md` with the
      repository, tag, commit, original and destination paths, both checksum
      tables, the link mapping, and a reproduction recipe.
- [x] (2026-10-26T12:15Z) Read the v0.10.0, v0.9.0, v0.7.0 and v0.6.0
      migration guides and the CHANGELOG before touching manifests or tests.
- [x] (2026-10-26T12:20Z) Inventoried the workspace: six manifests, one
      lockfile, two Git-tag `ortho_config` declarations, one direct `figment`,
      one direct `serde_yaml`; zero `OrthoConfig` derives, zero
      `figment::Jail`, zero `cargo-orthohelp`, zero `[package.metadata
      .ortho_config]`, zero localizers or Fluent catalogues, zero
      `DocMetadata`/`AgentContext` literals.
- [x] (2026-10-26T12:25Z) Captured the baseline commit
      `a781258549c157e38dfaaf1482ccbc01f32d2ccd` and compiler identity
      `rustc 1.90.0-nightly (9748d87dc 2025-07-21)`, cargo
      `1.90.0-nightly (6833aa715 2025-07-13)`, LLVM 20.1.8, on the pin
      `nightly-2025-07-22`.
- [x] (2026-10-26T13:45Z) Created `docs/execplans/` and wrote this plan.
- [x] (2026-10-26T14:05Z) Fixed the mdtablefix drift in
      `docs/third-party-docs-provenance.md`; `make check-fmt` now reports
      "31 files left unchanged" and `make markdownlint` reports
      "0 issues in 0 files". The drift came from the untracked provenance
      document and ExecPlan added by this work, not from a pre-existing
      defect.
- [x] (2026-10-26T14:05Z) Collected the `scrutineer` baseline report
      (`aca35af63d9fe7b81`), which ran all six gates sequentially against
      `a781258` with zero tracked-file modifications. Results: `make fmt`,
      `make lint` (911s), `make typecheck` (478s), `make test` and
      `make nixie` pass; `make check-fmt` and `make markdownlint` failed only
      on the then-unformatted untracked Markdown added by this work.
      Baseline inventory:
      **481/481 nextest tests passed** (17 binaries; the cucumber binary is
      filtered out of nextest by profile, as designed), and **11 features /
      55 scenarios / 211 steps**, all passing, under `make test-cucumber`.
      Zero clippy warnings and zero `#[ignore]` attributes. The only cargo
      warning is the pre-existing unused `vendor/filetime` patch notice.
      Note: `make test`'s exit code was not recoverable from disk for the
      combined target, so the two halves were re-run with in-log status
      (`baseline-test-2` nextest exit 0 at 39s, `baseline-test-cucumber`
      exit 0 at 6s). Both green.
- [x] (2026-10-26T14:10Z) Revised the error-helper decision after reading
      `OrthoError::Merge`'s Display string at `error/types.rs:47`: it reads
      "Failed to merge CLI with configuration", which names a CLI merge this
      Figment does not perform. `map_err(OrthoError::from)` therefore stays.
- [x] (2026-10-26T14:09Z) **Milestone EP-M1 complete** (`9a975be`): repointed
      the dependency to `ortho_config = "0.10.0"` from crates.io, removed the
      direct `figment` entry from `crates/comenqd/Cargo.toml`, and refreshed
      `Cargo.lock`. `serde_yaml = "0.9"` was deliberately retained: it has no
      relationship to `ortho_config` and issue #237 does not ask for its
      removal. Plain `cargo update -p ortho_config` was required: the lockfile
      still keyed the package to the Git source, so `--precise 0.10.0` failed
      with `revspec '0.10.0' not found`. Graph proof: one `ortho_config v0.10.0`
      from the registry index, zero `source = "git+` entries, one
      `figment 0.10.19` reachable from both `comenqd` and `ortho_config`.
- [x] (2026-10-26T14:09Z) **Milestone EP-M2 complete** (`44cba89`): migrated
      `crates/comenqd/src/config.rs`, `config/single_token.rs` and
      `config/token_resolution.rs` to `OrthoResult`, `ortho_config::figment`,
      `Arc<OrthoError>`, and v0.10.0's `load_config_file`. All five
      `#[expect(clippy::result_large_err, …)]` attributes in those files were
      removed because the error is now pointer-sized; the unrelated Octocrab
      one in `worker/tokens.rs` remains. `cargo check --workspace
      --all-targets --all-features` passes.
- [x] (2026-10-26T15:20Z) **Milestone EP-M3 complete**: migrated
      `tests/steps/config_steps.rs` to `ortho_config::OrthoResult<Config>`
      (its `ConfigWorld` field type), added
      `crates/comenqd/src/config/tests/precedence.rs` (a precedence table plus
      the restated-default defect; the final case count is recorded in the
      later rework entries, which moved every environment case out), added
      `error_when_path_is_a_directory`
      to `config/tests/configuration.rs` to pin v0.10.0's non-regular-file
      rejection, and added `crates/comenqd/tests/startup_contract.rs` (3 cases
      running the real binary to pin exit status and stream placement).
      Evidence: `cargo nextest run -p comenqd --all-features` — 53 configuration
      and contract tests pass, including all 9 precedence cases and all 3
      process-level cases; `cargo check --workspace --all-targets
      --all-features` is clean apart from the pre-existing unused
      `vendor/filetime` patch notice. Note: an earlier `git checkout --` used to
      keep a docs commit clean discarded this one-line change unnoticed, so
      the first compile attempt after compaction failed with
      `E0308` at `config_steps.rs:130`; re-applying the change fixed it. The
      lesson is that discarding a file to scope a commit needs a follow-up
      compile, not just a follow-up `git status`.
- [x] (2026-10-26T14:37Z) **First gate pass, at `a53f41c`** (`scrutineer`):
      all ten gates exited 0 — `fmt`, `check-fmt`, `lint` (368s), `typecheck`
      (235s), `test` (171s), `test-cucumber`, `markdownlint`, `spelling`,
      `nixie`, `test-workflow-contracts`. nextest 494/494 passed (baseline 481
      plus the 13 new tests); cucumber 11 features / 55 scenarios / 211 steps,
      identical to baseline; workflow contracts 198 passed; zero clippy
      warnings; `typos.toml` current. **Superseded**, not used as the final
      evidence: `f312bce` changed `precedence.rs` at 14:36:24, after `make lint`
      (14:29) and while `make test` was finishing, so the run did not cover the
      final tree. Recorded here because the baseline-to-migrated count
      comparison it establishes is still valid and still useful.
- [x] (2026-10-26T14:45Z) **Authoritative gate pass at `386142e`**
      (`scrutineer`): all ten gates exited 0 with HEAD verified identical and
      `git status --short` empty before the first gate, after `make fmt`,
      after `make spelling`, and after the last gate, so the run covers the
      complete final tree — the defect that invalidated the earlier run
      cannot apply here. Logs are
      `/tmp/rerun-<action>-issue-237-…-v0-10-0.out`, one per gate, written in
      strict sequence (fmt 14:41:56 → check-fmt 14:42:00 → lint 14:43:01 →
      typecheck 14:43:04 → test 14:43:46 → test-cucumber 14:44:08 →
      markdownlint 14:44:14 → spelling 14:44:18 → nixie 14:44:20 →
      test-workflow-contracts 14:44:29). nextest **494/494 passed, 0 skipped**
      (baseline 481, +13 new); cucumber **11 features / 55 scenarios /
      211 steps**, identical to baseline; workflow contracts **198 passed**;
      clippy and the Whitaker dylint both clean; markdownlint 30 files, zero
      issues; nixie "All diagrams validated successfully". Every `^warning:`
      line in every log is the pre-existing unused `vendor/filetime` patch
      notice; there were no `[build-limits]` drain faults and no refusal or
      connection error. Three "panic" grep matches are `PASS` lines for a test
      whose name contains `panicked`, not failures.
- [x] (2026-10-26T15:02Z) `coderabbit review --agent` over `origin/main`
      returned four findings, all `minor`, all against
      `docs/third-party-docs-provenance.md`, and all correct. Deduplicated they
      are two defects, both in the prose added by this work rather than in
      imported text. **(a)** The re-fetch recipe said
      `show 38812c56:examples/hello_world/<path>`, but every entry in the
      listing already starts with `examples/hello_world/`, so the prefix would
      be doubled and `git show` would fail. **(b)** The flat
      `docs/ortho-config-<name>.md` placement broke the imports' relative
      links; CodeRabbit named one example, and checking the whole set showed
      the damage was wider — of the eleven links out of `users-guide.md`, six
      missed outright and two more resolved *falsely* onto this repository's
      own `docs/users-guide.md` and `docs/README.md`. A mapping table cannot
      fix that, because Markdown resolves links by path arithmetic, not by
      documentation.
- [x] (2026-10-26T15:05Z) Fixed both findings by moving the imports to
      `docs/ortho-config/` at their **upstream relative paths** (`git mv`, so
      history follows), importing `examples/hello_world/README.md` so the
      directory link lands on a real page, and rewriting the provenance
      document to describe the mirrored layout, the reason for it, and the
      corrected re-fetch command. Twelve hashes re-verified after the move and
      again after `make fmt`: every one still matches the recorded value, so
      the byte-for-byte obligation survives. Every relative link out of the
      five authoritative documents now resolves. `make check-fmt` reports
      "32 files left unchanged"; `make markdownlint` reports 30 files, zero
      issues.
- [x] (2026-10-26T21:10Z) Second `coderabbit review --agent` over
      `origin/main` confirmed both round-one findings cleared and raised two
      new ones, both correct. **(a)** This ExecPlan narrated the work in the
      first person ("by my own", "my then-unformatted", "without my
      noticing", "both mine"). Rewritten as neutral descriptions of the
      work. **(b)**
      `missing_token_exits_non_zero_and_reports_on_stderr` asserted only a
      non-zero exit, an empty stdout and a non-empty stderr, so any unrelated
      startup failure — a missing socket path, a bad address — would have
      satisfied it. It now also requires stderr to contain
      `provide github_token, github_token_file, or github_token_files`, the
      literal message built in
      `crates/comenqd/src/config/token_resolution.rs:51-55` and rendered by
      `OrthoError::Validation` as
      `Validation failed for '{key}': {message}`. Evidence: the built binary
      against a token-less configuration prints
      `Error:\n   0: Validation failed for 'github_token': provide github_token,
      github_token_file, or github_token_files` on stderr, and
      `cargo nextest run -p comenqd --features test-support -E
      'binary(startup_contract)'` reports 3/3 passed. Non-vacuity control: the
      same binary against a missing `--config` path and against malformed TOML
      both print zero matches for that string, so the new assertion rejects
      precisely the failures it is meant to reject rather than passing on any
      non-empty stderr.
- [x] (2026-10-26T23:05Z) Third `coderabbit review --agent` cleared the
      child-wait finding and raised three more, all correct.
      **(a)** `error_when_path_is_a_directory` asserted only `res.is_err()`.
      Both the v0.3.0 `Ok(None)` path and the v0.10.0 directory rejection end
      in an error once `from_file`'s `ok_or_else` runs, so the test could not
      detect the very behaviour change it documents. It now asserts
      `io::ErrorKind::InvalidInput` via `file_error_kind`, which reaches the
      `io::Error` source inside `OrthoError::File`. **(b)** The missing-file
      and malformed-TOML process tests asserted only non-empty stderr, so a
      loader that treated either as absent configuration would have failed
      later for the missing token and left both tests green. They now require
      `Configuration file error in` plus, respectively, `not found` and
      `TOML parse error`. **(c)** Eight documents linked from the imported
      copies were not imported, so those links could not resolve. Imported
      and recorded; see the provenance document for the boundary argument.
      Mutation evidence for (a) and (b): replacing
      `ortho_config::load_config_file(path)?` with `None` — the old
      `Ok(None)` behaviour — made all three tests fail with exactly the
      intended messages, and reverting restored 4/4 green with
      `crates/comenqd/src/config.rs` byte-identical.
- [x] (2026-10-26T23:29Z) **Fourth gate pass, at `e49ad2b`** (`scrutineer`,
      ten gates, strictly sequential, one exclusive occupant of the worktree):
      every gate exited 0 with HEAD and the working tree byte-identical before
      and after. lint 226s, typecheck 11s, test 114s (nextest **494/494**, 0
      skipped), test-cucumber 8s (1 feature / 10 scenarios / 33 steps), just
      under `make test`'s own embedded cucumber totals of 11 features / 55
      scenarios / 211 steps across the whole workspace; markdownlint 39 files
      with 0 issues, `typos.toml` regenerated identically, nixie scanned 42
      Markdown files, workflow contracts 198 passed in 30s. The two previously
      repaired behaviours are pinned in the log: `error_when_path_is_a_directory`
      at 233/494 and all three startup-contract tests at 457, 458 and 461/494.
      Both non-failure conditions in the logs are the ones already recorded
      earlier — the unused `vendor/filetime` patch notice and normal
      `[build-limits]` queueing with the sccache server not serving, so builds
      ran uncached.
- [x] (2026-10-27T00:15Z) **Corrected an issue-§5 violation found by
      re-reading the branch diff.** The first implementation of the tier tests put
      eight new `EnvVarGuard` uses in
      `crates/comenqd/src/config/tests/precedence.rs`, and this plan's trace
      link claimed "no new process-environment mutation". That claim was false:
      `git diff origin/main...HEAD` showed eight added `EnvVarGuard::` lines,
      all in that one file. Investigation established that the merge-layer
      environment tier *cannot* be exercised in-process — `figment`'s
      `Env::iter` calls `std::env::vars_os()`, `Env::new()` is private, and
      `ortho_config`'s `MapEnv`/`_with_sources` inject only the discovery
      environment — so §5's own alternative, isolated child processes with
      controlled environments, is the only compliant route.
- [x] (2026-10-27T00:15Z) Reworked the tests accordingly.
      `crates/comenqd/src/config/tests/precedence.rs` now holds only the file
      and command-line tiers, with no environment mutation;
      `crates/comenqd/tests/config_environment.rs` (new) runs every
      environment case in a child that re-executes the test binary with
      `--exact` on one child body and a cleared, rebuilt environment. Dropped
      one redundant case (`array_literal_sequence_populates_the_pool`) after
      confirming `token_pool.rs::token_files_form_the_rotation_pool` already
      pins array-literal ordering from the file layer, and removed an
      `LD_LIBRARY_PATH` restoration that `ldd` proved unnecessary (the binary
      links only system libraries).
- [x] (2026-10-27T00:15Z) **Non-vacuity, measured with two seeded faults.**
      Deleting the `figment` merge line fails 5 of the 8 new cases — exactly
      the five environment ones — while the two file/default-tier cases and
      the child body itself still pass.
      Swapping `Env` for `ortho_config::CsvEnv` fails exactly one:
      `comma_separated_token_files_are_not_split`, the error-becomes-a-working-
      pool regression the obligation exists for. `config.rs` was restored
      byte-identical to HEAD (`git diff --quiet`) after each mutation.
- [x] (2026-10-27T01:40Z) **The first full gate run caught a host-dependent
      test.** `make test` failed at `precedence.rs:85` on
      `case_1_default_when_no_layer_sets_a_value`:
      `left: "/run/user/1000/comenq/comenq.sock"`, `right:
      "/run/comenq/comenq.sock"`. `Config`'s default socket path comes from
      `comenq_transport::default_socket_path()`, which honours the ambient
      `XDG_RUNTIME_DIR`, so a row asserting the bare `DEFAULT_SOCKET_PATH`
      constant passes only on a host that leaves that variable unset. The
      suggested repair was to reintroduce `EnvVarGuard::remove(...)` — the very
      mutation §5 forbids — so instead the default tier was removed from the
      in-process table and left to `config_environment.rs`, where the child's
      environment is genuinely cleared. With `file_value` now always present,
      `TierCase` lost its `Option` wrapper. Re-verified under the failing
      condition: `XDG_RUNTIME_DIR=/run/user/1000 cargo nextest run … --lib
      config::tests::precedence` gives 5/5 pass.
- [x] (2026-10-27T02:10Z) Re-measured every obligation claim against the
      reduced suite rather than carrying forward numbers from the superseded
      design. All three seeded faults were re-run and `config.rs` restored
      byte-identical after each: dropping the `socket_path` CLI override fails
      only `case_2_command_line_overrides_file` (`left: "/tmp/file.sock"`,
      `right: "/tmp/cli.sock"`); guarding the cooldown override with
      `if secs != 960` fails only
      `command_line_value_equal_to_the_default_still_wins` (`left: 10, right:
      960`); and deleting the `figment` merge line fails exactly the five
      environment child cases while the rest pass. The `CsvEnv` swap still
      fails exactly `comma_separated_token_files_are_not_split`. This pass also
      corrected two plan claims that had been asserted rather than measured —
      an earlier draft said the restated-default control failed with `left:
      20`, which stopped being true when its environment value moved into the
      child harness. The measured value is `10`.
- [x] (2026-10-27T03:05Z) Third `coderabbit review --agent --base origin/main`
      pass raised three findings, all environment-isolation defects of the same
      class the gate failure exposed, and all correct. **(a)**
      `startup_contract.rs` removed named `COMENQD_*` keys one at a time, so a
      variable added to `Config` later, or an invalid inherited value such as
      `COMENQD_RESTART_MIN_DELAY_MS=invalid`, could make the token-less fixture
      fail before the credential check. Replaced with `env_clear()` on the
      daemon child, which is safe because the binary links only system
      libraries. Proven: with that variable exported, the unfixed version fails
      `missing_token_exits_non_zero_and_reports_on_stderr` and the fixed one
      passes 3/3. **(b)** The in-process file-tier rows in `precedence.rs` call
      `Config::from_file`, which merges the real environment, so an exported
      `COMENQD_SOCKET_PATH` would beat the file value the row expects. **(c)**
      The same applies to the scalar cases. Both were resolved by restricting
      `precedence.rs` to the command-line tier, where `apply_cli_overrides` runs
      last and overwrites every lower layer, and leaving the file tier to
      `config_environment.rs`.
- [x] (2026-10-27T03:05Z) **Measured the residual rather than asserting
      immunity.** With six well-formed `COMENQD_*` variables plus
      `XDG_RUNTIME_DIR` exported, all five command-line cases pass — the
      override is genuinely last. With a *malformed* inherited value
      (`COMENQD_GITHUB_TOKEN_FILES=/tmp/a,/tmp/b`), `fig.extract()` fails before
      any override, so no tier is reached; ten of the twenty baseline tests in
      `config/tests/configuration.rs` fail under the same condition, which
      establishes this as a pre-existing property of the loader rather than
      something this branch introduced. Closing it would change production
      behaviour, not test structure. The child harness is immune to both cases
      (8/8 pass with the malformed variable set), so every tier still has an
      environment-proof pin.
- [x] (2026-10-27T04:20Z) **Re-measured both seeded faults once more after the
      CodeRabbit rework**, because restricting `precedence.rs` to the
      command-line tier changed the failure set. The 02:10Z entry above was
      measured against the superseded suite and is kept only as history; the
      current figures live in `INV-PRECEDENCE`'s non-vacuity paragraph.
      Dropping the `cli.socket_path` assignment now fails *both* table rows —
      `left: "/tmp/file.sock"` against `right: "/tmp/cli.sock"` and
      `right: "/run/comenq/comenq.sock"` respectively — with the other three
      cases green. `if secs != 960` fails exactly
      `command_line_cooldown_equal_to_the_default_still_wins` with `left: 10,
      right: 960`. `config.rs` restored byte-identical after each.
- [x] (2026-10-27T05:40Z) **Fifth gate pass, at `c6ce5e6`** (`scrutineer`, ten
      gates, strictly sequential): every gate exited 0, with `HEAD`, the
      working tree and the `git diff` SHA-256 all identical before and after.
      nextest **498/498**, 0 skipped; cucumber 11 features / 55 scenarios / 211
      steps; markdownlint 39 files, 0 issues; spelling regenerated
      `typos.toml` byte-identical; nixie 42 files; workflow contracts 198
      passed. The run added no new tests, so 498 is the same total as the
      fourth pass — the +17 over baseline is unchanged.
- [x] (2026-10-27T06:50Z) **Fourth `coderabbit review --agent --base
      origin/main` pass raised three findings, all correct. (a)** Both child
      harnesses clear the environment, which also removes `LLVM_PROFILE_FILE`.
      `cargo llvm-cov` exports that variable to place each instrumented
      process's profile, and `.github/workflows/coverage-main.yml` runs the
      suite that way, so a cleared child writes to the default location and
      its coverage is silently dropped. Measured: with the forwarding removed,
      `cargo llvm-cov nextest … binary(config_environment)` leaves exactly 7
      stray `default_*.profraw` files in `crates/comenqd/` — one per spawned
      child — and `binary(startup_contract)` leaves 3. With the forwarding
      restored both leave 0 outside `target/llvm-cov-target`, and no test
      verdict changes. **(b, c)** The plan's verification blocks piped each
      gate into `tee` without `pipefail`, so a failing gate's status would be
      replaced by `tee`'s and the sequence would continue; both blocks now
      `set -o pipefail` and chain with `&&`.
- [x] (2026-10-27T01:45Z) **Sixth gate pass, at `506d62d` — the delivered
      HEAD** (`scrutineer`, ten gates, strictly sequential): every gate exited
      0 with `HEAD` and the working tree verified identical before the first
      gate, after `make fmt` and after `make markdownlint` (the two mutating
      targets), and after the last. nextest **498/498**, 0 skipped; cucumber 11
      features / 55 scenarios / 211 steps both standalone and embedded in `make
      test`; markdownlint 39 files, 0 issues; spelling left `typos.toml`
      unchanged rather than regenerating it; nixie "All diagrams validated
      successfully"; workflow contracts 198 passed. Zero stray `*.profraw`
      files outside `target/`. This pass exists because `52bdeff` and
      `506d62d` both landed *after* the fifth pass at `c6ce5e6`, so the earlier
      run no longer covered the delivered tree: `52bdeff` changed Rust in both
      child harnesses, and the PR body's claim that ten gates validated the
      final tree would otherwise have been false.
- [x] (2026-10-27T01:50Z) **Milestone EP-M5 complete**: pushed over SSH with
      `GIT_SSH_COMMAND=/usr/bin/ssh`, refreshed the tracking ref with a leading
      `+` refspec, set the named upstream, and confirmed `git ls-remote`
      reported `506d62d758c658f976f2741a0b074e48423a7c24`, equal to
      `git rev-parse HEAD`, with `git log @{upstream}..HEAD` empty. Draft PR
      **#239**, <https://github.com/leynos/comenq/pull/239>, base `main`, head
      `issue-237-migrate-from-ortho-config-v0-4-0-git-tag-to-v0-10-0`,
      `isDraft: true`, title including `(#237)`, body containing `Closes #237`
      and the session reference. Not merged and not released, per the issue.
- [x] (2026-10-27T01:54Z) **Fifth `coderabbit review --agent --base
      origin/main` pass raised two findings, which are one real defect counted
      twice**: the retry instruction in Plan of work still said
      `vsleep "$(shuf -i 45-90 -n 1)"`. That is the very bug recorded under
      Surprises at 01:45Z, left in place there while being documented
      elsewhere — the entry explained the missing suffix without fixing the
      command a reader would copy. Both findings were therefore correct, and
      the instruction now passes the suffix explicitly and states why it is
      required. No finding touched the migration itself; the review ran over
      the full 33-file change set and returned nothing about the source, the
      tests, the manifests, or the imported documents. The preceding
      four findings (two documentation corrections, then the
      `LLVM_PROFILE_FILE` forwarding and two `pipefail` defects) were all
      cleared in `506d62d` and `52bdeff`. This pass also confirmed the value of
      sleeping a *real* 58 minutes: the same command that had been returning
      `rate_limit` for hours ran on the first attempt afterwards.
- [x] (2026-10-27T07:16Z) **Sixth `coderabbit review --agent --base origin/main`
      pass: clean.** `status: review_completed`, `findings: 0`, `outcome:
      completed`, 35 files reviewed, and zero `finding` or `error` events in
      the raw stream — verified by parsing the JSON events individually rather
      than by grepping a summary line, because the earlier harness bug had
      already produced one false "findings=0". This clears the `guide-policy-ci`
      erratum finding from the fifth pass, and with it every concern raised
      across all six passes: two documentation corrections, the
      `LLVM_PROFILE_FILE` forwarding, two `pipefail` defects, the bare
      `vsleep` instruction, and the upstream erratum. Nothing raised against
      the migration itself was ever outstanding.
- [x] (2026-10-27T09:05Z) **Delivered tree.** The branch was pushed over SSH and
      confirmed against the remote after a `+`-prefixed tracking-ref refresh,
      with `git ls-remote` matching `git rev-parse HEAD` on each occasion. This
      entry deliberately does **not** restate the tip SHA, because writing it
      down changes it: four successive bookkeeping commits each recorded "the
      delivered head is X" and each thereby made X stale, which is a fixed point
      the plan cannot reach and should not chase. What is stable is stated
      instead. The tree that the ten deterministic gates validated is
      `506d62d`, and nothing after it changes code, tests or behaviour: every
      later commit edits only this ExecPlan or
      `docs/third-party-docs-provenance.md`, and each was gated with the five
      Markdown targets alone (`fmt`, `check-fmt`, `markdownlint`, `spelling`,
      `nixie`). The branch is 1 commit behind `origin/main`: `43b11a1` touches
      five `.github/workflows/*.yml` files, overlaps this branch in no file at
      all, and `git merge-tree` reports no conflict. It was deliberately not
      merged, both because the change is unrelated to this issue and because
      merging it would restart the remote CI. PR #239 remains a draft with
      `Closes #237` and the session reference; not merged, not released.
      **Superseded** by the 2026-10-27T08:27Z entry below: the decision not to
      rebase was later overridden by an explicit instruction to do so, and the
      reasoning recorded here — unrelated change, restarted CI — remains the
      trade-off that instruction accepted knowingly.
- [x] (2026-10-27T09:52Z) **Seventh and eighth CodeRabbit passes, on the
      documentation corrections.** The seventh raised one finding, and it was
      right: the v0.9.0 applicability row claimed this repository constructs
      `OrthoError::Merge`, while the decision log three lines earlier explains
      why `into_ortho_merge` is deliberately not adopted. Counting the
      construction sites showed the row was wrong in a second way too — the
      variants actually constructed are `File` twice and `Validation` three
      times, with `Gathering` arriving by conversion from
      `fig.extract().map_err(OrthoError::from)`; `Merge` appears nowhere in
      `crates/`. The row is corrected and `Merge` is stated as never
      constructed. The eighth pass, run after that fix, returned
      `review_completed` with `findings: 0`, verified by parsing the events
      individually (zero `finding` and zero `error` events across 35 files).
      Eight passes in total, and every finding across all of them has been
      cleared: two documentation corrections, the `LLVM_PROFILE_FILE`
      forwarding, two `pipefail` defects, the bare `vsleep` instruction, the
      upstream erratum, and this inventory.
- [x] (2026-10-27T09:45Z) **A count error in an amended commit message, caught
      before it was pushed.** The message first said `File` had five
      construction sites and `Validation` four, because a raw `grep` count had
      included matches inside doc comments (`token_resolution.rs:84` names
      `OrthoError::Validation` in prose). Filtering to actual `Arc::new(...)`
      construction gave `File` ×2 and `Validation` ×3, and the message was
      amended before the push, so the erroneous figure never left this machine.
      This is the same failure as the two in the 09:40Z entry and happened
      within minutes of writing the lesson: the habit has to be to derive the
      number at the moment of writing it, not to intend to check it later.
- [x] (2026-10-27T09:40Z) **Two documentation inaccuracies found by re-deriving
      the PR body's numbers instead of trusting them**, both in text written
      earlier in this task and both the same species as the raised-default
      vacuity the plan already records: a claim that was never checked against
      the artefact it described. The body said a single `fn` line was removed,
      where `git diff` shows six removed and six added, pairing one for one as
      `load`, `load_with_tokens`, `from_file`, `from_file_with_cli`,
      `read_github_token_file` and `expand_env_prefix` each change only its
      return type. It also described all eight `#[test]`s in
      `config_environment.rs` as tier cases, where the eighth is the child body
      the parent re-enters with `--exact` and is a no-op in an ordinary run.
      Neither changed a verdict: the +17 total and every gate result stand,
      because nextest counts the child body. What was wrong was the account of
      *why* the numbers are what they are, which is exactly what a reviewer
      uses to decide whether to trust the rest. Both are corrected in the PR
      body, and the tier-case wording is corrected here in `INV-PRECEDENCE`.
      Lesson: a count that appears in prose is a claim; derive it from the
      diff or the log every time it is restated, and read the test files rather
      than the test names. The same discipline applies to hashes. Recording
      "the delivered head is X" is self-refuting when the record is itself a
      commit, so the entry above names the tree the gates validated instead and
      lets `git log` answer for the tip.
- [x] (2026-10-27T06:30Z) **CodeRabbit verification is quota-bound, and the
      retry harness had two defects of its own.** After the errata commit the
      review still could not run. Three separate problems were found and
      fixed, each by measurement rather than assumption. **(a)** The detection
      was wrong: a rate limit arrives as `{"type":"error","errorType":
      "rate_limit"}`, not `{"type":"rate_limit"}`, so a loop grepping for the
      latter recorded a rate-limited run as "REVIEW RAN, findings=0" — a
      false clean verdict on a review that never happened. **(b)** `vsleep`
      can return early under host load (load average ~13 on 6 cores while
      several agents run gates), and an unguarded script then retries
      immediately; the harness now computes an absolute target and keeps
      sleeping until the wall clock passes it. **(c)** A background task is
      killed when it reaches its time limit, which silently truncated one
      round mid-wait. With all three fixed, a real 69-minute wait still
      returned `waitTime: 8m27s`, which is the shared, contended pool and not
      a defect in this branch; the retry continues. The lesson worth keeping
      is that a *harness* which reports success without exercising the thing
      it measures is the same failure this plan spends its Verification
      section guarding against.
- [x] (2026-10-27T03:31Z) **Remote CI green on `9ea79e5`.** `build-test` passed
      in 15m43s, alongside four cross-architecture release builds
      (`comenq`/`comenqd` for `x86_64` and `aarch64`) and `release / metadata`,
      and `gh pr view --json mergeStateStatus` reports `CLEAN`. This matters
      beyond corroborating the local gates: the release builds compile the
      crate for two targets this machine does not run, so the dependency
      repoint is confirmed against the registry source on a clean CI
      toolchain, not only against the local Cargo cache. `CodeRabbit`,
      `Kody Code Review` and `automerge` show `skipping` because the pull
      request is a draft, and `Gecko Security Review` passed.
- [x] (2026-10-26T15:40Z) Checked `docs/comenq-design.md:905-918`, which states
      that configuration merges `/etc/comenqd/config.toml`, `COMENQD_*`
      environment variables and CLI arguments, with CLI highest, then
      environment, then file, then defaults; and that the token pool outranks
      the single-token settings. Every clause still holds after the migration,
      and the section names no version, so it needs no edit. `serde_yaml`
      retention was confirmed separately: its only users are
      `test-support/src/workflow.rs` and the release and packaging BDD steps,
      which parse GitHub workflow YAML and have nothing to do with
      `ortho_config`.
- [x] (2026-10-27T08:35Z) **Rebased onto `origin/main`.** This supersedes the
      09:05Z decision to leave the branch one commit behind: an explicit
      instruction to rebase overrode it, knowingly accepting the two costs
      recorded there (an unrelated change entering the history, and the remote
      CI restarting). The boundary was established before replaying anything.
      `git merge-base origin/main HEAD` was `a781258`, which is exactly the
      parent of the single incoming commit `43b11a1` ("Bump the shared-actions
      group with 12 updates (#238)"), so `OLD_BASE` is `a781258`, the replay
      range is its 36 linear commits, and `git rev-list --merges` reports zero
      merges — this is the plain linear workflow, not a squash restack. A
      `git merge-tree --write-tree HEAD origin/main` pre-check exited 0 with
      tree `ebebf0d8`, so the conflict surface was predicted to be empty.
      Recovery refs were created first under
      `refs/backup/237-pre-rebase-{old-head,old-base,target}-20261027T082721Z`.
      The **Weave merge driver was bypassed deliberately**. `git check-attr`
      reported `merge: weave` for `*.rs`, but the rule lives only in
      `~/.config/git/attributes` (`*.rs merge=weave` under an
      "ANSIBLE MANAGED BLOCK - weave user-wide enrolment"), not in any tracked
      `.gitattributes` — this repository has none, and no `.git/info/attributes`
      either. That is ambient global configuration, not repository consent, and
      the weave skill's unattended rule is to bypass it for a multi-commit
      replay: the rebase ran with `-c core.attributesFile=/dev/null`. Had a
      conflict needed `--continue`, the same `-c` override would have been
      repeated on every continuation, because Git re-reads driver configuration
      per replayed commit. Result: 36/36 commits replayed, zero conflicts, and
      `range-diff` reports
      every one of the 36 as `=` — identical patches, nothing dropped, nothing
      collapsed to empty, nothing changed. The stronger check is that the final
      tree `ebebf0d8` is **byte-identical to the `merge-tree` result computed
      before the rebase**, which is an independent confirmation that replaying
      the series produces exactly what a clean merge would have. The tree delta
      against the old head is exactly the five `.github/workflows/*.yml` files,
      and those are byte-identical to `origin/main`'s. Neither side touched
      `Cargo.lock` or any manifest, so no lockfile resolution was needed and
      the deliberate `ortho_config` 0.10.0 repoint survives untouched. In
      `origin/main...HEAD` the workflow files drop out entirely, leaving 35
      changed files — the same 35 the CodeRabbit passes reviewed. Two details
      recorded so a later reader is not misled. First, the rebase
      rewrote every **committer** date to the replay instant (author dates are
      preserved), so `git log --format=%cd` now shows one timestamp for the
      whole series; `%aI` still gives the real authoring times. Second, the
      `Z`-suffixed timestamps throughout this plan are **local-clock readings**
      labelled as UTC, not true UTC: at this moment `date -u` reads
      `08:28:38Z` while a bare `date` reads `09:28:38+0100`, and the tip
      commit's committer epoch `1793089645` agrees with `date -u`, so the
      commits are right and this document's suffix is the thing that is off by
      an hour. The timestamps are kept as written rather than rewritten, because
      their ordering and spacing are what carry meaning; the offset is noted
      here instead. **Gates.** The four targets the rebase instruction names
      were run sequentially against the rebased tree, then the three Markdown
      targets that cover the only edited file: `make check-fmt` pass
      (`40 files left unchanged`), `make test` pass (nextest **498/498**, 0
      skipped, across 19 binaries; cucumber **11 features / 55 scenarios / 211
      steps**, summed across all eleven chunks), `make typecheck` pass,
      `make lint` pass (zero clippy warnings, Whitaker dylint clean),
      `make markdownlint` pass (39 files, 0 issues), `make spelling` pass
      (`typos.toml` current), `make nixie` pass. The totals are identical to
      the pre-rebase run, which is the point: the replay changed no patch, so
      a changed total would have indicated a bad resolution. No gate modified a
      tracked file.

- [x] (2026-10-31) **Babysitting: cleared the one introduced CodeScene
      finding.** This repository has no `.codescene/` configuration and no
      hosted `CodeScene Code Health Review` check; its only CodeScene wiring is
      a coverage *upload* on `main` (`coverage-main.yml`), and `ci.yml` states
      in a comment that pull requests "neither publish a report nor contact
      CodeScene". The gap in hosted coverage was therefore closed locally with
      the installed CLI (`cs version 1.0.52 (efcaeed)`), using the same
      base/head comparison: `cs delta 43b11a1 7a92f36` — the PR's actual base
      rather than a movable `origin/main`, which had advanced to `2e2f1be`
      (#241, a CI pin touching only `.github/workflows/ci.yml`, a path this PR
      does not modify; `git merge-tree` against the new main reported no
      textual conflict, which says nothing about merged-tree behaviour).
      The delta reported exactly one finding, `Duplicated Assertion Blocks`,
      `introduced`, `new-score` 9.68, in
      `crates/comenqd/tests/startup_contract.rs` — a file this PR adds. The
      threshold is 2 and three tests tripped it
      (`missing_configuration_…`, `malformed_configuration_…`,
      `missing_token_…`), each repeating the same four-assertion contract:
      non-zero exit, empty stdout, non-empty stderr, plus one case-specific
      diagnostic. Section 4 of the babysitting skill puts refactoring before
      every other remedy, and `AGENTS.md` independently asks for `rstest`
      parameterized cases in place of duplicated tests, so the repair was a
      decomposition rather than a suppression: the shared exit-status and
      stream assertions became `assert_startup_failure(output, what,
      must_contain)`, and the three bodies became one
      `#[rstest]` function with `#[case::missing_configuration]`,
      `#[case::malformed_configuration]` and `#[case::missing_token]` rows
      carrying their own `must_contain` needles. Nothing was weakened. The
      diagnostic needles are unchanged verbatim (`"Configuration file error in"`
      plus `"not found"` / `"TOML parse error"`; and
      `"provide github_token, github_token_file, or github_token_files"`), and
      the per-case `what` string keeps each failure message specific
      ("a missing configuration", "malformed TOML", "a token-less
      configuration") rather than collapsing to one generic phrase. The three
      case names remain distinct in test output, so the contract each one pins
      is still identifiable. Result: `cs delta 43b11a1` now emits `[]` and
      `cs check` reports `Code health score: 10.00` for the file.
      **Gates**, run sequentially against this tree: `make fmt` pass (0 issues),
      `make check-fmt` pass (40 files unchanged), `make test` pass (nextest
      **498/498**, 0 skipped, across 19 binaries; cucumber **11 features / 55
      scenarios / 211 steps**), `make typecheck` pass, `make lint` pass,
      `make markdownlint` pass, `make spelling` pass, `make nixie` pass. The
      nextest total is the same 498 as the pre-refactor run and cucumber's
      11/55/211 are unchanged, which is the evidence that the parameterization
      neither dropped nor duplicated a case; the three renamed cases appear in
      the output as `case_1_missing_configuration`,
      `case_2_malformed_configuration` and `case_3_missing_token`, all passing.
      `make lint` reported no clippy warning; its only warnings in 34 minutes of
      output were the three instances of the pre-existing
      `vendor/filetime` patch notice already documented in the PR body. Long
      durations (lint 2808 s, typecheck 1566 s, test 1857 s) were cold-cache
      builds behind a heavily contended compile queue; `build-limits-status`
      reported `capacity 4, held 4, healthy` throughout, so this was ordinary
      queueing, not a hung or unhealthy gate.

- [x] (2026-10-31) **Babysitting: renewed the candidate-bound assessment for
      the repaired head.** The prior clearance,
      <https://github.com/leynos/comenq/pull/239#issuecomment-6470567550>,
      named head `7a92f36` and therefore did not cover `9961ed4`; the
      assessment reference requires renewed candidate-bound evidence after a
      repair. Following it, a follow-up question was posted through the
      manual-comment route (pool identity `pandalump`, deliberately not the
      lifecycle account, per the comment-identity contract) as
      <https://github.com/leynos/comenq/pull/239#issuecomment-6544126165>,
      carrying `Assessment-ID:
      comenq-239-ortho-config-0100-followup-9961ed4` and naming base
      `43b11a1` and head `9961ed4`. It linked the original request and
      response, described the CodeScene finding and the `rstest` repair,
      listed the eight gate results with the 498/498 and 11/55/211 totals, and
      recorded the three known limitations (Sourcery's 150,000-character
      review-limit decline, the absence of hosted CodeScene analysis, and
      `origin/main` having advanced to `01a7128` with no overlap between the
      two changed-file sets — which bounds direct conflict risk but is not
      evidence about interaction or merged-tree behaviour).
      The substantive reply arrived 83 seconds later as
      <https://github.com/leynos/comenq/pull/239#issuecomment-6544141609>: it
      echoed the Assessment-ID and both SHAs, confirmed exactly one commit
      after `7a92f36` touching only `startup_contract.rs` and this plan, and
      assessed the parameterization as behaviour-preserving on the strength of
      the source cases themselves rather than the reported totals. Its
      conformance table cleared all eight obligations (published dependency
      and graph, precedence, error-API migration, token behaviour, regression
      coverage, test isolation and coverage collection, documentation
      provenance, and optional capabilities and toolchain). Two caveats were
      stated rather than glossed: it did not independently re-run builds,
      tests, or CodeScene, and it assessed the requested base/head rather than
      a tree merged with newer `main`. Neither is an in-scope defect. The
      first is covered by the gate evidence above. The second is *not*
      discharged by the empty file-set intersection with `01a7128`: that
      intersection reduces direct conflict risk but proves nothing about
      interaction between the two change sets, so integration with the newer
      `main` stays marked unverified until a merged tree is validated. This
      bullet is a summary with references, not a verbatim reproduction of the
      conversation; the full bodies are preserved under the evidence directory
      and at the two URLs above.

- [x] (2026-10-31) **Babysitting: recorded proof scrutiny as not applicable.**
      The assessment reference requires an explicit satisfactory assessment
      only for proofs the change introduces or materially alters, and asks for
      a justified `not applicable` otherwise. This diff introduces none. It
      adds no `kani`, `verus`, `crosshair`, or `lemmascript` artefact
      (`git ls-files` matches nothing for any of those names), and it adds no
      property test: `proptest` is already a workspace dependency and is used
      elsewhere, but the PR's 669 added test lines are `rstest` cases,
      integration tests over a child process, and one behavioural step —
      example-based coverage, not proofs. The `rstest` additions are wider than
      the `9961ed4` refactor alone: besides the parameterized startup-contract
      cases, the PR adds the two-row precedence table
      `socket_path_resolves_to_the_highest_present_tier` (with rows
      `command_line_overrides_a_differing_file_value` and
      `command_line_value_equal_to_the_built_in_default_still_wins`), and
      further `rstest` tables in `config/tests/configuration.rs`. All of them
      are example-based specification, not proofs.
      The startup-contract cases in particular are *preservation* tests: the
      behaviour is meant to be unchanged, so they pass before and after the
      migration, and the case-specific diagnostic needles serve to distinguish
      the intended startup failure from an unrelated later failure rather than
      to demonstrate a red-then-green transition. The non-vacuity evidence for
      this suite is the recorded seeded-fault and mutation checks
      (`INV-RESTATED-DEFAULT` and the two seeded faults), not the needles.
      Posting an empty proof inventory would have been ceremonial, so no
      question was posted.

- [x] (2026-10-31) **Babysitting: dispositioned four documentation findings
      from the second assessment.** The plan-only commit moved the head to
      `c9849f6`, so a third question
      (<https://github.com/leynos/comenq/pull/239#issuecomment-6544173786>)
      asked whether the prior assessment still held. The reply
      (<https://github.com/leynos/comenq/pull/239#issuecomment-6544184832>)
      retained the implementation assessment and kept the proof-scrutiny
      `not applicable` conclusion, but raised four wording defects. All four
      were true and all four were fixed in the plan text:

  1. *The empty changed-file intersection was presented as covering the
     integration caveat.* Corrected. Disjoint file sets bound direct conflict
     risk; they say nothing about interaction between the two change sets, so
     integration with `01a7128` is now marked unverified until a merged tree is
     validated. The same overclaim appeared twice more and was corrected
     there too, at the `2e2f1be` note (a conflict-free `git merge-tree` proves
     no textual conflict, not merged-tree behaviour) and in the limitation
     summary.
  2. *"The only `rstest` additions are the parameterized startup-contract
     cases" was false for the whole PR.* Corrected. The PR also adds the
     two-row `socket_path_resolves_to_the_highest_present_tier` precedence
     table and further `rstest` tables in `config/tests/configuration.rs`; the
     bullet now names them rather than restricting the claim to the `9961ed4`
     refactor.
  3. *The diagnostic needles were said to discharge a red-then-green
     obligation.* Corrected. These are preservation tests for unchanged
     behaviour, so they are expected to pass before and after; the needles
     distinguish the intended startup failure from an unrelated later one.
     Non-vacuity rests on the recorded seeded-fault and mutation checks, and
     the bullet now says so.
  4. *The first bullet read as a verbatim reproduction.* Corrected. It is now
     labelled a summary with references, with the full bodies held in the
     evidence directory and at the two comment URLs.

     The reviewer explicitly authorized not re-running the compile-based gates
     for this prose-only delta and asked that their results stay attributed to
     `9961ed4`; that attribution is unchanged above. Documentation gates were
     re-run for the corrected text.

- [x] (2026-10-31) **Babysitting: both assessments clear for `af964c9`.** The
      correction reply
      (<https://github.com/leynos/comenq/pull/239#issuecomment-6544215563>)
      confirmed all four fixes by line and retained the implementation verdict
      and the proof-scrutiny `not applicable` conclusion. Its own scope check
      found exactly one commit after `c9849f6`, touching only this plan, and
      confirmed no implementation, test, dependency, workflow or
      imported-document file changed. Both prerequisites in the assessment
      reference are therefore discharged for the current candidate: the
      implementation assessment holds, and proof scrutiny is justified as not
      applicable. The outstanding conditions named in the replies are
      operational rather than textual — the hosted checks must conclude green
      on this head, and integration with newer `main` stays unverified until a
      merged tree is validated.

- [x] (2026-10-31) **Babysitting: assessment carried forward to `14b3573`.**
      A fourth question
      (<https://github.com/leynos/comenq/pull/239#issuecomment-6544231968>)
      asked whether the assessment transferred to the commit recording the
      clearance. The reply
      (<https://github.com/leynos/comenq/pull/239#issuecomment-6544238614>)
      carried the implementation assessment forward, found no defect in the
      appended bullet, and retained the proof-scrutiny `not applicable`
      conclusion. It read "both prerequisites discharged" as referring to the
      two assessment prerequisites, which is the intended meaning, and noted
      that the following sentence already preserves the operational
      conditions. Assessment coverage is therefore current for `14b3573`,
      which is the head intended for readiness and merge.

- [x] (2026-10-31) **Babysitting: formal review, seven errata, and merge.**
      The PR was marked ready, which caused CodeRabbit's formal review
      (`5723404161`, `CHANGES_REQUESTED`) to run. It returned seven findings
      and all seven were genuine. None could be fixed in place, because each
      sits in a byte-for-byte copy that issue #237 requires to stay unedited,
      so all seven were verified against the pinned bytes and recorded under
      `### Known upstream errata` in
      `docs/third-party-docs-provenance.md` at `945df03`. Two of CodeRabbit's
      suggested remedies were declined on evidence: a fabricated `design.md`
      stub would make the link resolve without reaching the section it names,
      and a bare `ExternalHelp` deletion note is less useful than naming the
      mechanism that actually works. CodeRabbit discarded the stub remedy
      itself, endorsed the second explanation, and independently recomputed
      all twenty recorded SHA-256 values. Integration with the advanced
      `main` (`01a7128`) was verified by building the real merge rather than
      arguing from a disjoint file set:
      `--no-ff --no-commit`, committed as `ef41660` in a temporary worktree.
      All eight gates passed there — `fmt`, `check-fmt`, `lint` (clippy and
      Whitaker), `test` (505/505 nextest, 11 features, 55 scenarios,
      211 steps), `typecheck`, `markdownlint`, `spelling`, `nixie`. The
      validated tree is preserved as
      `refs/backup/pr239-merged-tree-validated-ef41660`, so the evidence
      survives the worktree's removal. One caveat, recorded rather than
      smoothed over: the first `lint` attempt was killed by a harness
      background cap mid-compile, so its PASS rests on the completed re-run.
      The first approval request
      (<https://github.com/leynos/comenq/pull/239#issuecomment-6545975881>)
      was skipped with `Approval skipped: unresolved review threads`. That was
      an omission in this work, not a service fault: the findings had been
      dispositioned and accepted, but six of the seven inline threads were
      still open. The six were resolved through the review-thread API after
      re-verifying that all seven errata were present in the committed file,
      and the second request
      (<https://github.com/leynos/comenq/pull/239#issuecomment-6546000081>)
      produced a real approval review object (`5723747395`, `APPROVED`,
      16:22:06Z). Resolving those threads did not change a single byte: the
      defects remain in the preserved upstream text. The PR squash-merged as
      `b264e216f37d0b34d624c3212c33846cfbc79692` at
      2026-10-31T16:22:47Z, guarded by `--match-head-commit` against the
      accepted head `945df03`, with no administrator bypass. `Coverage (main)`
      is the only workflow a `main` push triggers here; `release.yml` fires
      only on version tags, so nothing was published. Every prior objection
      that had blocked the merge was either satisfied by evidence or, where
      the evidence showed the objection was mine to fix, corrected.

## Surprises & discoveries

- Observation: the repository never uses `#[derive(OrthoConfig)]`. Every
  configuration path is hand-built Figment layering plus a post-load token
  step. Evidence: `grep -rn "OrthoConfig" crates/ test-support/ src/` returns
  only the `ortho_config::` type paths in `config.rs`, `single_token.rs` and
  `token_resolution.rs`. Impact: the v0.10.0 guide's derive-focused sections
  (Fluent identifiers, `cli_default_as_absent`, profiles, behaviour metadata,
  stacked scopes) have no caller to change. This shrinks the applicable set
  considerably and is recorded row-by-row in the applicability matrix.

- Observation: `ortho_config` v0.3.0's `load_config_file` returned
  `Result<Option<Figment>, OrthoError>` and v0.10.0 returns
  `OrthoResult<Option<Figment>>` = `Result<Option<Figment>, Arc<OrthoError>>`,
  but the `Ok(None)`-for-absent-file contract is unchanged. Evidence:
  `git show 5c27310a:ortho_config/src/file.rs` versus
  `ortho_config/src/file/loader.rs` at `38812c56`. Impact:
  `ortho_config::load_config_file(path)?` keeps working unchanged; only the
  error arm's type moves, and the repository's own `ok_or_else` that turns
  `None` into a `NotFound` error must build an `Arc<OrthoError>`.

- Observation: v0.10.0's file loader rejects a non-regular file (a directory, a
  FIFO) with `InvalidInput` where v0.3.0 silently returned `Ok(None)` for any
  path that was not `is_file()`. Evidence: `file_exists_and_is_regular` in
  `ortho_config/src/file/loader.rs`. Impact: strictly better and still an
  error, so the existing `error_when_missing_file` test keeps passing; no test
  asserts the old silent behaviour. Worth noting because it is a genuine
  behaviour change that is nevertheless compatible with every documented
  contract here.

- Observation: v0.10.0 also gained `extends` chain support, cycle detection
  (`OrthoError::CyclicExtends`), and canonical-path resolution inside
  `load_config_file`. Evidence: `with_cycle_detection` and `process_extends` in
  `ortho_config/src/file/loader.rs`. Impact: no `extends` key exists in any
  fixture here, so `process_extends` is a no-op on this repository's files. It
  does mean a malformed `extends` would now error where v0.3.0 ignored the key;
  that is acceptable and undocumented in this repository's contract.

- Observation: `OrthoError::Merge` and `OrthoError::Gathering` both hold
  `Box<figment::Error>`, and `From<figment::Error> for OrthoError` produces
  `Gathering`, while `OrthoMergeExt::into_ortho_merge` produces `Merge`, whose
  Display text is `Failed to merge CLI with configuration: {source}`. Evidence:
  `ortho_config/src/error/conversions.rs`, `result_ext.rs`, and
  `error/types.rs:47`. Impact: the two helpers are not interchangeable for this
  repository. `Merge` names a CLI merge that never happens in this Figment, so
  the existing `map_err(OrthoError::from)` stays; see the decision log.

- Observation: `Arc<E>` where `E: Error + Send + Sync + 'static` itself
  satisfies `Error + Send + Sync`, so it coerces to `&dyn Error` and can be
  boxed into `OrthoError::File { source }`, and `color_eyre::eyre::Context`
  accepts it. Evidence: standalone `rustc` compile of a minimal reproduction,
  exit 0. Impact: the `?` propagation in `main.rs` and the wrapping in
  `single_token.rs` keep compiling without an adapter.

- Observation: Figment's `Env` does **not** split a comma-bearing value, and a
  `Vec<PathBuf>` field therefore *rejects* `COMENQD_GITHUB_TOKEN_FILES=/a,/b`
  rather than reading it as two paths. Evidence: a first draft of the comma
  test asserted a one-element vector and failed with
  `InvalidType(Str("/tmp/…/first.token,/tmp/…/second.token"), "a sequence")`
  from `Gathering`. `ortho_config::CsvEnv` enables comma splitting by default,
  so under `CsvEnv` the same input would have produced a two-token pool.
  Impact: this is a sharper argument for keeping `Env` than the scalar-parsing
  difference alone, because it changes a startup **error** into a working
  configuration. The regression test now pins the rejection, and a companion
  test pins the supported spelling — a TOML-style array literal, which Figment
  does parse into a sequence.

- Observation: the daemon's failure contract is exactly as required, measured
  on the real binary rather than inferred: a malformed config exits `1` with
  zero bytes on standard output and a `color_eyre` report on standard error; a
  missing file and a token-less file behave the same way; a valid config keeps
  running (observed as `timeout` reporting `124`), so the assertions are not
  vacuous. Evidence: `target/debug/comenqd --config /tmp/badcfg.toml` →
  `exit=1`, `stdout` 0 bytes.

- Observation: removing the direct `figment` dependency from
  `crates/comenqd/Cargo.toml` does not drop the `env` or `toml` features.
  Evidence: `ortho_config`'s registry manifest declares
  `features = ["env", "test"]` unconditionally on its `figment` dependency, and
  maps its default `toml` feature to `figment/toml`;
  `cargo tree -e features -i figment` after removal still lists both. Impact:
  the dependency is genuinely redundant and was removed rather than retained
  "for safety", which keeps the manifest honest.

- Observation: `vsleep` takes a *suffix* for its unit, and a bare number means
  **seconds**, not minutes. Evidence: `vsleep --help` prints
  `s seconds (default), m minutes, h hours, d days`. Impact: every rate-limit
  cooldown in this session written as `vsleep "$M"` for a 45–90 "minute" wait
  actually slept 45–90 **seconds**, which is why the shared CodeRabbit quota
  appeared never to recover: `waitTime` fell only from `2:42` to `0:18` across
  roughly three minutes of real elapsed time while the log claimed hours. The
  fix is `vsleep "${M}m"`. Recorded because the misleading log lines ("sleeping
  72 minutes" followed one minute later by another round) are otherwise
  impossible to interpret correctly on a later reading. The `waitTime` figures
  themselves are a shared, contended pool, so a freed slot can also be taken by
  another user between polls; neither cause is a defect in this branch.

- Observation: every `Z`-suffixed timestamp in this plan is a **local-clock
  reading**, not UTC, so the whole document is one hour ahead of the truth
  during BST. Evidence: at the moment of writing, `date -u` printed
  `2026-10-27T08:28:38Z` while a bare `date` printed `09:28:38+0100`; the tip
  commit's committer epoch `1793089645` equals the `date +%s` reading and
  `date -u`, so the commits carry correct timestamps and only this document's
  suffix is wrong. Impact: ordering and spacing between entries are unaffected,
  so the timestamps are left as written rather than mass-rewritten; the offset
  is now stated once here instead of silently misleading a later reader. The
  habit that would have avoided it is to print `date -u` rather than `date`
  when stamping a document that claims to be UTC.

- Observation: `git rebase` rewrites the **committer** date of every replayed
  commit to the replay instant while preserving the author date. Evidence:
  after the 36-commit replay, `git log --format=%cd` reports one identical
  timestamp across the whole series, while `%aI` still shows the original
  authoring times spread over two days. Impact: any future check that infers
  when the work happened must read the author date, not the committer date, or
  it will conclude the entire branch was written in the same second.

- Observation: this repository has **no hosted CodeScene code-health review**,
  so "the CodeScene check is green" is the wrong test of whether CodeScene
  issues are outstanding here. Evidence: `cs` and `codescene` appear in
  `Makefile`, `clippy.toml`, three docs files and two workflows, but the only
  hosted use is `upload-codescene-coverage` in `coverage-main.yml`, guarded on
  `github.ref == 'refs/heads/main'`; `ci.yml` says in as many words that a pull
  request "neither publish[es] a report nor contact[s] CodeScene". No
  `codescene-access[bot]` review or comment exists on PR #239, and no CodeScene
  check run appears among the eleven checks on head `7a92f36` — only
  `build-test`, four cross-architecture release builds, `release / metadata`,
  `Gecko Security Review`, and three skipped checks (`CodeRabbit`,
  `Kody Code Review`, `automerge`). Impact: the absence of a red gate is not
  evidence of absence of findings. Code health has to be established by running
  `cs delta BASE HEAD` locally. Doing so found one real introduced finding that
  no hosted check would ever have surfaced, which is the whole reason the skill
  requires the local run rather than trusting the check list.

- Observation: the repository's own `AGENTS.md` rule ("Replace duplicated tests
  with `#[rstest(...)]` parameterized cases") and the CodeScene
  `Duplicated Assertion Blocks` rule point at the same code, so the CodeScene
  repair needed no judgement call about which remedy to prefer. Evidence: the
  three startup-failure tests each opened with the identical four-assertion
  block before diverging only in their fixture and their expected diagnostic.
  Impact: the refactor satisfies both authorities at once and, because the
  case-specific diagnostic needles were preserved verbatim, it is a pure
  de-duplication rather than a change in what the tests assert.

## Decision log

- Decision: leave the `typos.toml` drift that `make spelling` regenerates out of
  this PR, and do not revert it into the branch as a "fix". Rationale:
  `make spelling` runs `typos-config-builder gate`, which silently rewrites
  `typos.toml` when the builder (pinned to the mutable Git tag `v0.1.3`, not to
  a commit) produces a different file from the one committed. The gate then
  exits **0**, so the refresh is invisible to hosted CI: CI ran green on this
  exact tree before any refresh. The drift is also entirely pre-existing and
  unrelated — `typos.toml` at `HEAD` is byte-identical to `origin/main`, and
  the tokens the refreshed file changes (`#[tokio::main(flavor = "local")]`
  versus a broad `\bflavor =` pattern) appear only in
  `crates/comenqd/tests/metrics.rs` and
  `docs/behavioural-testing-in-rust-with-cucumber.md`, neither of which this PR
  touches. `AGENTS.md` asks that such an unrelated regeneration be committed
  separately so "each review diff stays focused", and creating a stacked
  prerequisite PR for a cosmetic regex refresh was not warranted here. Verified
  instead that the committed `typos.toml` keeps the gate green and idempotent:
  reverting the working-tree refresh and re-running `make spelling` exits 0 with
  `current: typos.toml`, and running it again on the refreshed file also exits
  0. So both versions pass, and committing the drift would add review noise
  without changing any verdict.

- Decision: treat `cs delta` against the PR's own comparison base as the
  authority on CodeScene findings for this repository, and repair the one
  finding it reported by extracting a shared assertion helper rather than by
  suppressing the rule, raising its threshold, or adding a `@codescene`
  directive. Rationale: the hosted CodeScene surface for this repository is a
  coverage upload on `main` only, so no pull-request check could ever have
  reported this finding — there was no red gate to satisfy, only an actual
  duplication to remove. Suppression was available (the skill documents it as a
  last resort) but the finding was true: three tests did repeat an identical
  assertion block, which is exactly what `AGENTS.md` asks contributors to
  replace with `#[rstest]` parameterized cases. Choosing the refactor costs one
  level of indirection and keeps every original assertion, every original
  diagnostic needle, and every original case name in the test output.

- Decision: keep the hand-built Figment layering on the v0.10.0 APIs rather
  than converting `Config` to `#[derive(OrthoConfig)]`. Rationale: the derive
  cannot express this repository's policy without changing behaviour. (a) The
  configuration path is a mandatory positional input defaulting to
  `/etc/comenqd/config.toml`, and an absent file is a hard error; the derive's
  `load()` treats an absent candidate as `Ok(None)` and proceeds to defaults.
  (b) The CLI is parsed with `clap::Parser::parse()` with no `ArgMatches`
  retained, so the derive's match-aware loaders and `cli_default_as_absent` are
  unavailable. (c) Token resolution mutates the loaded `Config` afterwards
  (`resolve_github_token(&mut self)`), which a derive does not model. Issue
  #237 explicitly asks for this determination and authorizes keeping the
  hand-built layering when the derive does not express the same policy.
  Date/Author: 2026-10-26, migration agent.

- Decision: keep `figment`'s `Env` provider, imported as
  `ortho_config::figment::providers::Env`, rather than switching to
  `ortho_config::CsvEnv`. Rationale: `CsvEnv` is a different parser with
  different defaults. It enables `csv` by default, so a comma-separated value
  becomes a list where figment yields a string; it forces lowercase keys in
  `split()`; and its `parse_scalar` recognizes `true`/`false`
  case-insensitively and then falls back to `FromStr` and finally to a string,
  with no `f64`, `usize`, or `isize` branches. `figment`'s `Value::from_str`
  does the opposite: it parses `true`/`false`, a `-`-or-`u`-signed integer via
  `usize`/`isize`, and a float only when the text contains `.`. Swapping would
  silently change what `COMENQD_COOLDOWN_PERIOD_SECONDS=960` and a
  comma-containing value mean. Importing through the re-export keeps one
  `figment` identity and satisfies the v0.9.0 guide's "use the re-exports"
  advice. Date/Author: 2026-10-26, migration agent.

- Decision: **remove** the direct `figment` dependency from
  `crates/comenqd/Cargo.toml` (it declared
  `version = "0.10", default-features = false, features = ["env", "toml"]`).
  Rationale: v0.9.0 §9 says to keep a direct dependency only when application
  source imports that crate itself or needs a different feature set.
  `crates/comenqd/src/config.rs` previously imported `figment::providers::Env`
  directly; after EP-M2 it imports `ortho_config::figment::providers::Env`, so
  there is no direct import left. On the feature side, both features this
  dependency asked for are supplied unconditionally or by the default set:
  `ortho_config`'s registry manifest declares
  `[dependencies.figment] features = ["env", "test"], default-features = false`
  (so `env` is always on) and `toml = ["figment/toml", "dep:toml"]` in its
  default feature set, which issue #237 requires keeping. Verified empirically:
  after deleting the dependency,
  `cargo check --workspace --all-targets --all-features` succeeds and
  `cargo tree -e features -i figment` still shows both `figment feature "env"`
  and `figment feature "toml"`. Date/Author: 2026-10-26, migration agent.

- Decision: keep the direct `serde_yaml` dependency.
  Rationale: `serde_yaml` here parses GitHub Actions workflow YAML in
  `test-support/src/workflow.rs`, `tests/steps/packaging_steps.rs` and
  `tests/steps/release_steps.rs`. It is not configuration parsing, so
  OrthoConfig's `yaml` feature (which now means `serde_saphyr`) does not
  replace it, and enabling `ortho_config/yaml` purely for this would add a
  dependency for no behaviour. Issue #237 says to remove a direct configuration
  dependency only when OrthoConfig provides the same behaviour and tests prove
  it; this one is not a configuration dependency. Date/Author: 2026-10-26,
  migration agent.

- Decision: adopt `OrthoResult<T>` and `Arc<OrthoError>` in the repository's
  own configuration API, removing every `#[expect(clippy::result_large_err)]`
  attached to those signatures. Rationale: a `Result` whose error is a pointer
  is small enough that the lint does not fire, so keeping the `#[expect]` would
  trip `unfulfilled_lint_expectations`, which the workspace denies. Using the
  crate's alias also keeps this repository consistent with the dependency it
  exposes in its public API. Date/Author: 2026-10-26, migration agent.

- Decision: report a failed `fig.extract()` as `OrthoError::Gathering`, keeping
  today's rendered prefix `Failed to gather configuration: ...`; do not adopt
  `OrthoMergeExt::into_ortho_merge` at this call site. Rationale: the Merge
  variant's Display string is
  `Failed to merge CLI with configuration: {source}`, and there is no CLI layer
  in this Figment — the composed layers are the file and the environment, and
  command-line values are applied afterwards by `apply_cli_overrides`. Adopting
  it would attribute an environment or file failure to a CLI merge that did not
  happen, which is a diagnostic regression, not an improvement. Issue #237
  requires preserving the exit-status and stream contract; keeping the message
  also keeps that contract literally unchanged. `map_err` with
  `OrthoError::from` stays, which is exactly what produces `Gathering` from a
  `figment::Error`. The v0.7.0 guide's `into_ortho_merge` advice is therefore
  recorded as *not applied*, with this reason, rather than silently skipped.
  Date/Author: 2026-10-26, migration agent.

- Decision: do not change the `rust-toolchain.toml` pin
  (`nightly-2025-07-22`) unless a build proves it necessary. Rationale: the pin
  is `rustc 1.90.0-nightly`, which is above the v0.10.0 floor of `1.89.0`.
  Issue #237 says a nightly date alone does not establish that a bump is
  necessary, and the issue authorizes a bump only if needed. Date/Author:
  2026-10-26, migration agent.

- Decision: do not add a `rust-version` key anywhere yet.
  Rationale: no manifest in this repository declares one today, and the issue
  says to retain any already-higher floor. Introducing
  `rust-version = "1.89.0"` would be a policy change beyond the migration and
  is not needed to make the build work; it is noted as a follow-up rather than
  done here. Date/Author: 2026-10-26, migration agent.

- Decision: record the `guide-policy-ci` defect as a **local erratum in
  `docs/third-party-docs-provenance.md`**, not as an edit to the imported
  `docs/ortho-config/docs/users-guide.md`. Rationale: issue #237 §1 requires
  the five authoritative documents to be imported byte for byte, and every one
  is hash-verified against `38812c56`; correcting the prose would break the
  invariant the issue exists to establish and would make the recorded SHA-256
  wrong. The defect is real — the documented CI recipe runs
  `jq -e '.mode != "off"'` as a separate command, and on a deny-mode violation
  `cargo orthohelp` both emits the report and exits non-zero, so the `jq` test
  succeeds on that report and the script exits 0 despite the finding. It
  affects only the upstream example, not this repository, which never runs
  `cargo orthohelp`. The provenance document is the right home because it is
  where a reader is already sent to check the import. Date/Author: 2026-10-27,
  migration agent.

- Decision: rebase the branch onto `origin/main` at `43b11a1` rather than leave
  it one commit behind, and do so with Git's built-in `zdiff3` merge machinery
  rather than the Weave driver. Rationale: the rebase was instructed
  explicitly, which overrides the earlier "unrelated to this issue" reasoning;
  the resulting PR diff is unchanged at 35 files because main's five workflow
  files drop out of `origin/main...HEAD`, so the repository's own contribution
  is provably untouched. Weave was bypassed because its selection came only
  from the global `~/.config/git/attributes` (`*.rs merge=weave`), not from any
  tracked `.gitattributes` — this repository has none — and for an unattended
  multi-commit replay ambient enrolment is not repository consent. The replay
  therefore ran with `core.attributesFile=/dev/null` and
  `merge.conflictStyle=zdiff3` passed as `-c` overrides. Idempotence: because
  every patch was already applied and `--reapply-cherry-picks` keeps an
  already-present commit rather than dropping it, the replay is deterministic;
  the final tree matched the pre-computed `merge-tree` result exactly.
  Date/Author: 2026-10-27, migration agent.

## Applicability matrix

Every row of the v0.10.0 migration guide's "Impact at a glance" table plus
every section, with its disposition for this repository. "Not applicable" rows
are verified by the grep evidence already recorded in `Surprises & discoveries`
and in the inventory of `Progress`.

| Guide item                                    | Disposition    | Reason and evidence                                                                                                                                                                                                                                                                                                                  |
| --------------------------------------------- | -------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| §1 Dependency versions                        | **Required**   | Two Git-tag declarations, both replaced with `0.10.0`; no `ortho_config_macros` direct dependency; no `cargo-orthohelp`.                                                                                                                                                                                                             |
| §2 Explicit CLI values win                    | **Review**     | The repository already applies CLI overrides by hand in `Config::apply_cli_overrides`, so an explicit flag always wins. A test will pin the restate-the-default case that the guide names, because that is the case the old defect masked.                                                                                           |
| §3 Fluent catalogues / IR 2.0                 | Not applicable | No Fluent catalogue, no `Localizer`, no `ORTHO_DOCS_IR_VERSION` reference, no generated message identifiers: zero derives.                                                                                                                                                                                                           |
| §4 `AgentContext`/`DocMetadata` literals      | Not applicable | Neither type is constructed anywhere; no documentation IR consumer exists in this repository.                                                                                                                                                                                                                                        |
| §5 Boolean flag spellings                     | Not applicable | Every `CliArgs` field is `Option<T>` or a `PathBuf` with a string default; there is no boolean flag, generated or handwritten, in the daemon CLI.                                                                                                                                                                                    |
| §6 Named profiles                             | Not applicable | Opt-in derive capability; the repository has no derive. Profiles would also add a precedence tier that would change behaviour.                                                                                                                                                                                                       |
| §6 Stacked configuration scopes               | Not applicable | Opt-in derive/`ConfigFilePolicy` capability; discovery here is a single explicit path, not scoped discovery.                                                                                                                                                                                                                         |
| §6 Injected environment sources               | Not applicable | `MapEnv` and the `_with_sources` entry points inject the *discovery* environment (XDG paths, the config-path selector, the home directory). This repository lays out its own merge with `figment::providers::Env` over a single explicit path, and nothing here calls discovery. See the test-isolation note in `Verification plan`. |
| §6 `cli_default_as_absent`                    | Not applicable | Requires the derive and `ArgMatches`. The one clap default here (`--config`) is consumed as a plain path before any merge and has no OrthoConfig layer.                                                                                                                                                                              |
| §6 Agent-native behaviour metadata            | Not applicable | Requires the derive plus `cargo-orthohelp`; neither exists here.                                                                                                                                                                                                                                                                     |
| §6 Cargo external-subcommand helper           | Not applicable | `comenq` and `comenqd` are not `cargo` subcommands; nothing invokes `cargo comenq`.                                                                                                                                                                                                                                                  |
| §6 Standalone `ortho_l10n` runtime            | Not applicable | No Fluent lookup anywhere; adding it would be a new dependency with no consumer.                                                                                                                                                                                                                                                     |
| Intermediate guide: v0.6.0 re-exports         | **Applied**    | `figment` is imported via `ortho_config::figment`, so a second `figment` identity cannot appear.                                                                                                                                                                                                                                     |
| Intermediate guide: v0.6.0 discovery policy   | Not applicable | `ConfigDiscovery` is not used; the file path is explicit.                                                                                                                                                                                                                                                                            |
| Intermediate guide: v0.6.0/v0.9.0 YAML        | Not applicable | Configuration files here are TOML only; `serde_yaml` is workflow parsing, not configuration.                                                                                                                                                                                                                                         |
| Intermediate guide: v0.7.0 layer composition  | Not applicable | `compose_layers*` is derive API; layering is two `Figment::merge` calls.                                                                                                                                                                                                                                                             |
| Intermediate guide: v0.7.0 post-merge hooks   | Not applicable | Requires the derive. The token step is application logic after load, not a merge hook.                                                                                                                                                                                                                                               |
| Intermediate guide: v0.7.0 localization       | Not applicable | No localizer.                                                                                                                                                                                                                                                                                                                        |
| Intermediate guide: v0.7.0 error helpers      | **Reviewed**   | `into_ortho_merge` is deliberately not used: its `Merge` text names a CLI merge that does not happen here. `map_err(OrthoError::from)` stays, yielding `Gathering` and preserving the message. See the decision log.                                                                                                                 |
| Intermediate guide: v0.9.0 discovery errors   | Not applicable | `load_first` is not called. The explicit `load_config_file` path already distinguishes absent (`Ok(None)` turned into a `NotFound` error) from malformed (`Err`).                                                                                                                                                                    |
| Intermediate guide: v0.9.0 `OrthoError` shape | **Applied**    | `OrthoError` is `#[non_exhaustive]` with new variants; this repository constructs only `File` and `Validation` and matches on none. The single `fig.extract().map_err(OrthoError::from)` adds `Gathering` by conversion. `Merge` is never constructed, matching the decision log.                                                    |
| Intermediate guide: v0.9.0 tracing/metrics    | Not applicable | Emitted by discovery and derive paths this repository does not use; no subscriber changes required.                                                                                                                                                                                                                                  |
| Intermediate guide: v0.9.0 agent context      | Not applicable | Nothing constructs it.                                                                                                                                                                                                                                                                                                               |

## Conformance basis

The authoritative upstream artefacts for this work are the twelve imported
documents under `docs/ortho-config/`, laid out at their upstream relative
paths, pinned to commit `38812c563e3953a490e3efc202609f37e21a0517` of
`leynos/ortho-config` and recorded with checksums in
`docs/third-party-docs-provenance.md`. There is no in-repository Terms of
Reference or technical design for this migration; the governing requirement set
is issue #237 (<https://github.com/leynos/comenq/issues/237>), sections 1–7,
and the repository's own policy in `AGENTS.md`.

Trace links:

```plaintext
issue-237 §3  ->  EP-M1  ->  `cargo tree -i ortho_config` shows 0.10.0 from crates.io
issue-237 §4  ->  EP-M2  ->  `config.rs` compiles on OrthoResult; `result_large_err` expectations removed
issue-237 §5  ->  EP-M3  ->  file and CLI tiers in-process; default and environment tiers in a child
issue-237 §6  ->  EP-M4  ->  full sequential gate run; baseline/migrated inventory comparison
issue-237 §7  ->  EP-M5  ->  draft PR body with provenance, versions, applicability, results
issue-237 §1  ->  EP-M0  ->  `docs/third-party-docs-provenance.md` checksum tables
```

## Verification plan

The migration introduces no new algorithm and no new persisted format, so the
obligations are behavioural-equivalence obligations over the configuration
pipeline. They are stated as invariants, with the method, artefact, evidence,
and non-vacuity argument for each.

- Obligation `INV-PRECEDENCE`: for any field `f` with a default, a value in the
  file, a value in the environment, and a value on the command line, the
  effective value is the command-line value; without a command-line value it is
  the environment value; without that, the file value; without that, the
  default. The obligation is discharged across two artefacts, split by what a
  process can control.

  In-process (`config/tests/precedence.rs`): only the command-line tier. Five
  cases: a two-row `rstest` table
  `socket_path_resolves_to_the_highest_present_tier` with rows
  `command_line_overrides_a_differing_file_value` and
  `command_line_value_equal_to_the_built_in_default_still_wins`, plus three
  named tests — `command_line_cooldown_equal_to_the_default_still_wins`,
  `command_line_scalar_values_parse_into_their_fields` and
  `command_line_comma_separated_token_file_is_not_split_into_a_pool`. The
  full-tuple check already present in
  `config/tests/configuration.rs::apply_cli_overrides_preserves_unset_values_and_applies_set_values`
  complements them. Every case supplies a command-line value because
  `apply_cli_overrides` runs last and overwrites every lower layer, which is
  what makes the verdict independent of a *well-formed* inherited variable.

  In a child process (`tests/config_environment.rs`): the file, environment and
  built-in-default tiers. A file-tier assertion cannot live in-process either,
  because `Config::from_file` merges the real environment over the file, so an
  exported `COMENQD_SOCKET_PATH` would beat the value the row expects. Rows:
  `defaults_apply_when_no_layer_sets_a_value`, `file_overrides_default`,
  `environment_overrides_file`, `environment_overrides_an_absent_file_value`.

  Rationale: the tiers are a finite, explicitly enumerated partition, so a
  table is exhaustive for the claim, and one table per environment makes the
  rows directly comparable. Domain: `socket_path` (a `PathBuf`), with the other
  fields covered by the named tests below.

  Evidence: `make test` passes, including all five in-process cases and all
  eight tests in the child harness — seven tier cases plus the child body the
  parent re-enters with `--exact`, which is a no-op in an ordinary run. The
  in-process table was re-run with `XDG_RUNTIME_DIR=/run/user/1000` and six
  well-formed `COMENQD_*` variables exported, and still passes; the child
  harness clears the environment outright and passes 8/8 even with a malformed
  inherited variable set, so it is host-independent by construction.

  Known residual, measured rather than assumed: a *malformed* inherited
  `COMENQD_*` variable makes extraction fail before any override, so no tier is
  reached and the in-process cases fail. Ten of the twenty baseline tests in
  `config/tests/configuration.rs` fail identically under the same condition, so
  this is a pre-existing property of the loader, not of this branch; closing it
  would change production behaviour.

  Non-vacuity: every row supplies a distinct literal for each present tier, so
  reading the wrong layer yields a wrong value rather than an accidental match.
  Two negative controls were run against this exact suite and reverted.
  Removing the `cli.socket_path` assignment entirely failed both table rows —
  `command_line_overrides_a_differing_file_value` with
  `left: "/tmp/file.sock", right: "/tmp/cli.sock"` and
  `command_line_value_equal_to_the_built_in_default_still_wins` with
  `left: "/tmp/file.sock", right: "/run/comenq/comenq.sock"` — leaving the
  other three in-process cases green. That is the file value leaking through in
  place of the command-line value, the wrong-tier signal the rows exist to
  catch. Separately, guarding the cooldown assignment with `if secs != 960`
  reproduced the v0.10.0 §2 defect: exactly one case failed,
  `command_line_cooldown_equal_to_the_default_still_wins`, with
  `assertion left == right failed, left: 10, right: 960` — the file value
  surviving because the guard suppressed the restated default. Both mutations
  were reverted and `config.rs` verified byte-identical with `git diff --quiet`.

- Obligation `INV-RESTATED-DEFAULT`: a command-line value equal to the
  built-in default still wins over the file. Method: named unit test.
  Rationale: this is precisely the v0.10.0 §2 defect, and it is one concrete
  scenario, so a named test states the requirement better than a table. Domain:
  file `cooldown_period_seconds=10` and `--cooldown-period-seconds 960`, where
  `960` is exactly `DEFAULT_COOLDOWN`, expecting `960`. The environment value
  the original scenario paired with it now lives in the child harness instead,
  since setting it here would be the mutation §5 forbids. Artefact:
  `command_line_cooldown_equal_to_the_default_still_wins` in
  `config/tests/precedence.rs`. Evidence: passes; it would fail if CLI
  overrides were conditioned on the value differing from the default, which the
  negative control confirmed. Non-vacuity: the two candidate values are distinct
  (`960` against `10`), so a wrong tier choice is observable; the default and
  the command line deliberately agree, which is the whole point — a working
  "only override when different" guard would leave the file's `10` in place.

- Obligation `INV-TOKENS`: the token selection order, the clearing rules, the
  64 KiB bound, the `${VAR}` expansion, and the duplicate rejection with no
  token disclosure are unchanged. Method: the existing parameterized and named
  tests in `config/tests/token_file.rs` and `config/tests/token_pool.rs` are
  the specification; the migration must keep them passing unmodified.
  Rationale: the behaviour is already partitioned into cases; rewriting them
  would destroy the comparison the issue asks for. Domain: every path through
  `resolve_github_token`. Artefact: the existing test files, with a diff
  requirement of zero changes except mechanical type adjustments in signatures.
  Evidence: `git show <rev>:<file>` over `a781258` and `a53f41c` yields the
  same 8 `token_file.rs` and 14 `token_pool.rs` test function names, in the
  same spelling, and `git status --short` on both files is empty — the diff is
  not merely free of renames, it is empty. Non-vacuity:
  `token_pool_rejects_duplicate_trimmed_values_without_disclosing_tokens`
  asserts a substring absence, which fails if the message ever embeds a token.

- Obligation `INV-ERROR-CONTRACT`: an absent file, a malformed file, a missing
  token, and an empty token each produce `Err`; a failed load exits non-zero
  with a message on standard error and nothing on standard output. Method: the
  existing `error_when_missing_file` and `invalid_configuration_errors` table
  plus a binary-level test that runs the built `comenqd` against a bad
  `--config` path and asserts the exit status and that standard output is
  empty. Rationale: exit status and stream placement are process-level
  properties that no in-process test can observe. Domain: absent file,
  malformed TOML, missing token, unterminated placeholder. Artefact: for the
  process-level half, `crates/comenqd/tests/startup_contract.rs` runs the built
  binary with `std::process::Command` (absent file, malformed TOML, missing
  token); for the unterminated-placeholder and empty-token halves, the existing
  in-process cases in `config/tests/token_pool.rs` and `token_file.rs`, where
  `Err` is the observable. Evidence: the child exits non-zero; `stdout` is
  empty; `stderr` is non-empty, and for the missing-token case `stderr` also
  contains the literal
  `provide github_token, github_token_file, or github_token_files` emitted by
  `Config::resolve_github_token`, which is what distinguishes this contract
  from an arbitrary startup failure. Non-vacuity: the daemon does not exit on
  success, so a zero-exit comparison is unavailable. The reachability control
  was run and reverted: inverting the expected status to `success()` in
  `missing_configuration_exits_non_zero_and_reports_on_stderr` made it fail with
  `a missing configuration must fail; stdout was ""`, which is the intended
  reason (the child really exits non-zero), while the other two cases stayed
  green. The file was restored byte-identically and re-run green. The
  `stdout`-is-empty assertion is reachable because the failure path writes
  through `color_eyre` to standard error.

  Liveness of the observation itself: the success path of these three tests is
  the child *exiting*, and a daemon that accepts its configuration does not
  exit. `Command::output()` therefore turns the exact regression
  `missing_token_exits_non_zero_and_reports_on_stderr` guards — token
  validation ceasing to reject — into an indefinite hang rather than a failure,
  which is the worst possible outcome for a suite. `run_daemon` now spawns the
  child and polls `try_wait` against a ten-second `STARTUP_DEADLINE`; on expiry
  it kills and reaps the child and panics with the child's own streams, so the
  failure is reported and nothing is left running. Non-vacuity of the deadline:
  a standalone probe driving the same loop against `/bin/sleep 300` with a
  two-second deadline panicked after 2.0 s and left zero `sleep` processes
  behind.

- Obligation `INV-ENV-PARSING`: the environment layer keeps `figment`'s
  parsing, so `COMENQD_COOLDOWN_PERIOD_SECONDS=420` resolves as an integer and
  `COMENQD_GITHUB_TOKEN_FILES=/a,/b` is a *rejected* startup configuration
  rather than a two-element pool. Rationale: the `Env`-versus-`CsvEnv` choice
  is the one place where a plausible "modernize" edit would silently change
  behaviour, so it gets a pinning test. Domain: `cooldown_period_seconds` (a
  `u64`) and `github_token_files` (a `Vec<PathBuf>`).

  Method: named tests that run the real configuration loader in an isolated
  child process with a controlled environment, in
  `crates/comenqd/tests/config_environment.rs`. In-process tests cannot do this:
  `figment`'s `Env::iter` calls `std::env::vars_os()` directly, `Env::new()`
  is private, and no constructor or builder method accepts an environment
  source, so the only in-process route would be mutating the test binary's own
  environment — which issue #237 §5 forbids. `ortho_config`'s `MapEnv` and
  `_with_sources` entry points are not a substitute: they inject the
  *discovery* environment, and this repository's loader merges by hand over a
  single explicit path.

  Test isolation: the child is this test binary re-executed via
  `std::env::current_exe()` with `--exact` selecting one child body and
  `--nocapture` for diagnosability. Its environment is cleared and rebuilt, so
  the only `COMENQD_*` variables present are the ones a case sets; the child
  body returns immediately when the marker is absent, so an ordinary run pays
  one no-op test per child rather than a second suite. The result travels on a
  file path passed by the parent, not on standard output, because libtest
  writes its own `running 1 test` banner there. The child's token comes from
  the fixture file, so token resolution cannot mask a merge failure. A bounded
  wait kills and reaps an overrunning child rather than blocking the suite.

  Evidence: the integer case resolves to `420` (deliberately not the `960`
  default, so the assertion cannot be satisfied by the default tier) and its
  untouched neighbour still reads `30`; the comma case exits non-zero and its
  stderr carries `Failed to gather configuration`, which names the merge
  failure rather than accepting any panic; the companion array-literal case
  resolves to a two-element vector, proving the rejection is about the
  separator and not a general inability to set the field from the environment.

  Non-vacuity, measured with two seeded faults. First, deleting the `figment`
  merge line altogether makes 5 of the 8 cases fail — precisely the five
  environment cases — while `defaults_apply_when_no_layer_sets_a_value`,
  `file_overrides_default` and the child body itself still pass, which is the
  correct partition. Second, swapping `Env::prefixed(...).split("__")` for
  `ortho_config::CsvEnv::prefixed(...).split("__")` makes exactly one case fail:
  `comma_separated_token_files_are_not_split`. That is the discriminator the
  obligation exists for — an error becoming a working two-token pool — and the
  scalar cases are unaffected by it, which is why the comma case rather than
  the integer case carries the obligation.

- Obligation `INV-NO-SUPPRESSION`: no new `#[allow]`, no
  `--ignore-rust-version`, no `#[ignore]`, and no increase in the number of
  `#[expect]` attributes. Method: `make lint` with `-D warnings` plus a diff
  audit. Artefact: the commit diff. Evidence, counted with
  `git grep -o '#\[expect' <rev> -- '*.rs'` and grouped by file: 30 at baseline
  `a781258` versus 22 at `a53f41c`, a net **reduction** of 8. The removed
  attributes are exactly the five `clippy::result_large_err` suppressions in
  `crates/comenqd/src/config.rs`, the two in `config/single_token.rs`, and the
  one in `config/token_resolution.rs`; `vendor/filetime`, `test-support`,
  `tests/steps/config_steps.rs`, `worker.rs`, `worker/tokens.rs` and
  `comenq-transport` are unchanged. The remaining `result_large_err` in
  `worker/tokens.rs` is unrelated to OrthoConfig (it propagates Octocrab
  errors) and was correctly left alone. Non-vacuity: the counts came from `git`
  object contents at fixed revisions, not from a dirty working tree, so the
  comparison cannot be distorted by formatting or partial staging.

- Obligation `INV-GRAPH`: exactly one `ortho_config` version and exactly one
  `figment` version are reachable, and no Git dependency remains. Method:
  `cargo tree -i ortho_config`, `cargo tree -i figment`, and
  `grep -c 'source = "git+' Cargo.lock`. Artefact: the transcripts below.
  Evidence, measured at `a53f41c`: `cargo tree -i ortho_config` shows exactly
  one node, `ortho_config v0.10.0`, reached from `comenqd` and (as a
  dev-dependency) from `comenq-lib`; `cargo tree -i figment` shows exactly one
  node, `figment v0.10.19`, reached only through `ortho_config v0.10.0`; and
  `grep -c 'source = "git+' Cargo.lock` returns `0`. Non-vacuity: the same
  commands against the pre-migration lockfile would have reported the
  `git+https://github.com/leynos/ortho-config.git?tag=v0.4.0` source, so the
  zero is a change in state, not an artefact of the pattern.

- Axioms relied on (not verified here, treated as documented third-party
  contracts): `ortho_config` 0.10.0's `load_config_file` returns `Ok(None)`
  only for an absent file; `OrthoResult<T> = Result<T, Arc<OrthoError>>`;
  `OrthoError` is `#[non_exhaustive]`; `figment` 0.10.19's `Env` semantics as
  read from its source; `Arc<E>: Error` for `E: Error + Send + Sync`.

- Red-Green-Refactor: the red step is the new tier and restate-the-default
  tests applied to the *migrated* code being written to fail first. Because the
  behaviour under test already exists, the honest sequencing is: add the tests,
  confirm they pass on the pre-migration tree (they must, since the behaviour
  is meant to be preserved), then confirm they still pass after the dependency
  change. A test that passes only after the migration would mean the migration
  changed behaviour and is a finding, not a success. The `INV-RESTATED-DEFAULT`
  case is the exception to check carefully: it passes before *and* after here,
  because this repository never had the derive defect.

## Plan of work

Stage A is complete: the guides are read, the inventory is recorded, and the
decisions above are settled.

Stage B (EP-M1) repoints the dependency. Edit the root `Cargo.toml` twice, at
`Cargo.toml:21` and `Cargo.toml:60`, replacing

```toml
ortho_config = { git = "https://github.com/leynos/ortho-config.git", tag = "v0.4.0" }
```

with the workspace's existing caret convention, `ortho_config = "0.10.0"`.
`crates/comenqd/Cargo.toml:26` inherits via `workspace = true` and needs no
change. Then run `cargo update -p ortho_config` and inspect the lockfile delta.
Do *not* add `--precise 0.10.0`: while the lockfile still keys the package to
the Git source, `--precise` resolves against that source and fails with
`revspec '0.10.0' not found`, as recorded under EP-M1 above. Confirm afterwards
that `cargo tree -i ortho_config` reports a single `v0.10.0` from the registry.
Commit the manifests and the lockfile together, and record the before/after
trees in this plan.

Stage C (EP-M2) migrates the source. In `crates/comenqd/src/config.rs`: replace
the five `Result<_, ortho_config::OrthoError>` signatures with
`ortho_config::OrthoResult<_>`; delete the five
`#[expect(clippy::result_large_err, ...)]` attributes on them; change
`use figment::providers::Env;` to `use ortho_config::figment::providers::Env;`;
and turn the `ok_or_else` that reports a missing file into one that builds
`Arc::new(OrthoError::File { ... })`. Keep
`fig.extract().map_err(ortho_config::OrthoError::from)`: the `From` conversion
does not change between versions and yields the `Gathering` variant whose text
is the published contract here; see the decision log for why `into_ortho_merge`
is deliberately not used. In `config/single_token.rs`, return
`OrthoResult<String>` and `OrthoResult<PathBuf>`, wrap the two constructed
errors in `Arc::new`, and drop the two `#[expect]` attributes. In
`config/token_resolution.rs`, return `OrthoResult<Vec<NamedToken>>`, wrap the
four constructed `Validation` errors in `Arc::new`, and drop the attribute.
Leave `config/tokens.rs` untouched: it works in `io::Error` and the mapping
happens in the caller. Build with `cargo check -p comenqd --all-features`.

Stage D (EP-M3) migrates the BDD harness. In `tests/steps/config_steps.rs:16`,
change `result: Option<Result<Config, ortho_config::OrthoError>>` to
`result: Option<ortho_config::OrthoResult<Config>>`. Nothing else in that file
names the error type, and the two `match world.result.take()` arms already use
`Some(Err(_))`. Add the regression tests from `Verification plan` to a sibling
module, `crates/comenqd/src/config/tests/precedence.rs`, declared from
`config/tests.rs`. This was the expected home because
`config/tests/configuration.rs` was already ~348 lines against the repository's
400-line limit: it ends at 362 lines with the one directory-path test added,
and the new module holds the remaining cases at 192 lines. The process-level
checks go in a new integration target,
`crates/comenqd/tests/startup_contract.rs`, so they exercise the real binary
rather than the library.

Stage E (EP-M4 and EP-M5) runs the gates, requests the review, and delivers.

## Concrete steps

Run every command from the worktree root.

1. Format, then check, then lint, then typecheck, then test — sequentially,
   each with a `tee` log. `pipefail` and `&&` matter here: without `pipefail` a
   pipeline returns the status of its last command, which is `tee`, so a
   failing gate would be reported as a success and the sequence would carry on
   to the next one.

```console
set -o pipefail
make fmt &&
  make check-fmt 2>&1 | tee /tmp/check-fmt-$(git branch --show-current).out &&
  make lint 2>&1 | tee /tmp/lint-$(git branch --show-current).out &&
  make typecheck 2>&1 | tee /tmp/typecheck-$(git branch --show-current).out &&
  make test 2>&1 | tee /tmp/test-$(git branch --show-current).out
```

1. Markdown-specific gates, which the migration also touches through the
   ExecPlan and provenance documents. Same shape, same reason:

```console
set -o pipefail
make spelling 2>&1 | tee /tmp/spelling-$(git branch --show-current).out &&
  make markdownlint 2>&1 | tee /tmp/markdownlint-$(git branch --show-current).out &&
  make nixie 2>&1 | tee /tmp/nixie-$(git branch --show-current).out
```

1. Dependency evidence:

```console
cargo tree -i ortho_config
cargo tree -i figment
grep -n -A3 'name = "ortho_config"' Cargo.lock
```

1. Review, then delivery:

```console
coderabbit review --agent
/usr/bin/ssh -T git@github.com
```

The SSH probe must report `Hi leynos!` before any push. If the CodeRabbit rate
limit is hit, retry in the foreground after a randomized cooldown:

```console
vsleep "$(shuf -i 45-90 -n 1)m"
```

The trailing `m` is required. `vsleep` reads a bare number as **seconds**, so
omitting it turns a 45–90 minute cooldown into 45–90 seconds and the shared
quota has no time to recover; see the corresponding entry under Surprises and
discoveries.

## Idempotence and recovery

Every step here is re-runnable. `cargo update -p ortho_config` is idempotent
once applied; if the graph needs to be undone,
`git checkout Cargo.lock Cargo.toml` restores the baseline because both are
tracked. The provenance document and the imported documents are add-only; a
mistaken edit to an imported file is detected by re-running the SHA-256 table
and restored with `git checkout --` of the upstream content re-fetched from
`38812c56`.

If a gate fails, read the cited `/tmp` log rather than re-running the gate.
Re-run only after a fix. If the compile queue is deep, wait: a queued build
printing `[build-limits] CRATE: waiting for a compile slot` is not hung.

## Artefacts and notes

Baseline identities to compare against:

```plaintext
commit          a781258549c157e38dfaaf1482ccbc01f32d2ccd
branch          issue-237-migrate-from-ortho-config-v0-4-0-git-tag-to-v0-10-0
toolchain       nightly-2025-07-22
rustc           1.90.0-nightly (9748d87dc 2025-07-21)
cargo           1.90.0-nightly (6833aa715 2025-07-13)
LLVM            20.1.8
```

Baseline dependency resolution:

```plaintext
ortho_config 0.3.0  git+https://github.com/leynos/ortho-config.git?tag=v0.4.0#5c27310a915795b9e7c7749c8e38111291aa6976
figment      0.10.19 registry+https://github.com/rust-lang/crates.io-index
serde_yaml   0.9.34+deprecated
clap         4.6.6
```

Baseline pre-existing condition: `cargo` reports
`warning: Patch filetime v0.2.25 (vendor/filetime) was not used in the crate
graph`.
This is unrelated to the migration and is recorded so it is not mistaken for a
new regression.

## Interfaces and dependencies

At the end of EP-M2 the following must hold in `crates/comenqd/src/config.rs`:

```rust
use ortho_config::figment::providers::Env;

impl Config {
    pub const DEFAULT_PATH: &'static str = "/etc/comenqd/config.toml";
    pub fn load() -> ortho_config::OrthoResult<Self>;
    pub fn load_with_tokens()
        -> ortho_config::OrthoResult<(Self, Vec<NamedToken>)>;
    pub fn from_file(path: &Path) -> ortho_config::OrthoResult<Self>;
    fn from_file_with_cli(path: &Path, cli: &CliArgs)
        -> ortho_config::OrthoResult<Self>;
    fn from_file_with_cli_and_tokens(path: &Path, cli: &CliArgs)
        -> ortho_config::OrthoResult<(Self, Vec<NamedToken>)>;
}
```

In `crates/comenqd/src/config/single_token.rs`:

```rust
pub(super) fn read_github_token_file(path: &Path)
    -> ortho_config::OrthoResult<String>;
pub(super) fn expand_env_prefix(path: &Path)
    -> ortho_config::OrthoResult<PathBuf>;
```

In `crates/comenqd/src/config/token_resolution.rs`:

```rust
pub(super) fn resolve_github_token(&mut self, has_cli_token: bool)
    -> ortho_config::OrthoResult<Vec<NamedToken>>;
pub fn github_tokens(&self) -> std::io::Result<Vec<NamedToken>>;
```

`github_tokens` keeps `io::Result` deliberately: it is a pure filesystem
operation with no Figment involvement, and its `io::Error` is what
`resolve_github_token` maps into `OrthoError::Validation`.

`Config`'s nine public fields, its `From` conversions, and
`apply_cli_overrides`'s signature are unchanged.

## Outcomes & retrospective

Delivered: the dependency moved from `ortho_config` v0.3.0 at Git tag `v0.4.0`
to v0.10.0 from crates.io, with no Git dependency left in `Cargo.lock`; the
daemon's configuration API moved to `OrthoResult`/`Arc<OrthoError>`; the
redundant direct `figment` dependency was removed; and eight
`clippy::result_large_err` suppressions disappeared because the error is now
pointer-sized. Behaviour is preserved, and the preservation is argued from
evidence rather than from the diff: cucumber totals are byte-identical to
baseline at 11 features / 55 scenarios / 211 steps, nextest went from 481 to
498 passing tests with all 17 additions accounted for, the 22 token tests were
carried across with an empty diff, and the real binary still exits 1 with an
empty standard output and a `color_eyre` report on standard error.

Delivered to trunk: squash-merged as `b264e216f37d0b34d624c3212c33846cfbc79692`
at 2026-10-31T16:22:47Z, after a formal review that found seven genuine defects
in the imported upstream documents. Those seven are now recorded upstream
errata rather than silent corrections, which is the honest outcome: this
migration vendored the text at a pinned commit and must not rewrite it, so a
reader is told what is wrong with it instead. Nothing was published —
`release.yml` fires only on version tags, so the merge triggered only
`Coverage (main)`.

What actually mattered was resisting two tempting edits. Adopting
`into_ortho_merge` would have relabelled every environment- and file-layer
failure as a CLI merge, in a pipeline whose CLI is applied after the Figment
merges — a diagnostic regression invisible to any test that only asserts
`is_err()`. Adopting `CsvEnv`, the newer and superficially better environment
provider, would have turned `COMENQD_GITHUB_TOKEN_FILES=/a,/b` from a startup
error into a silently working two-token pool. Both survived scrutiny only
because the evidence was read from the dependency's own source and manifest
rather than inferred from a migration guide.

Lessons worth keeping. A "no behaviour change" claim needs a witness that can
fail: the restated-default test was vacuous at first (it asserted `960`, which
is also the default) and became meaningful only after mutation testing showed
which value a wrong implementation would produce. Verification statements
written in the future tense ("the mutation must fail") are not evidence; they
become evidence when the mutation is actually run, which is how two of the
controls here were caught being untested. And scoping a commit with
`git checkout --` discards work silently: the one-line `config_steps.rs`
migration vanished that way and was only recovered because the next compile
failed. Scope with the index, and compile immediately after.

One more lesson, from review rather than from the build. A test named for a
contract must assert the contract and not merely its neighbours. The
missing-token process test checked a non-zero exit, an empty standard output
and a non-empty standard error — all true of every other startup failure too —
so it would have stayed green if token validation had been removed entirely.
Asserting the identifying text is what makes the name honest. The same review
caught first-person narration in this plan, which is worth noting because the
document describes the work and should not assume who performed it.

Not done, deliberately: no `rust-version` key was added (the pin already
exceeds the floor, and adding one would narrow the supported range without
evidence); no dated-nightly bump was needed; `serde_yaml` was retained because
it serves GitHub workflow YAML parsing, not configuration; and no optional
v0.10.0 capability (derive, profiles, localization, documentation IR) was
adopted, because each would have changed behaviour rather than replaced code
the migration already had to touch.
