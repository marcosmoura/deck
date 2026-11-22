---
type: shippable
status: todo
---

# 04 — The New button and menu

## Scope

Hovering a project row in the sidebar shows a `[ New ▾ ]` button; clicking it (or pressing `prefix N`, or `n` in sidebar focus) opens a small menu: `Terminal`, `Claude`, `OpenCode`, `Codex`. Each item creates a separate panel in that project. Owning crate: `deck-tui`; the binding in `deck-core`; end-to-end tests in `crates/deck/tests/`.

## Requirements

1. **Hover button**: while `hover == Hover::ProjectNew(id)` or the project row is the sidebar selection in sidebar focus, the project row shows `[ New ▾ ]` (9 cells) right-aligned with one cell of margin, drawn in the `accent` role (selection colours when the pointer is on the button itself). While shown it wins the right edge over everything else right-aligned on the row (the collapsed panel count, DECK-06's waiting count `!<n>`), which reappear when the hover ends; it takes its room off the name (the name truncates with `…`). The row under the pointer sets the hover from `Moved` events (sub-spec 02's state); the button is part of the row, so moving onto it keeps it.
2. **Opening**: a left click on the button, `prefix N` (binding `new_menu`, default `N`, in the Projects group of the help overlay; menu for the active project) and `n` in sidebar focus (menu for the selected project, or the project of the selected panel) open the menu. A click on the rest of the project row still toggles it. With no project, `prefix N` does nothing.
3. **Menu** (see [Screen](#screen)): a rounded overlay titled `New in <project>` (truncated), four items in a fixed order, each with its digit at the right. It is 22 cells wide including its borders (20 inside), at least wide enough for `New in <project>` and never wider than the window. Its top border is on the row below the project row and its right edge on the sidebar's last column (the one before the separator), so at the minimum sidebar width of 20 it extends over the panel area; it flips above the row (bottom border on the row above) when there is no room below. Opened from the keyboard, it anchors the same way on the project's row; with the sidebar hidden, at the top right of the window. The menu is an overlay: it takes the keyboard and the mouse (input-mode order step 1), and `on_mouse` routes events to it before the sidebar.
4. **Keys** (fixed): `Up`/`k`, `Down`/`j`, `Ctrl+p`/`Ctrl+n` move; `Enter` chooses; `1`–`4` choose directly; `Esc` or `q` closes; the prefix key is ordinary there. **Mouse**: hover moves the highlight, a click on an item chooses it, a click elsewhere closes the menu and is not passed on, the wheel does nothing.
5. **Choosing** sends the existing `Action::NewPanel { project, name: None }` for `Terminal` and `Action::NewPanelWith { project, launch: Agent(Claude | OpenCode | Codex) }` (sub-spec 03) for the others, then closes the menu. `Terminal` is a new panel with a shell in the project root: it is not a terminal added to the active panel's grid. The new panel becomes active and focused, in whichever project it was made (the sidebar expands that project if collapsed). An `ActionError` (a missing program, project closed meanwhile) shows a 4 s error flash and closes the menu.
6. **Closed target**: when the project is removed while the menu is open (another client), the menu closes with the flash `project "<name>" was closed` (DECK-02's rule).
7. **Without motion events**: the hover button never shows, but sidebar focus (`prefix Tab`, then `n`) and `prefix N` reach the same menu.
8. **Hints**: the sidebar-focus status-bar hints gain `n menu`: `↑↓ move  ⏎ open  ←→ fold  c new  n menu  r rename  x close  esc back`.
9. **Config**: `new_menu = "N"` in `[keys]`, listed in `default.toml`.

## Screen

100×24, sidebar 28 wide and the separator (`│`), panel area cut; the pointer is on the project `budget` and the menu was opened by a click on its button. The menu is 22 cells wide with its borders; its top border is on the row below the project row and its right edge on the sidebar's last column:

```text
 main                       │
                            │
 ▾ terminal-multiplexer     │
▌  editor                   │
   claude                   │
 ▾ budget         [ New ▾ ] │
   api╭─ New in budget ────╮│
   no │ ❯ Terminal       1 ││
 ▸ dot│   Claude         2 ││
      │   OpenCode       3 ││
      │   Codex          4 ││
      │ ⏎ choose  esc close││
      ╰────────────────────╯│
```

- The `❯` row is highlighted in the selection colours; the hint row is `muted`; the title and borders use `border`.
- The button uses `accent`; the name is truncated to make room (`budget` fits at 28 columns; a 20-character name shows as `terminal-mu…`).
- Sidebar-focus hints: `SIDEBAR  ↑↓ move  ⏎ open  ←→ fold  c new  n menu  r rename  x close  esc back`.

## Acceptance criteria

- [ ] Moving the pointer onto a project row shows `[ New ▾ ]` right-aligned (capture `new-button`); moving it to another row moves the button; moving it into the panel area removes it.
- [ ] A click on the button opens the menu with four items in order (capture `new-menu`); a click on the project's name still collapses it.
- [ ] `Terminal` adds one panel to that project with a shell whose `pwd` is the project root; the panel count of the project grows by one, the terminal count of the previously active panel is unchanged, and the new panel is active.
- [ ] With fake `claude`, `opencode` and `codex` scripts on `PATH`, items 2, 3 and 4 each add a panel running that script in the project root; the sidebar shows the new panels.
- [ ] With no `opencode` on `PATH`, choosing `OpenCode` shows the `not found in PATH` flash, closes the menu and adds no panel.
- [ ] `[agents.codex] command = "codex --fast"` is honoured.
- [ ] `⌥Space N` opens the menu for the active project; `⌥Space Tab`, `j`, `n` opens it for the selected project; `Esc` closes it; `3` chooses OpenCode.
- [ ] A click outside the menu closes it and does not toggle the row under the click.
- [ ] The menu flips above its row near the bottom of the window and stays inside the window at 40×12.
- [ ] A project removed by another client while the menu is open closes it with the flash.
- [ ] In sidebar focus the status bar shows the hints with `n menu`.
- [ ] A `[keys] new_menu = "M"` override moves the binding and shows in the help overlay.
- [ ] Hovering rows without clicking causes no extra frames beyond the hover change (sub-spec 02's no-redraw rule still holds).

## Tests

- `deck-core`: `new_menu` default and override, help-group listing, `default.toml` sync.
- `deck-tui` with `TestBackend`: the button on hover, truncation, menu drawing and anchoring (below, above, narrow window, hidden sidebar), key handling, mouse routing before the sidebar, the actions sent per item, flash on error, closing when the project is removed.
- `crates/deck/tests/sidebar_mouse.rs`: one scenario per criterion with SGR mouse sequences (a `Moved` event is button 35: `ESC[<35;5;7M`) and fake agent scripts on `PATH`.

## Notes

- Menu items carry no icons; DECK-09 icons can be added later through the same `icon_for` choice.
- `Terminal` here and `prefix t` differ on purpose: the first makes a panel (a sidebar row), the second a terminal inside the active panel's grid.
