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

### Imported files

| Upstream path                     | Destination in this repository                 | SHA-256                                                            |
| --------------------------------- | ---------------------------------------------- | ------------------------------------------------------------------ |
| `docs/users-guide.md`             | `docs/ortho-config-users-guide.md`             | `84d1c97b77024224dc9220a12d4b749205d9bb2db54849185da21c969dda5575` |
| `docs/v0-10-0-migration-guide.md` | `docs/ortho-config-v0-10-0-migration-guide.md` | `b5331aa6cfcc07b3dcd188c620db98d0a1a1f13b0c8ef96d88913d701bca25bf` |
| `docs/v0-9-0-migration-guide.md`  | `docs/ortho-config-v0-9-0-migration-guide.md`  | `c174c424f91c055201d6a8ec644b0a3b2c7aa681ffbd052d9d6152d78eb2f49d` |
| `docs/v0-7-0-migration-guide.md`  | `docs/ortho-config-v0-7-0-migration-guide.md`  | `d5692d45a784a5e148f6a4244eae46a649068ee26422309e2c9095daaabd106d` |
| `docs/v0-6-0-migration-guide.md`  | `docs/ortho-config-v0-6-0-migration-guide.md`  | `6fb6172f34aa85601ba3279e39e6c2af016a64d6360026a435d99143ac25be3e` |

### Imported link targets

The imported text is kept byte-for-byte identical, so its relative links are
resolved by importing their targets from the same commit rather than by
rewriting the links. Each target is stored under a `ortho-config-` prefixed
name in `docs/` to avoid colliding with this repository's own documents.

| Upstream path                                                 | Destination in this repository                                        | SHA-256                                                            |
| ------------------------------------------------------------- | --------------------------------------------------------------------- | ------------------------------------------------------------------ |
| `README.md`                                                   | `docs/ortho-config-readme.md`                                         | `548f267f373283b2fb142e536a8297d17a5c561fb43549da0e20a3a04dd14b1b` |
| `CHANGELOG.md`                                                | `docs/ortho-config-changelog.md`                                      | `9d1dccd18a0c5ffe0450cbec510ef412d4dfa3b6d4767e479fb13b6a625b8ec5` |
| `ortho_l10n/README.md`                                        | `docs/ortho-config-ortho-l10n-readme.md`                              | `4869853643a68425ece40b142290db5d131344aa76d1814d2bafc6fccc941c9f` |
| `docs/cargo-orthohelp-api-guide.md`                           | `docs/ortho-config-cargo-orthohelp-api-guide.md`                      | `b4b3de8186d1f0c47585d3958db7b8bdda62223d5f592008e9ce2a1a0e2e0266` |
| `docs/cargo-orthohelp-design.md`                              | `docs/ortho-config-cargo-orthohelp-design.md`                         | `145bffb414b64c34eec680bb19fd5a1e3fdb2981bcb7efd455eb61ce479f2ad5` |
| `docs/adrs/adr-0004-cargo-external-subcommand-entry-point.md` | `docs/ortho-config-adr-0004-cargo-external-subcommand-entry-point.md` | `ed50351581d046c1ad835b799902e86c9da8a8fd590520dff36d0c61d2d8d81a` |

`users-guide.md` also links `../examples/hello_world/` at directory
granularity. A directory cannot be imported as a file, so the file listing of
that example tree at this commit is recorded instead in
`docs/ortho-config-example-hello-world-files.txt`. Re-fetch any listed file with
`git -C <clone> show 38812c56:examples/hello_world/<path>`.

### Link mapping

Relative links inside the imported documents therefore resolve as follows.
`v0-6-0-migration-guide.md` and `v0-7-0-migration-guide.md` contain no relative
links; their remaining links are absolute or footnote references.

| Link written in the imported text                        | Resolves to                                                           |
| -------------------------------------------------------- | --------------------------------------------------------------------- |
| `users-guide.md`                                         | `docs/ortho-config-users-guide.md`                                    |
| `v0-9-0-migration-guide.md`                              | `docs/ortho-config-v0-9-0-migration-guide.md`                         |
| `v0-10-0-migration-guide.md`                             | `docs/ortho-config-v0-10-0-migration-guide.md`                        |
| `../CHANGELOG.md`                                        | `docs/ortho-config-changelog.md`                                      |
| `../README.md`                                           | `docs/ortho-config-readme.md`                                         |
| `../ortho_l10n/README.md`                                | `docs/ortho-config-ortho-l10n-readme.md`                              |
| `cargo-orthohelp-api-guide.md`                           | `docs/ortho-config-cargo-orthohelp-api-guide.md`                      |
| `cargo-orthohelp-design.md`                              | `docs/ortho-config-cargo-orthohelp-design.md`                         |
| `adrs/adr-0004-cargo-external-subcommand-entry-point.md` | `docs/ortho-config-adr-0004-cargo-external-subcommand-entry-point.md` |

Markdown link checkers resolve paths relative to the containing file, so a plain
`docs/`-relative link such as `users-guide.md` will not resolve from
`docs/ortho-config-users-guide.md` without the mapping above. This table is the
authoritative mapping; do not add redirect stubs, because a stub named
`docs/users-guide.md` would shadow this repository's own user's guide.

### Reproducing the import

```sh
git clone https://github.com/leynos/ortho-config.git /tmp/ortho-config
git -C /tmp/ortho-config fetch --tags
git -C /tmp/ortho-config rev-parse 'v0.10.0^{commit}'   # 38812c56...
git -C /tmp/ortho-config show 38812c56:docs/users-guide.md | sha256sum
```

[issue-237]: https://github.com/leynos/comenq/issues/237
