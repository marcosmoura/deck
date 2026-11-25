# Sidebar, panels and spaces

A session holds spaces, a space holds panels, and a panel shows one or more terminals in a grid. The sidebar on the left shows the attached session's spaces with their panels nested under them, and a panel with two or more terminals has one nested row per terminal.

## The sidebar

```text
 «  main      +  New space  │
                            │
 terminal-multiplexer    ▾  │
 ✎  editor                  │
 ✻  claude               !  │
 ❯  api + tests          !  │
  ├ ❯  zsh                  │
  ├ ✻  Write tests       !  │
  └ ✎  main.rs              │
 budget                  ▾  │
   no panels                │
 dotfiles              2 ▸  │
                            │
  +  New         ↻  Resume  │
```

- The header starts with a `«` chip that [collapses the sidebar](#collapsed-sidebar): the icon (two cells) with one cell of clickable padding on each side, columns 0 to 3. The session's name (accent, bold) starts at column 4, with a `+ New space` chip at the right, with one cell of clickable padding on each side and a one-cell margin at the right edge. The chip is an icon (`+`, two cells, like every icon: an image where images are on) and the label; it shows only the icon when the sidebar is under 26 columns wide. The name is cut with `…` to leave room for the chip, and a rename of the session shows at once. Each space is a row with its name and, at the right, `▾` (expanded) or `▸` (collapsed) in a two-cell slot before the one-cell margin and, when collapsed, its panel count before it. Each panel of an expanded space is a row with its name; a panel with two or more terminals is followed by one [terminal row](#terminal-rows) per terminal. A space with no panels shows a muted `no panels`. Names that do not fit end in `…`.
- The active panel's row is a band across the sidebar's full width in the selection colours (`selection_fg` and `selection_bg`, reversed while both are `default`), bold, with the status glyph, counts and notes in the row's text colour while the selection colours are the default reversed pair; the active space's name is bold. In sidebar focus only the selected row has the band, and the active panel's row is drawn in the accent colour, bold, with no band. The row or chip under the pointer gets the `hover` background (a row across the full width, a chip over its own cells), and muted text on it turns `sidebar_fg`; the `no panels` row, blank rows and the header's session name do not hover, and a row with the band keeps it under the pointer. The status slot at the right of a row shows the agent status of the panel's most urgent terminal (see [agents](agents.md)). A panel whose agent needed you while you were not looking is bold with a `•` after its name, and a collapsed space shows `!<n>` (always the Unicode `!`) for its waiting panels ([attention](agents.md#attention)).
- With more rows than fit, the list scrolls to keep the active panel (or, in sidebar focus, the selection) in view. The wheel scrolls it too, three rows a notch.
- The last row of the sidebar is a bottom bar with a `+ New` chip at the left (the `+` icon alone when the sidebar is under 26 columns wide), and the row above it is blank, so the list is two rows shorter. A sidebar under 8 rows tall draws neither. The right-hand end has the `Resume` button (the `↻` icon and `Resume`, or the icon alone under 26 columns), which opens the [restore dialog](agents.md#the-restore-dialog). All chips, the header's included, are muted and get the `hover` background under the pointer; a click is a press and a release on the same chip. With no space, `+ New` flashes `no space` and `Resume` flashes `no space to restore into`.
- The sidebar takes `sidebar_width` columns (default 28) and a separator; the panels get the rest. When the window is narrower than 41 columns the sidebar hides, and it comes back when the window grows, unless you hid it with `toggle_sidebar` (the [collapsed sidebar](#collapsed-sidebar) fits from 29 columns). A shrinking window narrows a wide sidebar so the panel area keeps its 20 columns (a sidebar at 60 in an 81-column window becomes 59 in an 80-column one); a growing window does not widen it again.
- The active space, active panel and which spaces are collapsed are shared by every client attached to the session. The selection, sidebar focus, prompts and overlays are each client's own.

## Bindings

After the prefix (see [keys](keys.md)): `c` new panel, `n` / `p` next and previous panel, `1`..`9` the N-th panel of the active space, `,` rename the panel, `X` close it, `P` new space, `N` the [New menu](#the-new-button-and-menu), `R` the [restore dialog](agents.md#the-restore-dialog), `.` rename the space, `Q` close it, `b` show or hide the sidebar, `Tab` move focus into the sidebar, `?` help. The full table, with the config names, is in [keys](keys.md#bindings).

- A new panel runs one shell in the space's directory and starts with the lowest unused number (`1`, `2`, ...) as its name, which is only a fallback for the moment before its first name: every panel, the first of a session included, is named after its shell at once.
- A panel keeps its name in step with what runs in it until you rename it: a coding agent shows its title without the spinner in front (`Fix login bug`), else the first line of its first prompt; any other program shows the title it sets (a shell's directory, an editor's file; a bare agent title such as `✳ Claude Code` loses its glyphs), else its own name (`nvim`, `zsh`). The name follows the panel's first terminal that has an agent, else its focused terminal, and is cut to 32 cells. When an agent exits, the name follows what is left. Renaming a panel (`,`, `deck rename-panel`) locks the name; `deck rename-panel --auto` unlocks it.
- Every panel row starts with an icon for what runs in its source terminal: a blank column, the icon (two cells), a space, then the name (` ✻  Fix login bug`), so names start at the same column (4) on every row. The status slot (two cells) is at the right end of the row, before the one-cell margin, and only for a panel with an agent (`Fix login bug        ⠹ `): a panel without one has no slot and its name gets the width. After the name come the unseen slot and the status slot, one space apart, and the name is cut with `…` before whatever is on its right. An icon is a glyph in the first cell of its slot and a blank second, or, in the image form, an image in both. The agent's own icon for Claude Code (`✻`), Codex (`◎`) and OpenCode (`◇`); `❯` for a shell and any unknown program; `✎` for an editor, `±` git, `◔` a monitor, `⇄` ssh, `▲` a build tool, `¶` a pager and `λ` a REPL (the Unicode glyphs; Nerd Font glyphs instead where the terminal has them). A renamed panel keeps its name and the icon follows its program. A panel with no terminals has no icon. The icons cannot be changed and come in three forms, picked by what the outer terminal says it can show; see [config](config.md#icons). As images every icon (program icons, the space carets, the status marks, the spinner included, and the unseen marker) is a small image two cells wide in the colour its glyph would have (the band's on the active panel, outside sidebar focus); Claude Code's icon, glyph or image, is always Claude's orange, the band included. The unseen marker after the name is a two-cell slot too.
- A kept terminal that exited and is alone in its panel adds ` · exited <code>` (muted) after the panel's name in its row; the name is cut first to make room.
- A panel row has no terminal count: a panel with several terminals shows them as rows.
- A new space asks `Directory` (prefilled with the directory deck was started in), then `Space name` (prefilled with the directory's name: `~` for the home directory, `root` for `/`). `~` and relative paths are resolved by the client, against that start directory. Its first panel starts with it.
- Closing a panel or a space asks `Close panel "<name>" and its terminal?` first, in a centered dialog with `y yes  n no` under the question. `y` confirms; `n` or `Esc` cancels; any other key is ignored.
- Closing the last panel of a space leaves the space empty. The panel area then shows `No panels in <space> — <key> to create one` (or `No spaces — <key> to add one`), and typing goes nowhere.

### Terminal rows

A panel with two or more terminals is followed, while its space is expanded, by one row per terminal in grid order; a panel with one terminal has none, and the rows come and go as terminals are added and closed. A terminal row starts with a muted tree glyph in column 2, under the panel's icon (`├`, or `└` for the last terminal), a space, the terminal's icon in the column of the panel names (from its own agent or program, two cells), a space and its label from column 7. The label is the terminal's title without its spinner and status glyphs, else its program; a kept terminal that exited adds ` · exited <code>` (muted), and the label is cut before the note. The status slot at the right shows that terminal's own agent status. A terminal row has no number, count or unseen marker. Collapsing the space hides the rows.

The focused terminal of the active panel is drawn in the accent colour, bold; the band stays on the active panel's row, and in sidebar focus it goes to the selected row, which may be a terminal row. The terminal rows of a dragged panel are dimmed with it.

## Collapsed sidebar

The `«` chip at the left of the header (or the `collapse_sidebar` binding, unbound by default) collapses the sidebar to a strip of 8 columns and the separator, which lists every terminal of the session as one entry: its status and a number. The panel area gets the columns the sidebar gives up, and the terminals are resized. The state is the client's own, like a hidden sidebar: other clients, and the state file, do not see it. `sidebar_collapsed = true` starts each client collapsed.

```text
   »    │   the expand chip (muted; the hover background under the pointer)
        │
 ⠹  1   │   claude: its agent's status mark, in the status colour
 ❯  2   │   zsh: no agent, so its program icon
 !  3   │   the focused terminal of the active panel: the band
 ────── │   a muted rule between two spaces
 ✎  4   │   dotfiles' only terminal (the space is folded, its terminal still shows)
        │
   +    │   the New chip
```

- One entry per terminal, in the sidebar's order: spaces in order, panels in order, terminals in grid order. Folded spaces count: collapsing shows every terminal. A panel with one terminal has one entry. A space with no terminals adds nothing, and no rule.
- An entry is a blank column, the mark (two cells: an image where images are on), a blank, and the number from column 4 (`1` to `999`). The mark is the terminal's agent status (spinner, `!`, `●`, `○`) in its status colour, else the terminal's program icon. The number is the entry's place in the list, counting entries only, not rules. A kept terminal that exited has its mark and number muted; the entries of a panel with unseen attention are bold. The numbers exist only here: the grid borders keep `<number> · <title>`.
- The focused terminal of the active panel has the band. In sidebar focus the band is on the selected entry, and the focused terminal is drawn in the accent colour, bold. Entries get the `hover` background under the pointer.
- With more entries than fit, the list scrolls to keep the focused terminal (or, in sidebar focus, the selection) in view, and the wheel scrolls it three rows a notch.
- The header has only the `»` chip (its icon in columns 3 and 4, with a column of clickable padding on each side, so the chip is centred in the strip with two columns on each side), which expands the sidebar again at its previous width (clamped to the window). The bottom bar has only the `+` chip, in the same columns, which opens the [New menu](#the-new-button-and-menu); there is no `Resume` chip (`Option+Space R` still opens the restore dialog).
- A click on an entry makes its panel and space active and focuses that terminal; a right click opens the terminal menu. A press on an entry never starts a drag, and a press on the separator does nothing while collapsed.
- The strip fits a window of at least 29 columns (8 + 1 + 20), so in a window of 29 to 40 columns the full sidebar does not fit: there the `»` chip and `collapse_sidebar` leave it collapsed and flash `window too narrow to expand`, and so does a config reload that sets `sidebar_collapsed = false` there. Under 29 columns the sidebar hides, and it comes back when the window grows. `collapse_sidebar` also shows a sidebar hidden with `toggle_sidebar`.
- In sidebar focus the selection moves over entries only (`Up`/`Down`), and `Enter`, `x`, `r`, `c`, `n` and `R` do what they do on a terminal row; `Left`/`h` and `Right`/`l` do nothing. Focus starts on the focused terminal of the active panel. Collapsing or expanding keeps the selection on the same terminal, or on its panel's or space's row once expanded.

## Sidebar focus

`Option+Space Tab` moves the keyboard into the sidebar, starting on the active panel; the [floating bar](keys.md#the-floating-bar) shows `SIDEBAR` and the hints. Keys no longer reach the terminal; the prefix still works (`Option+Space d` detaches).

| Keys                        | What they do                                                                         |
| --------------------------- | ------------------------------------------------------------------------------------ |
| `Up` / `k`, `Down` / `j`    | Move the selection (reversed)                                                        |
| `Enter`                     | Focus the selected panel or terminal and return to it; on a space, fold or unfold it |
| `Left` / `h`, `Right` / `l` | Fold or unfold the selected row's space                                              |
| `c`, `P`                    | New panel in the selected space, new space                                           |
| `n`                         | The [New menu](#the-new-button-and-menu) for the selected space                      |
| `R`                         | The [restore dialog](agents.md#the-restore-dialog)                                   |
| `r`, `x`                    | Rename the selected row, close it (with the confirmation)                            |
| `Esc`, `Tab`                | Return to the terminal                                                               |

On a terminal row, `Enter` makes its panel active and focuses that terminal, `r` renames its panel, `x` closes that terminal (`Close terminal <number>?`, at once for a kept terminal that exited), and `Left`/`Right`, `c` and `n` act on its space. `Up`/`Down` pass through terminal rows. When the selected terminal row disappears, the selection falls back to its panel's row, then its space's.

These keys are fixed in v1. Entering the sidebar ends scroll mode.

## Mouse

A click on a panel row focuses that panel; on a terminal row it makes its panel (and space) active and focuses that terminal; on a space row it folds or unfolds it. A click is a press and a release on the same row: the press alone does nothing, and a release elsewhere (or after the pointer left the row for another one, which is a [drag](#reordering-spaces-and-panels)) does nothing. The wheel scrolls the list. Clicking the `+ New space` chip in the header does what `prefix P` does: the `Directory` prompt, prefilled with the directory deck was started in, then `Space name`; it does nothing while a prompt, a confirmation or an overlay is open. Motion over the sidebar is never sent to a program, and the screen is redrawn only when the hovered row or chip changes; a click, an open dialog, overlay or menu, and the pointer leaving the sidebar end the hover, and a wheel scroll recomputes it. No sidebar key or click is needed first: the mouse works without sidebar focus.

### The New button and menu

The `+ New` chip of the bottom bar opens the menu for the **selected space**: in sidebar focus the selected row's space (or the selected panel's space), otherwise the active space; with no space it flashes `no space`. Space rows have no button of their own. `prefix N` (for the active space; nothing without one) and `n` in sidebar focus (for the selected space, or the space of the selected panel) open the same menu.

The rounded `New in <space>` menu has four items, each with its icon (two cells, then a space) and its digit at the right: `Terminal` (`❯`), `Claude`, `OpenCode` and `Codex`. The icons are images where images are on (see above) and glyphs elsewhere; the highlight covers the icon. The menu sits right above the bottom bar with its left edge on the sidebar's first column, so a narrow sidebar lets it extend over the panel area; it always stays inside the window, and with the sidebar hidden it sits at the top right.

| Keys / mouse                                  | What they do                              |
| --------------------------------------------- | ----------------------------------------- |
| `Up` / `k`, `Down` / `j`, `Ctrl+p` / `Ctrl+n` | Move the highlight                        |
| `Enter`, `1` to `4`                           | Choose the highlighted item, or that item |
| `Esc`, `q`                                    | Close the menu                            |
| Hover, click on an item                       | Move the highlight, choose the item       |
| Click elsewhere                               | Close the menu; the click goes no further |

The prefix key is ordinary while the menu is open, the wheel does nothing, and these keys are fixed in v1. Choosing `Terminal` adds a new panel with a shell in the space's directory (like `prefix c` for that space; `prefix t` adds a terminal to the active panel instead); the others add a panel running the agent's [`command`](agents.md#starting-an-agent-in-a-new-panel). The new panel becomes active and focused, the sidebar expands its space if it was folded, and the keyboard leaves sidebar focus once the daemon has accepted the launch (a refusal leaves it where it was). A refusal, such as a program that is not in `PATH`, shows a red flash for 4 seconds. When the space is closed meanwhile (by another client), the menu closes with `space "<name>" was closed`.

The `New panel` item of a space's [row menu](#row-menus) opens the same menu with its top-left corner on the cell of the right click instead of above the bar (moved left and up to stay in the window, and drawn over the sidebar and the separator).

### Row menus

A right click on a space or panel row opens a small context menu at the pointer: a rounded box with no title and no hint row, one item per row with its digit right-aligned and muted, at least 16 columns inside. The first item is highlighted; the row keeps its `hover` background while the menu is open, and the floating bar shows no mode hints (flashes still show). Its top-left corner is the pressed cell; near the right or bottom edge the box moves left or up to stay in the window. The sidebar never forwards the mouse, so these rows always open their menu.

| Row   | Items                                                                                                                        |
| ----- | ---------------------------------------------------------------------------------------------------------------------------- |
| Panel | `Rename` (1), `Close` (2): the rename prompt and the close question of the sidebar keys `r` and `x`                          |
| Space | `Rename` (1), `New panel` (2): the [New menu](#the-new-button-and-menu) at the click, `Close` (3): the close question of `x` |

Keys are those of the New menu: `Up`/`k`, `Down`/`j`, `Ctrl+p`/`Ctrl+n` move, `Enter` or a digit chooses, `Esc`/`q` close, hover moves the highlight and a click on an item chooses it. A press anywhere else closes the menu and goes no further (a right press elsewhere does not open another menu). The prefix key is ordinary while it is open, the wheel does nothing, and the keys are fixed. A dialog opened from an item replaces the menu. When the panel or space is closed meanwhile, the menu closes with `panel "<name>" was closed` or `space "<name>" was closed`. The header and its chips, the bottom bar, blank rows, `no panels` and the separator open no menu. A right click on a terminal row opens the [terminal menu](layout.md#the-terminal-menu) for that terminal, whichever panel it is in: its items act on that terminal and its panel, and `New terminal` makes the panel active and adds the terminal to it. The menu keeps the hover background of the sidebar row it was opened from; a menu opened on a terminal in the grid lights no sidebar row.

### Reordering spaces and panels

Press a space row or a panel row and move the pointer to another row of the list to drag it. A space moves with its panels, among the session's spaces; a panel moves inside its own space and never into another. While you drag, the list stays still: the dragged row stays in its place, dimmed (all its text `muted`, no band, not bold), and the row of the item whose place it will take, the target, gets a `➤` (the Nerd Font caret in the Nerd and Image tiers; `accent`, bold) and a space in its first two columns, with its whole content shifted two columns to the right (names are cut with `…` as usual and counts stay inside the sidebar); the row otherwise keeps its look, the band included (the caret and its space are drawn outside the band, which starts after them) (a space's target is its own row, not its panels). Over its own place nothing is highlighted, there is no hover, and the floating bar shows `MOVE` with `esc cancel`. The target is the place of the row under the pointer, read on the daemon's rows so the list does not jump under the pointer: for a space, the space whose row or block the pointer is on; for a panel, the panel row it is on, or the first or last panel of its space when the pointer is above or below them. Blank rows and `no panels` count as the space above. On the header, the bottom bar or in the panel area the last target is kept; on the header row, the blank row under it or the last row of the list, the list scrolls one row per motion event (up on the first two, down on the last).

The release drops: the list shows the new order at once and the daemon moves the item, every attached client sees the new order, `prefix 1`..`9`, `prefix n`/`p` and `Up`/`Down` in sidebar focus follow it, and it is saved. Releasing on the item's own place sends nothing; releasing over the panel area, `Esc`, closing the dragged item from another client, a window that loses focus or becomes too small, an overlay or a config reload that hides the sidebar cancel it with nothing sent. Any other key cancels the drag and then acts as usual. A terminal row cannot be dragged: moving the pointer from it to another row drops the press, and nothing happens on the release. A dragged panel carries its terminal rows (dimmed with it), and over a terminal row the target is its panel (a space drag's target is the terminal row's space). The press, the motion and the release are never sent to a program. The active space and panel, folded spaces and the selection stay as they are.

### Resizing the sidebar

Drag the separator column (the `│` between the sidebar and the panels) to make the sidebar wider or narrower; the terminals follow, because the panel area is the rest of the window. While you drag, the separator is drawn `┃` in the accent colour and the floating bar shows `SIDEBAR <width>`. The width stays between 20 and 60 columns and leaves the panel area at least 20 columns (in a 60-column window the sidebar stops at 39). The press and the drag are never sent to a program, and the terminals' ratios are kept. A press on the separator while the sidebar is hidden does nothing, and neither does one under a prompt or an overlay.

The dragged width belongs to this client: it is kept across `toggle_sidebar` and session switches, is not saved to `config.toml`, and is lost when the client exits. Editing `sidebar_width` in `config.toml` replaces it; any other config edit leaves it.

## Prompts

Rename and new-space questions are a centered dialog, a rounded box over everything else with the label as its title (`Rename panel`, `Directory`, `Space name`), a one-line editor and `⏎ save  esc cancel` at the bottom (at least 40 columns, 5 rows; the editor never widens it): `Left`, `Right`, `Home`, `End`, `Backspace`, `Delete`, `Ctrl+u` to clear, `Ctrl+w` to delete the word before the cursor, `Enter` to save, `Esc` to cancel. Pasted text is inserted with its newlines dropped. The prefix key is ordinary there and inserts nothing.

When the daemon refuses a name or a directory (empty, too long, a duplicate space, not a directory), the reason shows in red for 4 seconds on the dialog's bottom row (a long one widens the dialog up to the window width minus 2, then is cut with `…`) and the prompt reopens with what you typed (for a directory that does not exist, the `Directory` prompt). When the panel or space a prompt or confirmation is about is closed meanwhile, by another client or because its process exited, the dialog closes with `panel "<name>" was closed` (or `space ...`).

## Help

`Option+Space ?` shows every binding in a box, grouped (Session, Panels, Spaces, Terminal, Sidebar), from the binding table, so `[keys]` overrides show. The four focus keys share one line. In a window too short for the headings, the box lists the bindings without them. `Esc`, `q` or `?` close it.

## Configuration

```toml
sidebar = true # false hides it until toggled
sidebar_width = 28 # 20 to 60
sidebar_collapsed = false # true starts each client with the sidebar collapsed
sidebar_space_gap = 0 # 0 to 3 blank rows between spaces
```

A width outside 20 to 60 is an error naming `sidebar_width`. `sidebar_space_gap` puts that many blank rows between two spaces (after the previous space's last panel or its `no panels`), none before the first or after the last. A blank row draws only the sidebar background: it has no hover, no click and no context menu, `Up`/`Down` skip it, and it counts as a row for scrolling. A value outside 0 to 3 is an error naming `sidebar_space_gap`. Editing it reloads every attached client.
