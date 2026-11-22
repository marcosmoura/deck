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

- **One daemon per user**, started on demand by the first `deck` command that needs it (`deck daemon` re-execs the same binary in a new session with `setsid`, stdio to the log file). It owns every PTY, runs one terminal emulator per PTY, and keeps doing so while no client is attached. It exits when the last session is killed, or on `deck kill-server`.
- **Clients** are short or long lived `deck` processes. The **attach client** (`deck`, `deck attach`) draws the UI for one session. **Control clients** (`deck ls`, `new-project`, `new-panel`, `new-terminal`, `focus`, `close`, `rename-panel`, `send`, `send-keys`, `capture`, `wait`; DECK-05) send one request, print the answer and exit. They never start a daemon or create a session. Both kinds speak the same protocol on the same socket; `DECK_SOCKET`, when set, wins over the [paths](#paths).
- **Single instance.** The daemon holds an exclusive lock on `deck.lock` (`std::fs::File::try_lock`) for its lifetime, then binds `deck.sock`. A client that cannot connect removes a stale socket only when it can take the lock itself, then starts the daemon and waits up to 3 s for the socket. `deck kill-server` returns only once the daemon has released the lock and removed the socket (up to 5 s), so a `deck` right after it never races the old daemon.
- **Peer check.** The daemon accepts only connections whose peer uid equals its own (`getpeereid` on macOS, `SO_PEERCRED` on Linux). The runtime directory is mode `0700`.

## Crates

| Crate         | Path                  | Owns                                                                                       |
| ------------- | --------------------- | ------------------------------------------------------------------------------------------ |
| `deck-core`   | `crates/deck-core/`   | Model types, layout engine, protocol messages and framing, key encoding, config parsing, paths. Pure: no I/O beyond `std::path`, no threads, no async. |
| `deck-daemon` | `crates/deck-daemon/` | The daemon: socket server, sessions, PTYs, emulation (alacritty_terminal), frames, persistence, agent status. Library crate. |
| `deck-tui`    | `crates/deck-tui/`    | The attach client: screen mirror, rendering (ratatui), input handling, key bindings. Library crate. |
| `deck`        | `crates/deck/`        | The binary: CLI (clap), daemon start-up, control commands, wiring. Holds the end-to-end tests in `crates/deck/tests/`. |

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
  - *Temporary (22 Nov 2026, DECK-01 until DECK-02 lands):* with no way to create terminals yet, a session whose last terminal exits ends. DECK-02 removes this note.
- **Environment.** A session remembers the environment of the client that created it (sent in `Hello`); its terminals get that environment minus `DECK_*`, `TMUX`, `TMUX_PANE`, `COLUMNS`, `LINES` and `TERM_PROGRAM*`, plus `TERM=xterm-256color`, `COLORTERM=truecolor` and the `DECK_*` variables. The daemon's own environment is never passed on. Sessions restored after a daemon restart (DECK-04) use the environment of the client that started the new daemon.
- **Focus** (active project, active panel, focused terminal) is stored per session in the daemon, so every client attached to a session sees the same focus.

## Layout

The terminals of a panel are placed by one pure function in `deck-core::layout`:

```rust
pub fn layout(area: Rect, n: usize, ratios: &Ratios) -> Vec<Rect>
```

**Shape.** `n = 1`: the whole area. `n` even: a grid of `cols × rows = n` with `cols ≥ rows` and `cols − rows` as small as possible (2 → 2×1, 4 → 2×2, 6 → 3×2, 8 → 4×2, 12 → 4×3). `n` odd and > 1: terminal 1 takes the left part at full height (the **main** split); the other `n − 1` fill the right part as a grid with `rows ≥ cols` (the even rule turned on its side). Terminals fill a grid left to right, then top to bottom.

```text
n=2              n=3              n=4              n=5
┌──────┬──────┐  ┌──────┬──────┐  ┌──────┬──────┐  ┌──────┬───┬───┐
│      │      │  │      │  2   │  │  1   │  2   │  │      │ 2 │ 3 │
│  1   │  2   │  │  1   ├──────┤  ├──────┼──────┤  │  1   ├───┼───┤
│      │      │  │      │  3   │  │  3   │  4   │  │      │ 4 │ 5 │
└──────┴──────┘  └──────┴──────┘  └──────┴──────┘  └──────┴───┴───┘
```

**Ratios.** `Ratios { main: Option<u16>, cols: Vec<u16>, rows: Vec<u16> }` in per-mille. `main` is the left part's share for odd `n` (default 500); `cols` and `rows` are the grid's column widths and row heights (default equal). All arithmetic is integer, in `u32`: the main split's left width is `(width × main + 500) / 1000`, and grid tracks are `floor(len × share / 1000)` with the remainder going to the last track. Each rect includes the terminal's 1-cell border; the PTY gets the inner size (`width − 2`, `height − 2`, at least 1×1). When `n` changes, the ratios are adapted, not reset (`Ratios::adapt`): the layout is read as full-width columns (for odd `n`, the main split is the first column and the right grid's columns follow, scaled to the rest) and rows (the grid's rows; for odd `n`, the right grid's rows), each list keeps its existing boundaries, an added track gets an equal share (`1000 / new count`) while the existing ones shrink proportionally, a removed (last) track gives its share back to the others proportionally, and the result is converted back to the new shape. The defaults return only when the adapted ratios would put a terminal below the 10×3 minimum at the current viewport, or on `reset_layout`. Resizing moves one boundary in the direction of the key, in steps of 50 per-mille (at least one cell): the focused terminal's edge on that side when it has one, else its opposite edge. It never takes a terminal below a 10-column or 3-row inner size. A client sends new ratios as `SetRatios { panel, n, ratios }`, refused when `n` is no longer the panel's terminal count. Details: DECK-03.

The daemon and the client both call `layout` with the same inputs: the daemon to size PTYs, the client to draw. The client reports the size of its panel area; the daemon never guesses it.

## Protocol

- **Framing**: a `u32` little-endian length, then a `postcard` payload of `ClientMsg` or `ServerMsg` (in `deck-core::protocol`). Maximum frame 16 MiB.
- **Handshake**: the client's first message is `Hello { version, role, cwd, env }` with `role` = `Attach { session, create, cols, rows }` (`cols × rows` is the client's panel area until DECK-08; from DECK-08 it is the outer window size and the daemon derives the panel area from its config with `layout::panel_area`; either way a new session's first terminal starts at the right size) or `Control`. A version that differs from `PROTOCOL_VERSION`, or a first frame that does not decode past `version`, gets `Bye { reason: VersionMismatch { daemon } }`, and the client tells the user to run `deck kill-server`. `Hello` stays the first `ClientMsg` variant with `version` first, and `Bye`/`VersionMismatch` keep their layout, so mismatched versions can always say so.
- **Attach traffic**: the client sends `Viewport { cols, rows }` (its panel area), `Input { terminal, bytes }`, `Action(..)` (focus, new terminal, rename, …), `Scroll { terminal, .. }` and `Detach`. The daemon sends `SessionView` (the attached session's tree with names, focus and agent status, and `layout_viewport`, the panel area it sized the PTYs for; sent whole whenever it changes) and `Frame` updates for the terminals of the active panel only.
- **Frames**: `Frame { terminal, size, cursor, modes, display_offset, history, lines: Vec<(u16, Line)>, full }`, where `Line` is a list of `Run { col, text, style }` placed at the emulator column of their first cell. A wide character occupies two columns and its spacer cell sends no text; zero-width (combining) characters are appended to their base character; trailing blank default-styled cells are not sent. The daemon sends changed lines (from the emulator's damage tracking), coalesced to at most one frame per terminal every 16 ms, and a `full` frame on attach, on panel change and on resize. `modes` carries what the client needs to encode input (application cursor keys, bracketed paste, mouse reporting mode, focus reporting).
- **Several clients on one session** are allowed. PTY sizes follow the client that most recently sent input or a viewport, and the daemon's layout decides sizes and directional focus. Every client draws with the `layout_viewport` from the `SessionView`, cropped or padded to its own panel area.
- **Backpressure**: each client has a bounded queue. When it is full, the daemon drops that client's pending frames and sends a `full` frame once it drains; `SessionView`, `Bye` and `Response` messages are never dropped.
- **Control requests** (DECK-05) are thin wrappers over the same daemon functions as the attach client's actions, with ids resolved by the daemon. IDs are unique across kinds (one counter), so a bare id names a session, project, panel or terminal.
- **Notices**: `ServerMsg::Notice { level, text }` (DECK-08) carries daemon-side messages (a config error, a state file that could not be read) to every attached client, or to the next client that attaches when none is.
- Every message type is defined once in `deck-core`, with a round-trip test.

## Client

- **Terminal setup**: raw mode, alternate screen, mouse capture, bracketed paste and focus events through crossterm (the `ratatui::crossterm` re-export, so the versions always match). The outer terminal is restored on every exit path, including panics (cursor shape included). A blocking thread reads crossterm events and forwards them to the tokio runtime over a channel; crossterm's `event-stream` feature is not used. The cursor shape is set with crossterm's `SetCursorStyle`.
- **Screen**: sidebar (left, toggleable), the active panel's terminals, each in a rounded border with its title, and a one-line status bar. The client keeps a mirror of each visible terminal's lines and redraws on change, at most every 16 ms.
- **Input**: keys arrive as crossterm events. The prefix key (default `Ctrl+Space`) starts a binding; any other key is encoded to bytes by `deck-core::keys::encode(key, modes)` (xterm encoding, honouring application cursor keys) and sent to the focused terminal. Pasted text is wrapped in bracketed-paste markers when the terminal asked for them. Mouse events go to the terminal when it enabled mouse reporting, otherwise the client handles them (focus, scroll, sidebar, border drags). The kitty keyboard protocol is not used in v1.
- **Look**: colours by role only (accent, muted, warning, error, ok), mapped to the outer terminal's ANSI palette by default so the user's theme applies; overridable in `[theme]` (DECK-08). Unicode box drawing with rounded corners; Nerd Font glyphs are never required.

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

| What                 | Path                                                                                    |
| -------------------- | --------------------------------------------------------------------------------------- |
| Config               | `$XDG_CONFIG_HOME/deck/config.toml`, default `~/.config/deck/config.toml` (both OSes)   |
| State and log        | `$XDG_STATE_HOME/deck/`, default `~/.local/state/deck/`: `state.json`, `daemon.log`     |
| Socket and lock      | `$XDG_RUNTIME_DIR/deck/` if set, else `$TMPDIR/deck-<uid>/`, else `/tmp/deck-<uid>/`: `deck.sock`, `deck.lock` |
| Everything, isolated | `DECK_HOME=<dir>` puts the config at `<dir>/config.toml`, state in `<dir>/state/` and the socket in `<dir>/run/`. Used by tests and `just run`. |

Socket paths must stay under 100 bytes (the macOS limit is 104); `deck` refuses a longer one with a clear error. That is why `just run` puts its `DECK_HOME` in `${TMPDIR:-/tmp}/deck-dev-<hash of the checkout path>` rather than under `target/`: a socket inside `.worktrees/<slug>/target/` would exceed the limit.

## Config

`config.toml` at the path in [Paths](#paths) (`~/.config/deck/config.toml` by default on macOS and Linux), every key optional; a missing file means the defaults. Parsed in `deck-core::config`; an unknown key is a warning, a bad value an error naming the key. Until DECK-08 the daemon reads it at start and the client at attach; from DECK-08 on the daemon is the only reader, sends it to clients in `Welcome`, and reloads it live when the file changes (an invalid file keeps the last good config and shows an error notice). `[theme]` holds colour roles, the border style and glyphs (DECK-08); the client draws every non-terminal element from a `deck-core::theme::Theme`. `crates/deck-core/src/config/default.toml` documents every key with its default and is printed by `deck config default`; a test keeps it in sync, so a spec that adds a key adds it there. Keys are added by the spec that needs them, starting with `prefix`, `shell`, `shell_args`, `scrollback_lines` and `[keys]` (DECK-01). A `[keys]` entry maps an action to one key or a list of keys.

## Testing

- **Unit tests** next to the code, run by cargo-nextest. `deck-core` carries most of them: layout tables, key encoding, protocol round trips, config parsing.
- **Daemon tests** in `crates/deck-daemon/tests/` start a daemon in-process on a temp `DECK_HOME` and talk to it through the protocol.
- **End-to-end tests** in `crates/deck/tests/` use the harness in `crates/deck/tests/support/`: it runs the real `deck` binary in a PTY of a fixed size with a temp `DECK_HOME` (config: `shell = "/bin/sh"`, `shell_args = []`, `PS1='$ '`), feeds its output to an alacritty_terminal emulator and records the raw bytes, and offers `send_keys`, `wait_for_text(text, timeout)`, `screen_text()`, `cell(row, col)`, `raw_output()` (for asserting on escape sequences), `tty_mode()` and `capture(name)`, which writes the screen to `target/e2e-captures/<test>/<name>.txt`. DECK-01 sub-spec 04 defines it. Every test kills its daemon at the end. Tests run on macOS and Linux.

## Platform modules

`unsafe` and `libc` calls live only in `crates/deck-daemon/src/platform/` (`macos.rs`, `linux.rs`: peer uid, a process's cwd (DECK-04), the PTY's foreground process and its argv (DECK-06)) and `crates/deck/src/platform.rs` (`setsid` when starting the daemon). Each `unsafe` block has a `// SAFETY:` comment.

## Dependencies

Declared once in the root `[workspace.dependencies]` with these versions (the newest patch at implementation time is fine); `Cargo.lock` is committed. Adding anything else needs an approved spec or task and a line here.

| Crate                | Version | Used by              | For                                              |
| -------------------- | ------- | -------------------- | ------------------------------------------------ |
| `serde`              | 1.0     | all                  | derive for protocol, state and config            |
| `postcard`           | 1.2     | core                 | protocol payloads (`use-std`)                    |
| `serde_json`         | 1.0     | daemon, deck         | `state.json`, `--json` output, hook payloads; `preserve_order` feature in `deck` for editing agent config files (DECK-06) |
| `toml`               | 1.2     | core                 | config parsing                                   |
| `thiserror`          | 2.0     | core, daemon, tui    | error types in libraries                         |
| `anyhow`             | 1.0     | deck                 | errors in the binary                             |
| `tokio`              | 1.54    | daemon, tui, deck    | async runtime, Unix sockets, timers, signals     |
| `portable-pty`       | 0.9     | daemon, deck (tests) | PTYs and child processes; the e2e harness's PTY  |
| `alacritty_terminal` | 0.26    | daemon, deck (tests) | terminal emulation; the e2e harness's screen     |
| `ratatui`            | 0.30    | tui                  | rendering, with its `crossterm` re-export        |
| `clap`               | 4.6     | deck                 | CLI (derive)                                     |
| `tracing`            | 0.1     | daemon, tui, deck    | logging                                          |
| `tracing-subscriber` | 0.3     | deck                 | log output to `daemon.log`                       |
| `libc`               | 0.2     | daemon, deck (+ tests) | the platform modules; `tcgetattr` in the harness |
| `unicode-width`      | 0.2     | core                 | glyph width checks in `[theme]` (DECK-08); already in the tree through ratatui |
| `tempfile`           | 3.27    | dev-dependency       | temp `DECK_HOME` in tests                        |

Tools, pinned in `.mise.toml`: Rust (stable channel at a pinned version, with rustfmt and Clippy), `just`, `cargo-nextest`, `lefthook`.

## Decisions

| Date        | Decision                                                                                                      |
| ----------- | ------------------------------------------------------------------------------------------------------------- |
| 22 Nov 2026 | Client/daemon split over a Unix socket; the daemon owns emulation so sessions keep running while detached.    |
| 22 Nov 2026 | alacritty_terminal for emulation (maintained, damage tracking, used as a library by other multiplexers).      |
| 22 Nov 2026 | Automatic grid only, with resizable ratios that adapt (keep their boundaries) when the terminal count changes. No free-form splits. |
| 22 Nov 2026 | Configurable prefix key, default `Ctrl+Space`. No modal UI.                                                   |
| 22 Nov 2026 | Restore after a daemon restart: layout, cwd, explicit commands and agents (resumed); no scrollback.           |
| 22 Nov 2026 | macOS and Linux only.                                                                                         |
| 22 Nov 2026 | One config file, `~/.config/deck/config.toml`, read only by the daemon and reloaded live; `[theme]` with semantic colour roles and border style; `deck config default/check/path`. |
