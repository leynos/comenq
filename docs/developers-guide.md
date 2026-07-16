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
