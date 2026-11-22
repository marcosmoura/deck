---
type: shippable
status: done
---

# 05 — Attach client

## Scope

The user can run `deck`: a shell opens in a rounded frame with a status bar, typing works like in any terminal, resizing the window resizes the shell, `Option+Space d` detaches and `deck` reattaches to the same running shell. Programs inside see the outer terminal's colours, and their OSC 52 copies reach the clipboard. Owning crates: `deck-tui` (the client), `deck` (wiring `deck` and `deck attach` to it).

## Requirements

1. **Attach**: `deck` and `deck attach [<name>]` (sub-spec 04 does the start-up and guards) attach: with no name, to the most recent session or a new `main`; with a name, to that session (error and exit code 1 when it does not exist). `Hello` carries the current dir, the process environment, the panel area of the current window (clamped to at least 20×4) and the outer colours (requirement 3).
2. **Terminal setup** as in [architecture.md#client](../../architecture.md#client): raw mode, alternate screen, mouse capture, bracketed paste, focus events. A guard restores the outer terminal on normal exit, on error, on panic (panic hook) and on `SIGTERM` and `SIGHUP`: leave the alternate screen, disable the modes, show the cursor and reset its shape (`SetCursorStyle::DefaultUserShape`); a panic message is printed after the restore.
3. **Outer colours**: before entering the alternate screen, the client asks the outer terminal for its foreground and background (`OSC 10;?` and `OSC 11;?`) followed by a DA1 query (`ESC[c`), and reads until the DA1 reply arrives or 200 ms pass, so late answers never reach a program; it sends them in `Hello` as `OuterColors`, or `None` when the terminal does not answer. Bytes read while waiting that are not part of an answer go through the normal key path (prefix bindings included); unterminated answer fragments are dropped.
4. **Event loop**: a blocking thread reads crossterm events and forwards them to the tokio runtime over a channel (no crossterm `event-stream` feature); the runtime also reads server messages and a redraw timer.
5. **Mirror**: the client keeps the lines, cursor and modes of each terminal from `Frame`s (full frames replace, others patch lines) and the latest `SessionView`.
6. **Layout and drawing** (see [Screen](#screen)): the panel area is the whole screen minus the bottom status-bar row. The client sends `Viewport` with the panel area's size on every resize. It draws with the `SessionView.layout_viewport` the daemon used: the terminal's border and contents are laid out for that size and cropped or padded (blank) to the client's own panel area when the two differ (another client is driving the size). The title goes in the top border, the lines inside by the cell model of [02-core-types](02-core-types.md) (runs placed at their column, wide characters taking two cells), and the cursor at the terminal's cursor when visible, with its shape set through crossterm's `SetCursorStyle`. Runs keep their colours and attributes (bold, dim, italic, underline, reverse, strikethrough). Every non-terminal colour and glyph comes from `Theme::default()` ([02-core-types](02-core-types.md) notes). Redraws happen at most every 16 ms and only when something changed.
7. **Too small**: below 20 columns or 5 rows the client draws only `deck: window too small` centered (word-wrapped when the window is narrower than the message), sends no `Viewport` (the PTY keeps its last size), and still detaches on the binding.
8. **Input**: keys go through the prefix state machine of the overview's decisions. Bound keys run their action (character bindings ignore `Shift`, [02-core-types](02-core-types.md) requirement 5); other keys are converted to `deck-core::KeyEvent`, encoded with `keys::encode` and the focused terminal's modes, and sent as `Input`. Paste events send the text, wrapped in `ESC[200~`/`ESC[201~` when the terminal's modes ask for bracketed paste.
9. **Bindings**: the default table of [02-core-types](02-core-types.md) (requirement 6) with the config's `[keys]` overrides; this sub-spec implements `detach`.
10. **Clipboard**: on `ServerMsg::Clipboard { selection, text }` the client writes `ESC ] 52 ; <selection> ; <base64 of text> BEL` to the outer terminal (`deck-core::base64`).
11. **Exit messages** after the terminal is restored: `[detached from <session>]` (exit 0), `[session <session> ended]` (exit 0), `[deck daemon stopped]` (exit 0 on `ServerShutdown`), `[lost connection to the deck daemon]` (exit 1), and `Bye { Error }` texts as `deck: <text>` (exit 1).
12. A hidden env var `DECK_TEST_PANIC=1` (not documented outside the tests) makes the client panic right after its first draw, for the restore test.

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
 main  terminal-multiplexer                                    ⌥Space d detach
```

- Border title: the terminal's OSC title when set, else the program's name (the shell's basename). Truncated with `…` to fit.
- Status bar: session name (accent, bold), a space, the project name (muted); right-aligned hints (muted).
- Prefix armed: a `PREFIX` badge (warning colour, reverse) replaces the session name until the next key.
- A flash message (`no binding for x`) replaces the hints for 2 s.

## Acceptance criteria

- [ ] `deck` with no daemon running starts one, attaches, and the frame and status bar of the mockup appear (capture `attached`).
- [ ] Typing `echo hello` + `Enter` shows `hello` inside the frame.
- [ ] `Option+Space d` restores the outer screen, prints `[detached from main]` and exits 0; `deck ls` then lists `main` with 1 terminal and no `(attached)`.
- [ ] `deck` again shows the earlier `hello` (capture `reattached`).
- [ ] In an 80×24 outer terminal the first `stty size` prints `21 78`; resizing the outer PTY to 100×30 makes it print `27 98`.
- [ ] `Option+Space Option+Space` sends exactly the second key (`ESC` and a space): with `stty raw -echo; od -An -tx1 -N3` running in the shell and `Option+Space Option+Space a` typed, `od` prints `1b 20 61`. The non-breaking space (`C2 A0`), which macOS terminals send for Option+Space when Option is not Alt, also arms the prefix, and sent twice reaches the shell as itself. `Option+Space x` flashes `no binding for x` and sends nothing (`od` then sees only the next key).
- [ ] Every byte `0x01`–`0x7f` except `0x1b`, sent one at a time through the outer PTY to a program in raw mode that echoes its input bytes, arrives unchanged (this covers crossterm's `Ctrl+4`…`Ctrl+7` reports for `0x1c`–`0x1f`).
- [ ] With `[keys] detach = "X"` in the config, `Option+Space X` (typed as Shift+x) detaches.
- [ ] The prefix badge shows after `Option+Space` and disappears after the next key or `Esc` (capture `prefix`).
- [ ] `exit` in the shell prints `[session main ended]`.
- [ ] `deck kill-server` while attached ends the client with `[deck daemon stopped]`.
- [ ] `SIGTERM` to the client leaves the outer terminal cooked and on the main screen (`tty_mode()`, `raw_output()`).
- [ ] A 15×4 outer terminal shows `deck: window too small`, and the shell's size equals its size before the shrink when the window grows back.
- [ ] Colours, bold and reverse from `printf '\033[1;31mred\033[0m'` reach the screen (checked through `cell()`).
- [ ] `printf '日本 🙂 éx'` is drawn with every character in its column and the right border in place (capture `wide`; also a `TestBackend` test).
- [ ] When the harness answers the client's OSC 11 query with `rgb:ffff/ffff/ffff`, `printf '\033]11;?\007'` inside deck receives white; when it does not answer, the client attaches within 1 s and the program receives the fallback.
- [ ] `printf '\033]52;c;aGk=\007'` inside deck makes `raw_output()` contain `ESC]52;c;aGk=BEL`.
- [ ] After `DECK_TEST_PANIC=1`, `tty_mode()` is cooked and `raw_output()` ends with leaving the alternate screen and showing the cursor.
- [ ] A shell started by deck is a login shell: with `shell = "<test script>"` and the default `shell_args`, the script receives `-l` (it records its arguments).
- [ ] `shell_args = ["-c", "exit 3"]` ends the client with `deck: terminal exited at once (status 3); see <log path>` and exit 1.
- [ ] Two clients of different sizes on one session: the smaller one draws the larger one's layout cropped, without errors.

## Tests

- `deck-tui`: drawing tests with ratatui's `TestBackend` for the mockup, the prefix badge, the flash message, the too-small state, wide and combining characters, and cropping to a smaller area; mirror tests for full and partial frames; the OSC 10/11 answer parser; key conversion from crossterm events (including `Ctrl+4`…`Ctrl+7` and shifted characters).
- `crates/deck/tests/attach.rs`: one scenario per end-to-end criterion above, through the harness of sub-spec 04.

## Notes

- Keep the status-bar hint list generic: DECK-02 and DECK-03 add actions and hints.
- `Shift+Enter` is `Enter`, without the kitty keyboard protocol ([architecture.md#client](../../architecture.md#client)).
- `scroll = "["` is bound here but does nothing until sub-spec 06; mouse capture is on but mouse events are ignored until then.
- The harness sends mouse and focus events with `send_bytes` and their raw sequences (sub-spec 06), and answers the OSC 10/11 queries by writing the reply to the PTY when a test asks it to.
