---
type: shippable
status: done
---

# 03 — Resizing

## Scope

The user resizes the terminals of a panel with prefix keys (repeatable) or by dragging a border with the mouse, and resets the ratios. Owning crates: `deck-tui`, `deck-daemon`, `deck-core` (action, bindings).

## Requirements

1. **Action** `SetRatios { panel, n, ratios }` (protocol version bumps): `n` is the terminal count the client computed the ratios for. The daemon refuses the action with `layout changed, try again` when `n` differs from the panel's count (another client or an exiting process changed it), and with `invalid layout ratios` when `Ratios::is_valid_for(n)` fails. Otherwise it stores them, resizes the PTYs and sends the new `SessionView` and `full` frames. A refused `SetRatios` from a key or a drag shows no flash; the client resyncs from the next `SessionView`.
2. **Keys**: `resize_left = "H"`, `resize_right = "L"`, `resize_up = "K"`, `resize_down = "J"`: the client computes `layout::boundary_for` and `layout::resize` with the sign `boundary_for` returns and a step of 50‰ (at least one cell, as `layout::resize` guarantees) for the focused terminal, starting from the effective ratios (`layout::fit` at the daemon's `layout_viewport`), and sends the result as `SetRatios`; it becomes the stored ratios. Nothing happens when there is no boundary or no room.
3. **Repeat**: after a resize key, the next resize keys within 1 s act without the prefix; any other key, a paste or a focus event ends the repeat and is handled normally. While repeating, the status bar shows a `RESIZE` badge (same style as `PREFIX`).
4. **Reset**: `reset_layout = "="` sends `SetRatios` with `Ratios::default_for(n)`.
5. **Mouse drag**: pressing the left button on a border found by `layout::hit_boundary` starts a drag (never forwarded to a terminal, even with mouse reporting on); motion sends `SetRatios` from `layout::ratio_at`, at most every 33 ms; release ends it. While dragging, the moving border is drawn in the `accent` role. A panel with one terminal has no border and nothing to drag.
6. **Persistence of ratios** inside the daemon: the panel keeps its stored ratios across viewport changes and clients; when the count changes they are adapted by `Ratios::adapt` (sub-spec 02, requirement 4); a window too small for them gets a clamped copy, never a reset. DECK-04 saves them to disk.

## Screen

While repeating a resize, in the status bar:

```text
 RESIZE  main  terminal-multiplexer › tests                                  H J K L resize  = reset
```

## Acceptance criteria

- [ ] In the 3-terminal layout, `⌥Space L` grows terminal 1 by 5% of the panel width and `L` again within 1 s (no prefix) grows it by another 5%; `stty size` updates (capture `resized`).
- [ ] A key other than a resize key ends the repeat and goes to the terminal.
- [ ] `⌥Space J` on terminal 2 moves the boundary between 2 and 3.
- [ ] Resizing stops at the 10×3 minimum and never errors.
- [ ] `⌥Space L` on terminal 2 of the 3-terminal layout moves the main boundary right (terminal 2 shrinks); `⌥Space J` on terminal 3 moves the row boundary down (terminal 3 shrinks).
- [ ] A paste during the repeat ends it and the pasted text reaches the terminal.
- [ ] `⌥Space =` restores the 50/50 main split.
- [ ] Dragging the border between terminal 1 and the right half moves it with the pointer; a program with mouse reporting on in terminal 1 receives no events from the drag.
- [ ] After `⌥Space L` twice on terminal 1 of a 2-terminal panel (60/40), adding a terminal gives the big terminal 60% of the width; closing it again returns to 60/40.
- [ ] With 2 terminals resized to 90/10 and the window narrowed to 40 columns (sidebar auto-hidden), terminal 2 is drawn with the clamped copy (`stty size` shows at least 10 columns); widening the window back gives the exact 90/10 sizes again.
- [ ] With 4 terminals in default ratios, adding a fifth gives terminal 1 exactly half the width, as with no resize at all.
- [ ] `SetRatios` with a wrong length is refused by the daemon, and one whose `n` differs from the panel's count is refused with `layout changed, try again`.

## Tests

- `crates/deck-daemon/tests/`: `SetRatios` validation and sizing.
- `deck-tui`: repeat timing (with an injected clock), drag state machine, `RESIZE` badge drawing.
- `crates/deck/tests/resize.rs`: one scenario per acceptance criterion.

## Notes

- `=` (`reset_layout`) also acts during the repeat, because the status-bar mock lists it, and it ends the repeat.
- The repeat starts on any resize key of a panel with two or more terminals, even when nothing moves (no room), so a key held at the limit never reaches the terminal. A single terminal has no border: its resize keys do nothing and start no repeat.
