---
type: shippable
status: done
---

# 02 — Working directory tracking

## Scope

deck knows where each shell is. After `cd` in a shell and a daemon restart, the shell comes back in that directory, and a new terminal in a panel opens where that panel's focused terminal is. Owning crates: `deck-daemon` (tracking, platform queries), `deck-core` (OSC 7 parser).

## Requirements

1. **OSC 7** scanner in `deck-core::osc7`: a streaming scanner fed the PTY output chunks (sequences may be split across chunks) that yields paths from `ESC ] 7 ; file://<host>/<path> (BEL | ESC \)`, percent-decoded. It accepts a host that is empty, `localhost`, or the machine's hostname (passed in), compared case-insensitively, either in full or as its short name (before the first `.`), so `Mac.local` and `mac` both match. It yields only paths that decode to valid UTF-8, are absolute and contain no NUL; others are ignored. It buffers at most 4 KiB of one sequence; a longer or unterminated sequence is dropped. The bytes still go to the emulator unchanged.
2. **OS query** in `deck-daemon::platform`: `process_cwd(pid) -> Option<PathBuf>`: `readlink /proc/<pid>/cwd` on Linux, `proc_pidinfo(PROC_PIDVNODEPATHINFO)` on macOS (declare the struct locally if `libc` does not expose it). It returns `None` on any failure and for paths that are not valid UTF-8. The pid is the terminal's own child process (the shell or the explicit program, from portable-pty), never the PTY's foreground process group.
3. **Tracking**: a terminal's `cwd` is updated from OSC 7 immediately. A terminal that has ever reported OSC 7 is never polled again. For the others, one tick every 5 s queries `process_cwd` for all of them in one `spawn_blocking` call. A cwd that differs from the stored one marks the state dirty (sub-spec 01's save); an unchanged one does not. The interval can be shortened for tests with the hidden environment variable `DECK_TEST_CWD_POLL_MS`, read by the daemon at start.
4. **Uses**: the saved and restored cwd; `NewTerminal` starts in the cwd of the target panel's focused terminal (falling back to that terminal's launch directory, then the project root, taking the first that exists); `SessionView` carries each terminal's cwd (DECK-05 prints it).
5. **Cost**: the query never runs on the async worker threads, a tick covers every terminal in one blocking call, and frame delivery continues while a query is slow.

## Acceptance criteria

- [ ] The OSC 7 scanner yields `/tmp/a b` from `ESC]7;file://localhost/tmp/a%20b BEL`, split at every byte boundary; accepts the hostname in other case and as a short name; ignores a foreign host, a relative path, a path decoding to invalid UTF-8 or containing NUL, and an unterminated 5 KiB sequence (after which it still finds the next valid one).
- [ ] With a shell that emits OSC 7 (`printf '\033]7;file://localhost%s\007' "$PWD"` from its prompt command), `cd /tmp` then restart restores the shell in `/tmp`.
- [ ] With `/bin/sh` and no OSC 7, and `DECK_TEST_CWD_POLL_MS=200`, `cd /tmp` is picked up (no sooner than one poll, within 5 s) and survives a restart. The test runs on the OS of the machine (macOS or Linux).
- [ ] Running `(cd /var && sleep 5)` in a tracked shell sitting in `/tmp` keeps the terminal's cwd at `/tmp`.
- [ ] A shell that reported OSC 7 once and then runs a long command in another directory keeps its OSC 7 cwd.
- [ ] `⌥Space t` after `cd /tmp` in the focused terminal opens the new terminal in `/tmp` (`pwd`).
- [ ] `process_cwd` of a child started in `/tmp` returns `/tmp` (daemon test).
- [ ] With OSC 7, a cwd change made 2 s before `kill -9` of the daemon is restored.
- [ ] With a fake `process_cwd` that blocks until the test releases it (test hook in the daemon), an attached client still receives frames for output produced during the query (daemon test; no wall-clock bound).
- [ ] A tick with no cwd change does not trigger a save.

## Tests

- `deck-core`: OSC 7 scanner table, split-chunk test, limits.
- `crates/deck-daemon/tests/`: `process_cwd` on the current OS, tracking updates and precedence, no dirty mark for unchanged values, slow-query frame delivery, `NewTerminal` cwd.
- `crates/deck/tests/persistence.rs`: the end-to-end criteria above.

## Notes

- The worker states in its report which OS it ran the platform tests on; the lead runs them on the other OS before merge when possible, else lists it as not verified.
- Compare paths after `std::fs::canonicalize` in tests: on macOS `/tmp` is a symlink to `/private/tmp`, and the OS query returns the resolved path.
