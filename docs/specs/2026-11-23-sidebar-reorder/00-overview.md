---
branch: feat/sidebar-reorder
backlog: DECK-18
---

# Sidebar reorder and space gaps

## Goal

Let the user put the sidebar in the order they want and give it some air. Space rows and panel rows can be dragged with the mouse to a new place; the new order is the daemon's, so every client sees it and it survives a daemon restart. A config key adds blank rows between spaces.

Asked on 23 Nov 2026:

- Implement drag to reorder for sidebar items, for both the sessions and spaces (agreed: space rows and panel rows; sessions are not sidebar rows).
- Implement a config that adds empty lines between projects (agreed: a number of blank rows, `sidebar_space_gap`, default 0).

## Builds on

- [DECK-02 sessions-sidebar](../2026-11-22-sessions-sidebar/00-overview.md): the sidebar rows, `Action`, the session view the daemon broadcasts, sidebar focus and its selection.
- [DECK-04 persistence](../2026-11-22-persistence/00-overview.md): the state file stores spaces and panels as ordered lists.
- [DECK-08 config-theming](../2026-11-22-config-theming/00-overview.md): top-level config keys, `default.toml` and its sync test, hot reload.
- [DECK-10 sidebar-mouse](../2026-11-22-sidebar-mouse/00-overview.md): clicks on rows, the wheel, the separator drag.
- [DECK-17 floating-bar-menus](../2026-11-23-floating-bar-menus/00-overview.md): the floating bar and its badges, `hover`, the row menus.

## Non-goals

- Moving a panel to another space. A panel is reordered inside its own space only.
- Reordering sessions, or terminals inside a panel.
- Reordering with the keyboard, or with a `deck` command or a control API request. Both can come later on top of the same actions.
- Blank rows anywhere else (between panels, before the first space), or a gap that differs per space.

## Decisions

- **Two new actions,** `Action::MoveProject { project, to }` and `Action::MovePanel { panel, to }`, where `to` is the index the item ends at in its list (the session's spaces, or its space's panels), clamped to the last index. The daemon applies them like any other action: it broadcasts the new session view and saves the state. An unknown id is refused with the existing `no space` / `no panel` errors. The active space, active panel and focus do not change. No persisted field changes: the state file already keeps the lists in order. `PROTOCOL_VERSION` goes from 17 to 18.
- **The order is everything's order.** Panel numbers (`prefix 1`..`9`), `prefix n`/`p` and sidebar-focus `Up`/`Down` follow the new order, because they already follow the lists.
- **A press on a row no longer acts at once.** A left click on a row (press and release on the same item, compared by its id, not its screen row, with no drag in between) does what a press does today: focus the panel, or fold or unfold the space. This is what lets a press start a drag without folding a space or switching panels first. The whole gesture, from the press to the release, belongs to the sidebar wherever the pointer goes, as the separator drag's does: its motion and release never reach a program. A pending press is dropped (nothing happens on release) when its item is closed or hidden, or an overlay opens.
- **A drag starts** when the pointer, with the left button down after a press on a space or panel row, reaches another row of the list. Moving within the pressed row does nothing.
- **While dragging,** the sidebar shows the list as it will be after the drop: the dragged row (a space with its panels as one block) sits at its target place, drawn with the `hover` background and bold; this wins over the selection band when the dragged row is the active panel or the selection. The floating bar shows a `MOVE` badge with `esc cancel`; `MOVE` comes right after the separator drag's `SIDEBAR <width>` in the bar's precedence and wins over `PREFIX` and `SIDEBAR`.
- **The target** is computed on the daemon's rows (the list without the preview), as the scroll offset shows them, so it does not flicker as the preview moves rows under the pointer. A space's target is the place of the space whose row or block the pointer is over; a panel's target is the place of the panel row the pointer is over, clamped to the first or last panel of its own space when the pointer is above or below it. Blank rows (sub-spec 01) and `no panels` take the target of the row above them. A pointer outside the list (the header, the bottom bar and its spacer, the panel area) keeps the last target. With the pointer on the header row or on the last list row while the list can scroll that way, the list scrolls one row per motion event.
- **The drop** is the release, wherever it is: if the target differs from the item's place, the client sends the move action; otherwise nothing. A refusal from the daemon shows the usual red flash.
- **Cancel.** `Esc` cancels the drag and goes no further (it does not leave sidebar focus and never reaches a program). Any other key cancels the drag and is then handled as usual, as with the separator drag. The drag is also cancelled, with nothing sent, when the dragged item is closed or its row disappears (another client folds a dragged panel's space), a config reload hides the sidebar, an overlay opens, the terminal window loses focus, or the window becomes too small to draw.
- **The drag is client state,** like the separator drag: no other client sees it until the drop. Hover is off while dragging; the press, motion and release are never sent to a program.
- **`sidebar_space_gap`** is a top-level config key, an integer from 0 to 3 (default 0): that many blank rows between two spaces, none before the first or after the last. A value outside 0 to 3 is an error naming the key. It reloads like `sidebar_width`. Blank rows do not hover, open no menu, are skipped by the selection and count for scrolling.

## Sub-specs

| Sub-spec                 | Type      | Crate                                          | Summary                                                                   | Depends | Status |
| ------------------------ | --------- | ---------------------------------------------- | ------------------------------------------------------------------------- | ------- | ------ |
| [01](01-space-gap.md)    | shippable | deck-core, deck-tui, deck (tests)              | `sidebar_space_gap`: blank rows between spaces                            | —       | todo   |
| [02](02-drag-reorder.md) | shippable | deck-core, deck-daemon, deck-tui, deck (tests) | `MoveProject` / `MovePanel` actions; drag space and panel rows to reorder | 01      | todo   |

Both change the sidebar's rows and mouse handling in `deck-tui`, so they run in order; 01 first, so the drag knows about blank rows.

## Affected docs

- `docs/features/sidebar.md`: a click is now a press and a release, dragging rows, the `MOVE` bar, the dragged row over the band, the gap in the mockup notes and the configuration block.
- `docs/features/config.md` and `default.toml`: `sidebar_space_gap`.
- `docs/features/keys.md`: the `MOVE` floating bar and its place in the precedence table.
- `docs/architecture.md`: the protocol version and the new actions, wherever actions are listed.

Reviewed by spec-reviewer on 23 Nov 2026.
