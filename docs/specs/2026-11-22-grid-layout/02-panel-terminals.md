---
type: shippable
status: done
---

# 02 — Several terminals per panel

## Scope

The user adds terminals to a panel and sees them in the automatic grid, moves focus between them, and closes them. Each terminal's shell gets its own size from the layout. Owning crates: `deck-daemon`, `deck-tui`, `deck-core` (actions, model, bindings).

## Requirements

1. **Model**: `Panel` gains `ratios: Ratios` (the stored ratios, default for its count; DECK-01 deferred them) and uses its existing `focused_terminal` (DECK-01) as the focused terminal, always `Some` while the panel has terminals. `SessionView` carries both and the terminals in order. Protocol version bumps.
2. **Actions**: `NewTerminal { panel }`, `CloseTerminal { terminal }`, `FocusTerminal { terminal }`, `FocusDirection { dir }` (resolved by the daemon with `layout::neighbor` on the last viewport). All act on the attached session's active panel unless they name an id.
3. **Daemon sizing**: on every viewport change, count change, ratio change or focus change of panel, the daemon computes `layout(viewport, n, layout::fit(viewport, n, stored, 10, 3))` for the active panel and resizes each PTY and emulator to its `inner(rect, n)` size (the whole panel area for a single terminal, as since DECK-02); then sends `full` frames for the terminals whose size changed. Terminals of inactive panels are resized when their panel becomes active (DECK-02 sub-spec 01, requirement 8). A session no client has sized yet (`deck new -d`) uses an 80×24 viewport until a client attaches (DECK-02 sub-spec 01).
4. **Count changes**: adding or removing a terminal adapts the stored ratios with `Ratios::adapt`. They are never replaced because they do not fit: sizing and drawing use `layout::fit` (requirement 3), so the exact stored sizes return when the window grows. `NewTerminal` is refused with `not enough room for another terminal` only when `layout::fits` fails at 10×3 for `n + 1` with `Ratios::default_for(n + 1)`. The new terminal gets focus. When the focused terminal is closed or exits, focus goes to the terminal that took its position, else the previous one. The last terminal closing closes the panel (DECK-02 lifecycle).
5. **Client drawing**: the daemon's layout decides PTY sizes and directional focus. `SessionView` carries the viewport the daemon sized the active panel with (`layout_viewport`); the client calls `layout` with that viewport and `layout::fit` of the panel's ratios for it, so its rects always match the PTY sizes, and crops or pads the result to its own panel area when the two differ (several clients of different sizes). With one terminal it draws it without a border, as DECK-02 does. With two or more it draws each terminal in its rect with its own border in the theme's `border_style`: the focused one in `border_focused` with its title in `accent`, the others in `border` and `title` (DECK-08 roles); each title is `<index> · <title>`. The status bar keeps showing the focused terminal's title after the panel name (DECK-02). Each terminal's cursor is drawn only for the focused terminal. Input goes to the focused terminal.
6. **Mouse**: a click inside an unfocused terminal focuses it (`FocusTerminal`) and is not forwarded; later clicks are forwarded under DECK-01 sub-spec 06 rules. The wheel scrolls the terminal under the pointer, focused or not, by DECK-01 sub-spec 06 rules, without changing focus and without entering the keyboard scroll mode.
7. **Bindings** (in the `deck-core` table):

   | Action           | Default                                         | Does                                                          |
   | ---------------- | ----------------------------------------------- | ------------------------------------------------------------- |
   | `new_terminal`   | `t`                                             | `NewTerminal` in the active panel                             |
   | `close_terminal` | `x`                                             | Confirm `Close terminal <index>? (y/n)`, then `CloseTerminal` |
   | `focus_left` …   | `Left` `Right` `Up` `Down`, and `h` `l` `k` `j` | `FocusDirection`                                              |

   The focus actions take two default keys each (the binding table maps an action to a list of keys since DECK-01). These keys apply after the prefix; the sidebar-focus keys of DECK-02 are a separate table.

8. **Sidebar**: the panel row's terminal count (DECK-02) reflects the count.

## Screen

Panel `tests` with 3 terminals, terminal 2 focused (accent), at 100×24 with the sidebar hidden: each box is 50 columns wide, as `layout` gives; rows are cut, so heights are not to scale:

```text
╭─ 1 · nvim ─────────────────────────────────────╮╭─ 2 · cargo watch ──────────────────────────────╮
│                                                ││Finished dev profile in 0.41s                   │
│                                                ││                                                │
│                                                │╰────────────────────────────────────────────────╯
│                                                │╭─ 3 · sh ───────────────────────────────────────╮
│                                                ││$ █                                             │
│                                                ││                                                │
╰────────────────────────────────────────────────╯╰────────────────────────────────────────────────╯
 main  terminal-multiplexer › tests · cargo watch                           ⌥Space ? help  d detach
```

## Acceptance criteria

- [ ] `⌥Space t` twice in a 100×24 window with the sidebar hidden gives the 3-terminal layout of the mockup, and `stty size` in each terminal matches its `inner` rect (capture `three`).
- [ ] A fourth and fifth terminal give the 2×2 and the main + 2×2 layouts, with matching `stty size` (captures `four`, `five`).
- [ ] `⌥Space Right`/`l`, `Left`/`h`, `Up`/`k`, `Down`/`j` move focus as `layout::neighbor` says; typed text reaches only the focused terminal.
- [ ] `⌥Space x` asks, then closes the terminal; the others reflow and their `stty size` updates; focus follows requirement 4.
- [ ] `exit` in one of three terminals reflows to two side by side; closing one of two leaves a single terminal with no border, whose `stty size` is the whole panel area.
- [ ] In a 40×12 window (sidebar auto-hidden, panel area 40×11), terminals 2 to 4 are added and the fourth `⌥Space t` (a 5th terminal) is refused with the room error, because even the default 5-terminal ratios give its grid columns 10 cells (8 inside).
- [ ] The wheel over an unfocused terminal scrolls it without moving focus or entering scroll mode.
- [ ] A click in an unfocused terminal focuses it without sending the click; a second click is forwarded to a program with mouse reporting on.
- [ ] Two clients of the same size on one session see the same focus and layout; a smaller second client sizes the PTYs when it attaches (DECK-01's viewport claim); once the larger client sends input, the smaller one draws the daemon's layout cropped.

## Tests

- `crates/deck-daemon/tests/`: sizing after add, close and viewport change; `fit` applied when stored ratios do not fit; focus rules; the room refusal.
- `deck-tui`: `TestBackend` drawing of 2, 3 and 5 terminals with focus styles and titles.
- `crates/deck/tests/grid.rs`: one scenario per acceptance criterion.

## Notes

- Help overlay: the four focus actions share one row ("Focus the terminal by direction"), and the group headings are dropped when the window is too short for them plus all the rows (24 rows), so the new bindings fit.
