# Config

Read when you need where deck's settings live and how to check them.

deck reads one file, `~/.config/deck/config.toml` (`$XDG_CONFIG_HOME/deck/config.toml` when that variable is set; `$DECK_HOME/config.toml` for an isolated setup such as the tests). Every key is optional and a missing file means all defaults. The daemon is the only reader of the file; attached clients get the config from it. Changes apply while deck runs (see [Live reload](#live-reload)).

## Commands

None of them starts a daemon.

| Command                    | What it does                                                                                           |
| -------------------------- | ------------------------------------------------------------------------------------------------------ |
| `deck config default`      | Prints the commented default config, with every key: the key reference. Save it as your config to start. |
| `deck config check [file]` | Validates the file (the one in use without an argument). Prints `ok: <path>` or one line per finding.   |
| `deck config path`         | Prints the path in use, with ` (not found)` after it when the file does not exist.                      |

```sh
deck config default > ~/.config/deck/config.toml
```

`check` prints `<path>:<line>:<col>: error: <message>` or `… warning: …` per finding and exits 1 when there is at least one error; warnings alone exit 0. It reports TOML syntax errors, bad values (naming the key), unknown keys (warnings), theme errors and key conflicts. Keys before a syntax error are still checked. A missing file prints `no config file at <path>; using defaults` and exits 0.

`path` and a `check` without a file resolve the path from the current environment (`DECK_HOME`, `XDG_CONFIG_HOME`, the default). A running daemon reads the file of the environment it started with, which differs when those variables differ.

## Live reload

Save the file and, within about a second, the colours, border style, glyphs, key bindings, prefix, sidebar and sidebar width change in every attached client, with `config reloaded` in the status bar. Editors that save through a temporary file and a rename work, and deleting the file returns everything to the defaults.

- Only what changed in the file changes on screen: a sidebar you hid with the `toggle_sidebar` key stays hidden unless `sidebar` itself changed. An armed prefix or an open prompt is kept.
- `shell`, `shell_args` and `scrollback_lines` apply to terminals started afterwards; running terminals are not touched.
- A mistake changes nothing: the status bar shows `config.toml:<line>:<col>: <message> (kept the previous config)` for 6 seconds. If the file is still being written the daemon looks again before reporting. Warnings (an unknown key, a key that replaces another action's default) do not block the reload and show as one warning.
- A file that is invalid when the daemon starts means the defaults, with the same error shown to the first client that attaches. Messages with no client attached wait for the next one: the last 20 are kept, and a message identical to an earlier one replaces it, so many reloads show `config reloaded` once.
- Status bar messages from the daemon are coloured and timed by level (info `muted` 2 s, warning 4 s, error 6 s) and shown one after the other. A long one is cut to the width of the window; `deck config check` prints the whole text.

## Key conflicts

`[keys]` covers the prefix bindings only. A key you set for an action replaces the same key on another action's default keys, with a warning. A key you set for two actions is an error naming both actions and the key. `detach = []` unbinds an action. See [keys](keys.md).

## The reference

`crates/deck-core/src/config/default.toml` is the reference and is embedded in the binary. A test keeps it equal to the defaults and requires every key and action to appear in it, so each new key is documented there.
