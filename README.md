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
- `make release` &ndash; produce optimised release binaries
- `make test` &ndash; execute the full test suite
- `make test-cov` &ndash; run tests with coverage and print a text report.
  Pass `COV_MIN=75` to fail if line coverage drops below 75%
- `make test-cov-lcov` &ndash; run tests with coverage and write
  `coverage/lcov.info`. Also honours `COV_MIN`
- `make lint` &ndash; run Clippy with warnings denied
- `make fmt` &ndash; format Rust and Markdown files

## Running the binaries

After building, launch the daemon and queue comments with the client:

```bash
make build
./target/debug/comenqd &
./target/debug/comenq owner/repo 123 "Comment body"
```

Queued requests persist on disk and are posted sequentially by the daemon.
