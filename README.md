# deck

A terminal multiplexer written in Rust, built for coding agents. A background daemon keeps your sessions running; `deck` opens a rounded frame around a shell, and you can detach and come back to it later.

Status: early. Today `deck` attaches to a session with one shell. Spaces, panels, the sidebar and agent support come with later work (see [docs/backlog.md](docs/backlog.md)).

## Install

macOS (Apple silicon or Intel) or Linux (x86_64 or aarch64), from the latest release into `~/.local/bin`:

```sh
curl -fsSL https://raw.githubusercontent.com/marcosmoura/deck/main/install.sh | sh
```

For a given version or another directory, set the variables on `sh`: `curl -fsSL … | DECK_VERSION=0.1.0 DECK_INSTALL_DIR=~/bin sh`.

From source, with the toolchain pinned in `.mise.toml` (`mise install`), into `~/.local/bin` (or `PREFIX=<dir> just install` for `<dir>/bin`):

```sh
just install
```

To try it without installing, with an isolated socket and state directory: `just run`.

## Use

```sh
deck                # attach to the most recent session, or start one called "main"
deck attach work    # attach to the session "work" (it must exist)
deck ls             # list the sessions
deck new work -d    # create a session without attaching
deck kill-session work
deck kill-server    # stop the daemon and every session
```

Press `Option+Space d` (`⌥Space d`) to detach: the shell keeps running in the daemon, and `deck` brings you back to it exactly as it was. On macOS, Option must act as Alt in your terminal for `Option+Space` (deck also accepts the non-breaking space some terminals send instead), and a launcher such as Raycast or Alfred may claim `Option+Space`: change either one, or set `prefix` in the config ([keys](docs/features/keys.md)).

## Configuration

deck reads `~/.config/deck/config.toml` (`$XDG_CONFIG_HOME/deck/config.toml` when that is set); every key is optional.

```sh
deck config default > ~/.config/deck/config.toml   # a commented file with every key
deck config check                                  # what is wrong in your file, with line and column
deck config path                                   # which file deck reads
```

```toml
prefix = "C-a"
sidebar_width = 32

[keys]
detach = ["d", "q"]

[theme]
accent = "#cba6f7"
border_style = "plain"
```

More: [sessions](docs/features/sessions.md), [keys](docs/features/keys.md), [config](docs/features/config.md).

## Agents

Inside deck, a coding agent can drive deck itself: run a test suite or a dev server in a visible terminal next to it, read that terminal's screen, type into it, wait for it to finish and organize panels. The commands are described in [control API](docs/features/control-api.md); the skill in [skills/deck](skills/deck/SKILL.md) teaches an agent how to use them, and it only applies when `DECK=1` is set in the environment.

Link the skill once. Claude Code (OpenCode reads `~/.claude/skills` too, so this one link serves both):

```sh
ln -s <repo>/skills/deck ~/.claude/skills/deck
```

Codex:

```sh
ln -s <repo>/skills/deck ~/.agents/skills/deck
```

Replace `<repo>` with the path of your clone. For an agent without skill support, `deck guide` prints the same text:

```sh
deck guide
```

To see what Claude Code, Codex and OpenCode are doing in the sidebar (working, waiting for you, done), install their hooks once:

```sh
deck hooks install claude codex opencode
deck hooks status
```

This edits `~/.claude/settings.json` and `~/.codex/hooks.json` (or `$CLAUDE_CONFIG_DIR` and `$CODEX_HOME`) after keeping a backup, and writes the plugin `~/.config/opencode/plugins/deck.js` (or under `$OPENCODE_CONFIG_DIR`). `deck hooks uninstall` removes only deck's entries and that plugin. See [agents](docs/features/agents.md#hooks).

The socket is open to every process of your user, so an agent can type into any of your terminals: it has the power of your shell. Agent sandboxes (Claude Code, Codex) may block the Unix socket. The agent then sees `no deck daemon running` or a permission error, and the sandbox has to allow the socket path (`$DECK_SOCKET` inside deck).
