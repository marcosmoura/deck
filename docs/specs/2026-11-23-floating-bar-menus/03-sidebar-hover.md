---
type: shippable
status: done
---

# 03 — Sidebar highlight and hover

## Scope

The active panel's row is a band of the selection colours instead of a `▌` marker, and every sidebar row and chip shows a `hover` background under the pointer. Owning crates: `deck-core` (the theme role and the removed glyph), `deck-tui` (drawing and hover state); end-to-end tests in `deck`.

## Requirements

1. **Theme.** `Theme` gains `hover: Color`, parsed from `[theme] hover` like the other roles (a role name such as `accent` is allowed, as for `border_focused`), default `bright-black`. `Glyphs::active` and the `[theme.glyphs] active` key go; a config that sets it gets the usual unknown-key warning. `default.toml` lists `hover` with a comment and loses `active`; its sync test still passes.
2. **Active row.** Outside sidebar focus, the active panel's row is drawn across the sidebar's full width (every column left of the separator) in `selection(theme, …)` (the selection colours, reversed while both are `default`), bold. Column 0, where `▌` was, is blank, so names keep their columns. The status glyph, icon and logo of the row are drawn on that background (a logo's tint follows the row's resolved colours, as `Images::cell_for` does today).
3. **Sidebar focus.** Only the selected row has the selection band. The active panel's row keeps `accent` and bold, with no band, when the selection is elsewhere.
4. **Hover.** The row or chip under the pointer gets `hover` as its background across its width (a row: the full sidebar width; a chip: its own cells): space rows, panel rows, the `+ New space` chip, `+ New` and `Restore`. The `no panels` row, blank rows and the header title do not hover. On a hovered row or chip, cells whose foreground would be `muted` use `sidebar_fg` instead (`muted` and `hover` are both `bright-black` by default; the chips are drawn `muted`). A row that has the selection band keeps it under the pointer. The chips stop using the selection colours under the pointer.
5. **Hover state.** `Hover` gains the rows (`Row(Sel)`, or an equivalent keyed by the row's space and panel ids, not its index, so a scroll or a change of rows recomputes it with `refresh_hover`). Motion redraws only when the hovered target changes, as today. Hover ends when the pointer leaves the sidebar, on a click, and while an overlay, a dialog or a menu is open; a wheel scroll recomputes it for the rows now under the pointer.
6. **Clicks.** What a click on a row or chip does does not change.

## Screen

80×12, sidebar 28, pointer over the `budget` space row:

```text
 main          + New space │
                           │
 ▾ deck                    │
███❯  zsh██████████████████│   ← active panel: selection band, bold
   ✎  editor               │
 ▸ budget                2 │   ← hover background, count in sidebar_fg
 ▾ dotfiles                │
   no panels               │
                           │
                           │
                           │
  + New            Restore │
```

(`█` marks the band's cells; the text is drawn on it.) In sidebar focus with the selection on `editor`, the band moves to `editor` and `zsh` is drawn in `accent`, bold, with no band.

## Acceptance criteria

- [ ] No `▌` anywhere; the active panel's row is a full-width band in the selection colours (reversed with default colours), bold.
- [ ] In sidebar focus only the selection has the band; the active row stays accent and bold.
- [ ] Space rows, panel rows and the three chips get the `hover` background under the pointer, muted text on a hovered row or chip turns `sidebar_fg`, and nothing else hovers.
- [ ] Motion over the same row or chip does not redraw; moving to another one does.
- [ ] `[theme] hover` is read, reloads with the config, and is in `default.toml`; `[theme.glyphs] active` is gone and warns as an unknown key.
- [ ] `docs/features/sidebar.md` (mockup, active row, hover) and `docs/features/config.md` describe it.

## Tests

- `deck-core`: `hover` parses, defaults to `bright-black`, accepts a role name; `active` under `[theme.glyphs]` is an unknown-key warning; `default.toml` sync.
- `deck-tui` (`sidebar_tests.rs`, `sidebar_bar_tests.rs`): the band on the active row across the width, with default and custom selection colours; sidebar focus moves the band; hover on a space row, a panel row and each chip, with muted text turned `sidebar_fg`; no hover on `no panels` or the header title; the band wins under the pointer; motion over the same row sets no redraw, over another row does; hover cleared by a click and by an overlay, recomputed after a wheel scroll.
- End-to-end: a scenario moves the pointer over a panel row and finds the hover background on that row's cells, then clicks it and finds the band there.

## Notes
