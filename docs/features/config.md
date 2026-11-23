# Config

Read when you need where deck's settings live and how to check them.

deck reads one file, `~/.config/deck/config.toml` (`$XDG_CONFIG_HOME/deck/config.toml` when that variable is set; `$DECK_HOME/config.toml` for an isolated setup such as the tests). Every key is optional and a missing file means all defaults. The daemon is the only reader of the file; attached clients get the config from it. Changes apply while deck runs (see [Live reload](#live-reload)).

## Commands

None of them starts a daemon.

| Command                    | What it does                                                                                             |
| -------------------------- | -------------------------------------------------------------------------------------------------------- |
| `deck config default`      | Prints the commented default config, with every key: the key reference. Save it as your config to start. |
| `deck config check [file]` | Validates the file (the one in use without an argument). Prints `ok: <path>` or one line per finding.    |
| `deck config path`         | Prints the path in use, with ` (not found)` after it when the file does not exist.                       |

```sh
deck config default > ~/.config/deck/config.toml
```

`check` prints `<path>:<line>:<col>: error: <message>` or `… warning: …` per finding and exits 1 when there is at least one error; warnings alone exit 0. It reports TOML syntax errors, bad values (naming the key), unknown keys (warnings), theme errors and key conflicts. Keys before a syntax error are still checked. A missing file prints `no config file at <path>; using defaults` and exits 0.

`path` and a `check` without a file resolve the path from the current environment (`DECK_HOME`, `XDG_CONFIG_HOME`, the default). A running daemon reads the file of the environment it started with, which differs when those variables differ.

## Live reload

Save the file and, within about a second, the colours, border style, glyphs, key bindings, prefix, sidebar, sidebar width and sidebar space gap change in every attached client, with `config reloaded` in the floating bar. Editors that save through a temporary file and a rename work, and deleting the file returns everything to the defaults.

- Only what changed in the file changes on screen: a sidebar you hid with the `toggle_sidebar` key stays hidden unless `sidebar` itself changed. An armed prefix or an open prompt is kept.
- `shell`, `shell_args` and `scrollback_lines` apply to terminals started afterwards; running terminals are not touched.
- A mistake changes nothing: the floating bar shows `config.toml:<line>:<col>: <message> (kept the previous config)` for 6 seconds. If the file is still being written the daemon looks again before reporting. Warnings (an unknown key, a key that replaces another action's default) do not block the reload and show as one warning.
- A file that is invalid when the daemon starts means the defaults, with the same error shown to the first client that attaches. Messages with no client attached wait for the next one: the last 20 are kept, and a message identical to an earlier one replaces it, so many reloads show `config reloaded` once.
- Messages from the daemon show in the [floating bar](keys.md#the-floating-bar), coloured and timed by level (info `muted` 2 s, warning 4 s, error 6 s) and shown one after the other. A long one is cut to the width of the window; `deck config check` prints the whole text.

## Key conflicts

`[keys]` covers the prefix bindings only. A key you set for an action replaces the same key on another action's default keys, with a warning. A key you set for two actions is an error naming both actions and the key. `detach = []` unbinds an action. See [keys](keys.md). `restore = "R"` (the [restore dialog](agents.md#the-restore-dialog)) is one of them.

`notify` (`"osc9"`, the default, `"bell"` or `"none"`) is how the client tells you through the outer terminal that an agent needs you or finished while you were not looking; see [agents](agents.md#attention). `[theme.glyphs] unseen` is the marker of such a panel.

## Sidebar colours

`[theme] hover` (default `bright-black`, a role name such as `accent` is allowed) is the background of the sidebar row or chip under the pointer; muted text on it is drawn in `sidebar_fg`. The active panel's row and the selected row use the `selection_fg` and `selection_bg` roles. `[theme.glyphs] active` is gone (the active row is a band, not a marker); a config that still sets it gets an unknown-key warning.

## Icons

`[theme] icons` picks the icons in front of each panel's name in the sidebar: `"unicode"` (the default) uses plain Unicode glyphs that every font has, `"nerd"` uses [Nerd Font](https://www.nerdfonts.com) glyphs (3.5 or later: the Claude and OpenAI logos for Claude Code and Codex, `▣` for OpenCode, which has no Nerd Font glyph), `"none"` draws no icons. `[theme.icon_glyphs]` sets the glyph of single kinds (`claude`, `codex`, `opencode`, `terminal`, `editor`, `git`, `monitor`, `remote`, `build`, `pager`, `repl`), each one cell wide; a wider glyph or an unknown kind is an error naming the key, and with `icons = "none"` the table is checked but not drawn. `deck config default` lists every glyph. Which kind a panel shows follows its agent, else its foreground program: `editor` for `vi`, `vim`, `nvim`, `hx`, `helix`, `nano`, `emacs`, `micro` and `kak`; `git` for `git`, `lazygit`, `tig` and `gitui`; `monitor` for `top`, `htop`, `btop` and `glances`; `remote` for `ssh`, `mosh` and `et`; `build` for `cargo`, `make`, `just`, `npm`, `pnpm`, `yarn` and `go`; `pager` for `less`, `more`, `man` and `bat`; `repl` for `python`, `python3`, `node`, `irb`, `ghci` and `iex`; `terminal` for shells and everything else. The table is fixed. Some terminals draw Nerd Font glyphs two cells wide; the space after the icon keeps the name readable.

`[theme.icon_colors]` sets the colour of the agent icons (`claude`, `codex`, `opencode`), text or logo, in the sidebar, the New menu and the restore dialog. Each takes the colour syntax of the roles (a role name included) or `"none"`, which draws the icon in its row's colour. The default is Claude's orange, `claude = "#d97757"`; Codex and OpenCode have no colour of their own (their logos are black and white) and default to `none`. The colour stays on the selection band and the active row. Any other key, or a bad value, is an error naming the key.

`[theme] icon_images` is `"auto"` (the default) or `"off"`. With `auto`, in Ghostty and kitty (and nowhere else) the icons of Claude Code, Codex and OpenCode are drawn as the agents' logos, in the colour the text icon would have had (its `[theme.icon_colors]` colour, else the row's); the other icons stay text. Inside tmux, Zellij or a deck panel, with 256 colours (no `COLORTERM=truecolor`), and with `icons = "none"` the text icons are drawn. `"off"` always draws text icons. Any other value is an error naming `theme.icon_images`. A change takes effect from the next frame; images already sent stay in the terminal until deck exits.

`[agents.<kind>] process` lists the program names deck takes for an agent (`claude`, `codex`, `opencode`); `[agents.<kind>] command` is what a new panel for that agent runs (default `claude`, `codex`, `opencode`; see [starting an agent](agents.md#starting-an-agent-in-a-new-panel)); `[agents.<kind>] resume` and `resume_latest` are the commands that reopen its conversation after a restart, and `resume_agents` (default `true`) turns that off; see [agents](agents.md#resume).

## The reference

`crates/deck-core/src/config/default.toml` is the reference and is embedded in the binary. A test keeps it equal to the defaults and requires every key and action to appear in it, so each new key is documented there.
