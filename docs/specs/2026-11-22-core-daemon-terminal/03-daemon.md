---
type: foundation
status: draft
---

# 03 — Daemon, PTYs and frames

## Scope

A working daemon in `deck-daemon`: it listens on the socket, creates a session with one shell, emulates the terminal, streams frames to attached clients, takes input and resizes, and survives clients coming and going. Nothing to try by hand yet; sub-spec 04 builds the client on it. Owning crates: `deck-daemon`; `deck` gets only the hidden `deck daemon` subcommand.

## Requirements

1. **Start-up** (`Daemon::run(paths, config) -> Result<()>`): create the runtime dir with mode `0700`, take the exclusive lock on `deck.lock` (return `Err(AlreadyRunning)` when held, which `deck daemon` prints as `deck daemon is already running` with exit code 1), remove a leftover `deck.sock`, bind it, and log through `tracing`. `SIGTERM`, `SIGINT` and `SIGHUP` shut down cleanly: send `Bye { ServerShutdown }`, hang up the PTYs (`SIGHUP`, then `SIGKILL` after 2 s), remove the socket, release the lock. The signal handling is installed by `deck daemon`, not by `Daemon::run`, which takes a shutdown future, so tests can run several daemons in one process.
2. **Peer check**: reject a connection whose peer uid differs from the daemon's, using `platform::peer_uid` (`getpeereid` on macOS, `SO_PEERCRED` on Linux).
3. **Handshake**: a client's first message must be `Hello` within 5 s, else the connection is closed. A version other than `PROTOCOL_VERSION`, or a first frame that fails to decode beyond its `version` field, gets a best-effort `Bye { VersionMismatch { daemon } }`.
4. **Sessions**: `Attach { session: None }` attaches to the most recently attached session, or creates `main` (`main-2`, … when taken). `Attach { session: Some(name), create }` attaches to that session, creates it when `create` is true, or fails with `Bye { Error("no session named <name>") }`. A new session has one project (the `Hello` cwd; name = the directory's basename, `~` for the home dir), one panel (`name_source = Default`, name `1`) and one terminal, spawned at the inner size of the `Attach`'s `cols × rows` ([architecture.md#layout](../../architecture.md#layout)). On attach the daemon sends `Welcome`, the `SessionView`, then `full` frames for the active panel's terminals.
5. **Environment**: a session stores the `env` of the `Hello` that created it. Its terminals get that environment minus `DECK_*`, `TMUX`, `TMUX_PANE`, `COLUMNS`, `LINES` and `TERM_PROGRAM*`, then `TERM=xterm-256color`, `COLORTERM=truecolor` and the `DECK_*` variables of [architecture.md#agents](../../architecture.md#agents). The daemon's own environment is not used (portable-pty's base environment is cleared first).
6. **PTYs**: each terminal is a `portable-pty` pair spawned from its `LaunchSpec`; with `program: None`, the program is the config's `shell`, else `SHELL` from the session's environment, else `/bin/sh`, with the config's `shell_args`. A blocking reader thread per PTY forwards output to the terminal's emulator; a blocking wait thread reports the exit status. Writes from `Input` go to the PTY writer without blocking the async runtime. `Input` (and later `Scroll`) for a terminal outside the client's session is ignored and logged.
7. **Emulation**: one `alacritty_terminal::Term` per terminal, with `scrollback_lines` of history, fed by the PTY output. The event listener handles `Title`/`ResetTitle` (stored on the terminal, pushed in a new `SessionView`), `PtyWrite` (answers such as device attributes and cursor position reports go back to the PTY), `ColorRequest` (answered with the emulator's default foreground and background, so programs querying OSC 10/11 do not wait for a timeout), `TextAreaSizeRequest` (answered from the PTY size), and ignores `ClipboardStore`/`ClipboardLoad` (OSC 52) and `Bell`.
8. **Frames**: changed lines of the active panel's terminals are turned into `Frame`s by the cell model of [02-core-types](02-core-types.md) (requirement 2) using the emulator's damage, coalesced to at most one frame per terminal per 16 ms. Each client has a bounded outgoing queue; when it is full, the daemon drops pending frames for that client (never `SessionView`, `Bye` or `Response` messages) and sends one `full` frame once the queue drains, so a slow client never stalls PTY reading or other clients.
9. **Viewport and several clients**: `Viewport { cols, rows }` is a client's panel area. PTY sizes follow the client that most recently sent input or a viewport: with one terminal, the PTY and the emulator get the inner size (`cols − 2`, `rows − 2`, at least 1×1). The size the daemon used is in `SessionView.layout_viewport`; a resize sends a new `SessionView` and `full` frames to every client of the session.
10. **Exit**: when a terminal's process exits, the daemon first drains the PTY to EOF (or `EIO`, which Linux returns once the child is gone) so the last output reaches the emulator, then removes the terminal. Until DECK-02 replaces this rule ([architecture.md#model](../../architecture.md#model)), a session whose last terminal exits is removed and its clients get `Bye { SessionEnded }`. When no session is left, the daemon shuts down as in requirement 1.
11. **Control**: `ListSessions` returns one `SessionSummary` per session, sorted by name; `KillServer` replies `Ok`, then shuts down as in requirement 1.
12. **Detach**: `Detach`, or the connection closing, removes the client; the session and its processes keep running.
13. **`deck daemon`** (hidden from `--help`) installs the signal handling and runs `Daemon::run` in the foreground with paths from the environment. Starting it in the background is sub-spec 04's job.

## Acceptance criteria

- [ ] A second `Daemon::run` on the same `DECK_HOME` returns `AlreadyRunning` while the first keeps serving.
- [ ] A client attaching with no session name and a size of 82×26 gets a `main` session whose terminal runs `/bin/sh`, receives frames, and the first `stty size` (with no `Viewport` sent) prints `24 80`.
- [ ] Sending `echo hi` as `Input` shows `hi` in a later frame.
- [ ] After the client disconnects and a new one attaches, the screen (including the earlier `hi`) arrives as a `full` frame.
- [ ] A `Viewport` of 102×32 gives the PTY 100×30, and the `SessionView` carries `layout_viewport = (102, 32)`.
- [ ] Two clients with different viewports: the PTY follows the one that sent input last, both receive the same frames and the new `SessionView`.
- [ ] A terminal created by a client whose `Hello` env has `FOO=bar` and `DECK_TERMINAL_ID=99` sees `FOO=bar`, its own `DECK_TERMINAL_ID`, and no variable that is only in the daemon's environment.
- [ ] `printf 'a\nb'; exit` shows `b` in the last frame sent before `Bye { SessionEnded }`; the daemon then exits and removes its socket.
- [ ] `KillServer` stops the daemon, and no child pid recorded during the test is alive afterwards (`kill(pid, 0)` fails).
- [ ] A client that stops reading while another terminal floods output (`yes` for 2 s) does not delay the other client's frames by more than 200 ms, and still receives every `SessionView`.
- [ ] A wrong protocol version, and a `Hello` with a corrupted tail, get `Bye { VersionMismatch }`.
- [ ] Wide and combining characters (`printf '日本 🙂 éx'`) come out of the emulator as runs at the right columns: `日` at 0, `本` at 2, `🙂` at 5, `é` (with its combining mark) at 8, `x` at 9.
- [ ] A program that sends OSC 11 (`printf '\033]11;?\007'`) receives an answer.
- [ ] `Input` for a terminal of another session is ignored.

## Tests

- `crates/deck-daemon/tests/`: start `Daemon::run` on a tokio task with a temp `DECK_HOME`, `shell = "/bin/sh"` and a shutdown channel, and talk to it with a small test client over the socket. One test per criterion above. Waits use timeouts (at most 5 s), never fixed sleeps.

## Notes

- Check the current alacritty_terminal 0.26 API (`Term::new`, `vte::ansi::Processor`, damage iteration, `Event` variants, cell flags) and portable-pty 0.9 (`CommandBuilder::env_clear`) before relying on memory.
- Daemon tests and the e2e harness set `shell_args = []` next to `shell = "/bin/sh"`, so no profile files make the screens flaky; the login-shell test overrides it per test.
- A terminal whose shell cannot be spawned (bad `shell`, missing cwd) ends the attach with `Bye { Error("could not start <shell>: <reason>") }`; test it.
- `deck daemon` installs a `tracing-subscriber` writing to stderr, which start-up redirects to `daemon.log`.
- The project for `/` is named `/`.
- The `cols × rows` in `Attach` count as a viewport claim, like `Viewport`.
