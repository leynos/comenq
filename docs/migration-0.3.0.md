# Migrate to 0.3.0

This guide covers the foreground `await` command and its daemon streaming
protocol.

## Install the matching client and daemon

Upgrade `comenq` and `comenqd` together before using `await`. The command sends
a streaming request that older daemons do not recognize. Existing one-shot
commands keep their request and response behaviour.

## Wait for a posted comment URL

Pass the eight-character identifier printed by `put` or `list`:

```bash
comenq await 1a2b3c4d
```

The command waits through posting retries, writes countdown reports to stderr,
and prints the full GitHub comment URL to stdout after the daemon records a
successful post. Interrupting or disconnecting the client leaves queue
processing unchanged; a later `await` can still return the URL.

## Queue and history compatibility

No manual queue or history conversion is required. Startup migrates lifetime
metadata for older queued entries before the worker claims them. The daemon
maintains deletion markers automatically so a reused identifier cannot reveal
an older comment URL after the newer entry is deleted.

Older successful history records remain readable but do not contain a comment
URL. `await` reports an explicit error for those records because their URL
cannot be reconstructed. New successful posts persist the URL with the
completion record.

For service setup and the full command behaviour, see the
[users' guide](users-guide.md).
