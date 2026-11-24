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

Save the file and, within about a second, the colours, border style, key bindings, prefix, sidebar, sidebar width and sidebar space gap change in every attached client, with `config reloaded` in the floating bar. Editors that save through a temporary file and a rename work, and deleting the file returns everything to the defaults.

- Only what changed in the file changes on screen: a sidebar you hid with the `toggle_sidebar` key stays hidden unless `sidebar` itself changed. An armed prefix or an open prompt is kept.
- `shell`, `shell_args` and `scrollback_lines` apply to terminals started afterwards; running terminals are not touched.
- A mistake changes nothing: the floating bar shows `config.toml:<line>:<col>: <message> (kept the previous config)` for 6 seconds. If the file is still being written the daemon looks again before reporting. Warnings (an unknown key, a key that replaces another action's default) do not block the reload and show as one warning.
- A file that is invalid when the daemon starts means the defaults, with the same error shown to the first client that attaches. Messages with no client attached wait for the next one: the last 20 are kept, and a message identical to an earlier one replaces it, so many reloads show `config reloaded` once.
- Messages from the daemon show in the [floating bar](keys.md#the-floating-bar), coloured and timed by level (info `muted` 2 s, warning 4 s, error 6 s) and shown one after the other. A long one is cut to the width of the window; `deck config check` prints the whole text.

## Key conflicts

`[keys]` covers the prefix bindings only. A key you set for an action replaces the same key on another action's default keys, with a warning. A key you set for two actions is an error naming both actions and the key. `detach = []` unbinds an action. See [keys](keys.md). `restore = "R"` (the [restore dialog](agents.md#the-restore-dialog)) is one of them.

`notify` (`"osc9"`, the default, `"bell"` or `"none"`) is how the client tells you through the outer terminal that an agent needs you or finished while you were not looking; see [agents](agents.md#attention).

## Sidebar colours

`[theme] hover` (default `bright-black`, a role name such as `accent` is allowed) is the background of the sidebar row or chip under the pointer; muted text on it is drawn in `sidebar_fg`. The active panel's row and the selected row use the `selection_fg` and `selection_bg` roles.

## Search colours

The search in [scroll mode](keys.md#search) draws the matches on screen with `search_match_fg` and `search_match_bg` (default black on yellow) and the current match with `search_current_fg` and `search_current_bg` (default black on bright yellow, bold). A selection's colours win over a match's.

## Icons

deck ships its own icons and no key changes them. Every icon (the program icons, the space carets, the agent status marks and the unseen marker) comes in forms the client picks from what the outer terminal can show: the form depends on what the terminal says about itself at start-up, not on a list of names. **Images**: the terminal answers the kitty graphics query and draws Unicode placeholders (Ghostty, kitty 0.28 or later), with truecolor and outside tmux, Zellij and a deck panel; every icon is then a small image made from a [Hugeicons](https://hugeicons.com) free icon, and the loading spinner is a ring that turns once a second at 60 frames per second. **Nerd Font glyphs**: the terminal bundles the symbols (Ghostty, kitty 0.36 or later, WezTerm), or its profile font is a Nerd Font (iTerm2, Terminal.app), or a Nerd Font is installed where font fallback finds it (`fc-list`, or the font folders on macOS; iTerm2 and Terminal.app do not fall back, so their own profile font has to be a Nerd Font; with a non-ASCII font set in iTerm2, that one decides). **Unicode** glyphs every font has, otherwise. The font checks run just after start-up, so a late answer can switch the icons from Unicode to Nerd Font glyphs once, with a redraw. Claude Code's icon is always Claude's orange (`#d97757`); the other icons take the colour of their row, and the status marks and carets their theme roles. Which program icon a panel shows follows its agent, else its foreground program: `editor` for `vi`, `vim`, `nvim`, `hx`, `helix`, `nano`, `emacs`, `micro` and `kak`; `git` for `git`, `lazygit`, `tig` and `gitui`; `monitor` for `top`, `htop`, `btop` and `glances`; `remote` for `ssh`, `mosh` and `et`; `build` for `cargo`, `make`, `just`, `npm`, `pnpm`, `yarn` and `go`; `pager` for `less`, `more`, `man` and `bat`; `repl` for `python`, `python3`, `node`, `irb`, `ghci` and `iex`; `terminal` for shells and everything else. Every icon takes two cells, a glyph in the first and a blank second, so a layout is the same in every form.

The keys that used to change the icons are gone: `[theme] icons`, `[theme] icon_images`, `[theme.glyphs]`, `[theme.icon_glyphs]` and `[theme.icon_colors]`. A config that still has one loads, each gives one warning, `theme.icons was removed: deck ships its own icons` (with the key's own name), and nothing else changes.

`[agents.<kind>] process` lists the program names deck takes for an agent (`claude`, `codex`, `opencode`); `[agents.<kind>] command` is what a new panel for that agent runs (default `claude`, `codex`, `opencode`; see [starting an agent](agents.md#starting-an-agent-in-a-new-panel)); `[agents.<kind>] resume` and `resume_latest` are the commands that reopen its conversation after a restart, and `resume_agents` (default `true`) turns that off; see [agents](agents.md#resume).

## The reference

`crates/deck-core/src/config/default.toml` is the reference and is embedded in the binary. A test keeps it equal to the defaults and requires every key and action to appear in it, so each new key is documented there.
