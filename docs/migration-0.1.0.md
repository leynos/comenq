# Migrate to 0.1.0

This guide shows how to update an existing Comenq deployment for 0.1.0's
token-file configuration, user-service socket discovery, and queue-management
subcommands. Use it when moving an existing configuration to the released
binaries.

## Prerequisites

- Install the 0.1.0 `comenq` and `comenqd` binaries.
- Keep the existing configuration and queue path available for rollback.
- Upgrade `comenq` and `comenqd` together: their JSON wire protocol changed in
  0.1.0 and the old client cannot talk to the new daemon (or vice versa).
- Create a readable file containing a non-empty GitHub Personal Access Token
  (PAT), with permissions restricted to the account running `comenqd`.

## Update the token source

1. Replace an inline `github_token` with a file path in the daemon
   configuration when the token must not appear in configuration or process
   arguments.

   ```toml
   github_token_file = "/path/to/comenq-token"
   ```

   A leading `${VAR}` is expanded from the environment, so a systemd credential
   can use `github_token_file = "${CREDENTIALS_DIRECTORY}/token"`.

2. Use `--github-token-file FILE` to select a token file for one invocation.
   Without `--github-token`, this command-line path replaces any configured
   token-file path and its trimmed contents override an inline `github_token`.
   An explicit `--github-token TOKEN` remains authoritative over every token
   file, including one supplied with `--github-token-file`.

   ```bash
   comenqd --config /etc/comenqd/config.toml \
     --github-token-file /path/to/comenq-token
   ```

3. Start the daemon and inspect its status. Configuration loading fails if the
   selected token file is unreadable or has empty trimmed contents, including a
   file that contains only whitespace, or exceeds 64 KiB. Correct the file path
   or contents before retrying. When no explicit `--github-token` is supplied,
   the daemon never falls back to an inline token after selecting a token file.

## Adopt socket discovery

Keep `socket_path` when an existing deployment needs a fixed location.
Otherwise remove that setting to let a user service listen at
`$XDG_RUNTIME_DIR/comenq/comenq.sock`. A client without `--socket` or
`COMENQ_SOCKET` probes the valid per-user socket first and then the system
socket at `/run/comenq/comenq.sock` when the first connection fails.

For a complete user-service installation using `LoadCredential`, see the
[users' guide](users-guide.md). For the full configuration reference, see
[Comenq design](comenq-design.md).

## Migrate the queue and client protocol

The 0.1.0 daemon replaces the former append-only queue with a persistent store
under `<queue_path>/entries`, containing one JSON file per pending comment and a
`<queue_path>/last_post` marker. The old queue files are not imported. Before
upgrading, let the old daemon drain its queue or save any pending requests so
they can be submitted again with the new client.

After both binaries are upgraded, use the subcommands to manage the new queue:

```bash
comenq put owner/repository 123 "Please review this change"
comenq list
comenq bump 1a2b3c4d
comenq bust 1a2b3c4d
comenq del 1a2b3c4d
```

`put` reports an identifier and approximate ETA; `list` reports the pending
schedule. `bump`, `bust`, and `del` operate on the identifier returned by `put`
or `list`, and `put --now` removes the default initial cooldown floor.
