---
branch: feat/grid-layout
backlog: DECK-03
---

# Grid layout

## Goal

A panel holds several terminals, placed by the automatic grid: with an odd count the first terminal takes the left half and the rest share the right half (3 terminals: 50% and two stacked 25%). The user adds, closes, focuses and resizes terminals from the keyboard and the mouse.

## Builds on

- [DECK-02 sessions-sidebar](../2026-11-22-sessions-sidebar/00-overview.md): actions, focus, frames following the active panel, bindings, prompts and confirmations.
- [DECK-08 config-theming](../2026-11-22-config-theming/00-overview.md): `default.toml` and its sync test, which every new action is added to.
- [architecture.md#layout](../../architecture.md#layout): the shape and ratio rules, which this spec implements as written.

## Non-goals

- Free-form splits (choosing horizontal or vertical), nested layouts, layout presets.
- Zooming one terminal to the whole panel.
- Reordering terminals (swap, promote): cut by the user on 22 Nov 2026.
- Moving a terminal to another panel.

## Decisions

- **New terminal** starts a shell in the project's root (DECK-04 switches it to the focused terminal's cwd once cwd tracking exists) and is appended last, so the existing terminals keep their order.
- **Room**: a new terminal is refused only when even the default ratios for the new count would give a terminal an inner size below 10×3 at the current viewport: `not enough room for another terminal`.
- **Focus** moves by direction to the terminal whose rect overlaps the focused one's span and is nearest in that direction; ties go to the one nearest the top (left/right moves) or the left (up/down moves). No wrap-around, and no memory of the previous focus: from the big terminal of an odd layout, `Right` always goes to terminal 2.
- **Custom sizes survive count changes** (user decision, 22 Nov 2026): adding or closing a terminal adapts the ratios instead of resetting them, by the rule in [architecture.md#layout](../../architecture.md#layout): a 60/40 split of two terminals stays 60/40 when a third is added (the big terminal keeps 60%), and untouched default sizes always adapt to the new defaults (5 terminals: the first still takes half). Stored sizes are never overwritten because a window is too small: deck draws a clamped copy for that window, and the exact sizes return when it grows (user decision, 22 Nov 2026). Only `prefix =` restores the defaults.
- **Resize** moves one boundary in the direction of the key, in steps of 50 per-mille (at least one cell): the terminal's edge on that side when it has one (the terminal grows), else its opposite edge (the terminal shrinks). Resize keys repeat without the prefix for 1 s after a resize; `=` restores the default ratios.
- **Borders**: a panel with one terminal has no border: the terminal fills the panel area and its title is in the status bar (DECK-02). With two or more terminals, every terminal has its own border in the theme's `border_style` (DECK-08); the focused one uses the `border_focused` role and its title the `accent`, the others use `border` and `title`. Their titles show the terminal's index in the panel (`1 · nvim`); the status bar keeps the focused terminal's title.

## Sub-specs

| Sub-spec                    | Type       | Crate                            | Summary                                                             | Depends          | Status |
| --------------------------- | ---------- | -------------------------------- | ------------------------------------------------------------------- | ---------------- | ------ |
| [01](01-layout-engine.md)   | foundation | deck-core                        | `layout`, ratios, adapt, fit, neighbours, resize, hit tests         | DECK-02, DECK-08 | done   |
| [02](02-panel-terminals.md) | shippable  | deck-daemon, deck-tui, deck-core | Add, close and focus terminals; per-terminal PTY sizes; clamped fit | 01               | done   |
| [03](03-resize.md)          | shippable  | deck-tui, deck-daemon, deck-core | Resize with keys and by dragging borders; reset                     | 02               | done   |

## Affected docs

- `docs/features/layout.md` (new): the grid rule with its diagrams, the keys, resizing.
- `docs/features/keys.md`: the new bindings.
- `crates/deck-core/src/config/default.toml` (DECK-08): every new action and its default keys (its sync test requires it).
- `docs/architecture.md#layout`: anything this spec settles differently.

Reviewed by spec-reviewer on 22 Nov 2026.

## Notes

- Updated on 22 Nov 2026 after DECK-02 and DECK-08 landed: a single terminal has no border (so `inner` depends on the count), borders use DECK-08's theme roles and `border_style`, the grid functions join the `deck-core::layout` module DECK-08 created (`panel_area`, `sidebar_cols`), and the model's existing `Panel.focused_terminal` is used instead of a new field.
