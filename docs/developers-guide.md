# Developers' guide

## Spelling gate

Run `make spelling` to enforce en-GB-oxendict spelling in tracked Markdown
prose. The target regenerates `typos.toml`, verifies that the generated file is
tracked and unchanged, then runs the pinned `typos` release over tracked
Markdown files. `make markdownlint` depends on this gate, and `make all` runs it
with the repository's release build.

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

## Mutation-testing workflow contract tests

This repository runs scheduled, informational mutation testing through a thin
caller workflow, [`.github/workflows/mutation-testing.yml`](../.github/workflows/mutation-testing.yml),
which delegates to the shared reusable workflow
`leynos/shared-actions/.github/workflows/mutation-cargo.yml`. The heavy lifting
— running `cargo-mutants`, sharding, and summarizing survivors — lives in
`shared-actions`; this repository carries only declarative configuration. The
run is **informational only**: it never gates a pull request. Survivors are
reported through the job summary and downloadable artefacts so they can be
triaged into tests, not enforced as a blocking check.

The workflow runs in two modes. A **daily schedule** fires a change-scoped run
that mutates only the source files touched within the detection window, so
quiet days are cheap no-ops. A **manual dispatch** (the Actions "Run workflow"
control) mutates the whole workspace, fanned out across shards; select a
branch in that control to exercise a feature branch.

The caller passes a small set of configuration inputs, each carrying intent:

- `paths` — the change-detection globs (`src/,crates/`) that decide whether a
  scheduled run has anything to mutate: the root `comenq-lib` crate lives
  under `src/`, and the `comenq` client and `comenqd` daemon crates live under
  `crates/`.
- `exclude-globs` — `test-support/**` and `crates/test-utils/**`, the
  test-helper scaffolding whose surviving mutants are noise rather than
  genuine test gaps, kept out of the survivors table.
- `extra-args` — `--workspace --all-features --test-workspace=true`, so every
  workspace member is mutated against the full workspace test run, matching
  the CI baseline (`make test` runs workspace-wide with `--all-features`);
  without `--workspace`, cargo-mutants would mutate only `comenq-lib` and skip
  the `crates/` members, since the root manifest is itself a package.

The `uses:` reference pins the shared workflow to a full 40-character commit
SHA rather than a branch or tag, so a force-push upstream cannot silently
change what runs here. The contract test asserts only that the pin is a full
commit SHA, not a particular value, so Dependabot bumps it automatically
without any accompanying test edit.

Because the caller is configuration rather than code, a contract test pins the
shape it must uphold, failing the pull request when the caller drifts —
repointing the pin at a branch, widening the token scope, or dropping a
configuration input — rather than letting the breakage surface only in a
scheduled run. Run it locally with `make test-workflow-contracts`. The test
validates:

- the `uses:` reference targets `mutation-cargo.yml` pinned to a full commit
  SHA;
- the `with:` block carries exactly the expected configuration (the paths,
  excludes, and workspace arguments above);
- job permissions are least-privilege (`contents: read`, `id-token: write`)
  and the workflow-level default token scope is empty;
- `concurrency` serializes runs per ref without cancelling one in progress;
  and
- the triggers keep the daily schedule and a plain `workflow_dispatch` with no
  legacy branch input.
