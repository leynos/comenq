# Migrate to 0.4.0

This guide covers environment handling in `comenq` and `comenqd`: the client's
socket override, the daemon's configuration precedence, and the injected
environment entry points for library consumers.

## Socket overrides

`COMENQ_SOCKET` now accepts a path that is not valid UTF-8. The client reads
the variable through OS-string support, so a socket path containing raw bytes
is honoured instead of being treated as unset. Earlier releases read the
variable as Unicode and silently fell back to socket discovery, which could
connect to a different daemon.

## Resolve the socket when the command runs

The client no longer binds `COMENQ_SOCKET` through its argument parser. The
variable is read when the command runs, so `--help` no longer shows an
`[env: COMENQ_SOCKET=]` annotation for `--socket`. Precedence is unchanged:
`--socket` wins over a non-empty `COMENQ_SOCKET`, which wins over discovery. An
empty value is treated as unset.

## Precedence and case in daemon configuration

Environment overrides keep matching `COMENQD_*` names without regard to ASCII
case. When an installation exports several casings of one name, the documented
upper-case spelling supplies the value, so the result no longer depends on
enumeration order. Precedence remains command-line flag > environment >
configuration file > built-in default. The [users' guide](users-guide.md) lists
the recognised variables.

## Library entry points

Programs that embed these crates can inject configuration instead of mutating
the process environment:

- `comenq::run_with_env` and `Args::socket_candidates_with_env` accept a
  `mockable::Env` provider.
- `comenqd::config::Config::github_tokens_with_env` resolves the token pool
  with the supplied provider.
- `comenq::ClientError` is re-exported at the crate root.

The existing `comenq::run` and `Config::load` entry points keep reading the
process environment.

For service setup and the full command behaviour, see the
[users' guide](users-guide.md).
