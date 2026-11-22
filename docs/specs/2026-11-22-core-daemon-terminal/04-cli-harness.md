---
type: foundation
status: done
---

# 04 — CLI and end-to-end harness

## Scope

The `deck` command line around the daemon (starting it, `deck ls`, `deck kill-server`, the guards) and the end-to-end harness every later shippable sub-spec tests with. Nothing new on screen yet: sub-spec 05 adds the client that `deck` attaches with. Owning crate: `deck` (CLI, `platform.rs`, `tests/support/`).

## Requirements

### CLI

1. **Subcommands** (clap): `deck` (no subcommand) and `deck attach [<name>]`, which attach (the client is sub-spec 05; here they only run the start-up and the guards, then hand over to `deck-tui`, a stub until 05); `deck ls`; `deck kill-server`; the hidden `deck daemon` (sub-spec 03); and a hidden `deck _probe [--session <name>]` for tests, which starts the daemon if needed, sends an attach `Hello` (creating the session like `deck` would), prints the `Welcome` session name and the daemon's version, sends `Detach` and exits 0.
2. **Guards**, checked before anything else when attaching: inside deck (`DECK` set) refuse with `deck: already inside deck (unset DECK to nest)`; when stdin or stdout is not a TTY refuse with `deck: attach needs a terminal (stdin and stdout must be a TTY)`. Both exit 1.
3. **Runtime dir check**: every command that connects first checks the runtime dir as the daemon does ([03-daemon](03-daemon.md), requirement 1) and refuses an unsafe one with the same message, exit 1.
4. **Daemon start-up** (attach and `_probe`): when the socket does not accept a connection, `deck` starts `<current exe> daemon` in a new session (`platform::detach_spawn`: `setsid` in `pre_exec`, stdin, stdout and stderr on `/dev/null`; `deck daemon` opens and rotates its own log, sub-spec 03) and retries the connection for up to 3 s. The client never removes the socket (only the daemon does, under its lock), so two `deck` processes starting at the same moment end up on the one daemon that got the lock. On failure it prints `deck: could not start the daemon; see <log path>` and exits 1.
5. **`deck ls`** prints one line per session: name, terminal count, and `(attached)` when a client is attached, aligned in columns. With no daemon running it prints `no sessions` and does not start one.
6. **`deck kill-server`**: sends `KillServer` and waits (up to 5 s) until `deck.lock` is released. When the daemon answers `VersionMismatch`, or the socket does not answer within 1 s while `deck.lock` is held, it reads the pid from `deck.lock` and sends it `SIGTERM` (`platform::terminate`), which takes the daemon's clean shutdown path, then waits the same way. With no daemon (lock free) it prints `no deck daemon running`. These cases exit 0; a lock still held after 5 s prints `deck: the daemon did not stop; see <log path>` and exits 1.
7. **Daemon errors** (`Bye { Error }`, `VersionMismatch`) are printed in one line; `VersionMismatch` adds: `run "deck kill-server" to restart it (sessions are kept from DECK-04 on)`.

### End-to-end harness

8. `crates/deck/tests/support/mod.rs`, as described in [architecture.md#testing](../../architecture.md#testing):
   - `Harness::deck(size)` spawns the built `deck` (`env!("CARGO_BIN_EXE_deck")`) and `Harness::spawn(size, argv)` any program, in a portable-pty of the given size, with a temp `DECK_HOME` (config `shell = "/bin/sh"`, `shell_args = []`), environment `PS1='$ '`, `COLORTERM=truecolor`, `DECK_TEST_PARENT_PID=<test process pid>`, and `ENV` and `DECK*` (other than those) removed. A test can add or override config keys and environment variables.
   - It mirrors the output in an alacritty_terminal `Term` and records the raw output bytes.
   - It provides `send_keys(&str)`, `send_bytes(&[u8])`, `resize(cols, rows)`, `wait_for_text(&str, Duration)`, `wait_for_exit(Duration)`, `screen_text()`, `cell(row, col)` (character and style), `raw_output()` (every byte written, for asserting on escape sequences), `tty_mode()` (raw or cooked), `run_deck(args) -> Output` (a second `deck` process on the same `DECK_HOME`) and `capture(name)` (writes `screen_text()` to `target/e2e-captures/<test>/<name>.txt`).
   - Dropping the harness runs `deck kill-server` on its `DECK_HOME`.
9. `crates/deck/tests/support/sys.rs` is the only test code allowed `unsafe`: `tcgetattr` for `tty_mode()` and `kill(pid, sig)` for process checks, each with a `// SAFETY:` comment ([architecture.md#platform-modules](../../architecture.md#platform-modules)).

## Acceptance criteria

- [ ] The harness works on its own: `Harness::spawn` of `/bin/sh` shows a prompt in `screen_text()`; `printf '\033[1;31mred\033[0m'` is red and bold in `cell()`; `raw_output()` contains the sequence; `tty_mode()` reports cooked, then raw after `stty raw`; `resize` changes `stty size`.
- [ ] `deck _probe` with no daemon running starts one and prints `main`; `deck ls` then lists `main` with 1 terminal.
- [ ] Two `deck _probe` started at the same moment (10 runs) both succeed against one daemon, and the socket exists afterwards.
- [ ] `deck ls` with no daemon prints `no sessions` and starts none (no `deck.lock` holder afterwards).
- [ ] `deck kill-server` stops the daemon, waits for the lock, and prints nothing; `deck ls` right after it prints `no sessions`, and `deck _probe` right after it starts a new daemon. With no daemon it prints `no deck daemon running`.
- [ ] A daemon started with `DECK_TEST_PROTOCOL_VERSION=999`: `deck _probe` prints the version-mismatch line with the `kill-server` hint and exits 1; `deck kill-server` then stops that daemon through `SIGTERM` and exits 0.
- [ ] `deck` and `deck attach` with `DECK=1` set refuse with the nesting message; with stdout redirected to a file they refuse with the TTY message.
- [ ] A runtime dir with mode `0755` makes `deck _probe` and `deck ls` refuse with the daemon's message.
- [ ] A test killed by nextest (simulated: the test process exits without dropping the harness) leaves no daemon running after 3 s.

## Tests

- `crates/deck/tests/harness.rs`: the harness criterion.
- `crates/deck/tests/cli.rs`: one scenario per CLI criterion.

## Notes

- `_probe` is hidden and only for tests and debugging; it is not documented in `docs/features/`.
- Upper time bounds in these tests are generous (3 to 5 times the expected time), as [architecture.md#testing](../../architecture.md#testing) says.
