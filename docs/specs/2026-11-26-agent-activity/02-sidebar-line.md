---
type: shippable
status: todo
---

# 02 — The activity line in the sidebar

## Scope

Every sidebar row of a panel or terminal with an agent gets a second line with the agent's activity (01). The user can try it with `just run`, an agent with deck's hooks installed, and watch the line follow the agent. Crate: `deck-tui` (`sidebar.rs`, `agent.rs`, `app.rs` for hit-testing).

## Requirements

- `Row::Activity(Sel)`: a row that follows the row of `Sel` (a `Panel` or a `Terminal`). `rows_with` inserts it:
  - after a panel row whose panel has one terminal, when that terminal has an agent;
  - after each terminal row whose terminal has an agent;
  - never after a panel row that is followed by terminal rows, and never in `collapsed_rows`.
- `Row::Activity` is not selectable (`sel()` is `None`), so keyboard movement skips it. For the mouse, hover and click, the context menu and the start of a row drag on it act as on its owner row; a drop target counts it as part of its owner.
- Text: `agent.activity`, else `describe_status(agent.status)`, control characters removed, cut with `…` to the row's width by display width (`draw::truncate`, `draw::width`: wide characters and emoji count two cells), with the one-cell right margin kept. A sidebar too narrow to show one cell after the start column shows nothing on the line (the row stays). It starts at the owner's name column (4 for a panel row, 7 for a terminal row), so it sits under the name; the cells before it and after it are blank. No icon, no status slot.
- Style: `muted`; `warning` when the status is `waiting`. On the band (active panel outside sidebar focus, or the selected row in sidebar focus) the line is part of the band: band colours, not bold. Hover covers both rows. A dragged row is dimmed with its line.
- The line is drawn from the `SessionView` like the rest of the row; its changes arrive with the views 01 throttles. It does not run its own timer.
- Scrolling: `ensure_visible` keeps the owner row and its line in view together when there is room.
- Gap rows, `no panels`, the header, the bottom bar and the collapsed sidebar are unchanged.

## Screen

Sidebar 28 columns, sidebar focus off; the active panel is `Rename tabs to agent` (band, shown with `▓` here for its cells).

```text
 «  main      +  New space  │
                            │
 deck                    ▾  │
 ✎  editor                  │
▓✻  Rename tabs to agent ⠹ ▓│   band, bold
▓   dispatching worker: im…▓│   band, not bold
 ✻  Fix login bug         ! │
    needs permission: Bash  │   warning
 ❯  api + tests             │
  ├ ❯  zsh                ⠹ │   (a running command: no line)
  ├ ✻  Write tests        ● │
  │    All 12 tests pass…   │   muted, under the label (column 7)
  └ ◎  codex              ○ │
       quiet                │   muted, status word (no hooks)
 dotfiles              2 ▸  │
                            │
  +  New         ↻  Resume  │
```

The tree glyph of a terminal row continues on its line (`│`) when another terminal row follows, and is blank after the last one (`└`).

## Acceptance criteria

- [ ] Rows: a single-terminal agent panel gets one line; a multi-terminal panel gets a line under each agent terminal row and none under the panel row; no line for panels or terminals without an agent, nor in the collapsed sidebar.
- [ ] The line shows the activity, else the status word, cut with `…`, in the colours above, on the band when its owner has it.
- [ ] Keyboard selection skips the line; a click, a hover, a right-click and a drag start on it act on its owner.
- [ ] The tree glyph continues as in the mockup.
- [ ] `ensure_visible` shows the active panel with its line.
- [ ] End to end: a fake hook sequence sent with `deck agent-event` inside `just run` changes the line on screen (screen captures in the report).

## Tests

- `deck-tui`: row building (the three panel shapes, collapsed), drawing (text, cut, colours, band, tree glyphs), selection skipping, hover and click mapping, drag with a line, `ensure_visible`. Update the existing sidebar tests whose expected rows change.
- `crates/deck/tests/`: one scenario with a fake agent terminal and `deck agent-event` payloads on stdin, reading the sidebar.
