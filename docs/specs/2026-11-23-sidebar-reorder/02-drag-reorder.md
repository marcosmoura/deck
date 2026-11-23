---
type: shippable
status: todo
---

# 02 — Drag space and panel rows to reorder

## Scope

Drag a space row to move the space (with its panels) among the session's spaces; drag a panel row to move the panel inside its space. The order is kept by the daemon, shared by every client and saved. Owning crates: `deck-core` (actions, protocol version), `deck-daemon` (applying the moves), `deck-tui` (the gesture, the preview, the `MOVE` bar).

## Requirements

- `deck-core`: `Action::MoveProject { project: ProjectId, to: usize }` and `Action::MovePanel { panel: PanelId, to: usize }`, with their wire round-trip in the protocol tests. `PROTOCOL_VERSION` 17 → 18.
- `deck-daemon`: a move takes the item out of its list and inserts it at `to`, clamped to the list's last index. Active space, active panel, collapsed state and focus are untouched. Unknown ids answer `NoSuchProject` / `NoSuchPanel`. The session view is broadcast and the state saved as for any action, so the order survives a daemon restart.
- `deck-tui`, the click: a left press on a space or panel row records the pressed row and does nothing else. The release on the same row, with no drag started, focuses the panel or folds/unfolds the space (today's press behaviour). A release elsewhere without a drag does nothing. Chips keep their own press/release handling.
- `deck-tui`, the drag: per the overview's decisions (start, preview, target rules, auto-scroll, drop, cancel). The drag state lives on `App` next to `sidebar_drag`. Panel targets never leave the dragged panel's space. While dragging there is no hover, the floating bar shows `MOVE` (same badge style as `SIDEBAR`, placed in `bar_mode`'s precedence as the overview says) and `esc cancel`, and no mouse event reaches a program. Keys follow the overview's cancel rules.
- On drop the selection (in sidebar focus) stays on the moved row. The client draws the preview until the daemon's new view arrives, then the view; it does not reorder its mirror itself.
- A refused move shows the error flash; the list falls back to the daemon's order.

## Screen

Dragging the panel `claude` above `editor` (pointer on the `editor` row), 28 columns:

```text
 main           + New space │
                            │
 ▾ terminal-multiplexer     │
 ! ✻  claude                │   ← dragged row: hover background, bold
   ✎  editor                │
 ▾ budget                   │
   no panels                │
 ▸ dotfiles               2 │
                            │
  + New            Restore  │
```

Floating bar over the bottom of the panel area while dragging:

```text
 MOVE  esc cancel
```

`MOVE` in the badge style of the other modes (`warning` reversed); `esc` in `status_fg` bold, `cancel` in `muted`, on `status_bg`.

## Acceptance criteria

- [ ] A click on a panel row focuses it, and on a space row folds or unfolds it, on release; a press alone does neither.
- [ ] Dragging a space row to another space's place moves the space and its panels; dragging a panel row moves it inside its space and never into another.
- [ ] The preview shows the list as it will be after the drop, the dragged row with the `hover` background and bold, and the floating bar shows `MOVE  esc cancel`.
- [ ] `Esc`, a release outside the list, and the dragged item closed by another client cancel with nothing sent.
- [ ] The new order shows in every attached client, drives `prefix 1`..`9` and `prefix n`/`p`, and survives a daemon restart.
- [ ] Dropping an item on its own place sends nothing.
- [ ] `docs/features/sidebar.md`, `docs/features/keys.md` and `docs/architecture.md` describe it.

## Tests

- Existing tests that only press on a row to focus a panel or fold a space (`crates/deck/tests/sidebar_mouse.rs`, the `deck-tui` sidebar tests) move to a press and a release.

- `deck-core`: wire round-trip of both actions; protocol version.
- `deck-daemon`: moves to the front, the back, the same place, past the end (clamped); active and focus untouched; unknown ids refused; a saved state keeps the new order.
- `deck-tui` (`sidebar_drag_tests.rs` or a new `sidebar_reorder_tests.rs`): a click acts on release only; motion within the pressed row starts no drag; space drag over a space block, over a gap row and over `no panels`; panel drag clamped to its space; the preview rows; `Esc` and release outside cancel; drop on the same place sends nothing; drop sends the right action with the right `to`; the dragged item vanishing from the view, or its space folded by another client, cancels; `Esc` does not leave sidebar focus or reach a program; another key cancels and acts; focus-out and a too-small window cancel; a pointer on the header or bottom bar keeps the last target and a release there drops; the target uses the daemon's rows when a large space is dragged past a small one; a press that leaves the sidebar sideways sends nothing to the program on release; the dragged row's background wins over the band; auto-scroll at the top and bottom; no hover while dragging.
- End-to-end (`crates/deck/tests/sidebar_mouse.rs`): two spaces; drag the second space's row above the first and see the order change on screen and in `deck ls --tree` after a daemon restart; drag a panel row above its sibling and see `prefix 1` focus it.

## Notes
