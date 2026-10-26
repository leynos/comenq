# Provenance of the imported `rstest-bdd` documentation

This document records the exact upstream revision from which the `rstest-bdd`
reference documentation in this repository was imported. It exists so that a
future maintainer can tell repository-authored prose apart from vendor
documentation, re-derive the import, and detect an accidental local edit.

## Upstream source

- Repository: `https://github.com/leynos/rstest-bdd`
- Tag: `v0.6.0`
- Resolved commit: `72fb22635670e456545ca368805ba4c1c9d7bd69`
- Tag object type: annotated tag pointing directly at that commit.

The tag was resolved with:

```plaintext
git rev-parse 'refs/tags/v0.6.0^{commit}'
```

which printed `72fb22635670e456545ca368805ba4c1c9d7bd69`. This matches the
commit recorded in the migration task, so the import is anchored to the
intended immutable revision rather than to a branch head.

## Imported files

Each destination is byte-for-byte identical to its source at the pinned commit.
The checksums below were taken from the pinned checkout and confirmed against
the imported copy.

| Source path                      | Destination path                            | SHA-256                                                            |
| -------------------------------- | ------------------------------------------- | ------------------------------------------------------------------ |
| `docs/users-guide.md`            | `docs/rstest-bdd-users-guide.md`            | `1e9f4d1b6607fdf83df979676f1a682218cd6751381d374cb98b701778b798b2` |
| `docs/v0-6-0-migration-guide.md` | `docs/rstest-bdd-v0-6-0-migration-guide.md` | `6e76c10028f9962134f6158732cf1645ae1b0beba3018bccd3bc3e88b2c38985` |
| `docs/v0-5-0-migration-guide.md` | `docs/rstest-bdd-v0-5-0-migration-guide.md` | `508d5b55cb2025915ced366b917a2327e572a6aa920fd290684b858b7afa2abc` |

Re-verify at any time with:

```sh
sha256sum docs/rstest-bdd-users-guide.md \
  docs/rstest-bdd-v0-6-0-migration-guide.md \
  docs/rstest-bdd-v0-5-0-migration-guide.md
```

### Why the destinations are prefixed

This repository already owned a document named
`docs/v0-6-0-migration-guide.md`. That file is Ortho Config's own guide for
upgrading an application from Ortho Config v0.5.0 to v0.6.0, and it is
unrelated to `rstest-bdd`. Importing the upstream `rstest-bdd` guide to the
unprefixed path would have destroyed it.

The `rstest-bdd-` prefix follows the convention already established in this
repository by `docs/rstest-bdd-users-guide.md` and
`docs/rstest-bdd-v0-5-0-migration-guide.md`, both of which were imported by an
earlier migration.

The v0.5.0 guide is included because the previous migration target was 0.5.0,
so a reader arriving from a pre-0.5.0 baseline needs both steps of the path.

## What the import replaced

The import overwrote two files that were already present, so this section
records what they were, and the evidence for its conclusion. Both were earlier
imports of the same upstream documents and neither contained
repository-authored text. Nothing of ours was lost.

| Destination                                 | Previous state                                     | Verification                                                                                                                                                                                                      |
| ------------------------------------------- | -------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `docs/rstest-bdd-users-guide.md`            | 1919 lines, an import of the upstream v0.5.0 guide | 86 lines differed from upstream's v0.5.0 copy, every one of them a reflow of the same prose, an en-GB spelling correction, or Mermaid node-label quoting. No repository term appears anywhere in the file.        |
| `docs/rstest-bdd-v0-5-0-migration-guide.md` | 290 lines, an import of the upstream v0.5.0 guide  | 107 lines differed from upstream's v0.6.0 copy of the same file, again all of them reflow or en-GB spelling. Line counts differ (290 against 222) because the upstream copy dropped a section in a later release. |

The earlier copies changed two particular upstream words to the repository's
spelling convention. The v0.6.0 import restores the upstream wording, trading
those two repository-specific changes for byte-for-byte integrity. The checksum
table above is the authority; a future edit that reintroduces them would
silently break it.

The overwrite destroyed no uncommitted work. This repository's session log for
the migration records the opening `git status` as `(clean)`, so the working
tree matched its commit before the import and the two modified files reported
afterwards are the import itself. The HEAD revision of the users' guide
contains zero occurrences of `ortho_config`, `orthohelp`, `ddlint` or
`cargo-ortho`, confirming it was vendor text rather than an annotated copy.

