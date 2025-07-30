# Implementation Roadmap

## Milestone 1: Project Scaffolding and Shared Library

- [x] Set up the Rust workspace with two binary crates (`comenq`, `comenqd`)
  and a shared library crate (`comenq-lib`), as outlined in the project
  structure.

- [x] Define the shared `CommentRequest` struct in the library crate, deriving
  `serde::Serialize` and `serde::Deserialize` for IPC.

- [x] Populate the root `Cargo.toml` with the specified workspace dependencies
  (`tokio`, `clap`, `serde`, `octocrab`, `yaque`, etc.).

## Milestone 2: `comenq` CLI Client

- [x] Implement the CLI argument parsing using `clap`'s derive macro to define
  the `Args` struct (`repo_slug`, `pr_number`, `comment_body`, `socket`).

- [x] Add validation for the `owner/repo` slug format.

- [x] Implement the client's `main` function to connect to the daemon's Unix
  Domain Socket using `tokio::net::UnixStream`.

- [x] Serialize the `CommentRequest` payload to JSON and write it to the
  socket stream.

- [x] Implement robust error handling and user feedback for connection
  failures or serialization errors.

## Milestone 3: `comenqd` Daemon Core

- [x] Implement configuration loading from a TOML file (done)
  (`/etc/comenqd/config.toml`) for parameters like `github_token`,
  `socket_path`, and `queue_path`.

- [x] Set up structured logging using the `tracing` and `tracing-subscriber`
  crates.

- [x] Initialize the `yaque` persistent queue at the path specified in the
  configuration.

- [x] Structure the daemon's `main` function to spawn the two primary,
  long-running `tokio` tasks: the UDS listener and the queue worker.

## Milestone 4: `comenqd` Daemon — UDS Listener Task

- [x] Implement the `run_listener` async task.

- [x] Bind a `tokio::net::UnixListener` to the configured socket path, ensuring
  any stale socket file is removed first.

- [x] Set the socket file permissions to `0o660` to enforce the security model.

- [x] Create an acceptance loop (`listener.accept().await`) that spawns a new
  task for each incoming client connection.

- [x] Implement the `handle_client` task to read the JSON payload, deserialize
  it into a `CommentRequest`, and enqueue it using the `yaque` sender.

## Milestone 5: `comenqd` Daemon — Queue Worker Task

- [x] Implement the `run_worker` async task.

- [x] Initialize the `octocrab` client with the GitHub PAT from the
  configuration.

- [x] Create the main worker loop that dequeues jobs one at a time using
  `yaque`'s transactional `receiver.recv().await`.

- [x] Implement the logic to post a comment to GitHub using the correct
  `octocrab` method: `issues().create_comment()`.

- [x] On successful API post, explicitly commit the job using `guard.commit()`.

- [x] On API failure, log the error and allow the `RecvGuard` to be dropped,
  automatically requeuing the job.

- [x] After processing each job (successfully or not), enforce the 16-minute
  (960 seconds) cooling-off period using `tokio::time::sleep`.

## Milestone 6: Automated Cross-Platform Packaging and Release

This milestone seeks to produce native packages for major Linux distributions
and macOS, simplifying installation and improving security and maintainability.

- [ ] **Implement Declarative Packaging with GoReleaser**

  - [x] Create a comprehensive `.goreleaser.yaml` configuration to define Linux
    build, packaging, and release process for both `comenq` and `comenqd`.

  - [x] Use GoReleaser's custom builder hooks to integrate the `cargo build`
    process for the Rust binaries.

- [ ] **Package for Linux Distributions (Fedora & Ubuntu)**

  - [x] Create a hardened `systemd` service unit file (`comenqd.service`) for
    the daemon, incorporating security best practices (`ProtectSystem`,
    `PrivateTmp`, `NoNewPrivileges`, etc.).

  - [x] Author `preinstall`, `postinstall`, and `preremove` scripts to be
    embedded in the packages. These will handle the creation of the dedicated
    `comenq` system user and manage the `systemd` service lifecycle.

  - [x] Configure GoReleaser's `nfpms` section to build and sign `.rpm` and
    `.deb` packages.

- [ ] **Automate the Release Workflow**

  - [ ] Implement a GitHub Actions workflow that triggers on new version tags
    (e.g., `v*`).

  - [ ] The workflow will orchestrate the entire release: checking out the
    code, installing dependencies, and executing GoReleaser.

  - [ ] GoReleaser will then build the binaries, create all packages, publish
    the Homebrew formula, generate a changelog from git history, and upload all
    assets to a draft GitHub Release.

- [ ] **Update Public Documentation**

  - [ ] Revise the `README.md` to feature the new, simplified installation
    instructions using `apt` and `dnf`

  - [ ] Add a new document to the `/docs` directory detailing the automated
    packaging process for future maintainers and contributors.
