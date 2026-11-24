---
type: shippable
status: done
---

# 01 — `deck stats` and the stats log line

## Scope

`deck stats` shows how much the daemon uses, and the log records it every 10 minutes. Owning crates: `deck-daemon` (platform measurement, the request, the timer), `deck-core` (protocol), `deck` (command).

## Requirements

1. **`platform::self_stats() -> SelfStats`** with `rss_bytes`, `fds`, `threads`, `cpu_user`, `cpu_system` (durations), each `Option` (a failed read gives `None`, never an error). macOS: `proc_pidinfo(getpid(), PROC_PIDTASKINFO)` for resident size and thread count, `proc_pidinfo(PROC_PIDLISTFDS)` for descriptors, `getrusage(RUSAGE_SELF)` for CPU. Linux: `/proc/self/statm` (resident pages × page size), the entries of `/proc/self/fd`, `Threads:` in `/proc/self/status`, `getrusage`. `unsafe` only in `platform/`, with `// SAFETY:` comments.
2. **`Request::Stats`** answered with `Response::Stats(Stats)`: the `SelfStats` fields, daemon pid, uptime, deck version, protocol version, counts of sessions, live terminals, kept-exited terminals and attached clients (TUI clients attached to a session, like `SessionInfo.attached_clients`; control connections, including the one asking, are not counted), and per terminal: id, session name, history lines, screen size, and total bytes read from its PTY since start (a new counter, kept by the reader thread in `terminal.rs`). `PROTOCOL_VERSION` goes up by one.
3. **`deck stats`** prints:

   ```text
   deck daemon pid 4312, up 3d 4h, version 0.4.0 (protocol 22)
   memory 84.2 MiB  fds 61  threads 38  cpu 312.4s user, 41.0s system
   2 sessions, 14 terminals (1 exited), 1 client

   terminal  session  size     history  read
   4         main     120x40   10000    48.1 MiB
   …
   ```

   `--json` prints one JSON document with the same fields (sizes in bytes, durations in seconds). Without a daemon it prints `no deck daemon running` and exits 0, starting none. It works inside deck (through `DECK_SOCKET`), like `deck ls`.

4. **Log line**: every 10 minutes, and once in the shutdown path, the daemon logs `stats: rss=<MiB> fds=<n> threads=<n> cpu=<user+system>s sessions=<n> terminals=<n> clients=<n>` at INFO (fields it could not read are left out). `DECK_TEST_STATS_LOG_MS` shortens the interval for tests, like the other `DECK_TEST_*` variables.
5. Measuring runs off the async workers (`spawn_blocking`).

## Acceptance criteria

- [ ] `self_stats` returns plausible values on the OS the test runs on: rss above 1 MiB, fds above 3, threads at least 1, CPU time non-decreasing between two calls.
- [ ] Opening 5 terminals raises `fds` and `threads` in `deck stats --json`; after closing them, polling for up to 15 s (tokio's blocking pool keeps idle threads for 10 s) sees both back within 2 of the earlier values.
- [ ] A terminal that printed 1 MiB shows at least 1 MiB in `read`.
- [ ] With `DECK_TEST_STATS_LOG_MS=200`, `daemon.log` gets `stats:` lines; `kill-server` adds a last one.
- [ ] `deck stats` with no daemon prints `no deck daemon running`, exits 0 and starts none.
- [ ] A protocol test pins `Request::Stats` and `Response::Stats`.

## Tests

- Unit tests inside `deck-daemon/src/platform/` for `self_stats` (the module is private); `crates/deck-daemon/tests/` for the request and the log line; `crates/deck/tests/` for the command and its output.

## Notes

- The worker states which OS it ran the platform tests on; the lead runs them on the other one before merge when possible.
