---
type: shippable
status: draft
---

# 05 — Scrollback, mouse and focus

## Scope

The user can scroll back through a terminal's history with the mouse wheel or a scroll mode, and programs that use the mouse (vim, htop, lazygit, the agents' TUIs) receive mouse events. Focus-in and focus-out reach programs that ask for them. Owning crates: `deck-core` (protocol and mouse encoding), `deck-daemon` (display offset), `deck-tui` (handling).

## Requirements

1. **Protocol** (`PROTOCOL_VERSION` stays 1: DECK-01 ships as one unit): `ClientMsg::Scroll { terminal, to: ScrollTo }` with `ScrollTo { Lines(i32), Pages(i32), Top, Bottom }`. The daemon scrolls the emulator's display (`display_offset`, clamped to the history) and sends a `full` frame carrying `display_offset` and `history` (lines of history available). Any `Input` to a terminal first scrolls it back to the bottom. While a terminal is scrolled back, new output that grows the history still sends a frame (with the new `display_offset` and `history`), even when no visible line changed, so the indicator stays right. `Scroll` for a terminal outside the client's session is ignored.
2. **Scroll mode**: the `scroll` binding (`[`, from [02-core-types](02-core-types.md)). In scroll mode, keys do not go to the terminal: `Up`/`k` and `Down`/`j` move one line, `PageUp`/`Ctrl+b` and `PageDown`/`Ctrl+f` one page, `Ctrl+u`/`Ctrl+d` half a page, `g`/`Home` to the top, `G`/`End` to the bottom; `q` and `Esc` leave scroll mode and return to the bottom. Reaching the bottom by scrolling does not leave scroll mode. The mode's badge and indicator follow the frame's `display_offset`: when another client's input returns the terminal to the bottom, scroll mode ends.
3. **Mouse wheel** over a terminal:
   - the terminal has mouse reporting on: the wheel is sent to it (buttons 64/65);
   - else, the terminal is on its alternate screen (less, man, an editor): the wheel sends 3 `Up`/`Down` presses encoded with `keys::encode` and the terminal's modes (so application cursor mode gives `ESC O A`);
   - else, when the terminal has history: it scrolls the history 3 lines per notch and enters scroll mode; scrolling back to the bottom with the wheel leaves it. With no history the wheel does nothing.
4. **Mouse reporting**: when the terminal's modes ask for it, clicks, releases, drags and motion (as each mode allows) inside the terminal's inner area are sent with coordinates relative to that area, in SGR encoding when SGR mode is on, else in the X10 encoding (coordinates above 223 are dropped in X10). Encoding lives in `deck-core::mouse` as a pure function. Events outside a terminal's inner area are never sent to it.
5. **Focus events**: when the outer terminal gains or loses focus, the client sends `ESC[I` / `ESC[O` to the focused terminal if its modes enable focus reporting.
6. **Indicator**: while a terminal is scrolled back, its top border shows `[<offset>/<history>]` at the right, in the warning colour, and the status-bar hints show `q exit  g top  G bottom`; the `SCROLL` badge (same style as `PREFIX`) replaces the session name.

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

- [ ] After `seq 1 500` in a 24-row window, `Ctrl+Space [` then `PageUp` shows earlier numbers and the `[offset/history]` indicator (capture `scrolled`); `G` returns to the bottom; `q` leaves the mode.
- [ ] The wheel over a shell with history scrolls it and enters scroll mode; wheel-down to the bottom leaves it. In a fresh shell with no history, the wheel does nothing.
- [ ] Typing a character while scrolled back (outside scroll mode) returns the terminal to the bottom and the character reaches the shell.
- [ ] While scrolled back, output from a background job started before scrolling (`(sleep 1; seq 1 50) &`) makes the indicator's history count grow without leaving scroll mode.
- [ ] With `less` open on a long file, the wheel moves through the file; with a program in application cursor mode on the alternate screen, the wheel sends `ESC O A`/`ESC O B` (checked by a program that echoes its input bytes).
- [ ] A test program that enables SGR mouse mode (`printf '\033[?1000h\033[?1006h'` then reading stdin) receives `ESC[<0;5;3M` for a left click at the terminal's inner column 5, row 3.
- [ ] A test program that enables focus reporting receives `ESC[I` and `ESC[O` when the harness sends focus events to the outer client.
- [ ] `deck-core::mouse` unit tests cover SGR and X10 encoding of press, release, drag and wheel, with modifiers, and the X10 coordinate limit.
- [ ] The daemon clamps `Scroll` beyond the history and ignores `Scroll` for a terminal that is not in the client's session.

## Tests

- `deck-core`: mouse encoding table.
- `deck-daemon`: scroll clamping, display-offset frames, the history-growth frame, through the protocol.
- `deck-tui`: drawing test for the indicator and the badge; scroll-mode key handling; the wheel rules.
- `crates/deck/tests/scroll_mouse.rs`: one scenario per end-to-end criterion above.

## Notes

- Several clients share a terminal's scroll position, because the daemon owns it. Acceptable for v1.
