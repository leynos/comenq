# Comenq

Comenq is a fault-tolerant service that queues GitHub Pull Request comments. It
follows a daemon-client model: the `comenqd` daemon enforces a 16-minute
cooling-off period for posting, while the `comenq` CLI simply enqueues
requests. The architecture and crate choices are described in
[docs/comenq-design.md](docs/comenq-design.md). Further guides in the
[`docs/`](docs/) directory detail testing approaches and library rationale.

## Building and testing

Use the provided `make` targets to manage the project:

- `make build` &ndash; compile debug binaries in `target/debug/`
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
make build
./target/debug/comenqd &
./target/debug/comenq put owner/repo 123 "Comment body"
```

Queued requests persist on disk and are posted sequentially by the daemon.
`put` prints the comment's deterministic eight-character identifier and an
approximate ETA. The queue can be inspected and reordered by identifier:

```bash
comenq list          # schedule of pending comments with IDs and ETAs
comenq bump 1a2b3c4d # move to the head of the queue
comenq bust 1a2b3c4d # move to the tail of the queue
comenq del 1a2b3c4d  # remove from the queue
```

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

The socket defaults to `$XDG_RUNTIME_DIR/comenq/comenq.sock` and the client
discovers whichever daemon (user or system) is running. The GitHub token is
supplied through systemd's credential system
(`LoadCredential=token:%h/pandalump-token` with
`github_token_file = "${CREDENTIALS_DIRECTORY}/token"`), keeping the secret out
of the unit file and process environment.
