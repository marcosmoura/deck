---
type: shippable
status: done
---

# 02 — Floating bar

## Scope

The status bar goes away. The sidebar and the panel area take the full height, and a one-row floating bar shows the modes and the flashes over the bottom of the panel area. The session name moves to the sidebar header. Owning crates: `deck-core` (`layout::panel_area`, `PROTOCOL_VERSION`), `deck-tui`; the `deck-daemon` integration tests and the end-to-end tests in `deck` follow the full height (no daemon source change).

## Requirements

1. **Full height.** `deck_core::layout::panel_area` no longer subtracts a row (its doc comment and `layout/tests.rs` follow), so the client's viewport and the daemon's attach-time sizing gain it. `PROTOCOL_VERSION` goes from 16 to 17. In `deck-tui`, `draw` gives the sidebar and the panel area every row of the window, and every place that subtracts the status row goes: the sidebar heights (`sidebar.rs`), `App::terminal_at` (`app.rs`, so the last row takes the mouse), `resize.rs`, `menu.rs`'s `menu_area`, `restore.rs`, the help and picker areas (`draw.rs`). `too_small` keeps its limits.
2. **When the bar shows.** The bar is drawn when at least one holds: the prefix is armed, the resize repeat is on, a terminal border drag or the sidebar separator drag is in progress, scroll mode, sidebar focus, or a flash is live. It is not drawn while a dialog (sub-spec 01) is open (the dialog shows the flash), while the help overlay, the session picker, the New menu or the restore dialog is open (a flash is still drawn, alone, over them), or while the window is too small.
3. **What it shows.** The first state that holds, in this order, picks the badge (today's precedence, with the border drag next to the resize repeat):

   | State                                                         | Badge         | Then                                                                                |
   | ------------------------------------------------------------- | ------------- | ----------------------------------------------------------------------------------- |
   | Sidebar separator drag                                        | `SIDEBAR <w>` | nothing                                                                             |
   | Prefix armed                                                  | `PREFIX`      | `esc cancel  ? help` (`?` = first key of `help`; left out when unbound)             |
   | Resize repeat or border drag, no scroll mode or sidebar focus | `RESIZE`      | the repeat: `Bindings::resize_hints()` (`H J K L resize  = reset`); a drag: nothing |
   | Scroll mode                                                   | `SCROLL`      | `[offset/history]` in `warning` when scrolled back, then `q exit  g top  G bottom`  |
   | Sidebar focus                                                 | `SIDEBAR`     | `SIDEBAR_HINTS`                                                                     |
   | Flash only                                                    | none          | the flash text                                                                      |

   A live flash replaces the hints (not the badge or the scroll indicator), in its level colour (info `muted`, warning `warning`, error `error`, as today).

4. **How it looks.** One cell of padding, the badge (its text with one space on each side, 8 cells for `PREFIX`) in `warning` reversed, a space, the hints: each `key label` pair with the key in `status_fg` bold and the label in `muted` (the hint sources become lists of key and label pairs: `SCROLL_HINTS`, `SIDEBAR_HINTS`, `Bindings::resize_hints()`; `H J K L` is one key text), two spaces between pairs; one cell of padding after. Background `status_bg`, foreground `status_fg`. Its width is its content, at most the panel area's width; it is centered in the panel area (an odd remainder goes to the right) on the window's last row. Too wide, the hints are cut with `…`; when not even the badge fits, the badge is cut. With the sidebar hidden, the panel area is the whole window.
5. **Under the bar.** The cells it covers are not drawn from the terminal while it shows, and come back on the next frame without it. The cursor is not shown when it is on a covered cell. A mouse press whose cell is covered by the bar is not sent to a program and starts no selection or drag; motion and the wheel there reach nothing. A selection, border drag or separator drag already in progress continues when the pointer crosses the bar, as it does over any other cell.
6. **Session name.** The sidebar header shows the session name (`accent`, bold, cut with `…`) where `Spaces` was; the `+ New space` chip stays. Before the first session view the title is empty. A session rename updates it at once.
7. **Dropped.** The breadcrumb, a single terminal's title, `<n> waiting`, the `a next` hint, the idle hints and the restore dialog's status-bar hints are no longer drawn anywhere. `Bindings::hints`, `HINTED` and `restore::hints` go if nothing else uses them.
8. **Exited note.** A kept terminal that exited and is alone in its panel adds ` · exited <code>` after the panel's name in its sidebar row, in `muted`, inside the name's room (the name is cut first).
9. **Logos.** The status bar's icon image is gone: the image registration before each draw no longer asks for it.

## Screen

80×12, sidebar 28 (27 columns and the separator), prefix armed (`⌥Space` pressed). The bar is 29 columns, centered in the 52-column panel area (columns 28 to 79): 11 columns on its left, 12 on its right, so the bar's padding cell is column 39, the badge's first cell column 40 and `P` column 41.

```text
 main          + New space │~/Projects/deck $ cargo test
                           │   Compiling deck v0.1.0
 ▾ deck                    │    Finished test
▌  ❯  zsh                  │
                           │
                           │
                           │
                           │
                           │
                           │
                           │
  + New            Restore │             PREFIX  esc cancel  ? help
```

The bar's cells: padding, `PREFIX` reversed in `warning`, a space, `esc` bold, ` cancel` muted, two spaces, `?` bold, ` help` muted, padding. The window's last row is the sidebar's bottom bar row too. The `▌` stays until sub-spec 03.

Sidebar focus, 80 columns: `SIDEBAR` then the hints cut to fit with `…`. A flash with no mode: `config reloaded` in `muted`, centered.

## Acceptance criteria

- [ ] No status row: the sidebar and the panel area are as tall as the window, and the size the daemon gives the shells (on attach and after a `Viewport`) uses the full height.
- [ ] A client and daemon of different protocol versions refuse each other with the existing message.
- [ ] The bar shows in each state of requirement 3 with that content, and in no other state; it disappears when the state ends.
- [ ] The bar is centered in the panel area, cut with `…` when too long, and covers terminal cells only while shown; the cursor is hidden under it; presses on it reach no program; drags in progress cross it.
- [ ] Flashes and daemon notices show in the bar for their usual time, alone or in place of a mode's hints, and over the help, picker, menu and restore overlays.
- [ ] The sidebar header shows the session name; a rename of the session updates it at once.
- [ ] A kept, exited terminal alone in its panel shows ` · exited <code>` in its sidebar row.
- [ ] `docs/features/` (sidebar, keys, layout, sessions, agents, control-api, config) and `docs/architecture.md` describe the bar, and nothing describes a status bar.

## Tests

- `deck-core`: `panel_area` keeps every row.
- `deck-daemon` integration tests (`crates/deck-daemon/tests/`): the support's default attach window, the tests that pass their own size, the asserted viewports, frame sizes and `stty size` results, and the comments that explain the status row follow the full height.
- `deck-tui` unit tests (`draw.rs` and a new `bar_tests.rs`, as the crate's split test files do): the bar's content per state and the badge precedence (scroll mode and sidebar focus over a resize); a flash replacing hints; centering with odd and even remainders; cutting with `…` and the badge cut; no bar with a dialog open, and a flash alone over an overlay; the cursor hidden under the bar; a press on the bar sends nothing and starts no selection; a drag crossing the bar continues; the header shows the session name; the exited note. The status-bar tests of `draw.rs` are replaced, not kept.
- End-to-end (`crates/deck/tests/`): every test that reads the status bar row reads the bar or the sidebar header instead, and every assertion on a shell's height gains a row; one scenario arms the prefix and finds `PREFIX` and `? help` on the last row, then presses `Esc` and finds the terminal's own text there.

## Notes
