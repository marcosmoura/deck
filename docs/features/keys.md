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

`Shift+Enter` arrives as `Enter`: deck does not use the kitty keyboard protocol.

## The floating bar

No row of the window is reserved. A one-row floating bar shows over the last row of the panel area, centered, only while one of these holds, with the first that applies picking the badge:

| State                                        | Badge         | Then                                                                         |
| -------------------------------------------- | ------------- | ---------------------------------------------------------------------------- |
| Dragging the sidebar separator               | `SIDEBAR <w>` | nothing                                                                      |
| The prefix is armed                          | `PREFIX`      | `esc cancel  ? help` (`?` is the first key of `help`; left out when unbound) |
| A resize repeat or a terminal border drag    | `RESIZE`      | the repeat: `H J K L resize  = reset`; a drag: nothing                       |
| Scroll mode                                  | `SCROLL`      | `[offset/history]` when scrolled back, then `q exit  g top  G bottom`        |
| Sidebar focus                                | `SIDEBAR`     | the sidebar hints                                                            |
| A message (a flash or a daemon notice) alone | none          | the text, in its level colour                                                |

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
| `focus_sidebar`      | `Tab`       | Move the keyboard into the sidebar                                                                  |
| `next_attention`     | `a`         | Jump to the next agent that needs you ([agents](agents.md#attention))                               |
| `help`               | `?`         | Show the help overlay                                                                               |

A key you set replaces the same key on any other action's default keys, with a warning (`key "c" of toggle_sidebar replaces new_panel's`). When you set two actions to one key, that is an error naming both actions and the key (`deck config check` shows it, and a running deck keeps the previous config). `detach = []` unbinds an action; the help overlay lists it as `unbound`.

## Help overlay

`Option+Space ?` lists every binding. When the window is too short to show all of them, `Up` / `k`, `Down` / `j`, `PageUp`, `PageDown`, `Home` and `End` scroll the list, and the bottom border says how many lines are hidden (`↓ 1 more · esc close`, and `↑ n more` once scrolled). `Esc`, `q` and `?` close it. These keys are fixed; they are not under `[keys]`.

## Restore dialog

`Option+Space R` (or `R` in sidebar focus, or the `Resume` button) opens the [restore dialog](agents.md#the-restore-dialog). Its keys are fixed, and the prefix key is ordinary text there:

| Keys                                  | What they do                                                                                                                    |
| ------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| `Left` / `Right`, `Tab` / `Shift+Tab` | Move between the columns (the arrows skip empty columns; `Tab` visits every column, empty or failed ones too, and wraps around) |
| `Up` / `Down`, `Ctrl+p` / `Ctrl+n`    | Move between the rows                                                                                                           |
| `PageUp` / `PageDown`, `Home` / `End` | Move by a page, to the first or the last row                                                                                    |
| Printable keys, `Backspace`, `Ctrl+u` | Edit the filter, which applies to all three columns; `Ctrl+u` clears it                                                         |
| `Enter`                               | Resume the selected session in a new panel                                                                                      |
| `Esc`                                 | Clear the filter; with an empty filter, close the dialog                                                                        |

## Scroll mode

`Option+Space [` scrolls back through the focused terminal's history. The floating bar shows a `SCROLL` badge with the hints `q exit  g top  G bottom`, and `[offset/history]` follows the badge: how many lines the terminal is scrolled back out of how many it holds.

| Keys                                       | What they do                          |
| ------------------------------------------ | ------------------------------------- |
| `Up` / `k`, `Down` / `j`                   | One line up or down                   |
| `PageUp` / `Ctrl+b`, `PageDown` / `Ctrl+f` | One page up or down                   |
| `Ctrl+u`, `Ctrl+d`                         | Half a page up or down                |
| `g` / `Home`, `G` / `End`                  | The top of the history, the bottom    |
| `q`, `Esc`                                 | Leave scroll mode, back at the bottom |

- No key reaches the terminal in scroll mode, and pastes are dropped. Other keys do nothing. Reaching the bottom does not leave the mode.
- The prefix works in scroll mode: its bindings run, and `detach` leaves scroll mode first. Pressing the prefix twice sends it to the terminal and leaves scroll mode.
- Scroll mode ends when the focused terminal changes, and when another client types into the terminal or scrolls it back to the bottom.
- New output keeps the view where it is and the history count in the indicator grows.
- Typing into a scrolled-back terminal returns it to the bottom first. All clients of a session share a terminal's scroll position.

## Mouse and focus

The wheel works over any terminal, focused or not, and never enters scroll mode:

- A program that asked for mouse reporting (vim, htop, lazygit, the agents' TUIs) gets the wheel.
- Otherwise, on the alternate screen (`less`, `man`), the wheel sends three `Up` or `Down` presses per notch (the program's own `?1007` setting; with it off the wheel does nothing there).
- Otherwise the wheel scrolls the history, 3 lines per notch, and shows the indicator. Without history it does nothing.

A click in an unfocused terminal focuses it and is not sent to the program ([layout](layout.md#focus)); the wheel scrolls the terminal under the pointer. Clicks, releases, drags and motion inside the focused terminal go to programs that asked for them, with coordinates relative to the terminal's screen (inside its border, if it has one; SGR encoding when the program asked for it, X10 otherwise). Events on the sidebar, its separator or the floating bar are never sent: a press on the bar reaches no program and starts no selection or drag, while a selection or drag already in progress goes on across it. When the outer terminal gains or loses focus, a program that asked for focus reporting gets `ESC[I` or `ESC[O` (a focus report does not move a scrolled-back terminal).

## Selecting and copying

Drag with the left button in the focused terminal to select text; releasing copies it to your clipboard (through OSC 52, as a program's copy does). A double-click selects a word and copies it, a triple-click the whole line; dragging after either extends the selection by words or by lines. The selection stays inside its terminal, even when the pointer moves into another one or out of the panel, and covers only the rows on screen (scroll back with the wheel first to copy older lines). Each row's trailing blanks are dropped and rows are joined with newlines.

- When the program asked for mouse reporting (nvim, OpenCode, Claude Code), the mouse is the program's; hold `Option` (Alt) while pressing to select in deck instead. Your terminal's own selection (usually `Shift`-drag) still works as before.
- In scroll mode a press always selects.
- The selection is drawn in the theme's selection colours and clears at the next press, or when the program redraws a selected row, the terminal is resized or it scrolls.

## Configuration

In `config.toml` (`~/.config/deck/config.toml`, or `$DECK_HOME/config.toml`), every key optional:

```toml
prefix = "M-Space"
sidebar = true
sidebar_width = 28

[keys]
detach = "d" # one key
# detach = ["d", "q"] # or a list
```

- A key is written `a`, `C-a` (Ctrl), `M-x` (Alt), or a name (`Space`, `Enter`, `Tab`, `Esc`, `Backspace`, `Insert`, `Delete`, `Up`, `Down`, `Left`, `Right`, `Home`, `End`, `PageUp`, `PageDown`, `F1`..`F12`) with optional `C-`, `M-` and `S-` before it.
- A character binding ignores `Shift`: `detach = "X"` matches the shifted `x`. `S-` is only for named keys (`S-Tab`).
- `C-` with an uppercase letter is refused; write the lowercase letter.
- An unknown action in `[keys]` is a warning; a bad key is an error naming the setting.
