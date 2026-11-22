# Keys

Keys go to the focused terminal, except the prefix and what follows it.

## The prefix

The default prefix is `Option+Space` (`M-Space`; shown as `⌥Space` on macOS). Press it, then a binding's key:

- While the prefix is armed, the status bar shows a `PREFIX` badge.
- A bound key runs its action and disarms.
- `Esc` disarms without doing anything.
- An unbound key disarms and shows `no binding for <key>` for 2 seconds; nothing reaches the terminal.
- Pressing the prefix twice sends the key as it arrived to the terminal (`ESC` and a space for `Option+Space`).

On macOS:

- Option must act as Alt in your terminal (iTerm2: Profiles > Keys > Left Option key: Esc+; Terminal.app: Use Option as Meta key). When it does not, the terminal sends a non-breaking space for `Option+Space`, and deck accepts that as the prefix too (only for `M-Space`). Pressed twice, it sends the non-breaking space.
- Launchers such as Raycast and Alfred may claim `Option+Space` before the terminal sees it: change the launcher's shortcut or set `prefix` here.
- `Ctrl+Space` is not the default because macOS reserves it for switching input sources.

`Shift+Enter` arrives as `Enter`: deck does not use the kitty keyboard protocol.

## Bindings

Pressed after the prefix.

| Action           | Default   | What it does                                                          |
| ---------------- | --------- | --------------------------------------------------------------------- |
| `detach`         | `d`       | Detach from the session ([sessions](sessions.md))                     |
| `sessions`       | `s`       | Open the session picker ([sessions](sessions.md#switching-sessions))  |
| `rename_session` | `$`       | Rename the session you are in                                         |
| `scroll`         | `[`       | Enter [scroll mode](#scroll-mode) for the focused terminal            |
| `new_panel`      | `c`       | New panel in the active project ([sidebar](sidebar.md))               |
| `next_panel`     | `n`       | Focus the next panel                                                  |
| `prev_panel`     | `p`       | Focus the previous panel                                              |
| `panel_1`..`panel_9` | `1`..`9` | Focus the N-th panel of the active project                          |
| `rename_panel`   | `,`       | Rename the active panel                                               |
| `close_panel`    | `X`       | Close the active panel, after a confirmation                          |
| `new_project`    | `P`       | New project: asks a directory, then a name                            |
| `rename_project` | `.`       | Rename the active project                                            |
| `close_project`  | `Q`       | Close the active project, after a confirmation                        |
| `toggle_sidebar` | `b`       | Show or hide the sidebar                                              |
| `focus_sidebar`  | `Tab`     | Move the keyboard into the sidebar                                    |
| `help`           | `?`       | Show the help overlay                                                 |

A key you set replaces the same key on any other action's default keys, with a warning (`key "c" of toggle_sidebar replaces new_panel's`). When you set two actions to one key, the one that sorts first by name wins, with a warning.

## Scroll mode

`Option+Space [` scrolls back through the focused terminal's history. The status bar shows a `SCROLL` badge instead of the session name, with the hints `q exit  g top  G bottom`, and `[offset/history]` follows the badge: how many lines the terminal is scrolled back out of how many it holds.

| Keys                       | What they do                         |
| -------------------------- | ------------------------------------ |
| `Up` / `k`, `Down` / `j`   | One line up or down                  |
| `PageUp` / `Ctrl+b`, `PageDown` / `Ctrl+f` | One page up or down  |
| `Ctrl+u`, `Ctrl+d`         | Half a page up or down               |
| `g` / `Home`, `G` / `End`  | The top of the history, the bottom   |
| `q`, `Esc`                 | Leave scroll mode, back at the bottom |

- No key reaches the terminal in scroll mode, and pastes are dropped. Other keys do nothing. Reaching the bottom does not leave the mode.
- The prefix works in scroll mode: its bindings run, and `detach` leaves scroll mode first. Pressing the prefix twice sends it to the terminal and leaves scroll mode.
- Scroll mode ends when the focused terminal changes, and when another client types into the terminal or scrolls it back to the bottom.
- New output keeps the view where it is and the history count in the indicator grows.
- Typing into a scrolled-back terminal returns it to the bottom first. All clients of a session share a terminal's scroll position.

## Mouse and focus

The wheel works over any terminal and never enters scroll mode:

- A program that asked for mouse reporting (vim, htop, lazygit, the agents' TUIs) gets the wheel.
- Otherwise, on the alternate screen (`less`, `man`), the wheel sends three `Up` or `Down` presses per notch (the program's own `?1007` setting; with it off the wheel does nothing there).
- Otherwise the wheel scrolls the history, 3 lines per notch, and shows the indicator. Without history it does nothing.

Clicks, releases, drags and motion inside a terminal go to programs that asked for them, with coordinates relative to the terminal, which fills the panel area (SGR encoding when the program asked for it, X10 otherwise). Events on the sidebar, its separator or the status bar are never sent. When the outer terminal gains or loses focus, a program that asked for focus reporting gets `ESC[I` or `ESC[O` (a focus report does not move a scrolled-back terminal).

## Configuration

In `config.toml` (`~/.config/deck/config.toml`, or `$DECK_HOME/config.toml`), every key optional:

```toml
prefix = "M-Space"
sidebar = true
sidebar_width = 28

[keys]
detach = "d"          # one key
# detach = ["d", "q"] # or a list
```

- A key is written `a`, `C-a` (Ctrl), `M-x` (Alt), or a name (`Space`, `Enter`, `Tab`, `Esc`, `Backspace`, `Insert`, `Delete`, `Up`, `Down`, `Left`, `Right`, `Home`, `End`, `PageUp`, `PageDown`, `F1`..`F12`) with optional `C-`, `M-` and `S-` before it.
- A character binding ignores `Shift`: `detach = "X"` matches the shifted `x`. `S-` is only for named keys (`S-Tab`).
- `C-` with an uppercase letter is refused; write the lowercase letter.
- An unknown action in `[keys]` is a warning; a bad key is an error naming the setting.
