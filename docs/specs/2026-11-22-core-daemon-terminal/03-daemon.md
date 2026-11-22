---
type: foundation
status: done
---

# 03 — Daemon, PTYs and frames

## Scope

A working daemon in `deck-daemon`: it listens on the socket, creates a session with one shell, emulates the terminal, streams frames to attached clients, takes input and resizes, and survives clients coming and going. Nothing to try by hand yet; sub-spec 04 adds the CLI and the harness, sub-spec 05 the client. Owning crates: `deck-daemon`; `deck` gets only the hidden `deck daemon` subcommand.

## Requirements

1. **Start-up** (`Daemon::run(paths, config, shutdown) -> Result<()>`):
   - Check the runtime dir with `lstat`: create it with mode `0700` when missing; when it exists, refuse unless it is a directory owned by the effective uid with mode `0700` (`Err(UnsafeRuntimeDir)`, naming the path and what is wrong). Create the state dir with mode `0700`.
   - Take the exclusive lock on `deck.lock` (return `Err(AlreadyRunning)` when held, which `deck daemon` prints as `deck daemon is already running` with exit code 1) and write the daemon's pid into it. Take the exclusive lock on `state.lock` in the state dir (`Err(StateInUse)` when held: another daemon with a different runtime dir owns this state).
   - Remove a leftover `deck.sock` (only the daemon ever removes the socket, and only under the lock), bind it, and log through `tracing`.
   - `shutdown` is a future: when it completes, send `Bye { ServerShutdown }`, hang up the PTYs (`SIGHUP`, then `SIGKILL` after 2 s), remove the socket, release both locks. Signal handling is installed by `deck daemon`, not by `Daemon::run`, so tests can run several daemons in one process.
