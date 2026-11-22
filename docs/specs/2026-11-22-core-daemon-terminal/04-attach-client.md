---
type: shippable
status: draft
---

# 04 — Attach client

## Scope

The user can run `deck`: a shell opens in a rounded frame with a status bar, typing works like in any terminal, resizing the window resizes the shell, `Ctrl+Space d` detaches and `deck` reattaches to the same running shell. Also lays down the end-to-end test harness every later shippable sub-spec uses. Owning crates: `deck-tui` (the client), `deck` (CLI, daemon start-up, harness).

## Requirements

### CLI (`deck`)

1. `deck` (no subcommand) and `deck attach [<name>]` attach: with no name, to the most recent session or a new `main`; with a name, to that session (error and exit code 1 when it does not exist). `Hello` carries the current dir, the process environment and the panel area of the current window. Inside deck (`DECK` set in the environment) they refuse with `deck: already inside deck (unset DECK to nest)` and exit code 1.
2. `deck ls` prints one line per session: name, terminal count, and `(attached)` when a client is attached, aligned in columns. With no daemon running it prints `no sessions` and does not start one.
3. `deck kill-server` stops the daemon and waits (up to 5 s) until the daemon has released the lock and removed the socket, so a `deck` right after it starts a fresh daemon. With none running it prints `no deck daemon running`. Both exit 0; a daemon still holding the lock after 5 s prints `deck: the daemon did not stop; see <log path>` and exits 1.
4. **Daemon start-up**: when attaching and the socket does not accept a connection, `deck` takes the lock to check that no daemon is running, releases it, removes a stale socket, starts `<current exe> daemon` in a new session (`platform::detach_spawn`: `setsid` in `pre_exec`, stdin from `/dev/null`, stdout and stderr appended to `daemon.log`), and retries the connection for up to 3 s. On failure it prints `deck: could not start the daemon; see <log path>` and exits 1.
5. Errors from the daemon (`Bye { Error }`, `VersionMismatch`) are printed in one line; `VersionMismatch` adds: `run "deck kill-server" to restart it (sessions are kept from DECK-04 on)`.

### Client (`deck-tui`)

