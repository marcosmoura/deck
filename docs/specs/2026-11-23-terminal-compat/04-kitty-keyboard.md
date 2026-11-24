---
type: shippable
status: done
---

# 04 — Kitty keyboard protocol

## Scope

Programs that ask for the kitty keyboard protocol get it inside deck: Shift+Enter, Ctrl+Enter, Ctrl+Tab, Ctrl+Backspace, Ctrl+I versus Tab, key releases when asked, and every key with every modifier. Claude Code's and Codex's Shift+Enter newline and editors' extra bindings work as in Ghostty or kitty. Owning crates: `deck-core` (encoding, `Modes`), `deck-daemon` (flags from the emulator), `deck-tui` (outer terminal, input).

## Requirements

1. **Flags**: the emulator keeps each terminal's kitty keyboard flag stack (push `CSI > f u`, pop `CSI < n u`, set `CSI = f ; m u`, separate for the primary and alternate screens) and answers the program's query `CSI ? u` with its current flags. DECK-19 drops that reply on purpose and pins its absence in a test; this sub-spec removes the filter and updates that test. The daemon reads the current flags from `libghostty-vt` if its binding exposes them; otherwise `Emu` tracks the push, pop and set sequences itself (per screen) and says so in `docs/architecture.md`. `Frame.modes` carries the current flags (5 bits) of the terminal. Protocol: on the bump of [03](03-underline-styles.md).
2. **Outer terminal**: the client learns whether the outer terminal supports the protocol from its existing start-up query (`colors::QUERY`, which ends with DA1): `CSI ? u` is added before the DA1 and its answer read in `colors::probe`, with no second round trip. When it does and the focused terminal's flags are not 0, the client pushes to the outer terminal: flag 1 always; flag 2 when the program asked for 2; flag 4 when the program asked for 8 or 16 (crossterm then gives the shifted character, which the encoding needs); never 16 (crossterm cannot parse associated text; `encode_kitty` computes the text from the key). It pops them when focus moves to a terminal without flags, when an overlay takes the keyboard, and on exit (including panics and signals, with the rest of the terminal restore).
3. **Encoding**: `deck-core::keys::encode_kitty(key, event, flags)` produces the kitty encoding for flags 1 (disambiguate), 2 (event types: press, repeat, release), 4 (the shifted key, without the base-layout key), 8 (all keys as escape codes) and 16 (associated text, computed from the key). Keys the client sends to a terminal with flags set use it; terminals without flags keep today's `encode`. The daemon's `SendKeys` path (`deck send-keys`, "encoded for the terminal's current modes") uses it too. Release and repeat events reach the program only when it asked for flag 2; deck's own key handling still ignores releases.
4. **Input coverage**: the client maps from crossterm every key kitty can report that crossterm exposes (F13–F35, keypad keys, media keys, modifier keys themselves under flag 8, Super/Hyper/Meta modifiers); the legacy path keeps ignoring what it cannot encode.
5. **Outer terminal without the protocol**: a program that pushed flags gets the kitty encoding of the keys deck receives, but keys the outer terminal cannot tell apart (Shift+Enter arrives as Enter) are encoded as what arrived. `docs/features/keys.md` says Shift+Enter and similar keys need an outer terminal with the protocol (Ghostty, kitty, WezTerm, iTerm2, foot, Alacritty).
6. **Prefix**: the prefix key is recognised from its kitty-encoded form too; pressing it twice still sends it to the program, in the program's encoding.

## Acceptance criteria

- [ ] A program that pushes flags 1 and reads keys (a test script using `stty raw` and `od`) receives `CSI 13;2u` for Shift+Enter when the harness's outer terminal reports kitty support (the harness answers crossterm's query) and the legacy `\r` without it.
- [ ] `CSI ? u` from the program is answered with the flags it pushed; after `CSI < 1 u` with the previous ones.
- [ ] Focus moving between a terminal with flags and one without pushes and pops the outer flags (seen in the client's raw output), and detaching pops them.
- [ ] With flag 2, a key release reaches the program as `CSI …;1:3u`; without it, nothing.
- [ ] The prefix key works with and without flags, and twice sends it to the program.
- [ ] `encode_kitty` has a table test against the protocol's examples for each flag.

## Tests

- `deck-core` table tests for `encode_kitty`; `deck-daemon` tests for the flag stack and query reply through the emulator; e2e tests with the harness answering the kitty support query.

## Notes

- The harness's reference `alacritty_terminal` answers `CSI ? u` itself when its config's `kitty_keyboard` is on; the harness gains a builder option to turn it on per test. It stays off by default, so existing tests keep seeing no support.
