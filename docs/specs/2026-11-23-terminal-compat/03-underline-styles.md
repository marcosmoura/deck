---
type: shippable
status: todo
---

# 03 — Underline styles and colour

## Scope

Curly, double, dotted and dashed underlines and coloured underlines, as editors use them for diagnostics and spelling, show inside deck as they do outside. Owning crates: `deck-core` (`Style`), `deck-daemon` (from the emulator), `deck-tui` (to the outer terminal).

## Requirements

1. `protocol::Style` replaces the `UNDERLINE` flag with `underline: Underline` (`None`, `Single`, `Double`, `Curly`, `Dotted`, `Dashed`) and adds `underline_color: Color` (`Default` meaning the text colour). `PROTOCOL_VERSION` goes up by one.
2. The daemon fills them from the emulator's cell attributes (SGR `4`, `4:0`–`4:5`, `21`, `24`, `58:2::r:g:b`, `58:5:n`, `59`).
3. `deck capture --ansi` writes them back as SGR `4:n` and `58`.
4. The client writes them to the outer terminal. ratatui cells carry the underline colour (`underline-color`, a default feature) and `UNDERLINED`; the kind is carried inside the same `Cell` so that `Buffer::diff` sees a change of kind alone, for example in modifier bits deck never uses otherwise (the blink bits: blink is a non-goal), with the mapping documented in `docs/architecture.md`. Drawing goes through deck's own `Backend` implementation: its `draw` is deck's loop over the changed cells, writing the style itself and `4:n` after the modifier changes (crossterm's `CrosstermBackend::draw` would write plain `4` and reset after it); the other `Backend` methods delegate to crossterm. Outer terminals that do not support `4:n` show a plain underline (they ignore the sub-parameter); no detection is needed.
5. The client never forwards an underline colour on the Unicode placeholder cells of DECK-19's images, where the foreground colour carries the image id. DECK-19 chose the first virtual placement because frames carried no underline colour; that choice can be revisited in a later task (noted in the report).
6. Without truecolour, an RGB underline colour is downsampled like other colours.

## Screen

Unchanged layout. A word under a curly red underline (`printf '\e[4:3m\e[58:2::255:0:0mword\e[0m'`) looks the same as when printed outside deck.

## Acceptance criteria

- [ ] Compat cases for each underline kind, `21`, `24`, `58` in 256-colour and RGB forms and `59` pass against the reference, comparing the underline kind and colour.
- [ ] `deck capture --ansi` round-trips them (capture, print, capture again: equal).
- [ ] The raw output of the client contains `4:3` for a curly underline and `58;2;r;g;b` (the form crossterm writes) for its colour.
- [ ] A `deck-tui` unit test: a frame where only a cell's underline kind changes redraws that cell.
- [ ] A protocol test pins the new `Style` encoding.

## Tests

- Compat cases; `deck-core` protocol tests; `deck-tui` unit tests for the backend wrapper's output.
