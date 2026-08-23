# Migrate to 0.1.0

This guide shows how to update an existing Comenq deployment for 0.1.0's
token-file configuration and user-service socket discovery. Use it when moving
an existing configuration to the released binaries; it does not move or delete
an existing queue.

## Prerequisites

- Install the 0.1.0 `comenq` and `comenqd` binaries.
- Keep the existing configuration and queue path available for rollback.
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
