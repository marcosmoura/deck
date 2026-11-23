---
type: shippable
status: todo
---

# 01 — Blank rows between spaces

## Scope

`sidebar_space_gap = N` in `config.toml` puts N blank rows between spaces in the sidebar. Owning crates: `deck-core` (config, `default.toml`), `deck-tui` (rows, drawing, hover, selection, scrolling).

## Requirements

- `deck-core` config: `sidebar_space_gap`, an integer from 0 to 3, default 0, top level next to `sidebar_width`. Out of range or not an integer is an error naming `sidebar_space_gap`. `default.toml` gains it under the sidebar comment, and its sync test passes.
- Hot reload: changing it redraws every attached client within the usual second, with `config reloaded`; nothing else changes (selection, hidden sidebar, scroll offset clamped to the new row count).
- `deck-tui`: `sidebar::rows` emits a `Row::Gap` N times between two consecutive spaces (after the previous space's last panel or its `no panels`, before the next space's row). None before the first space, none after the last.
- A gap row draws nothing but the sidebar background. It has no hover, no click, no context menu; the selection (`Up`/`Down`) skips it; it counts as a row for scrolling and for keeping the active panel or selection in view.

## Screen

`sidebar_space_gap = 1`, 28 columns:

```text
 main           + New space │
                            │
 ▾ terminal-multiplexer     │
   ✎  editor                │
 ! ✻  claude                │
                            │
 ▾ budget                   │
   no panels                │
                            │
 ▸ dotfiles               2 │
                            │
  + New            Restore  │
```

## Acceptance criteria

- [ ] `sidebar_space_gap` parses 0 to 3, defaults to 0, errors outside it naming the key, and is in `default.toml`.
- [ ] With N > 0 the sidebar has exactly N blank rows between spaces, none before the first or after the last; with 0 it is unchanged.
- [ ] Gap rows do not hover, open no menu, do nothing on click, and are skipped by `Up`/`Down` in sidebar focus.
- [ ] Editing the key reloads every attached client.
- [ ] `docs/features/sidebar.md` (configuration block, a sentence on the gap) and `docs/features/config.md` describe it.

## Tests

- `deck-core`: parse 0..3, default, errors for -1, 4 and a string; `default.toml` sync.
- `deck-tui` (`sidebar_tests.rs`): `rows` with gaps 0, 1, 3 across collapsed, empty and expanded spaces; no hover or menu on a gap row; selection skips gaps; scrolling keeps the active panel visible with gaps.
- End-to-end (`crates/deck/tests/config_reload.rs` or `sidebar.rs`): a session with two spaces; setting `sidebar_space_gap = 1` puts a blank row between them on screen.

## Notes
