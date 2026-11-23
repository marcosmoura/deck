---
type: shippable
status: done
---

# 01 — Resize the sidebar with the mouse

## Scope

The user drags the separator between the sidebar and the panel area to make the sidebar wider or narrower; the terminals follow because the panel area is the rest of the window. Owning crates: `deck-tui` (drag, drawing), `deck-core` (`layout::panel_area` takes the width), `deck-daemon` (its two `panel_area` call sites in `server.rs` pass the config's width).

## Requirements

1. **Pure width rule** in `deck-core::layout`: `clamp_sidebar_width(cols: u16, wanted: u16) -> u16` returns `wanted` limited to `20..=60` (the `sidebar_width` limits) and to `cols − 1 − MIN_PANEL_COLS` (so the panel area keeps its 20 columns). When `cols − 21` is under 20 (a window under 41 columns, where the sidebar is hidden anyway) the lower bound wins and the result is 20. `sidebar_cols` already takes the width; `panel_area` gains a `sidebar_width` argument instead of reading `&Config`; the daemon passes the config's value, the client passes its own `sidebar_width`.
2. **Drag**: a left press on the separator column (`column == sidebar_width`, any row above the status bar) starts a drag, never forwarded to a program. While it runs, the sidebar width is `clamp_sidebar_width(cols, pointer column)`, so the separator follows the pointer; motion is applied at most every 33 ms, the last position always on release. Release, a `Moved` event without the button, a focus loss, an overlay opening or a resize of the outer window end it.
3. **Effect**: each width change recomputes the client's panel area and sends `Viewport` (not `ReloadViewport`: the user acted, so these terminals follow this client), then redraws. The daemon resizes the PTYs as for any viewport change. Terminal ratios (DECK-03) are untouched.
4. **Where it lives**: `App.sidebar_width` is client state and is not saved (see the overview). A config reload that changes `sidebar_width` sets it; one that does not leaves it. `prefix b` hides and shows the sidebar at the current width. On an outer resize the client clamps its width with `clamp_sidebar_width` (so a width at the limit shrinks with the window instead of auto-hiding the sidebar one column later; it does not grow back when the window grows). The sidebar auto-hides only when the window is under 41 columns, and shows again at 20 columns or the clamped width when it grows. A press on the separator while the sidebar is hidden does nothing.
5. **Look**: while dragging, the separator is drawn in the `accent` role (like DECK-03's moving border) and the status bar shows `SIDEBAR <width>` in the `PREFIX` badge style. On the hovered separator without a drag nothing changes.
6. **Viewport after attach and switch**: after the first `SessionView` of an attach or a session switch, the client sends `Viewport` when its own `viewport()` (its width, and `prefix b` state) differs from the area the daemon derived from the config at attach (`panel_area(outer, config.sidebar_width, config.sidebar)`). This also fixes a sidebar hidden with `prefix b` before a switch.
7. **Overlays and prompts**: mouse events do nothing while an overlay is open (DECK-02 rule), so a drag cannot start under one.

## Screen

Dragging to column 40 in a 100-column window (the separator, drawn in accent, is at column 40; the panel area is 59 columns):

```text
 main                                    ┃ (accent)
                                         ┃
 ▾ terminal-multiplexer                  ┃  ╭─ nvim ───────────────────────────────╮
▌  editor                                ┃  │                                      │
 …
 SIDEBAR 40  terminal-multiplexer › editor                            ⌥Space ? help
```

The separator is drawn `┃` in accent while dragging and `│` in `border` otherwise. The `SIDEBAR 40` badge replaces the session name, as the `PREFIX` badge does.

## Acceptance criteria

- [ ] In a 100×24 window with `sidebar_width = 28`, dragging from column 28 to column 40 makes the shell's `stty size` report 59 columns (was 71); dragging back to 28 reports 71 again (capture `sidebar-dragged`).
- [ ] Dragging past 60 stops at a 60-column sidebar; dragging below 20 stops at 20.
- [ ] In a 60-column window the sidebar cannot grow past 39 columns (the panel area keeps 20).
- [ ] After a drag, `⌥Space s` and a switch to another session leaves that session's shell reporting the dragged panel width (`stty size`), and a sidebar hidden with `⌥Space b` before the switch gives the full-width `stty size` there.
- [ ] Shrinking the window by one column with the sidebar at 60 keeps it shown at 59; growing the window does not widen it.
- [ ] A program in the terminal with mouse reporting on receives no event from the drag.
- [ ] After a drag, `⌥Space b` twice returns to the dragged width, not to the config's.
- [ ] Editing `sidebar_width` in `config.toml` while a dragged width is in force applies the new value; an unrelated config edit keeps the dragged width.
- [ ] A press on the separator with the sidebar hidden does nothing; a press on a sidebar row still focuses it as before.
- [ ] With two terminals in the panel, a drag of the sidebar keeps their ratios (the 60/40 split is still 60/40 of the new area).
- [ ] The separator is drawn in the accent role during the drag and in `border` after it.

## Tests

- `deck-core`: `clamp_sidebar_width` table (limits, windows of 40, 41, 60 and 100 columns, the lower bound winning under 41), `panel_area` and `sidebar_cols` with the new argument.
- `deck-tui` with `TestBackend`: the drag state machine (press, throttled motion, release, ended by focus loss and by an overlay), the `Viewport` sent per width, config-reload rule, separator colour while dragging, the `SIDEBAR` badge.
- `crates/deck/tests/sidebar_mouse.rs`: the criteria above with the harness sending SGR mouse sequences (`ESC[<0;29;5M` presses column 28 of row 4, 1-based in the sequence) through `send_keys`.

## Notes

- The daemon computes the first panel area from the config at attach; today the client sends `Viewport` only on an outer resize and on `toggle_sidebar`, and a `Welcome` with an unchanged config sends nothing, so requirement 6 closes that gap.
- Several clients on one session: PTY sizes follow the client that most recently sent a viewport (architecture, protocol); the others crop or pad.
- The client also watches `SIGWINCH` through tokio and feeds a changed size to the app as a resize: crossterm 0.29's poll returns either the TTY or the signal event and drops the other's edge-triggered readiness, so a resize that arrives with typed input was lost and the client kept its old size.
