# Keys

Keys go to the focused terminal, except the prefix and what follows it.

## The prefix

The default prefix is `Option+Space` (`M-Space`; shown as `⌥Space` on macOS). Press it, then a binding's key:

- While the prefix is armed, the [floating bar](#the-floating-bar) shows a `PREFIX` badge with `esc cancel  ? help` (`?` is the first key bound to `help`).
- A bound key runs its action and disarms.
- `Esc` disarms without doing anything.
- An unbound key disarms and shows `no binding for <key>` for 2 seconds; nothing reaches the terminal.
- Pressing the prefix twice sends the key as it arrived to the terminal (`ESC` and a space for `Option+Space`).

On macOS:

- Option must act as Alt in your terminal (iTerm2: Profiles > Keys > Left Option key: Esc+; Terminal.app: Use Option as Meta key). When it does not, the terminal sends a non-breaking space for `Option+Space`, and deck accepts that as the prefix too (only for `M-Space`). Pressed twice, it sends the non-breaking space.
- Launchers such as Raycast and Alfred may claim `Option+Space` before the terminal sees it: change the launcher's shortcut or set `prefix` here.
- `Ctrl+Space` is not the default because macOS reserves it for switching input sources.

**The kitty keyboard protocol.** A program that asks for it (Claude Code, Codex, Neovim and other editors push flags such as `CSI > 1 u`) gets it inside deck, whatever your terminal: `Shift+Enter`, `Ctrl+Enter`, `Ctrl+Tab`, `Ctrl+Backspace`, `Ctrl+I` versus `Tab`, key releases when it asks for them, and every key with every modifier, as in Ghostty or kitty. Keys that your terminal cannot tell apart still arrive as one key, so `Shift+Enter` and similar keys need an outer terminal with the protocol (Ghostty, kitty, WezTerm, iTerm2, foot, Alacritty); in any other terminal they reach the program as plain `Enter`. deck asks your terminal at start-up whether it has the protocol, turns it on while the focused terminal's program uses it, and turns it off when focus moves to another terminal, when a dialog or menu opens, and when deck exits. Limits: a program that asks for every key as an escape code (flag 8) gets no presses or releases of modifier keys on their own, the base-layout key and the lock modifiers (Caps Lock, Num Lock) are not sent, and a shifted symbol loses its base key (`Shift+1` is `CSI 33u`). Key bindings ignore Hyper and Meta but not Super. The prefix key works the same either way, and pressed twice it sends the key to the program in the program's encoding.

## The floating bar

No row of the window is reserved. A one-row floating bar shows over the last row of the panel area, centered, only while one of these holds, with the first that applies picking the badge:

| State                                        | Badge         | Then                                                                             |
| -------------------------------------------- | ------------- | -------------------------------------------------------------------------------- |
| Dragging the sidebar separator               | `SIDEBAR <w>` | nothing                                                                          |
| Dragging a space or panel row                | `MOVE`        | `esc cancel`                                                                     |
| The prefix is armed                          | `PREFIX`      | `esc cancel  ? help` (`?` is the first key of `help`; left out when unbound)     |
| A resize repeat or a terminal border drag    | `RESIZE`      | the repeat: `H J K L resize  = reset`; a drag: nothing                           |
| Scroll mode                                  | `SCROLL`      | `[offset/history]` when scrolled back, then `q exit  v select  / search  y copy` |
| Scroll mode with a selection                 | `SELECT`      | the selection's size (`3 lines`), then `y copy  o other end  Esc clear`          |
| The search prompt is open (`/` or `?`)       | none          | the query and its cursor, in place of everything else                            |
| Sidebar focus                                | `SIDEBAR`     | the sidebar hints                                                                |
| A message (a flash or a daemon notice) alone | none          | the text, in its level colour                                                    |

A message replaces the hints and keeps the badge. The badge is the `warning` role reversed, each hint is its key in bold and its label `muted`, on `status_bg`; hints that do not fit are cut with `…`. The bar is not drawn under a dialog, in a window too small for deck, or, apart from a message, over the help overlay, the session picker, the New menu or the restore dialog. It covers the terminal's cells while it shows; the cursor is hidden under it, and a press on it reaches no program.

## Bindings

Pressed after the prefix.

| Action               | Default     | What it does                                                                                        |
| -------------------- | ----------- | --------------------------------------------------------------------------------------------------- |
| `detach`             | `d`         | Detach from the session ([sessions](sessions.md))                                                   |
| `sessions`           | `s`         | Open the session picker ([sessions](sessions.md#switching-sessions))                                |
| `rename_session`     | `$`         | Rename the session you are in                                                                       |
| `restore`            | `R`         | Open the [restore dialog](agents.md#the-restore-dialog): resume an agent session in a new panel     |
| `scroll`             | `[`         | Enter [scroll mode](#scroll-mode) for the focused terminal                                          |
| `new_terminal`       | `t`         | New terminal in the active panel ([layout](layout.md))                                              |
| `close_terminal`     | `x`         | Close the focused terminal, after a confirmation                                                    |
| `focus_left`         | `Left` `h`  | Focus the terminal on the left                                                                      |
| `focus_right`        | `Right` `l` | Focus the terminal on the right                                                                     |
| `focus_up`           | `Up` `k`    | Focus the terminal above                                                                            |
| `focus_down`         | `Down` `j`  | Focus the terminal below                                                                            |
| `resize_left`        | `H`         | Resize the focused terminal to the left ([layout](layout.md#resizing))                              |
| `resize_right`       | `L`         | Resize the focused terminal to the right                                                            |
| `resize_up`          | `K`         | Resize the focused terminal upward                                                                  |
| `resize_down`        | `J`         | Resize the focused terminal downward                                                                |
| `reset_layout`       | `=`         | Reset the panel's terminal sizes                                                                    |
| `new_panel`          | `c`         | New panel in the active space ([sidebar](sidebar.md))                                               |
| `next_panel`         | `n`         | Focus the next panel                                                                                |
| `prev_panel`         | `p`         | Focus the previous panel                                                                            |
| `panel_1`..`panel_9` | `1`..`9`    | Focus the N-th panel of the active space                                                            |
| `rename_panel`       | `,`         | Rename the active panel                                                                             |
| `close_panel`        | `X`         | Close the active panel, after a confirmation                                                        |
| `new_space`          | `P`         | New space: asks a directory, then a name                                                            |
| `new_menu`           | `N`         | New panel menu for the active space: shell or agent ([sidebar](sidebar.md#the-new-button-and-menu)) |
| `rename_space`       | `.`         | Rename the active space                                                                             |
| `close_space`        | `Q`         | Close the active space, after a confirmation                                                        |
| `toggle_sidebar`     | `b`         | Show or hide the sidebar                                                                            |
| `collapse_sidebar`   | unbound     | Collapse or expand the sidebar ([collapsed sidebar](sidebar.md#collapsed-sidebar))                  |
| `focus_sidebar`      | `Tab`       | Move the keyboard into the sidebar                                                                  |
| `next_attention`     | `a`         | Jump to the next agent that needs you ([agents](agents.md#attention))                               |
| `help`               | `?`         | Show the help overlay                                                                               |

A key you set replaces the same key on any other action's default keys, with a warning (`key "c" of toggle_sidebar replaces new_panel's`). When you set two actions to one key, that is an error naming both actions and the key (`deck config check` shows it, and a running deck keeps the previous config). `detach = []` unbinds an action; the help overlay lists it as `unbound`.

## Help overlay

`Option+Space ?` lists every binding, and the keys of scroll mode in the Terminal group. When the window is too short to show all of them, `Up` / `k`, `Down` / `j`, `PageUp`, `PageDown`, `Home` and `End` scroll the list, and the bottom border says how many lines are hidden (`↓ 1 more · esc close`, and `↑ n more` once scrolled). `Esc`, `q` and `?` close it. These keys are fixed; they are not under `[keys]`.

## Restore dialog

`Option+Space R` (or `R` in sidebar focus, or the `Resume` button) opens the [restore dialog](agents.md#the-restore-dialog). Its keys are fixed, and the prefix key is ordinary text there:

| Keys                                            | What they do                                                                                                                    |
| ----------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| `Left` / `Right`, `Tab` / `Shift+Tab`           | Move between the columns (the arrows skip empty columns; `Tab` visits every column, empty or failed ones too, and wraps around) |
| `Up` / `Down`, `Ctrl+p` / `Ctrl+n`              | Move between the rows                                                                                                           |
| `PageUp` / `PageDown`, `Home` / `End`           | Move by a page, to the first or the last row                                                                                    |
| Printable keys, `Backspace`, `Ctrl+u`, `Ctrl+w` | Edit the filter, which applies to all three columns; `Ctrl+u` clears it, `Ctrl+w` deletes the word before the cursor            |
| `Enter`                                         | Resume the selected session in a new panel                                                                                      |
| `Esc`                                           | Clear the filter; with an empty filter, close the dialog                                                                        |

## Scroll mode

`Option+Space [` scrolls back through the focused terminal's history. The floating bar shows a `SCROLL` badge with the hints `q exit  v select  / search  y copy`, and `[offset/history]` follows the badge: how many lines the terminal is scrolled back out of how many it holds.

A **copy cursor** (a cell in the theme's `accent` colour) starts on the terminal's own cursor and moves with the vi keys below. When it moves past the top or bottom of the screen, the view scrolls by the rows it crossed.

| Keys                                       | What they do                                                                      |
| ------------------------------------------ | --------------------------------------------------------------------------------- |
| `h` `j` `k` `l`, the arrows                | Move the cursor; `k` and `j` scroll one line at the top and bottom edge           |
| `w` `b` `e`                                | Next word, previous word, end of the word (a word is what a double-click selects) |
| `0` `^` `$`                                | Start of the line, first non-blank, last non-blank                                |
| `H` `M` `L`                                | Top, middle and bottom of the screen                                              |
| `g` / `Home`, `G` / `End`                  | The top of the history, the bottom                                                |
| `PageUp` / `Ctrl+b`, `PageDown` / `Ctrl+f` | One page up or down, the cursor staying on its screen row                         |
| `Ctrl+u`, `Ctrl+d`                         | Half a page up or down, the cursor staying on its screen row                      |
| `q`                                        | Leave scroll mode, back at the bottom                                             |
| `Esc`                                      | Clear the selection; else clear the search highlights; else leave scroll mode     |

### Copy

| Keys         | What they do                                                                                                               |
| ------------ | -------------------------------------------------------------------------------------------------------------------------- |
| `v`, `V`     | Start a selection of characters or of whole lines at the cursor; the same key again clears it, the other switches the kind |
| `o`          | Move the cursor to the other end of the selection                                                                          |
| `y`, `Enter` | Copy the selection to your clipboard (OSC 52) and clear it; `y` without a selection copies the cursor's line               |

A selection can span any number of screens: it is kept by row, not by screen position, and the floating bar flashes `Copied 31 lines`. Each row's trailing blanks are dropped and rows are joined with newlines, except where a row continues on the next one because the program's line wrapped. The text is cut at 8 MiB (`Copied the first 8 MiB`).

### Search

| Keys    | What they do                                                                                        |
| ------- | --------------------------------------------------------------------------------------------------- |
| `/`     | Search down, toward newer rows: opens the query prompt in the floating bar (`/error█`)              |
| `?`     | Search up, toward older rows (`?error█`)                                                            |
| `Enter` | Search from the copy cursor; an empty query repeats the last one, in the direction of the key typed |
| `Esc`   | Close the prompt without searching; scroll mode stays                                               |
| `n`     | The next match in the direction of the last search                                                  |
| `N`     | The next match in the other direction                                                               |

The prompt edits like deck's other prompts (`Backspace`, `Delete`, `Left`, `Right`, `Home`, `End`, `Ctrl+u` for the whole text, `Ctrl+w` for the word before the cursor; a paste goes into it) and takes every key, the prefix included. The query is plain text, searched in the terminal's whole history and screen, a row at a time: a match does not span two rows, soft-wrapped ones included. The search is smart case: a query in lower case ignores case, one with a capital letter matches exactly. The first match after the copy cursor (before it, for `?` and `N` after `?`) is found; the copy cursor moves to its start and the view scrolls to it, unless it is on screen already. Past the last match the search wraps around, and the bar flashes `Search wrapped`; when there is no match it flashes `Not found: <query>`. `v` after a search selects from the match's start, and a selection started before a search extends to the match, so `v`, `e`, `y` after a search copies the matched word.

While a search is active, every occurrence on screen is drawn with the `search_match_fg` and `search_match_bg` roles (default black on yellow) and the match the last search found with `search_current_fg` and `search_current_bg` (default black on bright yellow, bold); a selection's colours win over both ([config](config.md#search-colours)). The highlights stay until scroll mode ends, or `Esc` clears them when no selection is on.

- No key reaches the terminal in scroll mode, and pastes are dropped, except into the search prompt. Other keys do nothing. Reaching the bottom does not leave the mode.
- `w`, `b` and `e` stop at the edge of the screen: from the last word on screen they move the cursor to the next row's first cell and scroll, and the next press continues from there.
- `k`, `j` and the arrows used to scroll at once; they now move the cursor and scroll at the edge. `Ctrl+u`, `Ctrl+d`, `Ctrl+b`, `Ctrl+f`, `PageUp` and `PageDown` still scroll at once.
- The prefix works in scroll mode: its bindings run, and `detach` leaves scroll mode first. Pressing the prefix twice sends it to the terminal and leaves scroll mode.
- Scroll mode ends when the focused terminal changes, and when another client types into the terminal or scrolls it back to the bottom.
- New output keeps the view where it is and the history count in the indicator grows.
- Typing into a scrolled-back terminal returns it to the bottom first. All clients of a session share a terminal's scroll position; when another client scrolls, your cursor keeps its row and is shown on the nearest edge row of the view while that row is off screen. The cursor and any selection are dropped when the terminal's rows are renumbered (a resize that reflows the text, the alternate screen, a reset).

## Mouse and focus

The wheel works over any terminal, focused or not, and never enters scroll mode:

- A program that asked for mouse reporting (vim, htop, lazygit, the agents' TUIs) gets the wheel.
- Otherwise, on the alternate screen (`less`, `man`), the wheel sends three `Up` or `Down` presses per notch (the program's own `?1007` setting; with it off the wheel does nothing there).
- Otherwise the wheel scrolls the history, 3 lines per notch, and shows the indicator. Without history it does nothing.

A click in an unfocused terminal focuses it and is not sent to the program ([layout](layout.md#focus)); the wheel scrolls the terminal under the pointer. Clicks, releases, drags and motion inside the focused terminal go to programs that asked for them, with coordinates relative to the terminal's screen (inside its border, if it has one; SGR encoding when the program asked for it, X10 otherwise). Events on the sidebar, its separator or the floating bar are never sent: a press on the bar reaches no program and starts no selection or drag, while a selection or drag already in progress goes on across it. When the outer terminal gains or loses focus, a program that asked for focus reporting gets `ESC[I` or `ESC[O` (a focus report does not move a scrolled-back terminal).

## Selecting and copying

Drag with the left button in the focused terminal to select text; releasing copies it to your clipboard (through OSC 52, as a program's copy does), and the floating bar says how much was copied (`Copied 12 characters`). A double-click selects a word and copies it, a triple-click the whole line; dragging after either extends the selection by words or by lines. The selection stays inside its terminal, even when the pointer moves into another one or out of the panel, and can be scrolled: it follows its rows when the view moves, and in scroll mode a drag past the top or bottom of the screen (or onto its first or last row, where the terminal touches the window's edge) scrolls the view one line every 50 ms and extends the selection. Each row's trailing blanks are dropped and rows are joined with newlines.

- When the program asked for mouse reporting (nvim, OpenCode, Claude Code), the mouse is the program's; hold `Option` (Alt) while pressing to select in deck instead. Your terminal's own selection (usually `Shift`-drag) still works as before.
- In scroll mode a press always selects, and `y`, `o` and `Esc` work on a mouse selection as on a keyboard one. After a mouse selection in scroll mode, cursor motions move its end. A selection with rows off screen is copied through the daemon, with the same text as from the screen.
- The selection is drawn in the theme's selection colours and clears at the next press, or when the program redraws a selected row, the terminal is resized or its rows are renumbered.

## Configuration

In `config.toml` (`~/.config/deck/config.toml`, or `$DECK_HOME/config.toml`), every key optional:

```toml
prefix = "M-Space"
sidebar = true
sidebar_width = 28
sidebar_collapsed = false

[keys]
detach = "d" # one key
# detach = ["d", "q"] # or a list
```

- A key is written `a`, `C-a` (Ctrl), `M-x` (Alt), or a name (`Space`, `Enter`, `Tab`, `Esc`, `Backspace`, `Insert`, `Delete`, `Up`, `Down`, `Left`, `Right`, `Home`, `End`, `PageUp`, `PageDown`, `F1`..`F12`) with optional `C-`, `M-` and `S-` before it.
- A character binding ignores `Shift`: `detach = "X"` matches the shifted `x`. `S-` is only for named keys (`S-Tab`).
- `C-` with an uppercase letter is refused; write the lowercase letter.
- An unknown action in `[keys]` is a warning; a bad key is an error naming the setting.
