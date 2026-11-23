---
type: shippable
status: todo
---

# 03 — Search

## Scope

`/` and `?` in scroll mode search the focused terminal's whole history; matches on screen are highlighted; `n`/`N` move between them. Owning crates: `deck-tui` (prompt, highlighting), `deck-core` (theme roles).

## Requirements

1. **Prompt**: `/` (down, toward newer) or `?` (up, toward older) opens a one-line prompt in the floating bar (`/` or `?` followed by the query, the cursor after it), with editing as the existing prompts (Backspace, `C-u`, `C-w`, Left/Right). `Enter` searches; `Esc` cancels. An empty `Enter` repeats the last query in the new direction.
2. **Search**: from the copy cursor, `Request::Search`. On `Found`, the copy cursor moves to the match's start, the view scrolls to it (`ScrollTo::Row`), and with `wrapped` the bar flashes `search wrapped`. On `NotFound`, the bar flashes `not found: <query>`.
3. **`n`** searches again in the same direction from the cursor, **`N`** in the other.
4. **Highlights**: while a query is active (from the first search until scroll mode ends, or `Esc` with no selection clears it, per [02](02-copy-mode.md)'s `Esc` order), every occurrence on the visible rows is drawn with `search_match_*`, found by the client in its mirror with the same smart case, and the current match with `search_current_*`. A selection's colours win over a match's.
5. **Selection**: after a search, `v` starts at the match's start; a selection started before a search extends to the match.
6. **Theme roles** `search_match_fg`, `search_match_bg`, `search_current_fg`, `search_current_bg` in `deck-core::theme` with the defaults of the overview, in `default.toml` (its sync test covers them), and in `docs/features/config.md`. They change `Theme`'s wire format; the protocol bump of [01](01-protocol.md) covers it (no second bump).
7. **Help and docs**: the help overlay's Terminal group lists the scroll-mode keys (copy and search included), and `docs/features/keys.md` gains the Copy and Search subsections.

## Screen

Searching up for `error` in an 80×24 terminal:

```text
│ warning: unused variable `x`                                              │
│ [error]: cannot find value `y` in this scope                              │
│  --> src/main.rs:4:5                                                      │
│ ...                                                                       │
│ {error}: aborting due to 1 previous error                                 │
                                         ?error█
```

`[…]` marks `search_match_*`, `{…}` the current match `search_current_*`; the prompt is in the floating bar.

## Acceptance criteria

- [ ] `prefix [`, `?error`, `Enter` on a history with `error` 2,000 lines up scrolls to it, the cursor at its start; `n` goes to the next one up; `N` back down.
- [ ] A search past the oldest occurrence wraps and flashes `search wrapped`; a missing word flashes `not found: <word>`.
- [ ] All occurrences on screen are drawn in `search_match_*`, the current one in `search_current_*` (cell colours checked in the harness).
- [ ] `Error` finds only `Error`; `error` finds both (smart case).
- [ ] `v` after a search then `e` and `y` copies the matched word.
- [ ] `deck config default` lists the four roles; `deck config check` accepts them.

## Tests

- `deck-tui` unit tests for the prompt, the client-side highlight finder (smart case, wide characters) and drawing order; e2e tests for the criteria.
