---
type: shippable
status: done
---

# 04 — Context menus

## Scope

A right click opens a small menu on a terminal of the active panel, on a panel row and on a space row. The space row's `New panel` opens the `New in <space>` menu at the click. Owning crate: `deck-tui`; end-to-end tests in `deck`.

## Requirements

1. **Targets and items.**

   | Right press on                                          | Items (digit)                                                | Each item                                                                                                                                                               |
   | ------------------------------------------------------- | ------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
   | A terminal cell of the active panel (inside its border) | `New terminal` (1), `Rename panel` (2), `Close terminal` (3) | `Action::NewTerminal` as `prefix t` sends it (same refusal flash); `open_rename_panel` for the panel; the close-terminal confirmation for that terminal (requirement 9) |
   | A panel row                                             | `Rename` (1), `Close` (2)                                    | `open_rename_panel`; `confirm_close_panel`                                                                                                                              |
   | A space row                                             | `Rename` (1), `New panel` (2), `Close` (3)                   | `open_rename_project`; the New menu for that space, anchored at the click (requirement 5); `confirm_close_project`                                                      |

   Anything else (the header and its chip, the bottom bar, blank rows, `no panels`, the separator, terminal borders and titles, the floating bar, the empty panel area) opens no menu. A right press on an unfocused terminal's border or title still focuses it, as any press does today; anywhere else the press goes nowhere.

2. **Unfocused terminal.** A right press on a terminal that is not focused sends `FocusTerminal` for it and is swallowed, as a left press is today. It also opens the menu when deck has the right button there (requirement 3: the program has no mouse reporting, Option/Alt is held, or scroll mode is on); without that, it only focuses. The menu's items act on that terminal. The close item confirms with that terminal's number even if focus moves before it is chosen; if the terminal is gone by then, the menu closes with no action.
3. **Programs and the right button.** On a terminal whose program has mouse reporting on, the right press goes to the program as today, unless Option/Alt is held; then deck opens the menu and the program gets nothing. In scroll mode the right press always opens the menu. The release that follows a press that opened a menu is swallowed (`swallow_release`). The sidebar never forwards the mouse, so its rows always open their menu.
4. **The menu.** A rounded box (`border_type(theme.border_style)`, background `overlay_bg`, border `border`), no title, no hint row, one row per item: one cell of padding, the label, the digit right-aligned in `muted`, one cell of padding; inner width the longest label plus 4, at least 16. The highlight (first item when it opens) is drawn in the selection colours across the inner width. The top-left corner is the pressed cell; when the box would leave the window it moves left (right edge on the window's last column) and up (bottom edge on the last row). Keys and mouse as the New menu: `Up`/`k`, `Down`/`j`, `Ctrl+p`/`Ctrl+n` move; `Enter` or a digit chooses; `Esc`/`q` close; hover moves the highlight; a click on an item chooses it; a press anywhere else closes the menu and goes no further (a right press elsewhere too, it does not open another menu). The prefix key is ordinary while it is open; the wheel does nothing. These keys are fixed.
5. **The New menu at a point.** The New menu gains an optional anchor. Opened from a space row's `New panel`, it sits with its top-left corner at the right press's cell, moved to stay in the window as in requirement 4; from the `+ New` chip, `prefix N` and sidebar-focus `n` it sits where it does today. Everything else about it is unchanged.
6. **Stale targets.** When the menu's panel, space or terminal is closed meanwhile (another client, a process exit), the menu closes with `panel "<name>" was closed` / `space "<name>" was closed` as the New menu does; a terminal menu whose terminal is gone closes silently.
7. **State.** One menu at a time: the context menu and the New menu never show together (choosing `New panel` replaces one with the other). While a context menu is open the sidebar hover is cleared, the bar of sub-spec 02 is not drawn (flashes still are), and the row the menu belongs to keeps a `hover` background. The context menu is an overlay for `overlay_open()`, and has the mouse right after the restore dialog and before the New menu, as the New menu does today.
8. **Drawing order.** The context menu is drawn over the panels and the sidebar, under the dialogs of sub-spec 01 (a dialog opened from it replaces it anyway).
9. **Closing a given terminal.** `confirm_close_terminal` gains the terminal to close (the key binding passes the focused one). For the menu's terminal it keeps today's rules: a kept terminal whose process ended closes at once, any other asks `Close terminal <number>?` with that terminal's place in the panel.

## Screen

Right click on terminal 2 of a two-terminal panel (window cropped on the right):

```text
 main          + New space │╭ 1 · zsh ──────────╮╭ 2 · nvim ─────────╮
                           ││ $                 ││ fn main() {       │
 ▾ deck                    ││                   ││   ╭──────────────────╮
███❯  zsh██████████████████││                   ││   │ New terminal   1 │
                           ││                   ││   │ Rename panel   2 │
                           ││                   ││   │ Close terminal 3 │
                           ││                   ││   ╰──────────────────╯
```

Right click on the `budget` space row, then `New panel`:

```text
 ▸ budget     ╭────────────────╮
              │ Rename       1 │
              │ New panel    2 │
              │ Close        3 │
              ╰────────────────╯
```

The press was on column 14 of the `budget` row: the menu's corner is that cell, 16 columns inside (`New panel` plus 4, raised to the minimum of 16), drawn over the sidebar and the separator. The first item is highlighted in the selection colours. Choosing `New panel` replaces it with the `New in budget` menu, its corner on the same cell.

## Acceptance criteria

- [ ] A right click on a terminal, a panel row or a space row opens the menu with the items of requirement 1 at the pointer, kept inside the window; elsewhere it opens nothing.
- [ ] Each item does what its key binding or sidebar key does, for the clicked target; `New panel` opens the `New in <space>` menu at the click.
- [ ] An unfocused terminal is focused by the right press, which opens the menu only when deck has the right button there; the release is swallowed.
- [ ] A program with mouse reporting gets the right button unless Option/Alt is held; in scroll mode the menu always opens.
- [ ] Keys, hover, clicks and outside presses behave as in requirement 4; a closed target closes the menu with the message of requirement 6.
- [ ] `docs/features/sidebar.md` (row menus, the New menu at a point) and `docs/features/layout.md` (the terminal menu, the right button) describe it.

## Tests

- `deck-tui` unit tests (`menu_tests.rs` or a new `context_tests.rs`): the items per target; placement at the pointer and the moves at the right and bottom edges; each item's outcome (the action sent, the dialog opened, the New menu opened with its anchor); keys, digits, hover, click, outside press, right press elsewhere; the Option/Alt and scroll-mode rules against mouse reporting; the release swallowed; the unfocused terminal focused, with and without mouse reporting and Alt; closing a given terminal (asks with its number, an exited kept one closes at once); stale targets; no menu on the header, chips, borders, the bar or `no panels`.
- End-to-end: right-click a space row, choose `New panel`, then `Terminal`, and find a new panel row; right-click a terminal, choose `New terminal`, and find two terminals; right-click a panel row, choose `Close`, answer `y`, and find it gone.

## Notes

The highlighted item's digit is drawn in the selection colours, not `muted`, so it stays readable on the highlight (accepted in review).
