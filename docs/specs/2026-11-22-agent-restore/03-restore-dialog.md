---
type: shippable
status: done
---

# 03 — The Restore button and dialog

## Scope

The user clicks `Restore` at the right of the sidebar's bottom bar (or presses `prefix R`), sees the recent sessions of Claude Code, OpenCode and Codex in three columns, picks one and gets it resumed in a new panel. Owning crates: `deck-tui` (button, dialog), `deck-core` (binding).

## Requirements

1. **Button**: `Restore` (9 cells; `↻` under a 26-column sidebar) right-aligned in DECK-10 sub-spec 02's reserved slot, drawn and hovered like `New project`. A click opens the dialog. Binding `restore = "R"` (`[keys]`, Session group of the help overlay, default `R`) opens it from the keyboard; in sidebar focus `R` does too. With no project in the session, the button and the binding show the flash `no project to restore into` instead.
2. **Opening**: the dialog opens at once and sends three `ListAgentSessions { kind, limit: 50 }` requests, one per agent. The answers arrive in any order and are routed by `kind`, never through the picker's request queue. Each column shows `loading…` (muted) until its `Response::AgentSessions` arrives, then its list, or its `error` text in the `warning` role, or `no sessions` (muted). The dialog never blocks on a slow column; closing it drops late answers.
3. **Dialog** (see [Screen](#screen)): a centered rounded overlay, 90% of the width (at least 60 columns, at most the window) and 80% of the height (at least 10 rows), titled `Restore agent session`, with a filter line, three equal columns with a header each (`Claude Code`, `OpenCode`, `Codex`, with the count), and a key-hint line. Each session takes two lines: the title (truncated with `…`), and below it, `muted`, `<directory> · <age> ago` (the directory truncated from the left). Columns scroll independently, keeping the selection visible. Below 60 columns of window the dialog shows one column at a time (`Tab` switches; the header shows `‹ Claude Code ›`).
4. **Keys** (fixed): `Left`/`Right` and `Tab`/`Shift+Tab` move between columns, `Up`/`Down` (and `Ctrl+p`/`Ctrl+n`) between rows, `PageUp`/`PageDown` by a page, `Home`/`End` to the first and last row; printable keys and `Backspace` edit the filter (`Ctrl+u` clears it), which applies to all three columns at once with `matches` (sub-spec 01); `Enter` resumes the selected session; `Esc` closes (first `Esc` clears a non-empty filter). The prefix key is ordinary text here. The selection starts on the first non-empty column's first row, and moves to the nearest row when the filter changes. A column with no row is skipped by `Left`/`Right` only when another one has rows.
5. **Mouse**: hover highlights a row; a click selects it, a click on the selected row resumes it; the wheel scrolls the column under it; a click on a column header moves the selection to that column; a click outside the dialog closes it.
6. **Resuming**: the client picks the project with `project_for` (sub-spec 01) from the `SessionView`'s project roots and the active project, and sends `Action::NewPanelWith { project, launch: PanelLaunch::Resume { kind, session_id, cwd } }` (DECK-10 sub-spec 03). The dialog closes at once; the new panel becomes active and focused. An `ActionError` (a missing program, an invalid id) is a 4 s error flash. The daemon's warning about a vanished directory arrives as the usual notice.
7. **Help and hints**: the status bar shows the dialog's hints while it is open; `prefix ?` lists `restore`. The dialog is an overlay (input-mode order step 1): the prompt, picker or help cannot be open together with it, and opening it closes a hover state.
8. **Config**: `restore = "R"` in `default.toml`.

## Screen

100×30 window, the dialog open, filter `login`:

```text
 ╭─ Restore agent session ─────────────────────────────────────────────────────────────────────╮
 │ > login█                                                                                    │
 │                                                                                             │
 │ Claude Code (2)                │ OpenCode (1)                  │ Codex (0)                  │
 │ ─────────────────────────────  │ ───────────────────────────── │ ────────────────────────── │
 │ Fix the login redirect       ← │ Rework login form             │ no sessions                │
 │   ~/Projects/budget · 2h ago   │   ~/Projects/web · 3d ago     │                            │
 │ Add login rate limit           │                               │                            │
 │   ~/Projects/api · 5d ago      │                               │                            │
 │                                                                                             │
 │ ⏎ resume  ←→ column  ↑↓ row  type to filter  esc close                                      │
 ╰─────────────────────────────────────────────────────────────────────────────────────────────╯
```

- The selected row is drawn in the selection colours (`←` marks it here); the hovered row uses `muted` reversed. Headers are bold; the active column's header is in `accent`.
- While loading, a column shows `loading…`; with an error, the error text in `warning`.
- Window under 60 columns:

```text
 ╭─ Restore agent session ──────────────╮
 │ > █                                  │
 │ ‹ Claude Code (12) ›                 │
 │ Fix the login redirect               │
 │   ~/Projects/budget · 2h ago         │
 │ …                                    │
 │ ⏎ resume  tab column  esc close      │
 ╰──────────────────────────────────────╯
```

## Acceptance criteria

- [ ] With fake `CLAUDE_CONFIG_DIR`, `CODEX_HOME` and an `opencode` script on `PATH`, clicking `Restore` opens the dialog with three columns, each listing its fixture sessions newest first with title, directory and age (capture `restore`).
- [ ] The button is drawn at the right of the bottom bar in the reserved slot and highlights on hover (capture `restore-button`); at a 20-column sidebar it is `↻`.
- [ ] A slow fake `opencode` (3 s) leaves its column on `loading…` while the other two are already filled.
- [ ] A failing `opencode` shows its error in the column.
- [ ] Typing `login` filters all three columns; `Esc` clears the filter, a second `Esc` closes.
- [ ] `Right`, `Down`, `Enter` on the Codex column resumes that session: a fake `codex` records `resume <id>` and its cwd is the session's directory; the new panel is active.
- [ ] A session whose directory is inside the second project's root opens in that project's panel list, not the active one; a session outside every project opens in the active project.
- [ ] A session whose directory no longer exists starts in the project root and shows the notice.
- [ ] Mouse: a click selects, a second click resumes, a click outside closes, the wheel scrolls one column.
- [ ] `⌥Space R` and sidebar-focus `R` open the dialog; with no project the flash shows and nothing opens.
- [ ] Missing `claude` on `PATH`: choosing a Claude session shows the `not found in PATH` flash and creates no panel.
- [ ] With 50 sessions in one column, scrolling keeps the selection visible and `End` reaches the last.
- [ ] At a 50-column window the dialog shows one column at a time and `Tab` switches.
- [ ] The dialog closes with `Esc` while answers are still pending, and a late answer changes nothing.
- [ ] A `[keys] restore = "Z"` override moves the binding and shows in the help overlay.

## Tests

- `deck-tui` with `TestBackend`: dialog drawing (loaded, loading, error, empty, filtered, narrow), key and mouse handling, project choice for the action, closing with pending answers, the button at both widths and its hover.
- `deck-core`: binding default and override, `default.toml` sync.
- `crates/deck/tests/restore.rs`: the criteria above with fixture trees in a temp `CLAUDE_CONFIG_DIR` and `CODEX_HOME`, and fake `claude`, `codex` and `opencode` scripts on `PATH` (as the DECK-06 resume tests do).

## Notes

- The filter matches title and directory only; message bodies are never read past the first prompt.
- A session open in another deck panel is listed like any other; resuming it starts a second copy of the agent (non-goal).