6. **Terminal setup** as in [architecture.md#client](../../architecture.md#client): raw mode, alternate screen, mouse capture, bracketed paste, focus events. A guard restores the outer terminal on normal exit, on error and on panic (panic hook): leave the alternate screen, disable the modes, show the cursor and reset its shape (`SetCursorStyle::DefaultUserShape`); the panic message is printed after the restore.
7. **Event loop**: a blocking thread reads crossterm events and forwards them to the tokio runtime over a channel (no crossterm `event-stream` feature); the runtime also reads server messages and a redraw timer.
8. **Mirror**: the client keeps the lines, cursor and modes of each terminal from `Frame`s (full frames replace, others patch lines) and the latest `SessionView`.
9. **Layout and drawing** (see [Screen](#screen)): the panel area is the whole screen minus the bottom status-bar row. The client sends `Viewport` with the panel area's size on every resize. It draws with the `SessionView.layout_viewport` the daemon used: the terminal's rounded border and contents are laid out for that size and cropped or padded (blank) to the client's own panel area when the two differ (another client is driving the size). The title goes in the top border, the lines inside by the cell model of [02-core-types](02-core-types.md) (runs placed at their column, wide characters taking two cells), and the cursor at the terminal's cursor when visible, with its shape set through crossterm's `SetCursorStyle`. Runs keep their colours and attributes (bold, dim, italic, underline, reverse, strikethrough). Redraws happen at most every 16 ms and only when something changed.
10. **Too small**: below 20 columns or 5 rows the client draws only `deck: window too small` centered, sends no `Viewport` (the PTY keeps its last size), and still detaches on the binding.
11. **Input**: keys go through the prefix state machine of the overview's decisions. Bound keys run their action; other keys are converted to `deck-core::KeyEvent`, encoded with `keys::encode` and the focused terminal's modes, and sent as `Input`. Paste events send the text, wrapped in `ESC[200~`/`ESC[201~` when the terminal's modes ask for bracketed paste.
12. **Bindings**: the default table of [02-core-types](02-core-types.md) (requirement 6) with the config's `[keys]` overrides; this sub-spec implements `detach`.
13. **Exit messages** after the terminal is restored: `[detached from <session>]` (exit 0), `[session <session> ended]` (exit 0), `[deck daemon stopped]` (exit 0 on `ServerShutdown`), `[lost connection to the deck daemon]` (exit 1).
14. A hidden env var `DECK_TEST_PANIC=1` (not documented outside the tests) makes the client panic right after its first draw, for the restore test.

### End-to-end harness

15. `crates/deck/tests/support/` as described in [architecture.md#testing](../../architecture.md#testing): spawns the built `deck` (`env!("CARGO_BIN_EXE_deck")`) in a portable-pty of a given size with a temp `DECK_HOME` (config with `shell = "/bin/sh"` and `shell_args = []`; environment `PS1='$ '`, `ENV` and `DECK*` removed), mirrors its output in an alacritty_terminal `Term`, and also records the raw output bytes. It provides `send_keys(&str)`, `send_bytes(&[u8])`, `resize(cols, rows)`, `wait_for_text(&str, Duration)`, `wait_for_exit(Duration)`, `screen_text()`, `cell(row, col)` (character and style), `raw_output()` (every byte the client wrote, for asserting on escape sequences), `tty_mode()` (raw or cooked, from `tcgetattr` on the PTY), `run_deck(args) -> Output` (a second `deck` process on the same `DECK_HOME`) and `capture(name)`. Dropping the harness runs `deck kill-server` on its `DECK_HOME`. `portable-pty`, `alacritty_terminal` and `libc` are dev-dependencies of `deck` ([architecture.md#dependencies](../../architecture.md#dependencies)).

## Screen

80×24 outer terminal, one terminal. Colours by role ([architecture.md#client](../../architecture.md#client)): the focused terminal's border and the session name use the accent; hints are muted.

```text
╭─ sh ─────────────────────────────────────────────────────────────────────────╮
│$ echo hello                                                                  │
│hello                                                                         │
│$ █                                                                           │
│                                                                              │
│                                                                              │
╰──────────────────────────────────────────────────────────────────────────────╯
 main  terminal-multiplexer                                    ^Space d detach
```

- Border title: the terminal's OSC title when set, else the program's name (the shell's basename). Truncated with `…` to fit.
- Status bar: session name (accent, bold), a space, the project name (muted); right-aligned hints (muted).
- Prefix armed: a `PREFIX` badge (warning colour, reverse) replaces the session name until the next key.
- A flash message (`no binding for x`) replaces the hints for 2 s.

## Acceptance criteria

- [ ] `deck` with no daemon running starts one, attaches, and the frame and status bar of the mockup appear (capture `attached`).
- [ ] Typing `echo hello` + `Enter` shows `hello` inside the frame.
- [ ] `Ctrl+Space d` restores the outer screen, prints `[detached from main]` and exits 0; `deck ls` then lists `main` with 1 terminal and no `(attached)`.
- [ ] `deck` again shows the earlier `hello` (capture `reattached`).
- [ ] In an 80×24 outer terminal the first `stty size` prints `21 78`; resizing the outer PTY to 100×30 makes it print `27 98`.
- [ ] `Ctrl+Space Ctrl+Space` sends exactly one `0x00` byte: with `stty raw -echo; od -An -tx1 -N2` running in the shell and `Ctrl+Space Ctrl+Space a` typed, `od` prints `00 61`. `Ctrl+Space x` flashes `no binding for x` and sends nothing (`od` then sees only the next key).
- [ ] The prefix badge shows after `Ctrl+Space` and disappears after the next key or `Esc` (capture `prefix`).
- [ ] `exit` in the shell prints `[session main ended]`.
- [ ] `deck kill-server` while attached ends the client with `[deck daemon stopped]`; `deck ls` right after it prints `no sessions`, and `deck` right after it starts a new daemon.
- [ ] `deck` inside deck refuses with the nesting message.
- [ ] A 15×4 outer terminal shows `deck: window too small`, and the shell's size is unchanged when the window grows back.
- [ ] Colours, bold and reverse from `printf '\033[1;31mred\033[0m'` reach the screen (checked through `cell()`).
- [ ] `printf '日本 🙂 éx'` is drawn with every character in its column and the right border in place (capture `wide`; also a `TestBackend` test).
- [ ] After `DECK_TEST_PANIC=1`, `tty_mode()` is cooked and `raw_output()` ends with leaving the alternate screen and showing the cursor.
- [ ] A shell started by deck is a login shell: with `shell = "<test script>"` and the default `shell_args`, the script receives `-l` (it records its arguments).
- [ ] Two clients of different sizes on one session: the smaller one draws the larger one's layout cropped, without errors.

## Tests

- `deck-tui`: drawing tests with ratatui's `TestBackend` for the mockup, the prefix badge, the flash message, the too-small state, wide and combining characters, and cropping to a smaller area; mirror tests for full and partial frames.
- `crates/deck/tests/attach.rs`: one scenario per end-to-end criterion above, through the harness.

## Notes

- Keep the status-bar hint list generic: DECK-02 and DECK-03 add actions and hints.
- `Ctrl+@` and `Ctrl+Space` are the same key without the kitty keyboard protocol ([architecture.md#client](../../architecture.md#client)).
- A client that starts below the too-small limit sends `Attach` with its size clamped to at least 20×4, and no `Viewport` until the window is large enough.
- `scroll = "["` is bound here but does nothing until sub-spec 05; mouse capture is on but mouse events are ignored until then.
- The harness sends mouse and focus events with `send_bytes` and their raw sequences (sub-spec 05).
- Draw borders, titles, badges and the status bar from `Theme::default()` (02-core-types notes), never from literal colours.
