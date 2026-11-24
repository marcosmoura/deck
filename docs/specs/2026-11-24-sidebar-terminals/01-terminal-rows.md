---
type: shippable
status: todo
---

# 01 — Terminal rows under grid panels

## Scope

In the sidebar, a panel with two or more terminals shows one nested row per terminal. The user can see each terminal's program, title and agent status, and can focus a terminal with a click or with `Enter` in sidebar focus. Owning crate: `deck-tui`. End-to-end tests are in `crates/deck/tests/`.

## Requirements

- **Rows.** `sidebar::Row` gains `Terminal(ProjectId, PanelId, TerminalId)`, and `Sel` gains the same variant. `rows_with` pushes the terminal rows right after their panel row, for panels with at least two terminals in expanded spaces. A panel `Reorder` moves the terminal rows along with their panel. `Row::sel` maps a terminal row to `Sel::Terminal`, and `Sel::project` handles it.
- **Drawing** (`sidebar::draw`), per the overview's decisions:
  - The panel row of a grid panel has no count.
  - A terminal row has the tree glyph, the icon, the label, the exited note and the status slot.
  - The label is the title through `naming::title_core`, else the program when that is empty or missing, cut with `…`. When the label and the note do not both fit, the label is cut first, then the note, as `sidebar.rs` does for a panel row.
  - The focused terminal of the active panel is drawn in accent, bold, when the row has no band.
  - Band, hover, dimming and the selection colour patches work as they do on panel rows.
  - Add a terminal-level icon helper next to `panel_icon_kind` (from `term.agent` and `term.program`) and use it for both.
- **Selection and keys.** `selectable`, `move_selection`, `Enter`, `Left`/`Right`, `c`, `n`, `r`, `x` and `new_target()` handle `Sel::Terminal` as the overview says. `Enter` on a terminal row sends `Act::FocusPanel` then `Act::FocusTerminal`. `x` on a terminal row goes through `confirm_close_terminal` with that terminal (a kept terminal that exited closes without asking). A selection whose terminal is gone falls back the way a panel selection does today.
- **Mouse.**
  - `click_row` on `Sel::Terminal` sends `Act::FocusPanel { panel }` then `Act::FocusTerminal { terminal }` (the pair `NextAttention` sends), so a terminal of another panel or space becomes visible and focused.
  - `hover_at` returns `Hover::Row(Sel::Terminal(..))`.
  - A right press opens `ContextTarget::Terminal(terminal)`.
  - `ContextMenu` stores the sidebar row it was opened from (`Some(sel)` from `open_row_menu`, `None` from `open_terminal_menu`), and `ContextMenu::row` returns it, so only a menu opened from a sidebar row keeps a row's hover background.
  - The `Terminal` target's `New terminal` sends `Act::FocusPanel` for the target's panel, then `Act::NewTerminal { panel: Some(panel) }`; the `no panel to add a terminal to` check applies only when the target is gone.
- **Drag.**
  - A press on a terminal row is recorded so its release can click, but motion to another row ends the press with nothing sent, and the gesture is still swallowed.
  - `drop_target`: in the `Sel::Project` arm, a terminal row belongs to its project's block; in the `Sel::Panel` arm, a terminal row maps to its panel's index.
  - `place_of` and `drag_target_sel` get a `Sel::Terminal` arm that returns `None` (a terminal is never dragged).
  - The dragged panel's terminal rows are dimmed.
  - `check_row_gesture` handles a terminal row whose terminal is gone.
- **Docs.** Update `docs/features/sidebar.md` and `docs/architecture.md` as the overview lists.

## Screen

28 columns, space `deck` expanded. The active panel `api + tests` has three terminals; terminal 2 is focused and its agent waits for you. The band is on the active panel's row:

```text
 main         +  New space  │
                            │
 deck                    ▾  │
 ✻  Fix login bug        ⠹  │
 ❯  api + tests          !  │   ← band (active panel); status = most urgent
    ├ ❯  zsh                │   ← muted tree glyph; label in sidebar_fg
    ├ ✻  Write tests     !  │   ← focused terminal: accent, bold
    └ ✎  main.rs            │
 ❯  zsh                     │   ← single terminal: one row, unchanged
 dotfiles              2 ▸  │
                            │
  +  New         ↻  Resume  │
```

