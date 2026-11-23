---
type: foundation
status: todo
---

# 02 — The handoff

## Scope

The daemon hands itself over to a new binary on `SIGUSR2`: it freezes, writes its runtime state, keeps the fds the new image needs, execs it, and the new image adopts everything. Owning crates: `deck-daemon` (state, quiesce, adoption, the exec in `platform/`), `deck` (`deck daemon --adopt`, `deck daemon --handoff-check`, signal setup). [03](03-upgrade-command.md) adds the command that triggers it.

## Requirements

### Marker and trigger

1. **Marker** `<runtime>/deck.handoff` (0600), rewritten atomically (temp name, rename), with lines `handoff <HANDOFF_VERSION>`, `pid <pid>`, `generation <n>`, `terminals <n>` and, during a handoff, `upgrading`. `HANDOFF_VERSION` starts at 1 and lives in `deck-daemon`. `generation` starts at 0 and grows by one with each adoption. The daemon installs its `SIGUSR2` handler (next to the others in `daemon_cmd.rs`) before it first writes the marker, and removes the marker at the end of a normal shutdown.
2. **Trigger**: `SIGUSR2` makes the daemon read `<runtime>/deck.upgrade` and delete it. The file holds one absolute path. The daemon refuses when the file is missing, the path is not absolute, not a regular file, not executable, or not owned by the daemon's uid or by root.
3. **Preflight**: `<exe> daemon --handoff-check <HANDOFF_VERSION>` with a 10 s timeout; `deck daemon --handoff-check <n>` exits 0 when `1 <= n <=` its own `HANDOFF_VERSION`, else prints why and exits 1. A non-zero exit or a timeout refuses.
4. **Refusal and failure report**: a refusal (2, 3) or a rollback (11) writes `<runtime>/deck.upgrade.result` (0600) with one line `refused: <reason>` or `failed: <reason>`, logs `upgrade refused: <reason>` or `upgrade failed: <reason>`, sends an error notice, and leaves the daemon as it was.

### Quiesce

5. **Signals during the handoff**: from the start of quiesce, `SIGTERM`, `SIGINT`, `SIGHUP` and a second `SIGUSR2` are not acted on by the old image: a termination signal is recorded and written into the handoff file (the new image then shuts down cleanly after adopting) or acted on after a rollback; a second `SIGUSR2` is dropped (its client times out). Right before `execve`, the exec'ing thread blocks those four signals (`pthread_sigmask`); the mask survives `execve` and a signal arriving in that window stays pending until the new image has installed its handlers and unblocks them. The exec is a `libc::execve` call in `platform/` (std's `exec` resets the signal mask). A signal taken by another thread of the old image between writing the handoff file and the exec is lost; that window is accepted and documented.
6. **Freeze**, in this order:
   - write `upgrading` into the marker, stop accepting connections (the listener stays open; connections wait in its backlog);
   - refuse new actions, requests that open terminals, and `launch_restored` steps (`the daemon is upgrading`), and wait up to 2 s for spawns already in flight to finish;
   - queue `Bye` to every client (`ServerShutdown` here; [03](03-upgrade-command.md) makes it `Upgrading`) and flush the writers as at shutdown (`WRITER_FLUSH`);
   - wait up to 500 ms for each terminal's queued input to be written to its PTY, then drop the rest with one log line per terminal;
   - pause every reader ([01](01-snapshot-adopt.md)) and every wait thread: a wait thread stops calling `waitpid` and hands over any exit status it already reaped;
   - stop the cwd, agent, config and stats tasks;
   - from here the model does not change; take each terminal's snapshot through `with_emu`.
7. **Save**: write `state.json` synchronously (`persist::write`, not the shutdown path), so a failed adoption restores the current layout.

### Exec and adoption

