# Sidebar, panels and projects

A session holds projects, a project holds panels, and a panel shows one terminal (the grid comes later). The sidebar on the left shows the attached session's projects with their panels nested under them.

## The sidebar

```text
 main                       │
                            │
 ▾ terminal-multiplexer     │
▌  editor                   │
   claude                   │
 ▾ budget                   │
   no panels                │
 ▸ dotfiles               2 │
```

- The header is the session name. Each project is a row with `▾` (expanded) or `▸` (collapsed), its name and, when collapsed, its panel count at the right. Each panel of an expanded project is a row with its name and, when it holds more than one terminal, the terminal count at the right. A project with no panels shows a muted `no panels`. Names that do not fit end in `…`.
- The active panel is drawn in the accent colour, bold, with a `▌` in the left gutter; the active project's name is bold. The cell before each name is reserved for the agent status.
- With more rows than fit, the list scrolls to keep the active panel (or, in sidebar focus, the selection) in view. The wheel scrolls it too, three rows a notch.
- The sidebar takes `sidebar_width` columns (default 28) and a separator; the panels get the rest. When the window is narrower than `sidebar_width + 21` columns the sidebar hides, so the panel area never drops below 20 columns, and it comes back when the window grows, unless you hid it with `toggle_sidebar`.
- The active project, active panel and which projects are collapsed are shared by every client attached to the session. The selection, sidebar focus, prompts and overlays are each client's own.

## Bindings

After the prefix (see [keys](keys.md)): `c` new panel, `n` / `p` next and previous panel, `1`..`9` the N-th panel of the active project, `,` rename the panel, `X` close it, `P` new project, `.` rename the project, `Q` close it, `b` show or hide the sidebar, `Tab` move focus into the sidebar, `?` help. The full table, with the config names, is in [keys](keys.md#bindings).

- A new panel runs one shell in the project's directory and is named with the lowest unused number (`1`, `2`, ...).
- A new project asks `Directory:` (prefilled with the directory deck was started in), then `Project name:` (prefilled with the directory's name: `~` for the home directory, `root` for `/`). `~` and relative paths are resolved by the client, against that start directory. Its first panel starts with it.
- Closing a panel or a project asks `Close panel "<name>" and its terminal? (y/n)` first. `y` confirms; `n` or `Esc` cancels; any other key is ignored.
- Closing the last panel of a project leaves the project empty. The panel area then shows `No panels in <project> — <key> to create one` (or `No projects — <key> to add one`), and typing goes nowhere.

## Sidebar focus

`Option+Space Tab` moves the keyboard into the sidebar, starting on the active panel; the status bar shows `SIDEBAR` and the hints. Keys no longer reach the terminal; the prefix still works (`Option+Space d` detaches).

| Keys                        | What they do                                                                         |
| --------------------------- | ------------------------------------------------------------------------------------ |
| `Up` / `k`, `Down` / `j`    | Move the selection (reversed)                                                        |
| `Enter`                     | Focus the selected panel and return to the terminal; on a project, fold or unfold it |
| `Left` / `h`, `Right` / `l` | Fold or unfold the selected row's project                                            |
| `c`, `P`                    | New panel in the selected project, new project                                       |
| `r`, `x`                    | Rename the selected row, close it (with the confirmation)                            |
| `Esc`, `Tab`                | Return to the terminal                                                               |

These keys are fixed in v1. Entering the sidebar ends scroll mode.

## Mouse

A click on a panel row focuses that panel; on a project row it folds or unfolds it. The wheel scrolls the list. A click on the separator does nothing. No sidebar key or click is needed first: the mouse works without sidebar focus.

## Prompts

Rename and new-project questions are a one-line editor in the status bar: `Left`, `Right`, `Home`, `End`, `Backspace`, `Delete`, `Ctrl+u` to clear, `Enter` to save, `Esc` to cancel. Pasted text is inserted with its newlines dropped. The prefix key is ordinary there and inserts nothing.

When the daemon refuses a name or a directory (empty, too long, a duplicate project, not a directory), the reason shows in red for 4 seconds and the prompt reopens with what you typed (for a directory that does not exist, the `Directory:` prompt). When the panel or project a prompt or confirmation is about is closed meanwhile, by another client or because its process exited, the dialog closes with `panel "<name>" was closed` (or `project ...`).

## Help

`Option+Space ?` shows every binding in a box, grouped (Session, Panels, Projects, Terminal, Sidebar), from the binding table, so `[keys]` overrides show. `Esc`, `q` or `?` close it.

## Configuration

```toml
sidebar = true # false hides it until toggled
sidebar_width = 28 # 20 to 60
```

A width outside 20 to 60 is an error naming `sidebar_width`.
