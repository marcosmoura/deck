---
type: shippable
status: done
---

# 02 — Copy mode

## Scope

In scroll mode, a copy cursor moves with vi keys and selects text that can span any number of screens; `y` copies it. Mouse selections use the same row ids, so they survive scrolling and can grow past the screen's edges. Owning crate: `deck-tui`.

## Requirements

1. **Copy cursor**: entering scroll mode places it on the terminal's cursor cell (its row id and column). It is drawn in the theme's `accent` colour as its background, so it stays visible inside a selection drawn in the default reversed selection colours.
2. **Motions**: `h j k l` and the arrows; `w b e` (words, using the mouse selection's word rule); `0 $ ^`; `H M L` (top, middle, bottom of the screen); `g`/`Home` (top of the history), `G`/`End` (bottom); `C-u C-d` (half page), `C-b C-f` and PgUp/PgDn (page). A motion that crosses the top or bottom edge scrolls the view by exactly the rows it crossed (`ScrollTo::Lines`), so `k` at the top edge scrolls one line; `g`, `G` and search results use `ScrollTo::Row`. This changes scroll mode: `k`/`j` used to scroll at once, they now move the cursor first and scroll at the edge; `C-u`/`C-d`/`C-b`/`C-f` still scroll at once, carrying the cursor with the view. `docs/features/keys.md` says so.
3. **Shared view**: the scroll position belongs to the session, so another client's cursor motion, search or scroll can move this client's view. The copy cursor keeps its row id and, when it would be off screen, is drawn and moved from the nearest edge row (clamped). When this client's own `ScrollTo::Row` or `Lines` will reach the bottom, it sets `bottom_expected` as `scroll()` does today for the other variants, so its own motion never ends scroll mode. While the view is at the bottom (`display_offset` 0), new output moves rows up; the cursor keeps its row id and is clamped the same way.
4. **Selection**: `v` starts a character selection at the cursor, `V` a line selection, pressing the same key again clears it, the other key switches kind; `o` swaps the cursor with the other end. The selection is drawn in the selection colours, from row id to row id, clipped to the screen.
5. **Copy**: `y` or `Enter` with a selection sends `Request::Text` and, on the answer, writes OSC 52 like the mouse selection, flashes `copied <n> lines` (or `copied <n> characters` for one line), and clears the selection (scroll mode stays). Without a selection `y` copies the cursor's line. A truncated answer flashes `copied the first 8 MiB`. An error answer flashes it.
6. **`Esc`**, in order: clears a selection; else clears an active search's highlights ([03](03-search.md)); else leaves scroll mode as today.
7. **Mouse**: `TextSel` holds row ids instead of screen rows. A selection survives scrolling and new output (it is redrawn where its rows are). DECK-15's clearing rules become: the next press, a size change, a `numbering` change, and a change of a selected row's text, where `TextSel::stale` now compares rows by id (shifted by the change in `top_row`), not by screen position. In scroll mode, a drag past the top or bottom edge scrolls the view one row every 50 ms in that direction, extending the selection. The release copies through `Request::Text` when the selection spans rows off screen; otherwise from the mirror, joining rows by `Line.wrapped` the same way the daemon does.
8. **Bar**: in scroll mode the hints become `q exit  v select  / search  y copy`; while selecting, the badge reads `SELECT`, the indicator shows the selection's size (`3 lines`) and the hints are `y copy  o other end  Esc clear`.
9. The copy cursor and selections are dropped when `Frame.numbering` changes ([01](01-protocol.md)).

## Screen

Scroll mode on an 80×24 terminal, a selection of three lines, cursor at the end of the last:

```text
│ $ cargo build                                                             │
│ ▒▒▒▒Compiling deck-core v0.1.0▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒                             │
│ ▒▒▒▒Compiling deck-daemon v0.1.0▒▒▒▒▒▒▒▒▒▒▒▒▒▒                             │
│ ▒▒▒▒Finished `dev` profile█                                                │
│ $                                                                          │
                       SELECT  3 lines   y copy  o other end  Esc clear
```

`▒` is the selection colour, `█` the copy cursor; the bar is the floating bar of DECK-17 in the `warning` role like the `SCROLL` badge.

## Acceptance criteria

- [ ] `prefix [`, `v`, `k` ×30 (scrolling past the top), `y`: the OSC 52 payload in the client's raw output decodes to the 31 lines, equal to `capture --history`'s same range.
- [ ] `V` then `G` copies from the line to the bottom; `o` then `k` grows the other end.
- [ ] `w`/`b`/`e`/`0`/`$` move as specified on a line of mixed words and punctuation (unit tests).
- [ ] A mouse selection made, then scrolled away with the wheel and back, is still drawn and copies the same text.
- [ ] Dragging past the top edge in scroll mode scrolls and extends the selection.
- [ ] `y` with no selection copies the cursor's line; `Esc` with a selection clears it and stays in scroll mode; `Esc` again leaves.
- [ ] The bar shows the new hints and the `SELECT` badge.
- [ ] Two clients on one session: client A in scroll mode with a copy cursor; client B scrolls up 50 lines; A's cursor is clamped to the edge and A stays in scroll mode; A pressing `G` returns to the bottom without leaving scroll mode.
- [ ] `k` at the top edge scrolls exactly one line.
- [ ] A selection of a wrapped line copied from the screen and the same selection copied after scrolling it partly off screen give the same text.

## Tests

- `deck-tui` unit tests for motions, selection by row ids, edge scrolling and the bar; e2e tests in `crates/deck/tests/scroll_mouse.rs` (or a new `copy_mode.rs`) for the criteria with OSC 52 output.
