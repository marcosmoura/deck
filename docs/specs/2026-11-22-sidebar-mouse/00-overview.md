---
branch: feat/sidebar-mouse
backlog: DECK-10
---

# Sidebar mouse, buttons and New menu

## Goal

The sidebar and the panel area can be resized with the mouse, and the sidebar gains buttons for the most common actions: `New project` at the bottom left, and on each project row, on hover, a `[ New ▾ ]` button that opens a menu to start a separate panel with a shell, Claude Code, OpenCode or Codex in that project. The bottom right of the sidebar is reserved for the `Restore` button of [DECK-11](../2026-11-22-agent-restore/00-overview.md).

## Builds on

- [DECK-02 sessions-sidebar](../2026-11-22-sessions-sidebar/00-overview.md): the sidebar rows, `HEADER_ROWS`, `sidebar_mouse`, the new-project prompts, the overlay and input-mode order, flashes for `ActionError`.
- [DECK-03 grid-layout](../2026-11-22-grid-layout/00-overview.md): terminals inside a panel are already resized by dragging their borders ([03-resize](../2026-11-22-grid-layout/03-resize.md)); this spec adds the sidebar's own drag and reuses its drag rules (33 ms throttle, accent colour while dragging, never forwarded to a program).
- [DECK-05 control-api](../2026-11-22-control-api/00-overview.md): `LaunchSpec` and the daemon's `NewPanel` function with `launch: Option<LaunchSpec>` (sub-spec 01, requirement 1a).
- [DECK-06 agent-status](../2026-11-22-agent-status/00-overview.md): `AgentKind`, the `[agents.<kind>]` tables (this spec adds their `command` key) and the template parser of sub-spec 05, requirement 3.
- [DECK-08 config-theming](../2026-11-22-config-theming/00-overview.md): theme roles, `sidebar_width` limits, `default.toml` and its sync test.
- [architecture.md](../../architecture.md#client): mouse handling, overlays, config.

## Non-goals

- Dragging the border between the panel area and the status bar, or any border other than the sidebar's separator. Terminal borders inside a panel are DECK-03's.
- Saving a dragged sidebar width to `config.toml`, or per-project widths.
- Launch presets with arguments chosen in the menu, a git worktree per agent, or agents other than the three supported.
- Reordering projects or panels by drag.
- The `Restore` button and dialog (DECK-11); this spec only reserves its place.
- Showing the project button without mouse motion support: terminals that never report motion get the keyboard path only.

## Decisions

- **"Panels resizable by mouse" is covered by two drags.** Terminals of a panel: DECK-03 (already built). The panel area, which is everything right of the sidebar: dragging the sidebar's separator, since the area is `outer width − sidebar − 1`.
- **A dragged sidebar width is session-local client state.** It lives in the client's `sidebar_width` (already a field of `App`), is kept across detach-free session switches and `prefix b` toggles, and is lost when the client exits. It is not written to `config.toml` (the daemon is the only config reader and nothing in deck edits the file). A config reload that changes `sidebar_width` replaces the dragged width; a reload that does not leaves it. The range stays 20–60, and never leaves the panel area under 20 columns.
- **Hover needs no new tracking mode.** The client enables mouse capture through crossterm's `EnableMouseCapture`, which already switches on any-event tracking (`?1003h`, with 1000, 1002, 1015 and 1006), and `on_mouse` already receives `Moved` events and forwards them to programs that asked for motion. The cost today is that every sidebar mouse event sets the redraw flag; this spec makes motion redraw only when the hovered target changes. Terminals without motion reporting never show the hover button, so the same menu is reachable from the keyboard.
- **One launch action, DECK-05's function behind it.** `Action::NewPanel` and DECK-05's `Request::NewPanel` stay as they are. A new `Action::NewPanelWith { project, launch: PanelLaunch }` (`Agent(kind)`; DECK-11 adds `Resume`) lets the client ask for an agent without knowing its command: the daemon resolves it into a `LaunchSpec { program: Some(..), args, cwd: project root }` and calls DECK-05's `NewPanel` function, so the terminal is an explicit-program terminal for persistence (DECK-04) and DECK-06's resume (sub-spec 05, requirement 6). The `Terminal` menu item sends the existing `NewPanel`. A `command` whose program name is not in the kind's `process` list is not detected as an agent by DECK-06, so a daemon restart relaunches it fresh instead of resuming.
- **`Terminal` in the menu is a new panel, not a terminal in the grid.** It is the same as `prefix c` for the chosen project. Adding a terminal to the active panel's grid stays `prefix t`.
- **Agent panels run the program directly**, with no wrapper shell, as `deck new-terminal -- <program>` does: the panel closes when the agent exits (DECK-02 lifecycle). Resume after a daemon restart is DECK-06's.
- **Missing program**: the daemon looks the program up in the session environment's `PATH` before creating anything. When it is missing, no panel is created and the client shows a 4 s error flash: `"opencode" not found in PATH (set [agents.opencode] command)` (the key named is the one that supplied the program: `command`, or `resume` for DECK-11).
- **Menu keys**: `new_menu = "N"` (prefix binding, menu for the active project) and `n` in sidebar focus (menu for the selected project). The menu's own keys are fixed in v1 like the other modal tables.
- **Order with DECK-09.** No dependency on DECK-09: this spec changes project rows (a hover button), the bottom of the sidebar and its height, and DECK-09 changes panel rows. Both edit `sidebar.rs` and `sidebar_tests.rs`, so they never run in parallel; the queue puts DECK-09 first and the lead merges `main` into this branch before starting. The menu draws no icons; adding DECK-09's icons to it is a follow-up.

## Sub-specs

| Sub-spec                   | Type       | Crate                             | Summary                                                                                       | Depends | Status |
| -------------------------- | ---------- | --------------------------------- | --------------------------------------------------------------------------------------------- | ------- | ------ |
| [01](01-sidebar-resize.md) | shippable  | deck-core, deck-tui               | Drag the sidebar separator; the panel area follows; width is client state                     | —       | todo   |
| [02](02-bottom-bar.md)     | shippable  | deck-tui                          | Bottom bar with `New project`, reserved `Restore` slot, hover state that redraws on change    | 01      | todo   |
| [03](03-launch-actions.md) | foundation | deck-core, deck-daemon            | `NewPanelWith`, `[agents.<kind>] command`, program lookup and the missing-program error       | DECK-06 | todo   |
| [04](04-new-menu.md)       | shippable  | deck-tui, deck-core, deck (tests) | `[ New ▾ ]` on hovered project rows, the menu popup, `new_menu` binding and sidebar-focus `n` | 02, 03  | todo   |

01 and 02 both change `deck-tui`'s sidebar, so they run one after the other; 03 is independent of them.

## Affected docs

- `docs/features/sidebar.md`: the separator drag, the bottom bar, the `New` button and menu, hover.
- `docs/features/keys.md`: `new_menu`, sidebar-focus `n`.
- `docs/features/agents.md` (created by DECK-06): `command`, the missing-program error.
- `docs/features/config.md` and `default.toml`: `[agents.<kind>] command`, `new_menu`.
- `docs/architecture.md#client`: the sidebar width is client state; hover and the `Moved` handling.
- `docs/specs/2026-11-22-agent-status/00-overview.md` (DECK-06, `ready`): its non-goal "Launch presets or a menu to start agents" narrows to "launch presets". The lead edits that line on `main` when DECK-06 starts. DECK-06 does not gain `command`; this spec adds it to the `[agents.<kind>]` tables.

## Open questions

- **Tracking mode**: none to change (see Decisions). Recommendation: keep crossterm's capture as is, and cover the redraw cost with the unit test in sub-spec 02.
- **Double-click on the separator to reset the width**: not specified (needs click timing). Recommendation: leave out; dragging back or editing `sidebar_width` resets it.

Reviewed by spec-reviewer on 22 Nov 2026.
