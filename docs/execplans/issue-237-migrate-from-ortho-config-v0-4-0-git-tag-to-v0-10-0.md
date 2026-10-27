# Migrate `comenqd` configuration to `ortho_config` v0.10.0

This ExecPlan (execution plan) is a living document. The sections `Constraints`,
`Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`, `Decision log`,
`Outcomes & retrospective`, `Conformance basis`, and `Verification plan` must
be kept up to date as work proceeds.

Status: APPROVED

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

## Decision log

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
| Intermediate guide: v0.9.0 `OrthoError` shape | **Applied**    | `OrthoError` is `#[non_exhaustive]` with new variants; this repository constructs only `File`, `Validation` and (newly) `Merge`, and matches on none.                                                                                                                                                                                |
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
  eight child cases. The in-process table was re-run with
  `XDG_RUNTIME_DIR=/run/user/1000` and six well-formed `COMENQD_*` variables
  exported, and still passes; the child cases clear the environment outright
  and pass 8/8 even with a malformed inherited variable set, so they are
  host-independent by construction.

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