A repository-specific migration note therefore belongs outside these files, in
`docs/execplans/migrate-rstest-bdd-v0-6-0.md` and in this document, which is
why both exist separately from the imported guides.

## Relative links in the imported files

The imported documents link to sibling upstream documents that are not part of
this repository. Rather than rewrite the copied text, this section records the
mapping so a reader can resolve a link manually.

### `docs/rstest-bdd-v0-6-0-migration-guide.md`

| Link target                                                 | Upstream document at the pinned commit                           |
| ----------------------------------------------------------- | ---------------------------------------------------------------- |
| `developers-guide.md`                                       | `docs/developers-guide.md`                                       |
| `rstest-bdd-design.md`                                      | `docs/rstest-bdd-design.md`                                      |
| `adr-006-fallible-scenario-functions.md`                    | `docs/adr-006-fallible-scenario-functions.md`                    |
| `adr-007-harness-context-injection.md`                      | `docs/adr-007-harness-context-injection.md`                      |
| `adr-009-consistent-implicit-fixture-name-normalization.md` | `docs/adr-009-consistent-implicit-fixture-name-normalization.md` |
| `adr-012-guard-based-stepcontext-borrowing.md`              | `docs/adr-012-guard-based-stepcontext-borrowing.md`              |
| `users-guide.md`                                            | `docs/rstest-bdd-users-guide.md` in this repository              |

The first five resolve only within an upstream checkout. The upstream anchor
`users-guide.md#stateful-gpui-scenarios-with-durable-handles` and
`users-guide.md#published-gpui-022-stateful-step-variants` resolve inside
`docs/rstest-bdd-users-guide.md`, which is present in this repository.

### `docs/rstest-bdd-users-guide.md`

The `[gpui-migration]` reference targets
`v0-6-0-migration-guide.md#migrate-a-stateful-gpui-test`, and so resolves to
this repository's own `docs/v0-6-0-migration-guide.md`, which has no such
anchor; the intended upstream section is the one of that name in
`docs/rstest-bdd-v0-6-0-migration-guide.md`. All other link targets are absolute
`https://` URLs, rendered by the Markdown linter as unannotated bare links at
`docs/rstest-bdd-users-guide.md:2784`, `:2785` and `:3229`; the remaining
examples below read as external links of the same kind.

### `docs/rstest-bdd-v0-5-0-migration-guide.md`

The body links to the upstream `users-guide.md` and to the upstream
`CHANGELOG.md`. The users' guide maps to `docs/rstest-bdd-users-guide.md` here;
the changelog is not vendored.

## Scope of the imported migration guide

The pinned `docs/v0-6-0-migration-guide.md` contains two subsections that
describe releases **later** than v0.6.0:

- "Use the harness-context marker (v0.6.1 beta)".
- "Adopt guard-based `StepContext` borrowing (v0.7.0)", and its child
  "Retire the thread-local workaround".

Those sections are retained verbatim because the import is byte-for-byte, but
they are not instructions for this migration. The v0.6-compatible
`#[from(rstest_bdd_harness_context)]` spelling remains the supported form here,
and no v0.7 lifecycle or borrowing guarantee may be relied upon.

## Handling by repository tooling

`make fmt` and `make check-fmt` run `mdtablefix`, which rewraps Markdown prose
and would break the byte-for-byte guarantee recorded above if it had anything
to rewrite. These files need no exclusion, because upstream's own formatting
already satisfies the rule set this repository applies. Verified against
`mdtablefix 0.6.1`, the version CI pins, using the Makefile's exact rules:

```sh
mdtablefix --check --wrap --renumber --breaks --ellipsis --fences \
  docs/rstest-bdd-users-guide.md \
  docs/rstest-bdd-v0-6-0-migration-guide.md \
  docs/rstest-bdd-v0-5-0-migration-guide.md
```

which prints `3 files left unchanged.` and exits zero. `markdownlint-cli2`
likewise reports no issues, so the imported text passes both Markdown gates
unmodified.

This is worth re-checking on the next import rather than assumed: it holds for
this pinned revision, and a future upstream revision could introduce a table or
a paragraph that the local rules would rewrap. If that happens, prefer
correcting the import or adding a narrow, documented exclusion over accepting a
silent divergence between the imported bytes and their recorded checksums.

## Revision note

Created 2026-10-14 alongside the `rstest-bdd` 0.5.0 to 0.6.0 migration. If the
dependency is upgraded again, replace this document rather than appending to
it, so that it always describes exactly one pinned revision.
