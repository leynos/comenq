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
`~/.config/comenqd/token`. Create that file without exposing the token in shell
history, and restrict it to the account running the service:

```bash
install -m600 /path/to/token ~/.config/comenqd/token
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
The unit uses `LoadCredential=token:%h/.config/comenqd/token`; systemd makes
the credential available as `${CREDENTIALS_DIRECTORY}/token`, which the example
configuration selects through `github_token_file`. This keeps the PAT out of
the unit and process environment.

Credential values follow the daemon's configuration precedence. An explicit
`--github-token` value wins over every other source. Without it,
`--github-token-file` selects one file and clears any configured rotation pool.
Otherwise, a non-empty `github_token_files` list takes precedence over
`github_token_file` and `github_token`; the daemon reads those files in list
order and rotates posts through them. If the list is empty, `github_token_file`
overrides the inline token. A selected pool or file must be readable and
contain non-empty trimmed contents. Every token file is limited to 64 KiB.
Leading `${VAR}` placeholders in file paths expand from the daemon environment,
including paths under systemd's `CREDENTIALS_DIRECTORY`.

To rotate posts across multiple identities, store one PAT in each restricted
file and list the files in the order the daemon must use them:

```toml
github_token_files = [
  "/home/user/pandalump-token",
  "/home/user/buzzybee-token",
]
```

The latest posting-history record determines the next token, including after a
restart. An empty or unreadable history, or a hash that no longer matches the
configured pool, starts with the first token. Existing single-token settings
continue to act as a pool of one.

To select a token file explicitly, use the `--github-token-file FILE` form:

```bash
comenqd --config /etc/comenqd/config.toml \
  --github-token-file /run/credentials/comenqd/token
```

The file path overrides configured single-token paths and a token pool;
`--github-token` still wins if both CLI options are supplied.

## Connect the client

For a user service, `RuntimeDirectory=comenq` creates
`$XDG_RUNTIME_DIR/comenq`, and the daemon's default socket is
`$XDG_RUNTIME_DIR/comenq/comenq.sock`. Without an explicit socket override, the
client probes that socket first when `XDG_RUNTIME_DIR` contains a non-empty
absolute path. It falls back to `/run/comenq/comenq.sock` when the user socket
cannot be used, so a stale user socket does not hide a healthy system service.

Use `--socket PATH` or `COMENQ_SOCKET=PATH` to select exactly one socket
instead. Otherwise, use the queue-management subcommands:

```bash
comenq put owner/repository 123 "Please review this change"
comenq put --now owner/repository 123 "Post as soon as possible"
comenq list
comenq bump 1a2b3c4d
comenq bust 1a2b3c4d
comenq del 1a2b3c4d
comenq await 1a2b3c4d
comenq hist --limit 20
```

`put` prints a deterministic eight-character identifier and an approximate ETA.
`list` shows pending entries in posting order with their identifiers, ETAs,
targets, and one-line summaries. `bump` and `bust` move an entry to the head or
tail, and `del` removes it. The `--now` option lifts the default one-cooldown
delay for a fresh `put`; it does not bypass an earlier queued entry's schedule.

`list` summaries are at most 60 characters long. Control characters, including
line breaks, become spaces; longer summaries end with an ellipsis. If a GitHub
post fails, its entry remains queued and the worker retries it after a full
configured cooldown. A successful retry removes the entry from the queue.

`hist` shows posting attempts oldest first, including failed attempts that will
be retried. Each row includes the entry identifier, attempt age, outcome, the
first eight characters of the posting token's SHA-256 hash, repository target
and a one-line body summary; failures also show the error description. Older
records without a token hash display `--------`. Pass `-n` or `--limit` to show
only the most recent attempts while keeping chronological order. The daemon
stores attempts as JSON Lines in `<queue_path>/history.jsonl`. A history write
failure is logged and does not stop retries; unreadable lines are skipped when
the history is read.

The queue accepts at most 1,024 entries and 32 MiB of accounted data.
Accounting includes each regular entry file's bytes plus its remaining mutation
reservation. New puts reserve up to 80 bytes each; for existing entries, the
remaining reservation is derived from the serialized widths of `order`,
`claim_token`, and a missing `incarnation` field. Migrating a legacy entry
reserves 57 bytes for the incarnation UUID and is rejected before migration if
that space is unavailable. Claim, ordering and incarnation updates consume the
headroom. A new `put` is rejected if its data would exceed the per-entry limit
of 2 MiB minus the 80-byte reservation, or if it would exceed the aggregate
budget. The daemon refuses to start if existing regular entry files exceed the
count or if their raw bytes plus remaining reservations exceed the aggregate
limit. It also refuses startup if a valid entry lacks room under the 2 MiB file
cap for every reachable order, claim-token or incarnation update. It leaves
files untouched; remove or repair the excess data before restarting. Reads
accept files up to 2 MiB; oversized or corrupt files are logged and skipped,
but their bytes still count towards the budget. `list` returns an error, not a
partial result, if its complete response would exceed the 2 MiB client/daemon
response limit.

### Wait for a posted comment URL

Use `await` to keep a foreground client connected until a queued comment is
posted. The command requires the eight-character hexadecimal ID printed by
`put` or `list`:

```bash
comenq await 1a2b3c4d
```

After the daemon confirms the post, `await` writes the full GitHub comment URL,
including the comment fragment, as its only stdout line. If the caller has
closed stdout, `await` ignores the broken pipe and still exits successfully
after daemon confirmation, though the caller will not receive the URL. This
makes the URL safe to capture in a script; progress countdowns are
newline-terminated lines on stderr:

```bash
URL=$(comenq await 1a2b3c4d)
printf '%s\n' "$URL"
```

The command reports the first ETA immediately, using the same duration format as
`put` and `list` followed by `remaining`. A zero ETA is `due now`. It updates
the countdown on a bounded cadence, shortened to the remaining ETA and never
less than one second:

| Remaining ETA                 | Countdown interval |
| ----------------------------- | ------------------ |
| Up to 60 seconds              | 10 seconds         |
| 61 seconds to 10 minutes      | 30 seconds         |
| 10 minutes 1 second to 1 hour | 2 minutes          |
| 1 hour 1 second to 6 hours    | 10 minutes         |
| More than 6 hours             | 30 minutes         |

An unknown identifier fails before any countdown. A failed GitHub posting
attempt leaves its entry queued; the daemon retries it after a full cooldown,
and `await` continues waiting through the retry. Deleting the entry, losing the
connection, or shutting down the daemon ends the wait with an error. A
successful historical post with no recorded comment URL also returns an
explicit error. Interrupting or disconnecting `await` does not change the
queue; a later `await` on the same ID can still return the URL.

`await` exits with status 0 after the daemon confirms the post, status 1 on a
daemon, queue, or connection error, and status 2 when the command line is
invalid.

## Inspect local metrics

The daemon attempts to expose Prometheus metrics at
`http://127.0.0.1:9000/metrics`. This endpoint listens only on the local
loopback interface, so it is not reachable from other hosts. Scrape it from the
machine running `comenqd`:

