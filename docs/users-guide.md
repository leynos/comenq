# Users' guide

Comenq queues GitHub pull request comments through a local daemon. The
`comenqd` daemon stores requests, posts them in order, and enforces the
configured cooldown. The `comenq` client submits requests over a Unix domain
socket.

## Run Comenq as a user service

Build the release binaries and install them, the packaged user unit, and its
matching example configuration:

```bash
make release
install -Dm755 target/release/comenq ~/.local/bin/comenq
install -Dm755 target/release/comenqd ~/.local/bin/comenqd
install -Dm644 packaging/linux/comenqd-user.service \
  ~/.config/systemd/user/comenqd.service
install -Dm600 packaging/config/comenqd-user.toml \
  ~/.config/comenqd/config.toml
```

The packaged unit expects the GitHub Personal Access Token (PAT) at
`~/pandalump-token`. Create that file without exposing the token in shell
history, and restrict it to the account running the service:

```bash
install -m600 /path/to/token ~/pandalump-token
systemctl --user daemon-reload
systemctl --user enable --now comenqd.service
systemctl --user status comenqd.service
```

For a manual daemon launch, `comenqd` creates missing parent components for its
socket path and sets each component it creates to mode `0700`. Existing parent
directories retain their current modes, so this does not alter a
systemd-provisioned `RuntimeDirectory`. The packaged user unit stores the
persistent queue at `~/.local/state/comenq/queue`.

Change the source path in `LoadCredential` if the token is stored elsewhere.
The unit uses `LoadCredential=token:%h/pandalump-token`; systemd makes the
credential available as `${CREDENTIALS_DIRECTORY}/token`, which the example
configuration selects through `github_token_file`. This keeps the PAT out of
the unit and process environment.

Credential values follow the daemon's configuration precedence. An explicit
`--github-token` value wins over every other source. If that option is absent,
`--github-token-file` overrides the `github_token_file` path supplied by
`COMENQD_GITHUB_TOKEN_FILE` or the configuration file; the selected file's
trimmed contents then override `github_token`. If no file is selected, the
inline token from the CLI, environment, or configuration file is used. Startup
fails when no token source is configured, when the selected token file cannot
be read, or when its trimmed contents are empty (including a whitespace-only
file). Token files larger than 64 KiB are also rejected.

To select a token file explicitly, use the `--github-token-file FILE` form:

```bash
comenqd --config /etc/comenqd/config.toml \
  --github-token-file /run/credentials/comenqd/token
```

The file path overrides any configured token-file path; `--github-token` still
wins if both options are supplied.

## Connect the client

For a user service, `RuntimeDirectory=comenq` creates
`$XDG_RUNTIME_DIR/comenq`, and the daemon's default socket is
`$XDG_RUNTIME_DIR/comenq/comenq.sock`. Without an explicit socket override, the
client probes that socket first when `XDG_RUNTIME_DIR` contains a non-empty
absolute path. It falls back to `/run/comenq/comenq.sock` when the user socket
cannot be used, so a stale user socket does not hide a healthy system service.

Use `--socket PATH` or `COMENQ_SOCKET=PATH` to select exactly one socket
instead:

```bash
comenq owner/repository 123 "Please review this change"
comenq --socket /run/comenq/comenq.sock owner/repository 123 "Queued"
```

## Inspect local metrics

The daemon attempts to expose Prometheus metrics at
`http://127.0.0.1:9000/metrics`. This endpoint listens only on the local
loopback interface, so it is not reachable from other hosts. Scrape it from the
machine running `comenqd`:

```bash
curl http://127.0.0.1:9000/metrics
```

The stable metric names and labels are:

- `comenqd_task_restarts_total{task=listener|worker|writer}` for supervised
  task restarts.
- `comenqd_queue_writer_failures_total{queue_side=sender}` for queue-writer
  failures.
- `comenqd_client_channel_depth` for the bounded client-channel depth proxy.
- `comenqd_requests_total{outcome=accepted|rejected}` for request outcomes.
- `comenqd_cooldown_wait_duration_seconds` for cooldown wait durations.
- `comenqd_github_posts_total{outcome=success|api_error|timeout}` for GitHub
  comment-post outcomes.
- `comenqd_github_post_duration_seconds` for GitHub comment-post durations.

## Configure the cooldown

`cooldown_period_seconds` sets the minimum delay after a comment is posted. It
defaults to 960 seconds. The daemon's `--cooldown-period-seconds` option takes
precedence over the environment and configuration file.

`cooldown_flutter_seconds` adds a fresh uniformly random delay from zero up to
the configured number of seconds to every cooldown. Flutter only lengthens the
wait; zero disables it. Configure flutter in the TOML file or through
`COMENQD_COOLDOWN_FLUTTER_SECONDS`.

For the complete configuration model and service architecture, see
[Comenq design](comenq-design.md). For an existing deployment, see
[Migrate to 0.1.0](migration-0.1.0.md).