8. **Handoff file**: an unlinked 0600 temp file in the runtime directory, JSON, with `handoff_version`, the old protocol version, `next_id`, every session's model (as `state.json` stores it, plus each terminal's title, cwd, `exit_status`, agent state and `keep`), each session's environment, `last_attached`, pending notices, the table of recent exit statuses used by `wait --exit`, recorded termination signals, and per terminal: its id, pid, master fd number, size, whether it reports its cwd, an exit status already reaped, and its snapshot (base64). Also the fd numbers of the listener and both lock files. Kept terminals whose program exited carry no fd or pid.
9. **Exec**: the marker keeps saying `upgrading` through the exec (clients never signal a daemon whose marker says so, and wait instead; the new image rewrites it). As the last steps, clear `FD_CLOEXEC` on each terminal's master, the listener, the two lock fds and the handoff file, block the signals (5), and `execve` the new binary with `daemon --adopt <fd>` and the daemon's environment minus `DECK_TEST_PROTOCOL_VERSION` (an upgrade always moves to the new binary's own version; `DECK_TEST_PARENT_PID` and the other test variables stay). fd 2 (the log) is inherited as today; the reader and writer dups are CLOEXEC and close by themselves.
10. **Adopt** (`deck daemon --adopt <fd>`): skips log rotation and the stderr redirect (fd 2 already is the log; an adopt failure goes there) and the lock and bind steps. It:
    - first sets `FD_CLOEXEC` again on every inherited fd;
    - checks each inherited lock fd is the file on disk (`fstat` dev and inode against `deck.lock` and `state.lock`) and the listener is a listening Unix socket;
    - installs its signal handlers, then unblocks the four signals;
    - reads the handoff file and rebuilds the sessions; for each live terminal makes an `AdoptedMaster` and `AdoptedChild` (with an already reaped status, if any), a new `Emu` at the recorded size fed the snapshot, and starts the reader, writer and wait threads. A terminal that fails to adopt is dropped (its master closed, so its program gets `SIGHUP`), logged and reported in a notice; the others go on;
    - nudges each terminal's size (`TIOCSWINSZ` to rows − 1, or rows + 1 for a one-row terminal, then back after 100 ms) so full-screen programs get a real `SIGWINCH` and redraw;
    - writes the marker with `generation` + 1 and the number of adopted terminals, saves `state.json`, starts its tasks, accepts connections, and acts on any recorded termination signal.
      If the handoff file cannot be read at all, it starts like a fresh daemon from `state.json` (the programs are gone, the layout restores) with an error notice.
11. **Rollback**: if `execve` returns (for example `ENOENT` or `ENOEXEC` because the binary was replaced after the preflight), set `FD_CLOEXEC` back, unblock the signals, resume readers, waiters, tasks and actions, accept connections, rewrite the marker without `upgrading`, write the result file, and act on recorded termination signals.
12. **Logs**: `upgrading: <exe> (handoff N)` from the old image; `adopted <n> terminals from pid <pid> (protocol A → B)` from the new one after its `listening` line.
13. **Budget**: from `SIGUSR2` to `execve` at most 5 s plus the preflight; a reader or waiter that does not acknowledge its pause within 1 s aborts the upgrade (rollback). The adoption of 30 terminals with 10,000 lines of history each finishes within 10 s of the exec on the worker's machine (a stress-style ignored test records it).

## Acceptance criteria

- [ ] With the same binary as old and new daemon: a shell that ran `echo $$ > pidfile; sleep 100000` and printed 300 lines keeps its pid, its screen and its history after `SIGUSR2` (`capture --history` before and after), and its `sleep` still runs.
- [ ] A program writing continuously during the handoff (`seq 1 200000`) shows every number once, in order.
- [ ] `less` on the alternate screen is redrawn after the handoff (its screen matches the one before), and leaving it shows the shell's history behind it.
- [ ] A program that exits during the handoff (`sleep 0.2; exit 7` started just before the signal) and one that exits after it both report status 7.
- [ ] Ids, names, ratios, focus, titles, cwds and agent states are the same in `deck ls --json` before and after.
- [ ] A terminal opened after the handoff has no inherited descriptors beyond 0, 1 and 2 (`ls /dev/fd` inside it, compared with a terminal opened before any handoff).
- [ ] `deck kill-server` run during a handoff stops the daemon cleanly once the handoff is done (log shows `adopted` then `shutting down: SIGTERM`), and every program saw a normal hang-up, not a crash.
- [ ] A missing `deck.upgrade`, a relative path, a non-executable file, a binary whose `--handoff-check` exits 1, and a missing binary each leave the daemon running unchanged, with `deck.upgrade.result` and the log line.
- [ ] An exec forced to fail (hidden `DECK_TEST_EXEC_FAIL=1`, read by the old image) rolls back: terminals keep working and new clients can attach.
- [ ] A client connecting during the handoff gets an answer from the new daemon.
- [ ] `deck daemon --handoff-check 1` exits 0; `--handoff-check 99` exits 1.
- [ ] The marker exists with the right pid while the daemon runs, its `generation` grows by one per handoff, and it is gone after `kill-server`.

## Tests

- `crates/deck/tests/upgrade.rs` (new): the end-to-end criteria, triggered by writing `deck.upgrade` and sending `SIGUSR2` from the test (the command arrives in 03).
- `deck-daemon` unit tests for the handoff file round trip (every field), the marker format, and the validation of `deck.upgrade`.
- An ignored test for the budget in 13.

## Notes

- The handoff must not run `finish_shutdown` or hang up any terminal; the worker checks it is unreachable from the handoff path.
- macOS has no `memfd_create`; an unlinked file in the 0700 runtime directory is the portable choice.
