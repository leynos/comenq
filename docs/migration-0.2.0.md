# Migrate to 0.2.0

This guide covers the optional GitHub token pool and the foreground `await`
command introduced in 0.2.0. Existing single-token configuration remains
supported, so deployments do not need to change their token settings to upgrade.

## Configure token rotation

To rotate posts across several GitHub identities, store one Personal Access
Token (PAT) in each restricted file and replace the single-token setting with
an ordered list:

```toml
github_token_files = [
  "/etc/comenqd/tokens/pandalump",
  "/etc/comenqd/tokens/buzzybee",
]
```

Keep each file readable only by the account running `comenqd`. At startup, the
daemon reads every listed file, trims its contents, and rejects a missing,
empty, or oversized file. A leading `${VAR}` in a path expands from the daemon
environment, including systemd's `CREDENTIALS_DIRECTORY`.

The pool takes precedence over `github_token` and `github_token_file`. An
explicit `--github-token` still overrides every source; without it,
`--github-token-file` selects a one-token pool and overrides the configured
pool. The daemon selects the token after the one recorded in the latest history
entry, wrapping to the first token. If the history is empty or its hash is not
in the pool, posting starts with the first token.

No queue or history conversion is required. New history records include a
SHA-256 token hash, and `comenq hist` shows its first eight characters. Older
history records without a hash remain readable and display `--------`.

## Wait for a posted comment URL

Upgrade `comenq` and `comenqd` together to use the new streaming `await`
request. Pass the eight-character identifier printed by `put` or `list`:

```bash
comenq await 1a2b3c4d
```

The command waits through posting retries, writes countdown reports to stderr,
and prints the full GitHub comment URL to stdout after the daemon records a
successful post. Existing queue entries and history need no conversion. Older
successful history records without a comment URL remain readable, but `await`
reports an explicit error for those records because their comment URL cannot be
reconstructed.

The daemon stores deletion markers alongside the queue so a reused identifier
cannot resolve to an older post after the newer entry is deleted. These markers
are maintained automatically and are not part of `comenq hist` output.

For service setup and the complete precedence rules, see the
[users' guide](users-guide.md).