A narrow (20-column) sidebar: the label starts at column 9 and the status slot takes columns 17 and 18, so a terminal with an agent gets 7 cells for its label. A kept terminal that exited has no status slot and gets 10 cells; ` · exited 1` is 11, so the label goes entirely and the note is cut:

```text
    ├ ❯   · exited… │
    └ ✻  Review…  ⠹ │
```

Sidebar focus, selection on a terminal row. The band moves to it, and the active panel's row is accent, bold, with no band:

```text
 ❯  api + tests          !  │   ← accent, bold
    ├ ❯  zsh                │   ← band (selected)
```

Dragging the panel `api + tests` (dimmed, its terminal rows too) onto the place of `Fix login bug`:

```text
➤  ✻  Fix login bug      ⠹  │
 ❯  api + tests          !  │   ← dimmed
    ├ ❯  zsh                │   ← dimmed
    ├ ✻  Write tests     !  │   ← dimmed
    └ ✎  main.rs            │   ← dimmed
```

## Acceptance criteria

- [ ] A panel with two or more terminals shows one terminal row per terminal, in grid order, with `├`/`└`, the icon, the cleaned label and the terminal's own status. Its panel row has no count. A single-terminal panel is unchanged.
- [ ] Adding or closing a terminal adds or removes its row at once. Going from two terminals to one removes the terminal rows.
- [ ] The focused terminal of the active panel is accent, bold. The band stays on the active panel's row outside sidebar focus.
- [ ] In sidebar focus, `Up`/`Down` reach terminal rows. `Enter` on one focuses that terminal and leaves sidebar focus. `x` closes it through the terminal close confirmation (at once for a kept terminal that exited). `r` renames its panel. `Left`/`Right`, `c` and `n` act on its space.
- [ ] A click on a terminal row of any panel, active or not, makes its panel and space active and focuses that terminal. Hover works on terminal rows.
- [ ] A right click on a terminal row opens the terminal menu for that terminal. Its items act on that terminal and its panel even when the panel was not active; `New terminal` adds to that panel and makes it active. A menu opened on a grid terminal does not highlight a sidebar row.
- [ ] A terminal row cannot be dragged. A dragged panel carries its terminal rows, and the drop target rules treat terminal rows as their panel.
- [ ] A collapsed space hides its terminal rows. Scrolling and `ensure_visible` count terminal rows.
- [ ] `docs/features/sidebar.md` and `docs/architecture.md` describe the new rows.
- [ ] `just check` passes.

## Tests

- `deck-tui` unit tests, in `sidebar_tests.rs`, `sidebar_drag_tests.rs`, `sidebar_reorder_tests.rs` and `menu_tests.rs` (or the context menu tests):
  - `rows` with 1, 2 and 3 terminals, in collapsed and expanded spaces, and with a panel `Reorder`.
  - Drawing of a terminal row: tree glyphs, label cleanup, the exited note cut, the status slot, the focused terminal's accent, and the band in sidebar focus.
  - The panel row without its count.
  - Selection moves across terminal rows. `Enter`, `x` and `r` on a terminal row.
  - Click and hover on a terminal row, including one of a panel that is not active (both actions sent). A right press opens the terminal menu with the right target; `New terminal` from it sends `NewTerminal { panel: Some(..) }`. A menu opened on a grid terminal leaves its sidebar row without hover.
  - A press on a terminal row moved to another row sends nothing.
  - A panel drag over a terminal row targets that panel.
- End-to-end, in `crates/deck/tests/sidebar.rs` or `sidebar_mouse.rs`:
  - Open a panel, add two terminals (`prefix t`), and check that the screen shows three terminal rows under the panel.
  - Click the first terminal's row and check that the grid's focused border moves to terminal 1.
  - With a second panel active, click a terminal row of the grid panel and check that the grid panel becomes active with that terminal focused.
  - Close a terminal and check that its row goes away.

## Notes

Reviewed against the code on 24 Nov 2026: `FocusTerminal` alone does not change the active panel (daemon `server.rs`), hence the `FocusPanel` + `FocusTerminal` pair.
