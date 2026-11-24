---
branch: feat/sidebar-terminals
backlog: DECK-27
---

# Terminals in the sidebar

## Goal

Show the terminals of a grid panel in the sidebar. Today a panel that holds several terminals is one row with a terminal count, so you cannot see what runs in each terminal or which agent needs you without opening the panel. With this change, a panel with two or more terminals gets one nested row per terminal under its own row. Each terminal row shows its icon, title and agent status, and a click or `Enter` focuses that terminal.

Asked on 24 Nov 2026: "When we have multiple terminals in a grid, we need to be able to see them in the sidebar. Maybe we can have some sort of different layout for this case." Agreed layout: nested rows, always shown, not foldable per panel; a single-terminal panel stays one row.

## Builds on

- [DECK-02 sessions-sidebar](../2026-11-22-sessions-sidebar/00-overview.md): sidebar rows, sidebar focus and its selection.
- [DECK-03 grid-layout](../2026-11-22-grid-layout/00-overview.md): terminal order and numbers, `close_terminal`, focus.
- [DECK-06 agent-status](../2026-11-22-agent-status/00-overview.md) and [DECK-09 sidebar-titles](../2026-11-22-sidebar-titles/00-overview.md): status slot, icons, title cleanup (`naming::title_core`).
- [DECK-10 sidebar-mouse](../2026-11-22-sidebar-mouse/00-overview.md), [DECK-17 floating-bar-menus](../2026-11-23-floating-bar-menus/00-overview.md): clicks, hover, the row menus and the terminal menu.
- [DECK-18 sidebar-reorder](../2026-11-23-sidebar-reorder/00-overview.md): the press/release click, row drags, `rows_with`.

## Non-goals

- Folding a single panel's terminals. Spaces still fold; a panel does not.
- Reordering or moving terminals from the sidebar, or dragging a terminal row.
- Naming terminals. A terminal row shows the program's title, as the grid border does.
- Changing the daemon, the protocol or the state file. The client already receives each terminal's program, title, exit status and agent state, and `Act::FocusTerminal` already exists.
- A config key to turn this off.

## Decisions

- **Which panels.** A panel with two or more terminals is followed by one terminal row per terminal, in grid order, whenever its space is expanded. A panel with one terminal has no terminal row, and its row is unchanged. The rows come and go as terminals are added and closed.
- **The panel row** of a grid panel keeps its icon, name, unseen marker and status slot (the most urgent terminal). It drops its terminal count, because its terminal rows show the count. A collapsed space still shows its panel count.
- **A terminal row** starts with a muted tree glyph in column 4 (the panel names' column): `├` for every terminal but the last, `└` for the last. Then come a space, the terminal's icon (two cells, from its own agent or program, like a panel's icon), a space and its label. The label starts at column 9. The label is the terminal's title with the spinner and status glyphs removed (`naming::title_core`; the grid border keeps the raw title), else its program when the cleaned title is empty or missing. A kept terminal that exited adds ` · exited <code>` in muted text; the label is cut first, then the note, as on a panel row. The status slot at the right shows that terminal's agent status, with the same rules as a panel's slot. A terminal row has no number (the icon and title are enough, and a number costs two cells), no count and no unseen marker.
- **Focus.** The focused terminal of the active panel has its row drawn in the accent colour, bold. The band stays on the active panel's row, as today. In sidebar focus, the band goes to the selected row, which may be a terminal row.
- **Selection.** `Sel` gains `Terminal(ProjectId, PanelId, TerminalId)`. In sidebar focus, `Up`/`Down` pass through terminal rows. Entering sidebar focus still starts on the active panel's row. On a terminal row:
  - `Enter` makes its panel active and focuses that terminal (`Act::FocusPanel`, then `Act::FocusTerminal`, as `NextAttention` does), and returns to the terminal.
  - `Left`/`Right` fold or unfold its space.
  - `c` and `n` act on its space.
  - `r` renames its panel.
  - `x` closes that terminal through `confirm_close_terminal`: it asks `Close terminal <number>?`, except for a kept terminal that exited, which closes at once.
- **Mouse.** A click on a terminal row (press and release on the same row) sends `Act::FocusPanel` for its panel, then `Act::FocusTerminal` for it, so its panel and space become active (`FocusTerminal` alone only moves the panel's focused terminal). Terminal rows get the hover background like other rows. A right click opens the [terminal menu](../../features/layout.md#the-terminal-menu) with that terminal as its target (`New terminal`, `Rename panel`, `Close terminal`). `New terminal` makes the target terminal's panel active (`Act::FocusPanel`) and adds the terminal to it with `Act::NewTerminal { panel: Some(..) }`, never to another panel. The menu remembers the sidebar row it was opened from (`None` when opened from the grid), and only that row keeps the hover background while it is open; a menu opened on a grid terminal lights no sidebar row.
- **Drag.** A press on a terminal row never starts a drag: moving to another row drops the press, and nothing happens on release. While a panel is dragged, its terminal rows move and dim with it, and a terminal row counts as its panel's row for a panel drag's target and as part of its space's block for a space drag's. Only the panel row of the target gets the `➤` caret and the shift. `rows_with` keeps every terminal row after its own panel row when it applies a `Reorder`.
- **Scrolling.** `ensure_visible` keeps the active panel's row in view, or the selection in sidebar focus, as today. Terminal rows count as rows for the list height and the wheel.
- **Whole client.** Every change is in `deck-tui`, the end-to-end tests and the docs.

## Sub-specs

| Sub-spec                  | Type      | Crate                  | Summary                                                                      | Depends | Status |
| ------------------------- | --------- | ---------------------- | ---------------------------------------------------------------------------- | ------- | ------ |
| [01](01-terminal-rows.md) | shippable | deck-tui, deck (tests) | Terminal rows under grid panels: drawing, selection, keys, mouse, menu, drag | —       | todo   |

## Affected docs

- `docs/features/sidebar.md`: the mockup, the panel row without a count for grid panels, terminal rows, sidebar focus keys on a terminal row, clicks, the row menu table and reordering. Also fix "a panel shows one terminal (the grid comes later)".
- `docs/architecture.md`: the bottom bar and hover paragraph (`Hover::Row(Sel)` now covers terminal rows), the sidebar row gestures (a press on a terminal row, which never drags), and the context menu (opened from a terminal row of any panel, with the row it came from).

Reviewed by spec-reviewer on 2026-11-24.
