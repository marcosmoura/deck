# Architecture

Read when you write a spec or touch code. This page holds the decisions every spec builds on. A spec that changes one of them updates this page in the same change.

## Process model

```text
 terminal emulator (Ghostty, iTerm, …)
 └─ deck (client)  ── Unix socket ──  deck daemon ──┬─ PTY ─ $SHELL / agent
    ratatui UI, input                 sessions,     ├─ PTY ─ …
                                      emulation,    └─ state.json
 deck ls / new-terminal / … ────────┘ persistence
 (control)
```

- **One daemon per user**, started on demand by the first `deck` command that needs it (`deck daemon` re-execs the same binary in a new session with `setsid`, stdio to the log file). It owns every PTY, runs one terminal emulator per PTY, and keeps doing so while no client is attached. A PTY is opened and its process started (`fork`, `exec`) on tokio's blocking pool, never on an async task or under the state lock; each terminal then has blocking reader, writer and wait threads. Input waiting for a program that does not read it is capped at 1 MiB per terminal (one larger paste into an empty queue still goes through); past that, input is dropped with one warning in the log. It exits when the last session is killed, or on `deck kill-server`.
- **Clients** are short or long lived `deck` processes. The **attach client** (`deck`, `deck attach`) draws the UI for one session. **Control clients** (`deck ls`, `new-project`, `new-panel`, `new-terminal`, `focus`, `close`, `rename-panel`, `send`, `send-keys`, `capture`, `wait`; DECK-05) send one request, print the answer and exit. They never start a daemon or create a session. Both kinds speak the same protocol on the same socket; `DECK_SOCKET`, when set, wins over the [paths](#paths).
- **Single instance.** The daemon holds an exclusive lock on `deck.lock` (`std::fs::File::try_lock`) for its lifetime and writes its pid into it, then removes a leftover `deck.sock` and binds it; only the daemon ever removes the socket. It also holds `state.lock` next to `state.json`, so two daemons started with different runtime dirs (another `TMPDIR` or `XDG_RUNTIME_DIR`) can never share one state. A client that cannot connect starts the daemon and waits up to 3 s for the socket. `deck kill-server` asks the daemon to stop; when the daemon speaks another protocol version or does not answer, it sends `SIGTERM` to the pid in `deck.lock` (the same clean save path). Either way it returns only once the lock is released (up to 5 s), so a `deck` right after it never races the old daemon.
- **Peer check.** The daemon accepts only connections whose peer uid equals its own (tokio's `UnixStream::peer_cred()`, no `unsafe`). Client and daemon `lstat` the runtime directory and refuse it unless it is a directory owned by the effective uid with mode `0700`, so a squatted `/tmp/deck-<uid>/` never receives a client's environment.

## Crates

| Crate         | Path                  | Owns                                                                                                                                                   |
| ------------- | --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `deck-core`   | `crates/deck-core/`   | Model types, layout engine, protocol messages and framing, key encoding, config parsing, paths. Pure: no I/O beyond `std::path`, no threads, no async. |
| `deck-daemon` | `crates/deck-daemon/` | The daemon: socket server, sessions, PTYs, emulation (alacritty_terminal), frames, persistence, agent status. Library crate.                           |
| `deck-tui`    | `crates/deck-tui/`    | The attach client: screen mirror, rendering (ratatui), input handling, key bindings. Library crate.                                                    |
| `deck`        | `crates/deck/`        | The binary: CLI (clap), daemon start-up, control commands, wiring. Holds the end-to-end tests in `crates/deck/tests/`.                                 |

`deck-daemon` and `deck-tui` never depend on each other. Both depend on `deck-core`.

## Model

```text
Daemon
└─ Session        id, name, projects (ordered), active project
   └─ Project     id, name, root dir, panels (ordered), active panel, collapsed
      └─ Panel    id, name + name source (default | manual | auto), terminals (ordered), layout ratios, focused terminal
         └─ Terminal  id, launch spec (shell or command + args, cwd, env), title (OSC 0/2), cwd (live), exit status, agent
```

- IDs are `u64` newtypes (`SessionId`, `ProjectId`, `PanelId`, `TerminalId`) from one daemon-wide counter that is persisted, so an ID is never reused.
- **Names.** Session names are unique within the daemon; project names within a session; panel names are free. A panel's `name_source` is `default` (generated), `manual` (set by the user; never overwritten automatically) or `auto` (set from an agent, DECK-07).
- **Projects** are a name plus a root directory. New terminals in a project start in its root.
- **Lifecycle.** When a terminal's process exits, the terminal is removed, unless it was created with `keep` (control API), in which case it stays with its exit status shown. A panel with no terminals is removed. A project with no panels stays in the sidebar, empty. A session stays until it is killed. The daemon exits when no session is left.
- **Environment.** A session remembers the environment of the client that created it (sent in `Hello`); its terminals get that environment minus `DECK_*`, `TMUX`, `TMUX_PANE`, `COLUMNS`, `LINES` and `TERM_PROGRAM*`, plus `TERM=xterm-256color`, `COLORTERM=truecolor` and the `DECK_*` variables. On every attach, an allowlist is refreshed from the attaching client's `Hello` for terminals started afterwards: `SSH_AUTH_SOCK`, `SSH_CONNECTION`, `SSH_TTY`, `DISPLAY`, `WAYLAND_DISPLAY`, `XAUTHORITY`. The daemon's own environment is otherwise never passed on. The one exception: sessions restored after a daemon restart (DECK-04) use the daemon's start environment, which is the `Hello` environment of the client that started it. `state.json` never stores a session's environment, only `--env` additions given through the control API.
- **Focus** (active project, active panel, focused terminal) is stored per session in the daemon, so every client attached to a session sees the same focus.

## Layout

The terminals of a panel are placed by one pure function in `deck-core::layout`:

```rust
pub fn layout(area: Rect, n: usize, ratios: &Ratios) -> Vec<Rect>
```

**Shape.** `n = 1`: the whole area. Terminals keep the order they were created in (no swapping). `n` even: a grid of `cols × rows = n` with `cols ≥ rows` and `cols − rows` as small as possible (2 → 2×1, 4 → 2×2, 6 → 3×2, 8 → 4×2, 12 → 4×3). `n` odd and > 1: terminal 1 takes the left part at full height (the **main** split); the other `n − 1` fill the right part as a grid with `rows ≥ cols` (the even rule turned on its side). Terminals fill a grid left to right, then top to bottom.

```text
n=2              n=3              n=4              n=5
┌──────┬──────┐  ┌──────┬──────┐  ┌──────┬──────┐  ┌──────┬───┬───┐
│      │      │  │      │  2   │  │  1   │  2   │  │      │ 2 │ 3 │
│  1   │  2   │  │  1   ├──────┤  ├──────┼──────┤  │  1   ├───┼───┤
│      │      │  │      │  3   │  │  3   │  4   │  │      │ 4 │ 5 │
└──────┴──────┘  └──────┴──────┘  └──────┴──────┘  └──────┴───┴───┘
```

**Ratios.** `Ratios { main: Option<u16>, cols: Vec<u16>, rows: Vec<u16> }` in per-mille. `main` is the left part's share for odd `n` (default 500); `cols` and `rows` are the grid's column widths and row heights (default equal). All arithmetic is integer, in `u32`: the main split's left width is `(width × main + 500) / 1000`, and grid boundaries are cumulative, `floor(len × (sum of shares up to the boundary) / 1000)`, so the last track takes the remainder and a dragged boundary lands exactly on the pointer cell. With one terminal the rect is the whole panel area and the terminal has no border; with two or more, each rect includes the terminal's 1-cell border and the PTY gets the inner size (`width − 2`, `height − 2`, at least 1×1). When `n` changes, the ratios are adapted, not reset (`Ratios::adapt`), relative to the defaults: the layout is read as full-width columns (for odd `n`, the main split is the first column and the right grid's columns follow, scaled to the rest) and rows (the grid's rows; for odd `n`, the right grid's rows); each surviving track becomes `old_share × default_new / default_old`, an added track gets its default share, and the list is renormalised to 1000 with the floor-and-remainder rule. So `adapt(default_for(a), a, b) == default_for(b)` for every `a`, `b`: untouched layouts always follow the shape rule, and custom ones keep their proportions. Stored ratios are never overwritten because they do not fit: when they would put a terminal below the 10×3 minimum at the current viewport, `layout::fit` gives a clamped copy for that viewport only, which the daemon and the client both use,, and the stored ratios return when the window grows. Only `reset_layout` restores the defaults. `layout::panel_rects` is the one call that gives both sides the drawn grid (`layout` of the `fit` ratios at the 10×3 minimum, `layout::MIN_INNER`), and `layout::has_room_for` is the check behind `NewTerminal`'s refusal; the actions `NewTerminal`, `CloseTerminal`, `FocusTerminal` and `FocusDirection` change a panel's terminals and focus. Resizing moves one boundary in the direction of the key, in steps of 50 per-mille (at least one cell): the focused terminal's edge on that side when it has one, else its opposite edge. It never takes a terminal below a 10-column or 3-row inner size. A client sends new ratios as `SetRatios { panel, n, ratios }`, refused when `n` is no longer the panel's terminal count. Details: DECK-03.

The daemon and the client both call `layout` with the same inputs: the daemon to size PTYs, the client to draw. The client reports the size of its panel area (`Viewport`, or `ReloadViewport` when a config reload moved the sidebar: it is recorded without making that client the one the terminals follow); at attach it reports the outer window and the daemon computes the area with `layout::panel_area(outer, &config, sidebar_visible)`, which the client uses to draw too.

## Protocol

- **Framing**: a `u32` little-endian length, then a `postcard` payload of `ClientMsg` or `ServerMsg` (in `deck-core::protocol`). Maximum frame 16 MiB.
- **Handshake**: the client's first message is `Hello { version, role, cwd, env, colors }` (`colors`: the outer terminal's foreground and background, queried with OSC 10/11 at attach with a 200 ms timeout, `None` when it does not answer; answers that arrive after the timeout are taken out of the input for 3 seconds, so they never reach a terminal as keys; the daemon answers programs' OSC 10/11 queries with them, else a fixed fallback) with `role` = `Attach { session, create, cols, rows }` (`cols × rows` is the client's outer window size; the daemon derives the panel area from its config with `deck-core::layout::panel_area`, so a new session's first terminal starts at the right size) or `Control`. A version that differs from `PROTOCOL_VERSION`, or a first frame that does not decode past `version`, gets `Bye { reason: VersionMismatch { daemon } }`, and the client tells the user to run `deck kill-server` (which also stops a daemon of another version, see [Process model](#process-model)). `Hello` stays the first `ClientMsg` variant with `version` first, and `Bye`/`VersionMismatch` keep their layout, so mismatched versions can always say so.
- **Attach traffic**: the client sends `Viewport { cols, rows }` (its panel area), `Input { terminal, bytes }`, `Action(..)` (focus, new terminal, rename, …), `Scroll { terminal, .. }` and `Detach`; each `Action` is answered by `ActionResult(Result<(), ActionError>)`, in order. Control and attach clients also send `Request`s (`ListSessions`, `KillServer`, `NewSession`, `KillSession`, `RenameSession`), answered by `Response`. The daemon sends `SessionView` (the attached session's tree with names, focus and agent status, and `layout_viewport`, the panel area it sized the PTYs for; sent whole whenever it changes) and `Frame` updates for the terminals of the active panel only.
- **Frames**: `Frame { terminal, size, cursor, modes, display_offset, history, lines: Vec<(u16, Line)>, full }`, where `Line` is a list of `Run { col, text, style }` placed at the emulator column of their first cell. A wide character occupies two columns and its spacer cell sends no text; zero-width (combining) characters are appended to their base character; trailing blank default-styled cells are not sent. The daemon sends changed lines (from the emulator's damage tracking), coalesced to at most one frame per terminal every 16 ms, and a `full` frame on attach, on panel change and on resize. `modes` carries what the client needs to encode input (application cursor keys, bracketed paste, mouse reporting mode, focus reporting).
- **Several clients on one session** are allowed. PTY sizes follow the client that most recently sent input or a viewport, and the daemon's layout decides sizes and directional focus. Every client draws with the `layout_viewport` from the `SessionView`, cropped or padded to its own panel area.
- **Backpressure**: each client has a bounded queue. When it is full, the daemon drops that client's pending frames and sends a `full` frame once it drains. `Bye` and `Response` messages are never dropped; for `SessionView` only the latest pending one per client is kept (a newer one replaces an unsent one), and views triggered only by title changes are sent at most 10 times a second.
- **Clipboard**: a program's OSC 52 copy (`ClipboardStore`) is forwarded to the session's most recently active client, which writes OSC 52 to the outer terminal; OSC 52 reads (`ClipboardLoad`) are denied.
- **Control requests** (DECK-05) are thin wrappers over the same daemon functions as the attach client's actions, with ids resolved by the daemon. IDs are unique across kinds (one counter), so a bare id names a session, project, panel or terminal.
- **Config and notices** (DECK-08): `Welcome { session, config }` carries the daemon's resolved config (the client never reads the file), and `ServerMsg::Config` sends the new one after every reload. `ServerMsg::Notice { level, text }` (`Info`, `Warning`, `Error`) carries daemon-side messages (a config error, a state file that could not be read) to every attached client, or, when none is attached, to the next client that attaches (the last 20 are kept, oldest dropped, a repeat of the same level and text replaces the earlier one; all are also in `daemon.log`). The client shows them in the status bar: `muted` for 2 s (info), `warning` for 4 s, `error` for 6 s, one after the other.
- **Bytes, not strings**: paths and environment entries travel as raw bytes (`deck-core::wire`), so non-UTF-8 names round-trip; `encode` returns an error (never panics) and refuses a payload over the 16 MiB limit.
- Every message type is defined once in `deck-core`, with a round-trip test.

## Client

- **Terminal setup**: raw mode, alternate screen, mouse capture, bracketed paste and focus events through crossterm (the `ratatui::crossterm` re-export, so the versions always match). The outer terminal is restored on every exit path, including panics (cursor shape included). A blocking thread reads crossterm events and forwards them to the tokio runtime over a channel; crossterm's `event-stream` feature is not used. The cursor shape is set with crossterm's `SetCursorStyle`.
- **Screen**: sidebar (left, toggleable), the active panel's terminals, and a one-line status bar. A single terminal fills the panel area with no border and no title (its title is in the status bar); borders with titles come with several terminals in DECK-03. The client keeps a mirror of each visible terminal's lines and redraws on change, at most every 16 ms.
- **Input**: keys arrive as crossterm events. The prefix key (default `Option+Space`, `M-Space`) starts a binding; macOS reserves `Ctrl+Space` for input-source switching, so deck would never receive it, and terminals where Option is not Alt send a non-breaking space for `Option+Space`, which counts as the prefix too (only for `M-Space`); any other key is encoded to bytes by `deck-core::keys::encode(key, modes)` (xterm encoding, honouring application cursor keys) and sent to the focused terminal. Pasted text is wrapped in bracketed-paste markers when the terminal asked for them. Mouse events go to the terminal when it enabled mouse reporting, otherwise the client handles them (focus, scroll, sidebar, border drags). The kitty keyboard protocol is not used in v1, so keys such as `Shift+Enter` cannot be told apart from `Enter`. Character bindings ignore `Shift` (`X` matches a shifted `x`), and crossterm's `Ctrl+4`…`Ctrl+7` encode to `0x1C`…`0x1F`. The client also restores the outer terminal on `SIGTERM` and `SIGHUP`, and refuses to attach when stdin or stdout is not a TTY.
- **Look**: colours by role only (accent, muted, warning, error, ok), mapped to the outer terminal's ANSI palette by default so the user's theme applies; overridable in `[theme]` (DECK-08). Unicode box drawing with rounded corners by default (`border_style`); Nerd Font glyphs are never required. When `COLORTERM` is not `truecolor` or `24bit`, the client draws every RGB colour (theme and terminal contents) as the nearest 256-palette colour.

## Agents

- Every terminal gets `DECK=1`, `DECK_SOCKET`, `DECK_SESSION_ID`, `DECK_PANEL_ID` and `DECK_TERMINAL_ID` in its environment, so a program inside deck (an agent, or the agent's hooks) can find and control its own place through the control API.
- **Status** (`none`, `working`, `waiting`, `idle`) comes from the agents' own hooks (Claude Code hooks, Codex hooks, an OpenCode plugin), which call `deck agent-event`. Without hooks, deck recognises an agent by the PTY's foreground process name and shows only activity. Details: the agent status spec (DECK-06).
- Agents are relaunched after a daemon restart with their resume command (DECK-06 sub-spec 03).

## Persistence

- The daemon writes `state.json` (serde_json, `{ "version": 1, … }`) after every structural change, debounced by 500 ms, atomically (write a temp file in the same directory, fsync it, rename, fsync the directory). Safe against process crashes; best effort against power loss.
- **Shutdown** (`kill-server`, `SIGTERM`, `SIGINT`, `SIGHUP`) first marks the daemon as shutting down, so terminals ending because of it are not removed; then it saves synchronously and hangs up the terminals. It holds the model, the ID counter, each terminal's launch spec, last known cwd and agent resume info. Not scrollback.
- On start, the daemon takes the lock, reads the file, binds the socket, then launches the restored terminals: every terminal starts in the first directory that exists of its saved cwd, its project root and the home dir; explicit commands and agents are relaunched. A file it cannot read is moved aside to `state.json.bad-<timestamp>` and the daemon starts empty; a file from a newer deck stops the daemon without touching the file.
- Details: the persistence spec (DECK-04).

## Paths

| What                 | Path                                                                                                                                                           |
| -------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Config               | `$XDG_CONFIG_HOME/deck/config.toml`, default `~/.config/deck/config.toml` (both OSes)                                                                          |
| State and log        | `$XDG_STATE_HOME/deck/`, default `~/.local/state/deck/`: `state.json`, `state.lock`, `daemon.log` (rotated at start when over 10 MiB, one `daemon.log.1` kept) |
| Socket and lock      | `$XDG_RUNTIME_DIR/deck/` if set, else `$TMPDIR/deck-<uid>/`, else `/tmp/deck-<uid>/`: `deck.sock`, `deck.lock` (holds the daemon's pid)                        |
| Everything, isolated | `DECK_HOME=<dir>` puts the config at `<dir>/config.toml`, state in `<dir>/state/` and the socket in `<dir>/run/`. Used by tests and `just run`.                |

Socket paths must stay under 100 bytes (the macOS limit is 104); `deck` refuses a longer one with a clear error. That is why `just run` puts its `DECK_HOME` in `${TMPDIR:-/tmp}/deck-dev-<8 hex characters of just's sha256() of the checkout path>` rather than under `target/`: a socket inside `.worktrees/<slug>/target/` would exceed the limit.

## Config

`config.toml` at the path in [Paths](#paths) (`~/.config/deck/config.toml` by default on macOS and Linux), every key optional; a missing file means the defaults. Parsed in `deck-core::config`; an unknown key is a warning, a bad value an error naming the key. The daemon is the only reader (DECK-08): it reads the file at start (an invalid one means the defaults and an error notice), sends the config to clients in `Welcome`, and every second (`DECK_TEST_CONFIG_POLL_MS` for tests) compares the file's modification time, change time, size and inode with the last read, so an edit, a save through a rename and a deletion (back to the defaults) are all seen. A change is validated with `config::validate`; an invalid file that changed again by the next poll (an editor still writing) is read again, else it keeps the last good config and sends an error notice with its line and column. A valid one is sent to every client as `ServerMsg::Config` with a `config reloaded` notice (and one warning notice when there are warnings); clients apply only what changed, and `shell`, `shell_args` and `scrollback_lines` only affect terminals started afterwards. `[theme]` holds colour roles, the border style and glyphs (DECK-08); the client draws every non-terminal element from a `deck-core::theme::Theme`. `crates/deck-core/src/config/default.toml` documents every key with its default and is printed by `deck config default`; a `deck-core` test requires it to parse to `Config::default()` and to mention every key of `Config` and `Theme` and every action, either set or as a commented `# key = …` example (for optional keys such as `shell`), so a spec that adds a key or an action fails that test until it documents it there. `config::validate(text)` returns every error and warning with a line and column (used by `deck config check`; it never starts a daemon). `Config` is the resolved wire form (plain types, postcard-encodable); the string-or-array and string-or-integer TOML shapes are read and converted while loading. Keys are added by the spec that needs them, starting with `prefix`, `shell`, `shell_args`, `scrollback_lines` and `[keys]` (DECK-01). A `[keys]` entry maps an action to one key or a list of keys.

## Testing

- **Unit tests** next to the code, run by cargo-nextest. `deck-core` carries most of them: layout tables, key encoding, protocol round trips, config parsing.
- **Daemon tests** in `crates/deck-daemon/tests/` start a daemon in-process on a temp `DECK_HOME` and talk to it through the protocol.
- **End-to-end tests** in `crates/deck/tests/` use the harness in `crates/deck/tests/support/`: it runs the real `deck` binary in a PTY of a fixed size with a temp `DECK_HOME` (config: `shell = "/bin/sh"`, `shell_args = []`, `PS1='$ '`), feeds its output to an alacritty_terminal emulator and records the raw bytes, and offers `send_keys`, `wait_for_text(text, timeout)`, `screen_text()`, `cell(row, col)`, `raw_output()` (for asserting on escape sequences), `tty_mode()` and `capture(name)`, which writes the screen to `target/e2e-captures/<test>/<name>.txt`. DECK-01 sub-spec 04 defines it (`support/mod.rs`; `support/sys.rs` and `crates/deck-daemon/tests/support/sys.rs` are the only test modules allowed `unsafe`, for `tcgetattr` and `kill`). Every test kills its daemon at the end, and harness daemons get a hidden `DECK_TEST_PARENT_PID` and exit when that process is gone, so a test killed by nextest leaks nothing. Timing: upper bounds are asserted only in daemon tests with an injected clock; end-to-end tests assert lower bounds and generous upper bounds (3 to 5 times the expected time). Tests run on macOS and Linux.

## Platform modules

`unsafe` and `libc` calls live only in `crates/deck-daemon/src/platform/` (`macos.rs`, `linux.rs`: a process's cwd (DECK-04), the PTY's foreground process and its argv (DECK-06), exit signals), `crates/deck/src/platform.rs` (`setsid` when starting the daemon, `geteuid`, `dup2` of the daemon's stderr onto its log, `kill` for `kill-server`'s `SIGTERM` fallback, `poll` and `read` on stdin for the client's colour query, which `deck-tui` cannot do in safe code and takes as a closure), and the test support modules `crates/deck/tests/support/sys.rs` (`tcgetattr`, `kill`) and `crates/deck-daemon/tests/support/sys.rs` (`kill`). The lints do not enforce placement (`#![allow(unsafe_code)]` opts a module out); review does. The peer uid uses tokio's safe `peer_cred()`. Each `unsafe` block has a `// SAFETY:` comment.

## Dependencies

Declared once in the root `[workspace.dependencies]` with these versions (the newest patch at implementation time is fine); `Cargo.lock` is committed. Adding anything else needs an approved spec or task and a line here.

| Crate                | Version | Used by                | For                                                                                                                                                                      |
| -------------------- | ------- | ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `serde`              | 1.0     | all                    | derive for protocol, state and config                                                                                                                                    |
| `postcard`           | 1.2     | core                   | protocol payloads (`use-std`)                                                                                                                                            |
| `serde_json`         | 1.0     | daemon, deck           | `state.json`, `--json` output, hook payloads; `preserve_order` feature in `deck` for editing agent config files (DECK-06), which brings in `indexmap` (approved with it) |
| `toml`               | 1.2     | core                   | config parsing                                                                                                                                                           |
| `thiserror`          | 2.0     | core, daemon, tui      | error types in libraries                                                                                                                                                 |
| `anyhow`             | 1.0     | deck                   | errors in the binary                                                                                                                                                     |
| `tokio`              | 1.54    | daemon, tui, deck      | async runtime, Unix sockets, timers, signals                                                                                                                             |
| `portable-pty`       | 0.9     | daemon, deck (tests)   | PTYs and child processes; the e2e harness's PTY                                                                                                                          |
| `alacritty_terminal` | 0.26    | daemon, deck (tests)   | terminal emulation; the e2e harness's screen                                                                                                                             |
| `ratatui`            | 0.30    | tui                    | rendering, with its `crossterm` re-export                                                                                                                                |
| `clap`               | 4.6     | deck                   | CLI (derive)                                                                                                                                                             |
| `tracing`            | 0.1     | daemon, tui, deck      | logging                                                                                                                                                                  |
| `tracing-subscriber` | 0.3     | deck                   | log output to `daemon.log`                                                                                                                                               |
| `libc`               | 0.2     | daemon, deck (+ tests) | the platform modules; `tcgetattr` and `kill` in `tests/support/sys.rs`                                                                                                   |
| `unicode-width`      | 0.2     | core                   | glyph width checks in `[theme]` (DECK-08); already in the tree through ratatui                                                                                           |
| `tempfile`           | 3.27    | dev-dependency         | temp `DECK_HOME` in tests                                                                                                                                                |

Tools, pinned in `.mise.toml`: Rust (stable channel at a pinned version, with rustfmt and Clippy), `just`, `cargo-nextest`, `lefthook`, `oxfmt` (the native binary from the aqua backend, no Node; formats the Markdown, TOML, JSON and YAML files rustfmt does not, with its default settings).

The workspace denies Clippy's `unwrap_used`, `expect_used`, `panic` and `indexing_slicing` lints. The root `clippy.toml` allows these in tests. Integration-test helpers and closures that Clippy misses use scoped exceptions. Production exceptions document the invariant or test hook at the point of use. `just lint` checks every workspace target with warnings denied.

## Decisions

| Date        | Decision                                                                                                                                                                                                                                    |
| ----------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 22 Nov 2026 | Client/daemon split over a Unix socket; the daemon owns emulation so sessions keep running while detached.                                                                                                                                  |
| 22 Nov 2026 | alacritty_terminal for emulation (maintained, damage tracking, used as a library by other multiplexers).                                                                                                                                    |
| 22 Nov 2026 | Automatic grid only, with resizable ratios that adapt relative to the defaults when the terminal count changes and are never overwritten by a small window (clamped copy instead). No free-form splits, no swapping or promoting terminals. |
| 22 Nov 2026 | Configurable prefix key, default `Option+Space` (`Ctrl+Space` is taken by macOS). No modal UI.                                                                                                                                              |
| 22 Nov 2026 | Restore after a daemon restart: layout, cwd, explicit commands and agents (resumed); no scrollback.                                                                                                                                         |
| 22 Nov 2026 | macOS and Linux only.                                                                                                                                                                                                                       |
| 22 Nov 2026 | One config file, `~/.config/deck/config.toml`, read only by the daemon and reloaded live; `[theme]` with semantic colour roles and border style; `deck config default/check/path`.                                                          |
| 22 Nov 2026 | OSC 52 copy is forwarded to the outer terminal; clipboard reads are denied. No copy mode in v1.                                                                                                                                             |
| 22 Nov 2026 | `kill-server` stops a daemon of any version (pid in `deck.lock`, `SIGTERM`); a second lock next to `state.json` keeps one daemon per state.                                                                                                 |
