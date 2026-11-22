---
type: shippable
status: todo
---

# 06 — Scrollback, mouse and focus

## Scope

The user can scroll back through a terminal's history with the mouse wheel or a keyboard scroll mode, and programs that use the mouse (vim, htop, lazygit, the agents' TUIs) receive mouse events. Focus-in and focus-out reach programs that ask for them. Owning crates: `deck-core` (protocol and mouse encoding), `deck-daemon` (display offset), `deck-tui` (handling).

## Requirements

1. **Protocol** (`PROTOCOL_VERSION` stays 1: DECK-01 ships as one unit): `ClientMsg::Scroll { terminal, to: ScrollTo }` with `ScrollTo { Lines(i32), Pages(i32), Top, Bottom }`. The daemon scrolls the emulator's display (`display_offset`, clamped to the history) and sends a `full` frame carrying `display_offset` and `history` (lines of history available). Any `Input` to a terminal first scrolls it back to the bottom. While a terminal is scrolled back, new output that grows the history still sends a frame (with the new `display_offset` and `history`), even when no visible line changed, so the indicator stays right. `Scroll` for a terminal outside the client's session is ignored.
2. **Scroll mode** (keyboard): the `scroll` binding (`[`, from [02-core-types](02-core-types.md)) enters it for the focused terminal. In scroll mode, keys do not go to the terminal: `Up`/`k` and `Down`/`j` move one line, `PageUp`/`Ctrl+b` and `PageDown`/`Ctrl+f` one page, `Ctrl+u`/`Ctrl+d` half a page, `g`/`Home` to the top, `G`/`End` to the bottom; `q` and `Esc` leave scroll mode and return to the bottom. Reaching the bottom does not leave scroll mode. The prefix key still works inside scroll mode (its bindings run, and leave scroll mode first when they change focus or detach). Scroll mode belongs to the terminal it was entered for: it ends when the focused terminal or the active panel changes (DECK-02, DECK-03), and when another client's input returns that terminal to the bottom.
3. **Mouse wheel** over a terminal (focused or not), never entering scroll mode:
   - the terminal has mouse reporting on: the wheel is sent to it (buttons 64/65);
   - else, the terminal is on its alternate screen with alternate scroll on (`?1007`, on by default; a program can turn it off): the wheel sends 3 `Up`/`Down` presses encoded with `keys::encode` and the terminal's modes (so application cursor mode gives `ESC O A`); with `?1007` off, the wheel does nothing there;
   - else, when the terminal has history: it scrolls the history 3 lines per notch and shows the indicator; typing in the terminal returns it to the bottom (requirement 1). With no history the wheel does nothing.
   `Modes` gains the alternate-scroll flag for this.
4. **Mouse reporting**: when the terminal's modes ask for it, clicks, releases, drags and motion (as each mode allows) inside the terminal's inner area are sent with coordinates relative to that area, 1-based (the top-left inner cell is column 1, row 1), in SGR encoding when SGR mode is on, else in the X10 encoding (coordinates above 223 are dropped in X10). Encoding lives in `deck-core::mouse` as a pure function. Events outside a terminal's inner area are never sent to it.
5. **Focus events**: when the outer terminal gains or loses focus, the client sends `ESC[I` / `ESC[O` to the focused terminal if its modes enable focus reporting.
6. **Indicator**: while a terminal is scrolled back (by the wheel or in scroll mode), its top border shows `[<offset>/<history>]` at the right, in the warning colour. In scroll mode, the status-bar hints show `q exit  g top  G bottom` and the `SCROLL` badge (same style as `PREFIX`) replaces the session name; wheel scrolling shows only the indicator.

## Screen

```text
╭─ sh ───────────────────────────────────────────────────────────── [120/4096] ╮
│line 3880                                                                     │
│line 3881                                                                     │
│…                                                                             │
╰──────────────────────────────────────────────────────────────────────────────╯
 SCROLL  terminal-multiplexer                                q exit  g top  G bottom
```

## Acceptance criteria

- [ ] After `seq 1 500` in a 24-row window, `Ctrl+Space [` then `PageUp` shows earlier numbers, the `[offset/history]` indicator and the `SCROLL` badge (capture `scrolled`); `G` returns to the bottom without leaving the mode; `q` leaves it.
- [ ] In scroll mode, `Ctrl+Space d` detaches.
- [ ] The wheel over a shell with history scrolls it and shows the indicator but not the `SCROLL` badge; typing a character then returns the terminal to the bottom and the character reaches the shell. In a fresh shell with no history, the wheel does nothing.
- [ ] While scrolled back, output from a background job started before scrolling (`(sleep 1; seq 1 50) &`) makes the indicator's history count grow without leaving scroll mode.
- [ ] With `less` open on a long file, the wheel moves through the file; with a program in application cursor mode on the alternate screen, the wheel sends `ESC O A`/`ESC O B` (checked by a program that echoes its input bytes); after the program sends `ESC[?1007l`, the wheel sends nothing.
- [ ] A test program that enables SGR mouse mode (`printf '\033[?1000h\033[?1006h'` then reading stdin) receives `ESC[<0;5;3M` for a left click on the terminal's inner cell in column 5, row 3 (1-based).
- [ ] A test program that enables focus reporting receives `ESC[I` and `ESC[O` when the harness sends focus events to the outer client.
- [ ] `deck-core::mouse` unit tests cover SGR and X10 encoding of press, release, drag and wheel, with modifiers, 1-based coordinates, and the X10 coordinate limit.
- [ ] The daemon clamps `Scroll` beyond the history and ignores `Scroll` for a terminal that is not in the client's session.

## Tests

- `deck-core`: mouse encoding table.
- `deck-daemon`: scroll clamping, display-offset frames, the history-growth frame, the alternate-scroll mode flag, through the protocol.
- `deck-tui`: drawing test for the indicator and the badge; scroll-mode key handling, including the prefix inside it; the wheel rules.
- `crates/deck/tests/scroll_mouse.rs`: one scenario per end-to-end criterion above.

## Notes

- Several clients share a terminal's scroll position, because the daemon owns it. Acceptable for v1.
- DECK-02 and DECK-03 add focus and panel changes; their workers end scroll mode on them, as requirement 2 says.
