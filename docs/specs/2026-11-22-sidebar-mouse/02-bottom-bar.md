---
type: shippable
status: done
---

# 02 — Bottom bar and hover

## Scope

The sidebar gets a bottom bar with a `New project` button at the left that opens the existing new-project prompts. The right side is reserved for DECK-11's `Restore` button. Buttons highlight under the pointer. Owning crate: `deck-tui`.

## Requirements

1. **Bottom bar**: the last row of the sidebar area (just above the status bar) is the bar; the row above it is a blank spacer. The project list's height shrinks by these two rows (`list_height`), and `ensure_visible` and the wheel use it. A sidebar shorter than 8 rows draws no bar and no spacer.
2. **Buttons**: `New project` is a chip: one space, `+ New project`, one space (15 cells), at the left (column 1). At a sidebar width under 26 it shortens to `+` (3 cells) so the bar still has room for the `Restore` slot. The right-aligned slot (`Restore`, `Restore` 9 cells, or `↻` 3 cells under 26 columns; DECK-11 draws it) is reserved: nothing else is drawn there.
3. **Click** on `New project` (left button down and up on it) starts the same flow as `prefix P`: the `Directory:` prompt prefilled with the client's start cwd, then `Project name:`. It works without sidebar focus. Nothing happens while a prompt, a confirmation or an overlay is open.
4. **Hover state**: the client keeps `hover: Option<Hover>` (`Hover::NewProject`, `Hover::Restore`, and `Hover::ProjectNew(ProjectId)` from sub-spec 04) set from `Moved` events over the sidebar and cleared by a `Moved` event anywhere else, by a focus loss, by a prompt or overlay opening, by a click and by a wheel event. The client also keeps the last pointer position; after anything that moves rows under a still pointer (a wheel scroll of the sidebar, a `SessionView` that adds, removes or collapses rows, a resize) the hover is recomputed from that position, and the screen is marked dirty only when the result differs. A hovered button is drawn in the theme's selection colours (`selection_bg`, `selection_fg`); the idle chip uses `muted` text on the sidebar background.
5. **Redraw only on change**: `on_mouse` marks the screen dirty for a `Moved` event over the sidebar only when `hover` changed. (Today every sidebar event sets the flag; clicks and wheel events keep doing so.) Motion over the panel area is forwarded to programs as now and never touches `hover`.
6. **Sidebar focus**: `P` already starts the flow; the bottom bar is not selectable.
7. **Hints**: the status-bar hints do not change.

## Screen

Sidebar 28 wide and the separator (`│`), 24-row window, rows cut; pointer over the `New project` chip (drawn in the selection colours; muted when idle). Nothing is drawn in the reserved slot until DECK-11:

```text
 main                       │
                            │
 ▾ terminal-multiplexer     │
▌  editor                   │
 ▸ dotfiles               2 │
                            │
                            │
  + New project             │
```

With DECK-11's `Restore` chip in the slot (9 cells, ending in column 26; the last column is a margin):

```text
  + New project    Restore  │
```

At 20 columns, both short chips (3 cells each, ending in column 18):

```text
  +              ↻  │
```

## Acceptance criteria

- [ ] The bottom bar shows `+ New project` at the left of the last sidebar row (capture `bottom-bar`) and nothing at the right.
- [ ] Clicking `+ New project` opens the `Directory:` prompt prefilled with the start cwd; typing a directory and a name adds the project exactly as `prefix P` does.
- [ ] Clicking it while a prompt is open does nothing.
- [ ] Moving the pointer onto the chip draws it in the selection colours; moving off restores it.
- [ ] With 30 projects the list scrolls inside a list area two rows shorter, and the bar stays at the bottom.
- [ ] At a sidebar width of 20 the chip is `+`; at 26 it is the full label.
- [ ] A `Moved` event over the sidebar that does not change the hover target does not mark the screen dirty (unit test), and in the e2e harness, after the first of 100 such events has settled, `raw_output().len()` is unchanged across the other 99.
- [ ] A program with mouse motion reporting on still receives motion made over its terminal and none made over the sidebar.
- [ ] A 7-row sidebar shows no bar and the list uses the full height.

## Tests

- `deck-tui` with `TestBackend`: bar drawing at widths 20, 25, 26, 28 and 60, hover colours, short sidebar, `list_height` change and scrolling, click flow, hover clearing rules, the dirty flag.
- `crates/deck/tests/sidebar_mouse.rs`: click flow end to end, hover capture (`bottom-bar-hover`), the no-redraw criterion.

## Notes

- The reserved slot is only a rule for what this sub-spec may draw; the clickable area of `Restore` is DECK-11's.
- Hover is client-local (never sent), like sidebar selection.
