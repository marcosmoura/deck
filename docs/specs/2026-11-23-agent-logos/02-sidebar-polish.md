---
type: shippable
status: done
---

# 02 — Aligned icons, icons in the New menu and restore dialog, sidebar buttons

## Scope

Asked by the user on 23 Nov 2026 after trying sub-spec 01 in Ghostty. The icon column is the same width on every row; the New menu and the restore dialog show the agents' icons (logos where images are on); the sidebar header names the session and holds `+ New project`; one `+ New` button at the bottom replaces each project's `[ New ▾ ]`. Owning crate: `deck-tui`; `deck` for the e2e tests. This changes DECK-10's [sidebar-mouse](../2026-11-22-sidebar-mouse/00-overview.md) layout and DECK-09's icon column.

## Requirements

1. **Aligned icon column.** With icons on (`icons` not `none`), every icon takes two cells: a text icon is drawn in the first cell and the second is blank, a logo fills both. Names start at the same column on every panel row (`▌`, status, space, icon ×2, space, name), and the status bar uses the same two-cell icon (`project › ⠹ ✻  Fix login bug`). Nerd Font glyphs that render wider than one cell no longer touch the name. With `icons = "none"`, rows are as before.
2. **Header.** The sidebar's first row reads `Session <name>` (the word `Session` muted, the name as today), cut with `…` when it does not fit next to the chip of requirement 3. The header chip has no side padding, so `Session main` fits in the default 28 columns.
3. **`+ New project` at the top right.** The `+ New project` chip moves from the bottom bar to the header row, right-aligned with one cell of margin (`+` alone when the sidebar is under 26 columns). Same look, hover, click and behaviour as DECK-10's chip; the session name gives way to it.
4. **One New button at the bottom.** The bottom bar's left chip becomes `+ New` (`+` under 26 columns), with DECK-10's chip look and hover. A click opens the New menu for the **selected project**: in sidebar focus, the selected row's project (or the selected panel's project); otherwise the active project. With no project it flashes `no project` like the Restore button does. The menu sits above the bottom bar, its left edge on the sidebar's first column (same flip and window-clamp rules as before). The per-project `[ New ▾ ]` hover button is removed, and project rows keep their counts under the pointer. `prefix N` and `n` in sidebar focus are unchanged. The new panel is created in that project, as today.
5. **Icons in the New menu.** The menu's left gutter column (the phantom border before each item) is replaced by each item's icon, two cells and a space before the label: `Terminal` the terminal icon (`❯`, or the Nerd glyph), `Claude`, `OpenCode` and `Codex` their agent icons, drawn as logos where images are on (sub-spec 01) and as text icons elsewhere. The highlight covers the icon. The digits stay at the right.
6. **Icons in the restore dialog.** Each column heading starts with its agent's icon, two cells and a space (`✻  Claude Code (50)`), a logo where images are on.
7. **Logos everywhere they are drawn** go through sub-spec 01's pre-pass: every (logo, colour) pair a frame draws in the menu or dialog is sent before that frame, with the colour the cell shows (the highlighted menu item's logo in the selection colour). Nothing else in sub-spec 01 changes.

## Screen

Ghostty, 28-column sidebar (`C`, `O`, `◆` are two-cell logos, `❯·` a text icon and its blank cell):

```text
 Session main + New project │
                             │
 ▾ terminal-multiplexer      │
▌⠹ C  Fix login bug          │
   ❯· zsh                    │
 ! O  Review the API changes │
                             │
 + New               Restore │
```

The New menu, opened from `+ New` with `terminal-multiplexer` selected:

```text
╭ New in terminal-multiplexer ╮
│ ❯·  Terminal              1 │
│ C   Claude                2 │
│ ◆   OpenCode              3 │
│ O   Codex                 4 │
│ ⏎ choose  esc close         │
╰─────────────────────────────╯
 + New               Restore
```

Restore dialog headings: `C  Claude Code (50)`, `◆  OpenCode (0)`, `O  Codex (0)`.

## Acceptance criteria

- [ ] `TestBackend`: with text icons and with images on, every panel row's name starts at the same column; the status bar icon takes two cells; `icons = "none"` rows are unchanged.
- [ ] The header reads `Session main` with `+ New project` right-aligned (and `+` under 26 columns); a click on it opens the `Directory:` prompt.
- [ ] The bottom bar shows `+ New` and `Restore`; a click on `+ New` opens `New in <project>` for the selected project in sidebar focus and for the active project otherwise; with no project it flashes `no project`; no project row shows `[ New ▾ ]` under the pointer.
- [ ] The New menu has no gutter column; each item shows its icon (logo placeholders with images on, text icons off); choosing an item still creates the panel in that project.
- [ ] The restore dialog's headings start with their icons.
- [ ] Every logo drawn in the menu and dialog has been transmitted before its frame (registry test).
- [ ] End to end: the DECK-10 sidebar-mouse tests updated for the new chip positions pass; a new test clicks `+ New`, chooses `Terminal`, and sees the panel in the selected project; the `logos` capture shows aligned names.
- [ ] `docs/features/sidebar.md` (header, chips, New button and menu, icon column) and `docs/features/agents.md` (restore dialog headings) describe it.

## Tests

- `deck-tui`: drawing of the rows, header, bottom bar, menu and dialog with images on and off; hit-testing of the moved chips; the menu's project choice; the pre-pass covering menu and dialog logos.
- `crates/deck/tests/`: the sidebar-mouse and titles tests.

## Notes

- "Workspace" in the request is deck's project.
- Claude panels keeping the name `claude` was not a code bug: deck's hooks were not installed, so the first prompt never arrived (`deck hooks install`).