2. **Peer check**: reject a connection whose peer uid differs from the daemon's, using tokio's `UnixStream::peer_cred()`.
3. **Handshake**: a client's first message must be `Hello` within 5 s, else the connection is closed. A version other than `PROTOCOL_VERSION`, or a first frame that fails to decode beyond its `version` field, gets a best-effort `Bye { VersionMismatch { daemon } }`. A hidden env var `DECK_TEST_PROTOCOL_VERSION` makes the daemon report and expect that version instead, so tests (sub-spec 04) can run a daemon "of another version".
4. **Sessions**: `Attach { session: None }` attaches to the most recently attached session that still exists (each session keeps its last-attach time; a session never attached counts by creation), and creates `main` (`main-2`, … when taken) only when no session exists. `Attach { session: Some(name), create }` attaches to that session, creates it when `create` is true, or fails with `Bye { Error("no session named <name>") }`. A new session has one project (the `Hello` cwd; name = the directory's basename, `~` for the home dir, `root` for `/`), one panel (`name_source = Default`, name `1`) and one terminal, spawned at the inner size of the `Attach`'s `cols × rows` ([architecture.md#layout](../../architecture.md#layout)), which also counts as a viewport claim. On attach the daemon sends `Welcome`, the `SessionView`, then `full` frames for the active panel's terminals.
5. **Environment** ([architecture.md#model](../../architecture.md#model)): a session stores the `env` of the `Hello` that created it. Its terminals get that environment minus `DECK_*`, `TMUX`, `TMUX_PANE`, `COLUMNS`, `LINES` and `TERM_PROGRAM*`, then `TERM=xterm-256color`, `COLORTERM=truecolor` and the `DECK_*` variables of [architecture.md#agents](../../architecture.md#agents). Every later attach replaces the session's `SSH_AUTH_SOCK`, `SSH_CONNECTION`, `SSH_TTY`, `DISPLAY`, `WAYLAND_DISPLAY` and `XAUTHORITY` with the attaching `Hello`'s values (removing those it lacks), for terminals started afterwards. The daemon's own environment is not used (portable-pty's base environment is cleared first).
6. **PTYs**: each terminal is a `portable-pty` pair spawned from its `LaunchSpec`; with `program: None`, the program is the config's `shell`, else `SHELL` from the session's environment, else `/bin/sh`, with the config's `shell_args`. The daemon checks that the cwd is a directory before spawning (portable-pty silently falls back to the home dir); a missing cwd or a failed spawn ends the attach with `Bye { Error("could not start <shell>: <reason>") }`. A blocking reader thread per PTY forwards output to the terminal's emulator; a blocking wait thread reports the exit status. Writes from `Input` go to the PTY writer without blocking the async runtime. `Input` (and later `Scroll`) for a terminal outside the client's session is ignored and logged.
7. **Emulation**: one `alacritty_terminal::Term` per terminal, with `scrollback_lines` of history, fed by the PTY output. The event listener handles:
   - `Title`/`ResetTitle`: stored on the terminal and pushed in a new `SessionView` (rate-limited, requirement 8);
   - `PtyWrite`: answers such as device attributes and cursor position reports go back to the PTY;
   - `ColorRequest` (OSC 10/11/4): answered with the `OuterColors` of the most recently active client's `Hello` for foreground and background, else a fixed fallback (light grey on black), and the emulator's palette for indexed colours;
   - `TextAreaSizeRequest`: answered from the PTY size;
   - `ClipboardStore` (OSC 52 copy): sent as `ServerMsg::Clipboard { selection, text }` to the session's most recently active client only; `ClipboardLoad` (OSC 52 read) is denied (no answer) and logged;
   - `Bell`: ignored.
8. **Frames and views**: changed lines of the active panel's terminals are turned into `Frame`s by the cell model of [02-core-types](02-core-types.md) (requirement 2) using the emulator's damage, coalesced to at most one frame per terminal per 16 ms. Each client has a bounded outgoing queue. When it is full, the daemon drops pending frames for that client and sends one `full` frame once the queue drains. `Bye` and `Response` are never dropped. For `SessionView`, only the latest pending one per client is kept (a newer view replaces an unsent one), and views caused only by title changes are sent at most 10 times a second per session. A slow client never stalls PTY reading or other clients.
9. **Viewport and several clients**: `Viewport { cols, rows }` is a client's panel area. PTY sizes follow the client that most recently sent input, a viewport, or attached (an `Attach`'s size counts as a viewport claim): with one terminal, the PTY and the emulator get the inner size (`cols − 2`, `rows − 2`, clamped to 2..=1000 columns and 1..=1000 rows: alacritty_terminal needs at least 2 columns, and the maximum bounds memory). The size the daemon used is in `SessionView.layout_viewport`; a resize sends a new `SessionView` and `full` frames to every client of the session.
10. **Exit**: when a terminal's process exits, the daemon keeps reading its PTY until EOF, `EIO` (Linux once the child is gone) or 200 ms without data, whichever comes first (a background job such as `server &` may hold the PTY open forever), so the last output reaches the emulator; then it removes the terminal. Until DECK-02 replaces this rule ([architecture.md#model](../../architecture.md#model)), a session whose last terminal exits is removed and its clients get `Bye { SessionEnded }`. A terminal whose process exits with a non-zero status within 1 s of starting is logged with its status, and in that case the clients get `Bye { Error("terminal exited at once (status <n>); see <log path>") }` instead (a bad `shell` or `shell_args`). When no session is left, the daemon shuts down as in requirement 1.
11. **Control**: `ListSessions` returns one `SessionSummary` per session, sorted by name; `KillServer` replies `Ok`, then shuts down as in requirement 1.
12. **Detach**: `Detach`, or the connection closing, removes the client; the session and its processes keep running.
13. **`deck daemon`** (hidden from `--help`): rotates `daemon.log` when it is over 10 MiB (renamed to `daemon.log.1`, replacing an older one), once it holds `deck.lock` (a hook of `Daemon::run_locked`), opens `daemon.log` for append, installs a `tracing-subscriber` writing to it, and points its own stderr at it (`dup2` in `crates/deck/src/platform.rs`) so panics land in the log too; installs `SIGTERM`, `SIGINT` and `SIGHUP` as the shutdown future, and runs `Daemon::run` in the foreground with paths from the environment. With the hidden env var `DECK_TEST_PARENT_PID` set, it also shuts down when that process no longer exists (checked every second), so a test killed by nextest leaks no daemon. Starting it in the background is sub-spec 04's job.

## Acceptance criteria

- [ ] A second `Daemon::run` on the same `DECK_HOME` returns `AlreadyRunning` while the first keeps serving; `deck.lock` holds the first daemon's pid.
- [ ] A second daemon with a different runtime dir but the same state dir returns `StateInUse`.
- [ ] A runtime dir with mode `0755`, or owned by another uid (simulated with a symlink to a directory the test cannot own, or skipped with a message when not possible), makes `Daemon::run` return `UnsafeRuntimeDir`.
- [ ] A client attaching with no session name and a size of 82×26 gets a `main` session whose terminal runs `/bin/sh`, receives frames, and the first `stty size` (with no `Viewport` sent) prints `24 80`.
- [ ] Sending `echo hi` as `Input` shows `hi` in a later frame.
- [ ] After the client disconnects and a new one attaches, the screen (including the earlier `hi`) arrives as a `full` frame.
- [ ] A `Viewport` of 102×32 gives the PTY 100×30, and the `SessionView` carries `layout_viewport = (102, 32)`.
- [ ] Two clients with different viewports: the PTY follows the one that sent input last, both receive the same frames and the new `SessionView`.
- [ ] A terminal created by a client whose `Hello` env has `FOO=bar` and `DECK_TERMINAL_ID=99` sees `FOO=bar`, its own `DECK_TERMINAL_ID`, and no variable that is only in the daemon's environment. A second attach with `SSH_AUTH_SOCK=/new` makes a terminal started afterwards see `/new`, while `FOO` stays.
- [ ] `printf 'a\nb'; exit` shows `b` in the last frame sent before `Bye { SessionEnded }`; the daemon then exits and removes its socket.
- [ ] `sleep 1000 & exit` still ends the terminal (the background job holds the PTY; the drain stops after 200 ms of silence, measured with an injected clock).
- [ ] A `shell_args` of `["-c", "exit 3"]` ends the attach with the `terminal exited at once (status 3)` error, and the log has the status.
- [ ] A cwd that does not exist ends the attach with `could not start …` instead of a shell in the home dir.
- [ ] `KillServer` stops the daemon, and no child pid recorded during the test is alive afterwards (checked with `kill(pid, 0)` from the test-support module, [architecture.md#platform-modules](../../architecture.md#platform-modules)).
- [ ] A client that stops reading while another terminal floods output (`yes` for 2 s) does not stop the other client from receiving frames (at least 10 frames in those 2 s), and the stalled client, once it reads again, gets the latest `SessionView` and a `full` frame.
- [ ] A program setting its title 1,000 times in a second causes at most 15 `SessionView`s (an injected clock makes this exact in the daemon test), and the last one carries the last title.
- [ ] A wrong protocol version, and a `Hello` with a corrupted tail, get `Bye { VersionMismatch }`; a daemon run with `DECK_TEST_PROTOCOL_VERSION=999` answers a normal client with `VersionMismatch { daemon: 999 }`.
- [ ] Wide and combining characters (`printf '日本 🙂 éx'`) come out of the emulator as runs at the right columns: `日` at 0, `本` at 2, `🙂` at 5, `é` (with its combining mark) at 8, `x` at 9.
- [ ] A program that sends OSC 11 (`printf '\033]11;?\007'`) receives the background from the client's `OuterColors`, and the fallback when the client sent none.
- [ ] `printf '\033]52;c;aGk=\007'` makes the most recently active client receive `Clipboard { selection: "c", text: "hi" }` and the other client nothing; `printf '\033]52;c;?\007'` gets no answer.
- [ ] `Input` for a terminal of another session is ignored.
- [ ] A session created in `/` names its project `root`; one created in the home dir names it `~`.
- [ ] `deck daemon` with `DECK_TEST_PARENT_PID` set to a process that then exits shuts down within 3 s; an 11 MiB `daemon.log` is rotated to `daemon.log.1` at start.

## Tests

- `crates/deck-daemon/tests/`: start `Daemon::run` on a tokio task with a temp `DECK_HOME`, `shell = "/bin/sh"`, `shell_args = []` and a shutdown channel, and talk to it with a small test client over the socket. One test per criterion above. Waits use timeouts (at most 5 s), never fixed sleeps; upper time bounds are asserted only with an injected clock ([architecture.md#testing](../../architecture.md#testing)).
- The `deck daemon` criteria (parent pid, log rotation) run the binary from `crates/deck/tests/daemon_bin.rs`.

## Notes

- Check the current alacritty_terminal 0.26 API (`Term::new`, `vte::ansi::Processor`, damage iteration, `Event` variants including `ColorRequest`'s formatter, cell flags) and portable-pty 0.9 (`CommandBuilder::env_clear`, the cwd fallback) before relying on memory.
- The login-shell test (sub-spec 05) overrides `shell_args` per test; everything else uses `shell_args = []`, so no profile files make the screens flaky.
- `deck-daemon` has no `unsafe`: the peer check uses `peer_cred()`, and `kill(pid, 0)` lives in the test-support module.
