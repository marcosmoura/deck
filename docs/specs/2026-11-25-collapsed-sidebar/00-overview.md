---
branch: feat/collapsed-sidebar
backlog: DECK-48
---

# Collapsed sidebar

## Goal

Let the user shrink the sidebar to a narrow strip that still shows how every terminal is doing. A button in the header, right before the session name, collapses the sidebar. Collapsed, the sidebar is 8 columns wide and lists every terminal of the session as one entry: its agent status (or its program icon when it has no agent) and a number. A button at the top expands it again.

Asked on 25 Nov 2026: "implement a 'collapse sidebar' button. It should be displayed right before the session name. When collapsed, the sidebar should display the status of each terminal and a number for each." Agreed the same day: one entry per terminal, numbered 1..N across the whole session in sidebar order; the program icon for a terminal with no agent; each client's own state, with a key binding (unbound by default) and a config option to start collapsed.

## Builds on

- [DECK-02 sessions-sidebar](../2026-11-22-sessions-sidebar/00-overview.md): the sidebar, sidebar focus, `toggle_sidebar` and the narrow-window rule.
- [DECK-06 agent-status](../2026-11-22-agent-status/00-overview.md), [DECK-09 sidebar-titles](../2026-11-22-sidebar-titles/00-overview.md), [DECK-20 builtin-icons](../2026-11-23-builtin-icons/00-overview.md) (icon tiers and images): status marks, program icons, `icon_slot`.
- [DECK-10 sidebar-mouse](../2026-11-22-sidebar-mouse/00-overview.md), [DECK-17 floating-bar-menus](../2026-11-23-floating-bar-menus/00-overview.md): chips, hover, the New menu, the terminal menu.
- [DECK-27 sidebar-terminals](../2026-11-24-sidebar-terminals/00-overview.md): `Row::Terminal`, `Sel::Terminal`, and how a click or `Enter` on a terminal row focuses it.
- The sidebar width state and separator drag described in [architecture.md](../../architecture.md#client) (Sidebar width).

## Non-goals

- Showing the numbers anywhere else (grid borders keep `<number> · <title>` with the terminal's place in its panel), or a binding to jump to terminal N.
- Names in the collapsed sidebar, or a hover hint with the terminal's name.
- Resizing the collapsed sidebar, or dragging entries to reorder.
- Sharing the collapsed state between clients or saving it in the state file. It is client state, like a hidden sidebar.
- Changing the state file. The daemon changes only in how it derives the attach viewport from the config.
- Keeping the state across a live upgrade: the client re-execs and starts from `sidebar_collapsed`, as a hidden sidebar and a dragged width do today.

## Decisions

- **The button.** The header gets a chip at the left, before the session name: the `CollapseSidebar` icon (two cells) with one cell of clickable padding on each side (columns 0 to 3). The session name moves from column 1 to column 4 and is cut with `…` before the `New space` chip, as today. The chip is muted and gets the `hover` background under the pointer. A click (press and release on it) collapses the sidebar. Like the `New space` chip, it does nothing while a prompt, a confirmation or an overlay is open.
- **Collapsed width.** The collapsed sidebar is 8 columns plus the separator (`layout::COLLAPSED_SIDEBAR_WIDTH`). It shows when the window has at least 8 + 1 + `MIN_PANEL_COLS` (29) columns, by the same rule as the full sidebar. `App.sidebar_width` keeps the full width, so expanding returns to it (clamped to the window, as on a resize). The panel area gets the columns the sidebar gives up, and the client sends a `Viewport` on every change, as `toggle_sidebar` does. A window of 29 to 40 columns fits the collapsed strip but not the full sidebar (20 + 1 + 20 = 41): there the expand chip and `collapse_sidebar` leave it collapsed and flash `window too narrow to expand`, so the mouse never loses the sidebar.
- **Collapsed header.** The header row has only the `ExpandSidebar` chip at columns 2 to 5 (icon at 3 and 4, centred in the strip, two columns on each side). No session name, no `New space` chip. A click expands the sidebar.
- **Entries.** The list has one entry per terminal of the session: spaces in their order, panels in their order, terminals in grid order. A panel with one terminal has one entry. Folded spaces count: collapsing the sidebar shows every terminal, whatever is folded. Between the entries of two spaces there is one muted rule row (`─` in columns 1 to 6). A space with no terminals adds no entries and no rule. `sidebar_space_gap` does not apply.
- **An entry.** Column 0 blank, the mark in columns 1 and 2 (a two-cell icon slot, `icon_slot`, so it is an image where images are on), a blank, then the number, left-aligned from column 4 (`1` to `999`; three digits fit before the one-column margin). The mark is the terminal's status mark (spinner, `!`, `●`, `○`) in its status colour when its agent has a status, else the terminal's icon (`terminal_icon_kind`) in its icon colour. The number is the entry's place in the list, from 1, counting entries only (not rules). A kept terminal that exited has its mark and number muted. The entries of a panel with unseen attention are bold.
- **Highlight.** Outside sidebar focus, the focused terminal of the active panel has the band (selection colours, across the sidebar's width, as a panel row today). In sidebar focus the band is on the selected entry, and the focused terminal of the active panel is drawn in accent, bold. Hover, dimming and band colour patches work as on full rows.
- **Scrolling.** With more entries than fit, the list scrolls to keep the focused terminal of the active panel (or the selection, in sidebar focus) in view. The wheel scrolls it, three rows a notch.
- **Bottom bar.** The collapsed bottom bar has only the `+` chip (columns 2 to 5, icon at 3 and 4, aligned with the expand chip), which opens the New menu as today. No `Resume` chip; `prefix R` still opens the restore dialog. The 8-row rule for the bar is unchanged.
- **Mouse.** A click on an entry does what a click on a terminal row does: `Act::FocusPanel` then `Act::FocusTerminal`. A right click opens the terminal menu for that terminal. A press on an entry never starts a drag. Rule rows do not hover or click. A press on the separator does nothing while collapsed (no width drag, nothing forwarded).
- **Sidebar focus.** `focus_sidebar` works collapsed. The selection moves over entries only (`Sel::Terminal` for every entry, single-terminal panels included). On an entry, `Enter`, `x`, `r`, `c`, `n` and `R` do what they do on a terminal row; `Left` and `Right` do nothing. Entering sidebar focus starts on the focused terminal of the active panel. `Esc`, `Tab` and `P` work unchanged. `Left`/`h` and `Right`/`l` do nothing. While collapsed, a selection whose terminal is gone falls back to the first entry of its space, else to the focused terminal of the active panel.
- **Selection across a toggle** (sidebar focus). Collapsing: a terminal row stays on that terminal; a panel row goes to its focused terminal; a space row goes to its active panel's focused terminal, else its first entry; a space with no terminals goes to the focused terminal of the active panel. Expanding: a terminal goes to its terminal row when its panel has two or more terminals, to its panel's row when it has one, and to its space's row when the space is folded.
- **Key binding.** A new action `collapse_sidebar` (help group Sidebar, `Collapse or expand the sidebar`), unbound by default. It flips the collapsed state; when the sidebar is hidden (`toggle_sidebar`), it also shows it.
- **Config.** A new top-level key `sidebar_collapsed` (bool, default `false`) starts each client collapsed. A config reload that changes it applies it to every attached client; otherwise a client keeps the state the user set, as with `sidebar`. A reload that changes it sends `ReloadViewport`, as a reload that changes `sidebar` does. The daemon uses `COLLAPSED_SIDEBAR_WIDTH` for the attach viewport when `sidebar_collapsed` is set, so a client that starts collapsed does not make every terminal reflow twice on attach. Adding the field changes the wire form of `Config`, so `PROTOCOL_VERSION` goes up by one (27 when this spec is written; the next free number at merge).
- **Icons.** Two new icons: `Icon::CollapseSidebar` (Unicode `«`, Nerd `nf-fa-angles_left` U+F100) and `Icon::ExpandSidebar` (Unicode `»`, Nerd `nf-fa-angles_right` U+F101), each with an SVG and a mask made as `crates/deck-tui/assets/icons/README.md` says, from Hugeicons `ArrowLeftDouble` and `ArrowRightDouble` (or the closest free icons; the README records which).

## Sub-specs

| Sub-spec                      | Type      | Crate                                          | Summary                                                                           | Depends | Status |
| ----------------------------- | --------- | ---------------------------------------------- | --------------------------------------------------------------------------------- | ------- | ------ |
| [01](01-collapsed-sidebar.md) | shippable | deck-core, deck-tui, deck-daemon, deck (tests) | Collapse button, collapsed entries, keys, mouse, config key, binding, icons, docs | —       | done   |

## Affected docs

- `docs/features/sidebar.md`: the header's collapse chip and the moved session name; a new "Collapsed sidebar" section with its mockup, entries, numbers, highlight, bottom bar, mouse and sidebar focus; the narrow-window rule for the collapsed width.
- `docs/features/keys.md`: `collapse_sidebar` in the bindings table, `sidebar_collapsed` in the config sample.
- `docs/features/config.md`: `sidebar_collapsed` in the list of what a reload changes.
- `crates/deck-core/src/config/default.toml`: `sidebar_collapsed = false` and `collapse_sidebar = []`, with comments (a config test requires every key and action there).
- `docs/architecture.md`: the client's collapsed state next to Sidebar width (the width it reports, the disabled separator drag), `Hover` gaining the two chips, the icons list, and the `PROTOCOL_VERSION` history.
- `crates/deck-tui/assets/icons/README.md`: the two new icons in the sources table and the commands.

Reviewed by spec-reviewer on 2026-11-25.
