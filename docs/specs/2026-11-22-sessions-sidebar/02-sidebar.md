---
type: shippable
status: draft
---

# 02 — Sidebar, panels and projects

## Scope

The user sees the attached session's projects in a sidebar with their panels nested under them, and creates, renames, closes, collapses and switches between them with prefix keys, sidebar navigation or the mouse. A help overlay lists every binding. Owning crate: `deck-tui`; the binding table and config keys in `deck-core`.

## Requirements

1. **Layout**: when the sidebar is shown, it takes `sidebar_width` columns on the left plus a one-column separator; the panel area (sent as `Viewport`) is the rest. When the outer terminal is narrower than `sidebar_width + 1 + 20` columns, the sidebar is hidden automatically, so the panel area never drops below 20 columns; it shows again when the window grows, unless the user hid it.
2. **Sidebar content** (see [Screen](#screen)): the session name as a header; each project as a row with an expand marker (`▾`/`▸`), its name, and when collapsed its panel count; each panel of an expanded project as an indented row with its name and, right-aligned, its terminal count when more than 1. A project with no panels shows a muted `no panels` row. Each row reserves a one-cell status column before the name (blank until DECK-06). Names that do not fit end in `…`. When rows exceed the height, the sidebar scrolls to keep the active panel (or the sidebar selection) visible.
3. **Active and selected**: the active panel's row is drawn in the accent colour, bold, with a `▌` marker in the left gutter; the active project's name is bold. In sidebar focus, the selected row is drawn reversed.
4. **Bindings** (added to the table in `deck-core::config`, overridable under `[keys]`):

   | Action            | Default | Does                                                                 |
   | ----------------- | ------- | -------------------------------------------------------------------- |
   | `new_panel`       | `c`     | New panel in the active project                                      |
   | `next_panel`      | `n`     | `CyclePanel { delta: 1 }`                                             |
   | `prev_panel`      | `p`     | `CyclePanel { delta: -1 }`                                            |
   | `panel_1`…`panel_9` | `1`…`9` | Focus the N-th panel of the active project                          |
   | `rename_panel`    | `,`     | Prompt `Rename panel:` prefilled with its name                        |
   | `close_panel`     | `X`     | Confirm `Close panel "<name>" and its terminal? (y/n)`                |
   | `new_project`     | `P`     | Prompt `Directory:` (prefilled with the client's start cwd), then `Project name:` (prefilled with the directory's basename) |
   | `rename_project`  | `.`     | Prompt `Rename project:`                                             |
   | `close_project`   | `Q`     | Confirm `Close project "<name>" and its N terminals? (y/n)`           |
   | `toggle_sidebar`  | `b`     | Show or hide the sidebar                                             |
   | `focus_sidebar`   | `Tab`   | Move keyboard focus into the sidebar                                 |
   | `help`            | `?`     | Help overlay                                                         |

5. **Input modes** follow the precedence list in the overview's decisions. The prefix key still works in sidebar focus; inside prompts it is typed as text.
6. **Sidebar focus** keys: `Up`/`k`, `Down`/`j` move the selection; `Enter` focuses the selected panel (or, on a project row, toggles collapse); `Left`/`h` collapses and `Right`/`l` expands the selected project; `c` new panel in the selected project; `P` new project; `r` rename the selected row; `x` close the selected row (with the confirmation); `Esc`/`Tab` returns focus to the terminal. The status-bar hints show these keys while the sidebar has focus.
7. **Mouse**: a click on a panel row focuses that panel; on a project row toggles its collapse; the wheel scrolls the sidebar. A click on the separator does nothing.
8. **Prompts**: one-line editor in the status bar with a label, the cursor, `Left`/`Right`/`Home`/`End`/`Backspace`/`Delete`, `Ctrl+u` to clear, `Enter` to submit and `Esc` to cancel. `~` at the start of a directory, and a relative directory (against the client's start cwd), are resolved on the client. Pasted text is inserted as text (newlines dropped). An `ActionError` is shown as a flash in the error colour for 4 s and the prompt reopens with the entered text.
9. **Confirmations**: `y` confirms, `n` or `Esc` cancels; any other key is ignored.

   A prompt or confirmation whose target (panel or project) is removed meanwhile, by another client or by its process exiting, closes with the flash `panel "<name>" was closed` (or `project …`).
10. **Empty states** in the panel area, centered and muted: `No panels in <project> — ^Space c to create one` and `No projects — ^Space P to add one`. Typing goes nowhere (no `Input` is sent) while the panel area is empty.
11. **Help overlay**: a centered rounded box listing every action with its key (prefix first), grouped (Session, Panels, Projects, Terminal, Sidebar), generated from the binding table so overrides show; each action names its group in the table, and actions added by later specs (DECK-01's `detach` and `scroll`, DECK-03's terminal and resize actions) land in the open-ended Terminal group unless they name another; `Esc`, `q` or `?` closes it.
12. **Status bar** gains the active panel's name after the project name.
13. **Config**: `sidebar = true`, `sidebar_width = 28` (values outside 20–60 are an error naming the key).
14. **Several clients**: as in the overview's decisions, active project, active panel and collapse are shared through the `SessionView`; sidebar selection, sidebar focus, prompts and overlays stay local to the client.

## Screen

100×24 outer terminal, sidebar shown, `terminal-multiplexer` active with panel `editor`:

```text
 main                       │╭─ nvim ────────────────────────────────────────────────────────────╮
                            ││                                                                   │
 ▾ terminal-multiplexer     ││                                                                   │
▌  editor                   ││                                                                   │
   claude                   ││                                                                   │
   tests                    ││                                                                   │
 ▾ budget                   ││                                                                   │
   api                      ││                                                                   │
   no panels                ││                                                                   │
 ▸ dotfiles               2 ││                                                                   │
                            ││                                                                   │
                            │╰───────────────────────────────────────────────────────────────────╯
 main  terminal-multiplexer › editor                                         ^Space ? help  d detach
```

- Header row: session name, bold. A blank row separates it from the projects.
- Column 1 is the active-panel gutter (`▌`, accent); column 2 is the DECK-06 status column (blank); panel rows are indented by two more.
- `budget` shows how an empty project looks once its last panel closed (`no panels` is muted).

Sidebar focus, selection on `claude`, with the hints:

```text
 ▾ terminal-multiplexer     │
▌  editor                   │
   claude                   │   ← reversed row
 …
 SIDEBAR  ↑↓ move  ⏎ open  ←→ fold  c new  r rename  x close  esc back
```

Prompt:

```text
 Rename panel: tests█                                                       ⏎ save  esc cancel
```

## Acceptance criteria

- [ ] On attach, the sidebar of the mockup shows the session, its project and panel `1` (capture `sidebar`).
- [ ] `^Space c` adds panel `2`, focuses it, and the sidebar and status bar update.
- [ ] `^Space n`/`p` and `^Space 1` switch panels; the panel area shows the right shell (each shell has echoed a distinct marker).
- [ ] `^Space ,` renames the active panel; an empty name shows the error flash and reopens the prompt.
- [ ] `^Space P` with directory `~` proposes the home dir's basename as the name; accepting it, or typing `other`, adds a project whose shell's `pwd` is the home dir; a duplicate name shows the error; a relative directory resolves against the client's start cwd.
- [ ] `^Space X` asks, `n` keeps the panel, `y` closes it and focus moves as in sub-spec 01.
- [ ] Closing the last panel shows the `No panels` empty state (capture `empty-project`); `^Space c` recovers.
- [ ] `^Space Tab`, `j`, `Enter` focuses the selected panel; `h` collapses a project and its panel count appears (capture `sidebar-focus`).
- [ ] A mouse click on a panel row focuses it; on a project row toggles it.
- [ ] `^Space b` hides the sidebar and the shell's `stty size` grows by `sidebar_width + 1` columns.
- [ ] With `sidebar_width = 28`, the sidebar shows at 49 outer columns (panel area 20) and hides automatically at 48; with `sidebar_width = 60` it shows from 81 columns.
- [ ] `^Space ?` shows the help overlay with every default binding, and a `[keys]` override in the config shows its new key (capture `help`).
- [ ] With 30 panel rows in a 24-row window, the sidebar scrolls to keep the active panel visible.
- [ ] `^Space d` detaches while the sidebar has focus; `^Space` typed in a prompt inserts nothing special (the prompt treats it as a key, not the prefix).
- [ ] Two clients on one session: `^Space n` in one changes the active panel in both; sidebar focus and a prompt opened in one do not appear in the other; closing a panel from client A while client B has a rename prompt open for it closes B's prompt with the flash.

## Tests

- `deck-core`: binding table defaults and overrides; config validation for the new keys.
- `deck-tui`: `TestBackend` drawing tests for the sidebar (expanded, collapsed, empty project, long names, scrolling), sidebar focus, prompts, confirmations, empty states and the help overlay; unit tests for the prompt editor.
- `crates/deck/tests/sidebar.rs`: one scenario per acceptance criterion.

## Notes

- Colours and glyphs (`▾`, `▸`, `▌`) come from the `Theme` value (DECK-01 02-core-types notes); DECK-08 makes them configurable.
