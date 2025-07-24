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

- [ ] Implement the CLI argument parsing using `clap`'s derive macro to define
  the `Args` struct (`repo_slug`, `pr_number`, `comment_body`, `socket`).

- [ ] Add validation for the `owner/repo` slug format.

- [ ] Implement the client's `main` function to connect to the daemon's Unix
  Domain Socket using `tokio::net::UnixStream`.

- [ ] Serialize the `CommentRequest` payload to JSON and write it to the socket
  stream.

- [ ] Implement robust error handling and user feedback for connection failures
  or serialization errors.

## Milestone 3: `comenqd` Daemon Core

- [ ] Implement configuration loading from a TOML file
  (`/etc/comenqd/config.toml`) for parameters like `github_token`,
  `socket_path`, and `queue_path`.

- [ ] Set up structured logging using the `tracing` and `tracing-subscriber`
  crates.

- [ ] Initialize the `yaque` persistent queue at the path specified in the
  configuration.

- [ ] Structure the daemon's `main` function to spawn the two primary,
  long-running `tokio` tasks: the UDS listener and the queue worker.

## Milestone 4: `comenqd` Daemon - UDS Listener Task

- [ ] Implement the `run_listener` async task.

- [ ] Bind a `tokio::net::UnixListener` to the configured socket path, ensuring
  any stale socket file is removed first.

- [ ] Set the socket file permissions to `0o660` to enforce the security model.

- [ ] Create an acceptance loop (`listener.accept().await`) that spawns a new
  task for each incoming client connection.

- [ ] Implement the `handle_client` task to read the JSON payload, deserialize
  it into a `CommentRequest`, and enqueue it using the `yaque` sender.

## Milestone 5: `comenqd` Daemon - Queue Worker Task

- [ ] Implement the `run_worker` async task.

- [ ] Initialize the `octocrab` client with the GitHub PAT from the
  configuration.

- [ ] Create the main worker loop that dequeues jobs one at a time using
  `yaque`'s transactional `receiver.recv().await`.

- [ ] Implement the logic to post a comment to GitHub using the correct
  `octocrab` method: `issues().create_comment()`.

- [ ] On successful API post, explicitly commit the job using `guard.commit()`.

- [ ] On API failure, log the error and allow the `RecvGuard` to be dropped,
  automatically requeuing the job.

- [ ] After processing each job (successfully or not), enforce the 15-minute
  (900 seconds) cooling-off period using `tokio::time::sleep`.

## Milestone 6: Deployment and Operationalization

- [ ] Write an installation script (`install.sh`) to compile release binaries
  and place them in standard system locations (`/usr/local/bin`,
  `/usr/local/sbin`).

- [ ] The script should create a dedicated system user (`comenq`) and the
  necessary directories (`/etc/comenqd`, `/var/lib/comenq`, `/run/comenq`) with
  secure permissions.

- [ ] Create a `systemd` service unit file (`comenq.service`) for the daemon.

- [ ] Configure the service to run as the `comenq` user, restart on failure,
  and include security hardening directives (`ProtectSystem`, `PrivateTmp`,
  etc.).

- [ ] Document the process for securely creating the configuration file and
  setting its permissions (`chmod 640`).

- [ ] Update the `README.md` and other documentation to reflect the final
  implementation and usage instructions.
