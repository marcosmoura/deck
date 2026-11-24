---
type: shippable
status: todo
---

# 01 — The client exits when its terminal closes

## Scope

`deck-tui` (the reader thread and the exit path in `run_async`, `crates/deck-tui/src/lib.rs`), `deck` (the stdin calls in `crates/deck/src/platform.rs`, passed to `deck-tui` as a closure, and the exit messages in `control.rs` and `main.rs`), and the end-to-end harness (`crates/deck/tests/support/`). When it lands, closing the terminal tab of an attached `deck` leaves no `deck` client behind, and `deck ls` from another terminal still shows the session.

## Requirements

1. **Bounded join.** After the `'run` loop ends, set `stop`, then wait for the reader thread at most 200 ms (for example, the thread signals a channel when it returns, and the main thread waits on it with a timeout); if it has not finished, do not join it. The rest of the shutdown (closing the link, deleting images, restoring the terminal, the upgrade re-exec) runs as today.
2. **Stdin wait closure.** `crates/deck/src/platform.rs` builds a closure (name free, for example `wait_stdin: impl Fn(Duration) -> StdinState`) passed into `deck_tui::run` the way `read_stdin` is. It answers one of `Ready`, `Idle` (nothing within the timeout) or `HungUp`:
   - `poll(2)` on descriptor 0 for `POLLIN` with the given timeout;
   - `POLLHUP` or `POLLERR` → `HungUp`;
   - `POLLIN` → `ioctl(FIONREAD)`: more than 0 bytes → `Ready`; 0 bytes or a failure → `HungUp` (a live tty with `POLLIN` always has bytes queued; a revoked one reads as end of file);
   - nothing within the timeout → `tcgetattr` on descriptor 0: a failure → `HungUp`, otherwise `Idle`;
   - it never reads from descriptor 0, so no input is taken from crossterm;
   - when `isatty(0)` is false it always answers `Idle` after sleeping the timeout (the attach client already refuses a non-tty stdin; this only guards the closure).
     Each `unsafe` block has a `// SAFETY:` comment; `docs/architecture.md`'s placement list names the new calls.
3. **Reader loop order.** Each round of the reader thread:
   1. call the closure with a zero timeout; `HungUp` → return;
   2. drain crossterm: while `crossterm::event::poll(Duration::ZERO)` is true, `read()` and feed `late` as today (this empties crossterm's own parsed buffer, which `poll(2)` on the descriptor cannot see);
   3. call the closure with `INPUT_POLL`: `HungUp` → return; `Idle` → `late.flush` as today's `Ok(false)` branch; `Ready` → next round.
      The reader never calls `crossterm::event::poll` with a non-zero timeout. Returning drops `input_tx`, so the main loop ends with `Exit::Error("lost the terminal input")`.
4. **Quiet end on a dead terminal.** No exit path panics because the terminal is gone: the exit messages written with `println!` (`control.rs`, `Exit::Detached` and the other exits) and `eprintln!` (`main.rs:201`, which prints `Exit::Error` after `bail!`) are written with `writeln!` and the error ignored, at least on the attach path. Restoring the terminal ignores errors as today.
5. **No CPU while hung up.** Between the hang-up and the exit, neither thread loops without sleeping.

## Acceptance criteria

- [ ] With `SIGHUP` ignored by the client (seam below), an attach client whose PTY master is closed exits within 5 s, on macOS and Linux, both while output is flowing and with an idle session: only the reader's hang-up detection can end it.
- [ ] Without the seam, the same happens (the `SIGHUP` path with the bounded join).
- [ ] After that, the daemon is still running and `deck ls` lists the session.
- [ ] A client whose reader thread is stuck (seam below) still exits on `SIGTERM` within 5 s.
- [ ] Typed-ahead input is not delayed: a burst of 40 keys sent in one write reaches the terminal's program within the same bound as today (no 50 ms per buffered event).
- [ ] The existing client, upgrade and colour-query tests pass.
- [ ] `docs/features/sessions.md` and `docs/architecture.md` describe the behaviour, the new `unsafe` calls, the seams and `close_master`.

## Tests

- **Harness** (`crates/deck/tests/support/mod.rs`): today the PTY reader is a detached thread blocked in `read` that also holds a clone of the master and the writer (`support/mod.rs:544-545`), so dropping `Harness.master` leaves the master open and the slave never hangs up. Make that reader stoppable (it polls its descriptor with a timeout and checks a stop flag, then drops its clone and the writer), and add `close_master()`: set the flag, drop `master` and the writer, wait for the reader thread. The child handle stays, for its pid and `try_wait`.
- **Seams**, each a `DECK_TEST_*` variable named in `docs/architecture.md` with the others and read in `deck-tui` next to `DECK_TEST_PANIC` (`lib.rs:335`):
  - `DECK_TEST_IGNORE_SIGHUP=1`: the client does not act on `SIGHUP` (`portable_pty` makes the client a session leader with the PTY as its controlling terminal, so closing the master sends it `SIGHUP`, which alone would pass the tests through fix 1);
  - `DECK_TEST_STUCK_READER=1`: the reader thread blocks forever without spinning (`thread::park` in a loop).
- **End-to-end** (`crates/deck/tests/`):
  - `closing_the_terminal_ends_the_client_and_keeps_the_session`: under `DECK_TEST_IGNORE_SIGHUP=1`, attach, run a shell loop that prints, `close_master()`, assert the client exits within 5 s, then `deck ls` lists the session.
  - `closing_the_terminal_of_an_idle_session_ends_the_client`: the same with no output.
  - `closing_the_terminal_ends_the_client_through_sighup`: the first case without the seam.
  - `sigterm_ends_a_client_whose_reader_is_stuck`: under `DECK_TEST_STUCK_READER=1`, `SIGTERM` to the client ends it within 5 s.
  - `typed_ahead_keys_arrive_together`: 40 keys in one write appear in the shell within the usual bound.
- **Unit tests** in `deck-tui` for the reader loop with a fake closure: `HungUp` at either check ends the thread and closes the channel; `Idle` calls `late.flush`; crossterm is drained before waiting (with a fake event source if the loop is factored so).
- No CPU-time assertion: `getrusage(RUSAGE_CHILDREN)` counts only reaped children in total. Requirement 5 is checked by reading the loop.

## Notes

- The stuck clients found on 24 Nov 2026 had stdin, stdout and stderr shown as `(revoked)` by `lsof`, their `just run` parents and login shells orphaned to `launchd`. A closed PTY master in the test is the nearest reproducible case; the worker confirms on macOS and Linux which of `POLLHUP`, `POLLIN` with `FIONREAD` 0 or failing, and a failing `tcgetattr` each case gives, and records it here.
