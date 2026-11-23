# Sidebar, panels and projects

A session holds projects, a project holds panels, and a panel shows one terminal (the grid comes later). The sidebar on the left shows the attached session's projects with their panels nested under them.

## The sidebar

```text
 Workspaces   + New project │
                            │
 ▾ terminal-multiplexer     │
▌  ✎  editor                │
 ! ✻  claude                │
 ▾ budget                   │
   no panels                │
 ▸ dotfiles               2 │
                            │
  + New            Restore  │
```

- The header reads `Workspaces` (accent, bold) with a `+ New project` chip right-aligned on it, one cell from the edge (`+` alone when the sidebar is under 26 columns wide); the title is cut with `…` to leave room for the chip. The session's name is in the status bar. Each project is a row with `▾` (expanded) or `▸` (collapsed), its name and, when collapsed, its panel count at the right. Each panel of an expanded project is a row with its name and, when it holds more than one terminal, the terminal count at the right. A project with no panels shows a muted `no panels`. Names that do not fit end in `…`.
- The active panel is drawn in the accent colour, bold, with a `▌` in the left gutter; the active project's name is bold. The cell before each name shows the agent status of the panel's most urgent terminal (see [agents](agents.md)). A panel whose agent needed you while you were not looking is bold with a `•` after its name, and a collapsed project shows `!<n>` for its waiting panels ([attention](agents.md#attention)).
- With more rows than fit, the list scrolls to keep the active panel (or, in sidebar focus, the selection) in view. The wheel scrolls it too, three rows a notch.
- The last row of the sidebar is a bottom bar with a `+ New` chip at the left (`+` alone when the sidebar is under 26 columns wide), and the row above it is blank, so the list is two rows shorter. A sidebar under 8 rows tall draws neither. The right-hand end has the `Restore` button (`Restore`, or `↻` under 26 columns), which opens the [restore dialog](agents.md#the-restore-dialog). All chips, the header's included, are muted and are drawn in the selection colours under the pointer; a click is a press and a release on the same chip. With no project, `+ New` flashes `no project` and `Restore` flashes `no project to restore into`.
- The sidebar takes `sidebar_width` columns (default 28) and a separator; the panels get the rest. When the window is narrower than 41 columns the sidebar hides, and it comes back when the window grows, unless you hid it with `toggle_sidebar`. A shrinking window narrows a wide sidebar so the panel area keeps its 20 columns (a sidebar at 60 in an 81-column window becomes 59 in an 80-column one); a growing window does not widen it again.
- The active project, active panel and which projects are collapsed are shared by every client attached to the session. The selection, sidebar focus, prompts and overlays are each client's own.

## Bindings

After the prefix (see [keys](keys.md)): `c` new panel, `n` / `p` next and previous panel, `1`..`9` the N-th panel of the active project, `,` rename the panel, `X` close it, `P` new project, `N` the [New menu](#the-new-button-and-menu), `R` the [restore dialog](agents.md#the-restore-dialog), `.` rename the project, `Q` close it, `b` show or hide the sidebar, `Tab` move focus into the sidebar, `?` help. The full table, with the config names, is in [keys](keys.md#bindings).

- A new panel runs one shell in the project's directory and starts with the lowest unused number (`1`, `2`, ...) as its name, which is only a fallback for the moment before its first name: every panel, the first of a session included, is named after its shell at once.
- A panel keeps its name in step with what runs in it until you rename it: a coding agent shows its title without the spinner in front (`Fix login bug`), else the first line of its first prompt; any other program shows the title it sets (a shell's directory, an editor's file; a bare agent title such as `✳ Claude Code` loses its glyphs), else its own name (`nvim`, `zsh`). The name follows the panel's first terminal that has an agent, else its focused terminal, and is cut to 32 cells. When an agent exits, the name follows what is left. Renaming a panel (`,`, `deck rename-panel`) locks the name; `deck rename-panel --auto` unlocks it.
- Every panel row starts with an icon for what runs in its source terminal: `▌`, the status column, a space, the icon (always two cells: a text icon in the first and a blank second, a logo in both), a space, then the name (`▌⠹ ✻  Fix login bug`), so names start at the same column on every row, and Nerd Font glyphs wider than a cell do not touch the name. The status bar uses the same two-cell icon (`project › ⠹ ✻  Fix login bug`). With `icons = "none"` rows have no icon column. The agent's own icon for Claude Code (`✻`), Codex (`◎`) and OpenCode (`◇`); `❯` for a shell and any unknown program; `✎` for an editor, `±` git, `◔` a monitor, `⇄` ssh, `▲` a build tool, `¶` a pager and `λ` a REPL. A renamed panel keeps its name and the icon follows its program. A panel with no terminals has no icon. `[theme] icons = "nerd"` uses Nerd Font glyphs, `"none"` draws no icons; see [config](config.md#icons). In Ghostty and kitty the three agent icons are the agents' logos, drawn as small images two cells wide in the row's colour (`accent` on the active panel), in the sidebar and in the status bar; every other terminal, tmux, Zellij and a deck panel keep the text icon, and `[theme] icon_images = "off"` turns the images off.
- The status bar shows `project › <status> <icon> <name>` (the status glyph only with an agent) for the active panel, then ` · <title>` (the source terminal's title) only for a panel you named by hand, and only when the title differs from the name.
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
| `n`                         | The [New menu](#the-new-button-and-menu) for the selected project                    |
| `R`                         | The [restore dialog](agents.md#the-restore-dialog)                                   |
| `r`, `x`                    | Rename the selected row, close it (with the confirmation)                            |
| `Esc`, `Tab`                | Return to the terminal                                                               |

These keys are fixed in v1. Entering the sidebar ends scroll mode.

## Mouse

A click on a panel row focuses that panel; on a project row it folds or unfolds it. The wheel scrolls the list. Clicking the `+ New project` chip in the header does what `prefix P` does: the `Directory:` prompt, prefilled with the directory deck was started in, then `Project name:`; it does nothing while a prompt, a confirmation or an overlay is open. The chip turns to the selection colours while the pointer is over it. Motion over the sidebar is never sent to a program, and the screen is redrawn only when the highlighted button changes. No sidebar key or click is needed first: the mouse works without sidebar focus.

### The New button and menu

The `+ New` chip of the bottom bar opens the menu for the **selected project**: in sidebar focus the selected row's project (or the selected panel's project), otherwise the active project; with no project it flashes `no project`. Project rows have no button of their own and keep their panel count and waiting count under the pointer. `prefix N` (for the active project; nothing without one) and `n` in sidebar focus (for the selected project, or the project of the selected panel) open the same menu.

The rounded `New in <project>` menu has four items, each with its icon (two cells, then a space) and its digit at the right: `Terminal` (`❯`), `Claude`, `OpenCode` and `Codex`. The icons are the agents' logos where images are on (see above) and text icons elsewhere (none with `icons = "none"`); the highlight covers the icon. The menu sits right above the bottom bar with its left edge on the sidebar's first column, so a narrow sidebar lets it extend over the panel area; it always stays inside the window, and with the sidebar hidden it sits at the top right.

| Keys / mouse                                  | What they do                              |
| --------------------------------------------- | ----------------------------------------- |
| `Up` / `k`, `Down` / `j`, `Ctrl+p` / `Ctrl+n` | Move the highlight                        |
| `Enter`, `1` to `4`                           | Choose the highlighted item, or that item |
| `Esc`, `q`                                    | Close the menu                            |
| Hover, click on an item                       | Move the highlight, choose the item       |
| Click elsewhere                               | Close the menu; the click goes no further |

The prefix key is ordinary while the menu is open, the wheel does nothing, and these keys are fixed in v1. Choosing `Terminal` adds a new panel with a shell in the project's directory (like `prefix c` for that project; `prefix t` adds a terminal to the active panel instead); the others add a panel running the agent's [`command`](agents.md#starting-an-agent-in-a-new-panel). The new panel becomes active and focused, the sidebar expands its project if it was folded, and the keyboard leaves sidebar focus once the daemon has accepted the launch (a refusal leaves it where it was). A refusal, such as a program that is not in `PATH`, shows a red flash for 4 seconds. When the project is closed meanwhile (by another client), the menu closes with `project "<name>" was closed`.

### Resizing the sidebar

Drag the separator column (the `│` between the sidebar and the panels) to make the sidebar wider or narrower; the terminals follow, because the panel area is the rest of the window. While you drag, the separator is drawn `┃` in the accent colour and the status bar shows `SIDEBAR <width>` in the prefix badge's style. The width stays between 20 and 60 columns and leaves the panel area at least 20 columns (in a 60-column window the sidebar stops at 39). The press and the drag are never sent to a program, and the terminals' ratios are kept. A press on the separator while the sidebar is hidden does nothing, and neither does one under a prompt or an overlay.

The dragged width belongs to this client: it is kept across `toggle_sidebar` and session switches, is not saved to `config.toml`, and is lost when the client exits. Editing `sidebar_width` in `config.toml` replaces it; any other config edit leaves it.

## Prompts

Rename and new-project questions are a one-line editor in the status bar: `Left`, `Right`, `Home`, `End`, `Backspace`, `Delete`, `Ctrl+u` to clear, `Enter` to save, `Esc` to cancel. Pasted text is inserted with its newlines dropped. The prefix key is ordinary there and inserts nothing.

When the daemon refuses a name or a directory (empty, too long, a duplicate project, not a directory), the reason shows in red for 4 seconds and the prompt reopens with what you typed (for a directory that does not exist, the `Directory:` prompt). When the panel or project a prompt or confirmation is about is closed meanwhile, by another client or because its process exited, the dialog closes with `panel "<name>" was closed` (or `project ...`).

## Help

`Option+Space ?` shows every binding in a box, grouped (Session, Panels, Projects, Terminal, Sidebar), from the binding table, so `[keys]` overrides show. The four focus keys share one line. In a window too short for the headings, the box lists the bindings without them. `Esc`, `q` or `?` close it.

## Configuration

```toml
sidebar = true # false hides it until toggled
sidebar_width = 28 # 20 to 60
```

A width outside 20 to 60 is an error naming `sidebar_width`.
