---
branch: feat/floating-bar-menus
backlog: DECK-17
---

# Floating bar, sidebar highlight and context menus

## Goal

A lighter screen. The status bar at the bottom goes away; in its place a one-row floating bar appears over the bottom of the panel area only while something is going on (the prefix is armed, something is being resized, scroll mode, sidebar focus, a message). Prompts and confirmations become small centered dialogs. In the sidebar, the active row is shown by a background instead of the `▌` marker, and every row and chip reacts to the pointer. A right click opens a context menu on a terminal, a panel row or a space row.

Asked on 23 Nov 2026:

- Remove the footer in favour of a floating bar at the bottom, visible only when the leader keys are pressed or something is being resized: `[ PREFIX ] esc  cancel   ?  help`.
- The sidebar highlights the selection with a background colour instead of a `|` indicator at the left.
- Sidebar items, panels and everything have a hover effect.
- Right-click menus: on a terminal (`Split down`, `Split right`, `Rename`, `Close`), on a panel row (`Rename`, `Close`), on a space row (`Rename`, `New session`, `Close`); `New session` opens the `New` menu next to the click.

## Builds on

- [DECK-02 sessions-sidebar](../2026-11-22-sessions-sidebar/00-overview.md): the status bar, prompts and confirmations (`modal.rs`), flashes, the sidebar rows and sidebar focus.
- [DECK-03 grid-layout](../2026-11-22-grid-layout/00-overview.md): the resize repeat and its `RESIZE` badge, border drags, the click that focuses a terminal and is swallowed.
- [DECK-08 config-theming](../2026-11-22-config-theming/00-overview.md): theme roles, `[theme.glyphs]`, `default.toml` and its sync test, unknown-key warnings, `ServerMsg::Notice` flashes, `layout::panel_area`.
- [DECK-10 sidebar-mouse](../2026-11-22-sidebar-mouse/00-overview.md): the sidebar `Hover` state that redraws only on change, the chips, the separator drag and its `SIDEBAR <width>` badge, the `New in <space>` menu (`menu.rs`).
- [DECK-15](../../backlog.md#deck-15): text selection with the left button, the Option/Alt override against mouse reporting.
- [DECK-16](../../backlog.md#deck-16): every user-facing word says space.
- [architecture.md](../../architecture.md#client): the client's screen, overlays and mouse handling.

## Non-goals

- Directional splits. The grid stays automatic (DECK-03); the terminal menu's `New terminal` adds one where the grid puts it.
- Context menus anywhere else (the header, the chips, terminal borders, the session picker, the restore dialog), and right-click menus driven by the keyboard. Every menu item already has a key binding.
- Mouse support or hover in the session picker, the help overlay or the restore dialog beyond what they have.
- A configurable floating bar (position, contents, always-on), or any new key binding.
- Renaming the `status_fg` / `status_bg` theme roles: they colour the floating bar now.

## Decisions

- **No row is reserved.** The sidebar and the panel area take the whole window height; the terminals gain the row the status bar had. `deck_core::layout::panel_area` stops subtracting the row, so the client's viewport and the daemon's attach-time sizing (which calls the same function) both gain it, with no daemon source change. `PROTOCOL_VERSION` goes from 16 to 17, so an older daemon still running after an upgrade (which would size for one row less, while the client assumes they agree and sends no `Viewport`) refuses the client with the existing version-mismatch message. The floating bar is drawn over the last row of the panel area, centered on it, only while it has something to show. Mouse events on its cells are never sent to a program.
- **What the status bar carried goes here:**

  | Was in the status bar                                         | Now                                                               |
  | ------------------------------------------------------------- | ----------------------------------------------------------------- |
  | `PREFIX`, `RESIZE`, `SCROLL`, `SIDEBAR`, `SIDEBAR <w>` badges | The floating bar, with each mode's hints                          |
  | Flashes and daemon notices                                    | The floating bar, for their usual time, in their level colour     |
  | Rename, new-space and new-session prompts; y/n confirmations  | Centered dialogs (sub-spec 01)                                    |
  | Session name                                                  | The sidebar header, in place of `Spaces`                          |
  | `space › <panel>` breadcrumb, a single terminal's title       | Dropped: the sidebar shows the active space and panel             |
  | `<n> waiting` and the `a next` hint                           | Dropped: collapsed spaces show `!<n>`, panel rows show the status |
  | ` · exited <code>` of a kept terminal alone in its panel      | The panel's sidebar row: ` · exited <code>` after its name, muted |
  | The restore dialog's hints                                    | Dropped: the dialog draws its own hint row already                |
  | Idle hints (`⌥Space ? help  d detach`)                        | Dropped: the `PREFIX` bar shows `? help`                          |

  With the sidebar hidden, the session name, the active space and panel, attention and the exited note have no place on screen. That is accepted: `prefix b` brings the sidebar back.

- **The floating bar** is the badge (`PREFIX`, with its inner spaces, in the `warning` role reversed, as today) followed by hints, each a key in `status_fg` bold and a label in `muted`, two spaces between hints, one cell of padding at both ends, on `status_bg`. The prefix bar is `PREFIX` then `esc cancel  ? help` (`?` is the first key of `help`; the item is left out when `help` is unbound). A flash replaces the hints, as it did in the status bar; a flash with no mode shows alone. Too long for the panel area, the hints are cut with `…`; narrower than the badge, the bar shows the badge cut.
- **A terminal border drag** (DECK-03) also shows the bar, with the `RESIZE` badge and no hints, so every resize shows it.
- **Dialogs** replace the status-bar editor and question: a rounded box (the theme's `border_style`, `overlay_bg`) centered in the window, drawn over everything. A prompt has its label as the title (`Rename space`), the one-line editor, and `⏎ save  esc cancel` at the bottom; a refused value shows its reason in red on the bottom row for 4 s, as the flash did. A confirmation shows its question (without its ` (y/n)` suffix) and `y yes  n no`. Keys are the ones of today.
- **Active row and selection.** The active panel's row is drawn across the sidebar's width in the `selection` colours (reversed while both are `default`), bold; the `▌` marker and `[theme.glyphs] active` go away (a config that still sets `active` gets the unknown-key warning). In sidebar focus only the keyboard selection has the background, and the active panel keeps its accent and bold, so one row is highlighted at a time.
- **Hover** uses a new theme role, `hover` (default `bright-black`), as the background of the row or chip under the pointer: space rows, panel rows, the header chip, the bottom chips. Text drawn `muted` on a hovered row or chip is drawn in `sidebar_fg`, so it stays readable on the default (`muted` is `bright-black` too). The chips no longer use the selection colours under the pointer. The active (or selected) row keeps its selection background under the pointer.
- **Context menus** open on a right-button press and share the New menu's look and keys (rounded box, highlight in the selection colours, `↑↓`/`jk`/`Ctrl+p`/`Ctrl+n`, `⏎`, a digit, `esc`/`q`, hover, click, a click elsewhere closes it and goes no further). They have no title and no hint row. The box's top-left corner is the clicked cell, moved left and up so it stays inside the window. The release of the press is swallowed.

  | Right click on                 | Items                                            | Does                                                                                          |
  | ------------------------------ | ------------------------------------------------ | --------------------------------------------------------------------------------------------- |
  | A terminal of the active panel | `New terminal`, `Rename panel`, `Close terminal` | `new_terminal`, the rename-panel dialog, the close-terminal confirmation, for that terminal   |
  | A panel row                    | `Rename`, `Close`                                | The rename-panel dialog, the close-panel confirmation                                         |
  | A space row                    | `Rename`, `New panel`, `Close`                   | The rename-space dialog, the `New in <space>` menu at the click, the close-space confirmation |

  The user's `Split down` / `Split right` become one `New terminal` (the grid places it); `New session` is `New panel` (DECK-16's one-name rule: the sidebar's rows are panels).

- **Right click and programs.** A program with mouse reporting keeps the right button; with Option/Alt held the press opens deck's menu (DECK-15's rule for the left button). In scroll mode the press always opens the menu. On an unfocused terminal, a right press without Alt only focuses it (today's swallowed focus click), unless the program has no mouse reporting or scroll mode is on, in which case it focuses and opens the menu; with Alt it always focuses and opens the menu.
- **Hover redraws only on change**, as DECK-10 decided for the chips: the `Hover` state gains rows, and motion over the sidebar redraws only when the hovered row or chip changes.
- **No protocol message, persisted field or daemon source change.** `deck-core` changes `layout::panel_area`, `PROTOCOL_VERSION`, the theme (`hover`, the removed `active` glyph) and `default.toml`.

## Sub-specs

| Sub-spec                  | Type      | Crate                                                  | Summary                                                                                                | Depends | Status |
| ------------------------- | --------- | ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------ | ------- | ------ |
| [01](01-dialogs.md)       | shippable | deck-tui, deck (tests)                                 | Prompts and confirmations become centered dialogs; the status bar stays                                | —       | todo   |
| [02](02-floating-bar.md)  | shippable | deck-core, deck-tui, deck-daemon (tests), deck (tests) | Status bar removed, full-height layout, floating bar for modes and flashes, session name in the header | 01      | todo   |
| [03](03-sidebar-hover.md) | shippable | deck-core, deck-tui, deck (tests)                      | Active row and selection as a background, `hover` role, hover on rows and chips                        | 02      | todo   |
| [04](04-context-menus.md) | shippable | deck-tui, deck (tests)                                 | Right-click menus on terminals, panel rows and space rows; the New menu at the click                   | 03      | todo   |

All four change `deck-tui`'s drawing and mouse handling, so they run in order.

## Affected docs

- `docs/features/sidebar.md`: the mockup, the header's session name, the active row, hover, the exited note, the context menus of the rows, the New menu at a click, prompts as dialogs, the `SIDEBAR <width>` bar.
- `docs/features/keys.md`: the `PREFIX` bar, the `SCROLL` bar, mouse events on the bar.
- `docs/features/layout.md`: no status bar for a single terminal, the `RESIZE` bar (repeat and drag), the terminal context menu and the right button.
- `docs/features/sessions.md`: the session rename and new-session prompts as dialogs, the session name in the sidebar header.
- `docs/features/agents.md`: the dropped `<n> waiting` and `a next` hint.
- `docs/features/control-api.md`: the exited note of a terminal alone in its panel.
- `docs/features/config.md` and `default.toml`: `hover`, the removed `active` glyph, the comments of `status_fg`/`status_bg`, `accent` and `warning`, notices shown in the floating bar.
- `docs/architecture.md`: the client's screen (no status bar, the floating bar, dialogs), notices, agent logos (no status-bar icon), the panel area and the protocol version wherever they are described.

Reviewed by spec-reviewer on 23 Nov 2026.

Approved on 23 Nov 2026.
