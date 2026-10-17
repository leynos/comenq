# Comenq

[![Ask DeepWiki](https://deepwiki.com/badge.svg)](
https://deepwiki.com/leynos/comenq)

Comenq is a fault-tolerant service that queues GitHub Pull Request comments. It
follows a daemon-client model: the `comenqd` daemon enforces a 16-minute
cooling-off period for posting, while the `comenq` CLI manages queued requests
and can wait for a posted comment's URL. The architecture and crate choices are
described in [docs/comenq-design.md](docs/comenq-design.md). Further guides in
the [`docs/`](docs/) directory detail testing approaches and library rationale.

## Building and testing

Use the provided `make` targets to manage the project:

- `make build` &ndash; compile a debug binary in `target/debug/` (`APP` defaults
  to `comenq`; set `APP=comenqd` to build the daemon)
- `make release` &ndash; produce optimized release binaries
- `make test` &ndash; execute the full test suite
- `make test-cov` &ndash; run workspace-wide tests with coverage and print a
  text report. Set `COV_MIN=75` to fail if line coverage drops below 75%
- `make test-cov-lcov` &ndash; run workspace-wide tests with coverage and write
  `coverage/lcov.info`. Also honours `COV_MIN`
- `make lint` &ndash; run Clippy with warnings denied
- `make fmt` &ndash; format Rust and Markdown files

## Running the binaries

After building, launch the daemon and queue comments with the client:

```bash
make build APP=comenqd
make build
./target/debug/comenqd &
./target/debug/comenq put owner/repo 123 "Comment body"
```

Queued requests persist on disk and are posted sequentially by the daemon.
`put` prints the comment's deterministic eight-character identifier and an
approximate ETA. By default, a fresh comment waits one full cooldown (plus its
flutter) from enqueue even when the queue is idle; pass `--now` to post as soon
as the queue allows. The queue can be inspected and reordered by identifier:

```bash
./target/debug/comenq list          # schedule of pending comments with IDs and ETAs
./target/debug/comenq bump 1a2b3c4d # move to the head of the queue
./target/debug/comenq bust 1a2b3c4d # move to the tail of the queue
./target/debug/comenq del 1a2b3c4d  # remove from the queue
```

The daemon also keeps a posting-history log, recording every successful and
failed posting attempt. `hist` prints it, oldest first, with the first eight
characters of the posting token's hash:

```bash
comenq hist -n 20    # the twenty most recent posting attempts
```

Use `await` when a script needs the full GitHub comment URL. It writes only
that URL to stdout, so it can be captured directly; newline-terminated ETA
countdowns go to stderr. Reports use the usual ETA format followed by
`remaining`, or `due now` at zero. While still waiting at zero ETA, that line
repeats at most once per minute. Reports use a bounded cadence. A zero exit
status means the daemon confirmed the post; a closed stdout pipe does not
change that status. Capture the URL with:

```bash
URL=$(comenq await 1a2b3c4d)
printf '%s\n' "$URL"
```

The identifier must be the eight-character hexadecimal ID shown by `put` or
`list`. An unknown ID fails before any countdown. While the comment remains
queued, `await` keeps waiting through failed post attempts, which the daemon
retries after a full cooldown. Deletion, daemon shutdown or a lost connection
ends the wait with an error. Interrupting `await` leaves the queue unchanged;
you can await the same ID later. A successful historical post without a
recorded comment URL also returns an error. Success exits with status 0,
operation failures with status 1, and invalid command-line arguments with
status 2.

## Running as a user service

The daemon also runs unprivileged under `systemd --user`. Install the binaries
to `~/.local/bin`, copy
[`packaging/linux/comenqd-user.service`](packaging/linux/comenqd-user.service)
to `~/.config/systemd/user/comenqd.service` and
[`packaging/config/comenqd-user.toml`](packaging/config/comenqd-user.toml) to
`~/.config/comenqd/config.toml`, then enable it:

```bash
systemctl --user daemon-reload
systemctl --user enable --now comenqd.service
```

The socket defaults to `$XDG_RUNTIME_DIR/comenq/comenq.sock`. The client probes
the user socket first and falls back to `/run/comenq/comenq.sock` when the
connection fails. The GitHub token is supplied through systemd's credential
system (`LoadCredential=token:%h/.config/comenqd/token` with
`github_token_file = "${CREDENTIALS_DIRECTORY}/token"`), keeping the secret out
of the unit file and process environment.

### Rotating multiple GitHub tokens

To rotate posts across several personal access tokens, set `github_token_files`
instead of `github_token` or `github_token_file`:

```toml
github_token_files = ["/home/user/pandalump-token", "/home/user/buzzybee-token"]
```

Each file must exist and contain a non-empty token; the daemon checks every
file at startup. Posts use the successor of the token in the latest posting
history record, wrapping to the first configured token at the end. A single
token continues to work as a pool of one.

See the [users' guide](docs/users-guide.md) for complete setup and
configuration details.
