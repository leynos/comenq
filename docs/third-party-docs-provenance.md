# Third-party documentation provenance

This document records the origin of vendored upstream documentation so that a
reader can re-fetch exactly the same bytes, and so that a later refresh is a
reviewed change rather than an unverifiable edit.

These files are **copies**. Do not edit them. Local notes and migration records
belong in this repository's own documents.

## `ortho_config` v0.10.0

- Upstream repository: <https://github.com/leynos/ortho-config>
- Tag: `v0.10.0`
- Tag object: `996e5a1c9f22ace4f55bf5dcc196d2c0524262bf` (annotated)
- Peeled commit: `38812c563e3953a490e3efc202609f37e21a0517`
- Retrieved: 26 October 2026

The tag was resolved independently with
`git ls-remote origin 'refs/tags/v0.10.0*'`, which reported both the annotated
tag object and its peeled commit. Both values matched the identities named in
[issue #237][issue-237] exactly, so no discrepancy needed investigating. All
files below were copied from the peeled commit, not from `main`, `HEAD`, a
development branch, or a rendered documentation site.

`cargo tree -i ortho_config` and `Cargo.lock` both record the *pre-migration*
Git tag as resolving to crate version `0.3.0` at commit
`5c27310a915795b9e7c7749c8e38111291aa6976` (`refs/tags/v0.4.0`). That is the
version boundary this migration crosses.

### Placement

Everything is copied beneath `docs/ortho-config/`, preserving the upstream
relative path exactly. That is what lets the imported text stay byte-for-byte
identical: a relative link such as `v0-10-0-migration-guide.md` resolves
against its own directory, so keeping the sibling relationship intact is the
only way to satisfy the link without editing a single byte of the copy. A flat
`docs/ortho-config-<name>.md` layout cannot do that — a Markdown link is
resolved by the reader's path arithmetic, not by a mapping table, so the links
would break however carefully they were documented.

The subtree therefore mirrors upstream: `docs/ortho-config/README.md` is
upstream's `README.md`, `docs/ortho-config/docs/users-guide.md` is upstream's
`docs/users-guide.md`, and so on. The prefix also prevents a collision with
this repository's own `docs/users-guide.md` and `docs/README.md`.

### Imported files

The five documents named as authoritative by issue #237.

| Upstream path                     | Destination in this repository                      | SHA-256                                                            |
| --------------------------------- | --------------------------------------------------- | ------------------------------------------------------------------ |
| `docs/users-guide.md`             | `docs/ortho-config/docs/users-guide.md`             | `84d1c97b77024224dc9220a12d4b749205d9bb2db54849185da21c969dda5575` |
| `docs/v0-10-0-migration-guide.md` | `docs/ortho-config/docs/v0-10-0-migration-guide.md` | `b5331aa6cfcc07b3dcd188c620db98d0a1a1f13b0c8ef96d88913d701bca25bf` |
| `docs/v0-9-0-migration-guide.md`  | `docs/ortho-config/docs/v0-9-0-migration-guide.md`  | `c174c424f91c055201d6a8ec644b0a3b2c7aa681ffbd052d9d6152d78eb2f49d` |
| `docs/v0-7-0-migration-guide.md`  | `docs/ortho-config/docs/v0-7-0-migration-guide.md`  | `d5692d45a784a5e148f6a4244eae46a649068ee26422309e2c9095daaabd106d` |
| `docs/v0-6-0-migration-guide.md`  | `docs/ortho-config/docs/v0-6-0-migration-guide.md`  | `6fb6172f34aa85601ba3279e39e6c2af016a64d6360026a435d99143ac25be3e` |

### Imported link targets

The imported text is kept byte-for-byte identical, so its relative links are
resolved by importing their targets from the same commit and at the same
relative path, rather than by rewriting the links.

| Upstream path                                                 | Destination in this repository                                                  | SHA-256                                                            |
| ------------------------------------------------------------- | ------------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| `README.md`                                                   | `docs/ortho-config/README.md`                                                   | `548f267f373283b2fb142e536a8297d17a5c561fb43549da0e20a3a04dd14b1b` |
| `CHANGELOG.md`                                                | `docs/ortho-config/CHANGELOG.md`                                                | `9d1dccd18a0c5ffe0450cbec510ef412d4dfa3b6d4767e479fb13b6a625b8ec5` |
| `ortho_l10n/README.md`                                        | `docs/ortho-config/ortho_l10n/README.md`                                        | `4869853643a68425ece40b142290db5d131344aa76d1814d2bafc6fccc941c9f` |
| `docs/cargo-orthohelp-api-guide.md`                           | `docs/ortho-config/docs/cargo-orthohelp-api-guide.md`                           | `b4b3de8186d1f0c47585d3958db7b8bdda62223d5f592008e9ce2a1a0e2e0266` |
| `docs/cargo-orthohelp-design.md`                              | `docs/ortho-config/docs/cargo-orthohelp-design.md`                              | `145bffb414b64c34eec680bb19fd5a1e3fdb2981bcb7efd455eb61ce479f2ad5` |
| `docs/adrs/adr-0004-cargo-external-subcommand-entry-point.md` | `docs/ortho-config/docs/adrs/adr-0004-cargo-external-subcommand-entry-point.md` | `ed50351581d046c1ad835b799902e86c9da8a8fd590520dff36d0c61d2d8d81a` |
| `examples/hello_world/README.md`                              | `docs/ortho-config/examples/hello_world/README.md`                              | `72fc4514afc322e3ebbe3c5094596df782209da2cc125e8c7e0757ea86fb94d9` |

`users-guide.md` links `../examples/hello_world/` at directory granularity. A
directory cannot be imported as a file, so the landing page
`examples/hello_world/README.md` is imported so that the link resolves, and the
listing of the remaining files at this commit is recorded in
`docs/ortho-config/examples-hello-world-files.txt`. That listing is this
repository's own generated record, not an upstream copy, so it is exempt from
the byte-for-byte rule; it exists to describe the tree at the pinned commit.

### Second-level link targets

The files above link onward to further documents. Those links are followed one
level, so that every link out of the twelve files imported so far resolves.

| Upstream path                                                              | Destination in this repository                                                               | SHA-256                                                            |
| -------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| `docs/adrs/adr-0003-define-schema-ownership-for-agent-native-contracts.md` | `docs/ortho-config/docs/adrs/adr-0003-define-schema-ownership-for-agent-native-contracts.md` | `a899e178cea626329caeb99776503da4b6eff5b0b285a8daf6d3bcebc9ebf87c` |
| `docs/adrs/adr-0007-downstream-context-command-naming.md`                  | `docs/ortho-config/docs/adrs/adr-0007-downstream-context-command-naming.md`                  | `c5e2a0d87617e5c47885ce2b4df0ebc8cefd1170cdcb3ede446cbf8138495f41` |
| `docs/adrs/adr-0014-documentation-build-ownership.md`                      | `docs/ortho-config/docs/adrs/adr-0014-documentation-build-ownership.md`                      | `3bb79df6c2741928095e6d132480e8be9ba13d13e0e12918406d66be01f4de85` |
| `docs/agent-native-cli-design.md`                                          | `docs/ortho-config/docs/agent-native-cli-design.md`                                          | `26ee326f7a682b8953b8628d1cc1770471f3ebc613ca05fced98a21d0e2e8baf` |
| `docs/clap-mangen-cargo-orthohelp-migration-guide.md`                      | `docs/ortho-config/docs/clap-mangen-cargo-orthohelp-migration-guide.md`                      | `239da92dcc8e7b3e20e183c986048b80de1e991f5f5b59059b474aa2fed95cbe` |
| `docs/developers-guide.md`                                                 | `docs/ortho-config/docs/developers-guide.md`                                                 | `891839e596b3efd9fd95ba95c72a67e2992e136b2f920fd2f23c64b620b1fb25` |
| `docs/rstest-bdd-doc-provenance.md`                                        | `docs/ortho-config/docs/rstest-bdd-doc-provenance.md`                                        | `55266ba7e6010d76bcf2f0c53a7a08c95e5c5a8daca0bbe87d1bc3e545c5eca3` |
| `docs/rstest-bdd-v0-6-0-migration-guide.md`                                | `docs/ortho-config/docs/rstest-bdd-v0-6-0-migration-guide.md`                                | `6e76c10028f9962134f6158732cf1645ae1b0beba3018bccd3bc3e88b2c38985` |

The boundary is drawn here on purpose. Closing the link graph fully is not
achievable: computing the transitive closure from the five authoritative
documents reaches 48 files over five rounds, and even then seven of the targets
do not exist at the pinned commit at all — `docs/adr-006-…`, `docs/adr-007-…`,
`docs/adr-009-…`, `docs/rstest-bdd-design.md` and
`docs/docs/{design,roadmap,subcommand-refinements}.md` are dangling links
upstream. Vendoring those 36 extra files would also mean vendoring a document
that fails this repository's spelling gate (see below), and the result would
still contain broken links. One level of link-target import is therefore the
practical stopping point: it makes every link out of the twelve imported
documents resolve, and it leaves the deeper references inside the imported
copies linking to documents this repository does not carry — which is the same
state those references are in upstream.

| Level                             | Files | Gate result                                                        |
| --------------------------------- | ----- | ------------------------------------------------------------------ |
| Five authoritative documents      | 5     | pass all Markdown gates                                            |
| Direct link targets (both tables) | 12    | pass all Markdown gates                                            |
| Second-level targets added here   | 8     | pass `make markdownlint` and `make spelling`                       |
| Full closure (not imported)       | 48    | one spelling failure in `docs/execplans/9-1-1-profile-metadata.md` |

Every file in both tables was verified to round-trip: the bytes written were
re-hashed after writing and compared against the hash of `git show` output for
the same path, and all twenty matched.

### Known upstream errata

The imported copies are recorded byte for byte, so a defect in the pinned
upstream text is preserved rather than corrected. Corrections are recorded here
instead, so readers are not misled while the imported bytes and their hashes
stay verifiable against `38812c56`.

- `docs/ortho-config/docs/users-guide.md`, in the `guide-policy-ci` console
  block under "Lint the declared behaviour", documents a CI recipe as two
  sequential commands:

  ```console
  cargo orthohelp --check-agent-native --package my-cli --out-dir out
  jq -e '.mode != "off"' out/policy-report.json
  ```

  The second command runs even when the first fails. On a deny-mode violation
  `cargo orthohelp` both writes `out/policy-report.json` and exits non-zero,
  and because that report then contains a mode other than `off`, the `jq` test
  still succeeds — so a script made of these two lines, or a CI job using them,
  finishes successfully despite the policy finding. Join them with `&&` (or run
  under `set -e`) to keep the failure:

  ```console
  cargo orthohelp --check-agent-native --package my-cli --out-dir out &&
    jq -e '.mode != "off"' out/policy-report.json
  ```

  This affects only the documentation's example. Nothing in this repository runs
  `cargo orthohelp`, and the daemon's own behaviour is unaffected.

- `docs/ortho-config/docs/adrs/adr-0003-define-schema-ownership-for-agent-native-contracts.md`
  writes the policy-report owner as `cargo-orthohelp::policy` at lines 58 and
  74. The surrounding corpus uses the underscore form for the same module —
  `docs/ortho-config/CHANGELOG.md`, `cargo-orthohelp-api-guide.md`,
  `cargo-orthohelp-design.md` and `agent-native-cli-design.md` all write
  `cargo_orthohelp::policy`. Because these are backticked Rust paths rather
  than prose, the hyphenated spelling does not compile. Read both occurrences as
  `cargo_orthohelp::policy`. The hyphenated form does exist in the corpus, but
  only as the CLI binary's *name*, so the distinction is between the package
  and the module that package exposes.

- `docs/ortho-config/docs/adrs/adr-0014-documentation-build-ownership.md`
  links `[design-infrastructure]` to
  `../design.md#419-executable-documentation-test-build-ownership` at line 370.
  `docs/ortho-config/docs/design.md` was not imported and the target heading
  text appears nowhere in the imported corpus, so this reference is dangling.
  That is the boundary recorded above under "Second-level link targets": the
  deeper reference is in the same dangling state it occupies upstream. The
  related `[developer-guidance]` link to
  `developers-guide.md#public-documentation-examples` *does* resolve, because
  that file and its `### Public documentation examples` heading are both
  present. Only the `design.md` target is missing here.

- `docs/ortho-config/docs/cargo-orthohelp-design.md` registers one
  three-parameter scriptblock
  `param($wordToComplete, $commandAst, $cursorPosition)` at line 1102 and again
  at line 1373, then reuses it for both branches of the registration at lines
  1108-1112 and 1374-1381. That signature is correct only for the `-Native`
  branch. Without `-Native`, `Register-ArgumentCompleter` passes a different
  positional contract, so the fallback branch would bind the wrong values. Per
  Microsoft's `Register-ArgumentCompleter` reference, the scriptblock must
  accept these parameters in order. With `-Native`: `$wordToComplete`,
  `$commandAst`, `$cursorPosition`. Without it: `$commandName`,
  `$parameterName`, `$wordToComplete`, `$commandAst`, `$fakeBoundParameters`. A
  generated wrapper needs two scriptblocks, or one scriptblock that
  distinguishes the case. The design document's stated intent elsewhere is that
  registrations target the wrapper *function*, which requires the five-argument
  form.

- `docs/ortho-config/docs/cargo-orthohelp-design.md` sets
  `ExternalHelp = '<ModuleName>-help.xml'` in the minimum module manifest at
  line 1125, and `ExternalHelp = 'MyApp-help.xml'` at line 1392. `ExternalHelp`
  is not a module-manifest key. Per Microsoft's `about_Comment_Based_Help`,
  `.EXTERNALHELP` is a *comment-based help keyword*, and for a module-exported
  function the documented mechanism is a `<ModuleName>-help.xml` file placed in
  a language-specific subdirectory of the module directory, discovered without
  any manifest entry. The key as written is therefore inert. `HelpInfoUri`,
  which the same example already comments out, is the manifest key for
  *updatable* help and should be set only when hosting Update-Help payloads.

- `docs/ortho-config/docs/cargo-orthohelp-design.md` opens "14. Implementation
  plan (delta)" at line 1398 with "Bump the IR schema to 1.1". That item
  predates the current contract. The document is Revision 4 (line 14), and it
  states `ir_version = "2.0"` at lines 1213, 1287 and 1334. Read the 1.1 item
  as a historical Revision 2 stage; the binding version for the current
  revision is `2.0`.

- `docs/ortho-config/docs/v0-7-0-migration-guide.md` line 124 imports
  `FluentLocalizer`, `Localizer`, `langid` and
  `localize_clap_error_with_command` from `ortho_config`, but line 132 calls
  `Cli::command().localize(&localizer)`. The trait method `localize` is
  provided by `LocalizeCmd`, which the example does not import, so a consumer
  copying it verbatim gets an unresolved-method error. Add `LocalizeCmd` to the
  import list.

- `docs/ortho-config/examples/hello_world/README.md` renders the crate name's
  possessive with a space before the apostrophe, producing `ortho_config ’s`
  rather than `ortho_config’s`. This occurs twice: at line 48, in the
  "Localized help text" bullet, and at line 91, in the `src/localizer.rs`
  description. Only the spacing is wrong; the punctuation is the correct
  typographic apostrophe.

No file above was edited. Each entry names a defect in the pinned upstream text
at `38812c56`, which this repository preserves rather than rewrites, so every
recorded SHA-256 above remains valid.

### Reproducing the import

Every listed entry in `docs/ortho-config/examples-hello-world-files.txt`
already begins with `examples/hello_world/`, so pass it to `git show` as
written; prefixing it again produces a path that does not exist.

```sh
git clone https://github.com/leynos/ortho-config.git /tmp/ortho-config
git -C /tmp/ortho-config fetch --tags
git -C /tmp/ortho-config rev-parse 'v0.10.0^{commit}'   # 38812c56...
git -C /tmp/ortho-config show 38812c56:docs/users-guide.md | sha256sum
# For a file from the example listing, use the entry verbatim:
git -C /tmp/ortho-config show 38812c56:examples/hello_world/README.md | sha256sum
```

[issue-237]: https://github.com/leynos/comenq/issues/237