```bash
curl http://127.0.0.1:9000/metrics
```

The stable metric names and labels are:

- `comenqd_task_restarts_total{task=listener|worker}` for supervised
  task restarts.
- `comenqd_requests_total` with bounded `operation` labels (`put`, `list`,
  `bump`, `bust`, `del`, `hist`, `await`) and `outcome` labels. Ordinary
  operations use `accepted`, `failed`, or `rejected`; `await` uses the fixed
  terminal outcomes listed below and bounded store-error categories. Requests
  without a parsed operation omit the `operation` label.
- `comenqd_protocol_transaction_duration_seconds` measures complete listener
  transactions. Its bounded `operation` labels are `put`, `list`, `bump`,
  `bust`, `del`, `hist`, `await`, and `unknown`. `outcome` is `accepted`,
  `failed`, `rejected`, or `unknown` for ordinary operations. Await terminal
  outcomes are `posted`, `unknown_identifier`, `removed`, `rejected_capacity`,
  `shutdown`, `client_gone`, `posted_without_url`, and `posted_url_too_large`,
  plus bounded store-error categories. `error_kind` uses fixed classifications
  and is `none` on success.
- `comenqd_queue_store_operation_duration_seconds` measures queue-store
  operations. Its bounded `operation` labels are `put`, `list`, `bump`, `bust`,
  `del`, `hist`, `history`, `append_history`, `latest_history_token`,
  `next_due`, `claim`, `complete`, `resolve_await`, `release`, `recover`,
  `metrics`, and `unknown`; `outcome` is `success`, `failure`, or `unknown`.
  `error_kind` uses fixed classifications and is `none` on success.
- These histograms never use request values, paths, identifiers, payloads,
  tokens, or raw errors as labels.
- `comenqd_queue_entries` for the current pending-entry count, without labels.
- `comenqd_queue_bytes` for persisted entry-record bytes plus reserved
  mutation headroom, without labels.
- `comenqd_cooldown_wait_duration_seconds` for cooldown wait durations.
- `comenqd_github_posts_total{outcome=success|api_error|timeout}` for GitHub
  comment-post outcomes.
- `comenqd_github_post_duration_seconds` for GitHub comment-post durations.

## Configure the cooldown

`cooldown_period_seconds` sets the minimum delay after a comment is posted. It
defaults to 960 seconds and is used in the ETA reported by `put` and `list`.
The daemon's `--cooldown-period-seconds` option takes precedence over the
environment and configuration file.

`cooldown_flutter_seconds` sets the maximum uniformly random delay sampled when
each comment is enqueued. The sampled flutter is stored with that entry and
only lengthens its cooldown-derived ETA; zero disables it. Configure flutter in
the TOML file or through `COMENQD_COOLDOWN_FLUTTER_SECONDS`.

For the complete configuration model and service architecture, see
[Comenq design](comenq-design.md). For an existing deployment, see
[Migrate to 0.1.0](migration-0.1.0.md) and
[Migrate to 0.2.0](migration-0.2.0.md).
