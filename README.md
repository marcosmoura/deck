# deck

deck is a terminal multiplexer, like tmux or Zellij, made for working with coding agents such as Claude Code, Codex and OpenCode. It runs on macOS and Linux.

A sidebar lists your projects (**spaces**) and what runs in each (**panels**); a panel shows its terminals in a grid that deck arranges for you. Everything runs in a background daemon, so you can close the window or reboot and pick up where you left off.

- **Detach and come back** to everything still running, exactly as it was.
- **After a reboot**, your layout returns, shells reopen in their last directory and agents reopen on their conversation.
- **See your agents at a glance**: working, waiting for you, or done. One key jumps to the next one that needs you.
- **Agents can use deck too**: run tests in a terminal next to them, read its output, wait for it to finish.

## Install

```sh
brew install marcosmoura/tap/deck
```

Or, without Homebrew, into `~/.local/bin`:

```sh
curl -fsSL https://raw.githubusercontent.com/marcosmoura/deck/main/install.sh | sh
```

After installing a newer version, run `deck upgrade`: the daemon switches to it without stopping anything.

## Use

Run `deck`. It starts a session in the current directory, or brings back the one you used last.

Shortcuts start with `Option+Space` (`⌥Space`): press it, let go, then press the key. `⌥Space ?` lists them all.

| Keys                   | What they do                                   |
| ---------------------- | ---------------------------------------------- |
| `⌥Space d`             | Detach (everything keeps running)              |
| `⌥Space s`             | Switch sessions                                |
| `⌥Space t` / `x`       | Add a terminal / close it                      |
| `⌥Space h` `j` `k` `l` | Move between terminals                         |
| `⌥Space c` / `N`       | New panel with a shell / with a shell or agent |
| `⌥Space P`             | New space                                      |
| `⌥Space [`             | Scroll back; `/` searches, `y` copies          |
| `⌥Space a`             | Jump to the next agent that needs you          |

The mouse works too: click to focus, drag to resize or reorder, right-click for menus, select to copy.

On macOS, your terminal app must send Option as Alt, and launchers like Raycast or Alfred may take `⌥Space` first. Change either one, or pick another prefix in the config.

`deck ls`, `deck attach <name>`, `deck kill-session <name>` and `deck kill-server` manage sessions from the shell ([sessions](docs/features/sessions.md)).

## Agents

For accurate agent status in the sidebar, install deck's hooks once:

```sh
deck hooks install claude codex opencode
```

This edits the agents' config files (keeping a backup); `deck hooks uninstall` undoes it. See [agents](docs/features/agents.md#hooks).

To let an agent drive deck, link the [deck skill](skills/deck/SKILL.md), or have it read `deck guide`:

```sh
ln -s <repo>/skills/deck ~/.claude/skills/deck    # Claude Code and OpenCode
ln -s <repo>/skills/deck ~/.agents/skills/deck    # Codex
```

The commands it uses are in the [control API](docs/features/control-api.md). Note that any program running as your user can reach deck's socket, so anything inside deck can type into your other terminals.

## Configuration

Optional, in `~/.config/deck/config.toml`; most changes apply as soon as you save:

```toml
prefix = "C-a"

[theme]
accent = "#cba6f7"
```

`deck config default` prints every setting with comments; `deck config check` reports mistakes. See [config](docs/features/config.md) and [keys](docs/features/keys.md).

## More

- [docs/features](docs/features/README.md): how each feature works, in detail.
- From source: `mise install`, then `just install` (or `just run` to try it apart from your real sessions).
- Contributing: start with [AGENTS.md](AGENTS.md).

MIT licensed.
