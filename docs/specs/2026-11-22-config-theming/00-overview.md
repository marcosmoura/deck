---
branch: feat/config-theming
backlog: DECK-08
---

# Config and theming

## Goal

Users tweak deck through `~/.config/deck/config.toml`: key bindings (already there since DECK-01), colours and border style through a `[theme]` table, and every later setting. Changes apply live while deck runs. `deck config default` prints a commented file with every key, `deck config check` validates the user's file, and `deck config path` says which file is in use.

## Builds on

- [DECK-01 core-daemon-terminal](../2026-11-22-core-daemon-terminal/00-overview.md): `deck-core::config`, the config path ([architecture.md#paths](../../architecture.md#paths)), `[keys]`, and the `Theme` value with built-in defaults that the client draws with.
- [DECK-02 sessions-sidebar](../2026-11-22-sessions-sidebar/00-overview.md): the sidebar, status bar and overlays the theme colours, and the `sidebar`/`sidebar_width` keys.

## Non-goals

- Named built-in themes or separate theme files (a later task if wanted).
- Per-session or per-project config.
- Editing the config from inside deck.
- Watching files other than `config.toml` (no `include`).

## Decisions

- **One file**, at the path of [architecture.md#paths](../../architecture.md#paths): `$XDG_CONFIG_HOME/deck/config.toml`, default `~/.config/deck/config.toml` on macOS and Linux (`DECK_HOME` for tests). A missing file means all defaults.
- **The daemon is the only reader** from this spec on. It sends the parsed config to clients in `Welcome` and again after every reload, so a client never reads a different file than the daemon (replacing DECK-01's "the client reads it at attach").
- **Theme** is a `[theme]` table of semantic colour roles plus a border style and a few sidebar glyphs (see sub-spec 01). The defaults use the terminal's own ANSI palette, so the user's terminal theme applies until they override it.
- **Live reload**: the daemon checks the file's modification time and size every second (no file-watching crate). A valid new file applies at once: theme, keys, prefix, sidebar and notification settings change in every attached client; `shell`, `shell_args` and `scrollback_lines` apply to terminals started afterwards. An invalid file keeps the last good config and shows an error notice in the attached clients, naming the line.
- **One reference**: `crates/deck-core/src/config/default.toml` is the commented default config, embedded in the binary and printed by `deck config default`. A test keeps it equal to `Config::default()` and requires every key to appear in it, so each spec that adds a key documents it there.
- **First size**: from sub-spec 03, `Attach` carries the outer window size and the daemon derives the panel area from its config, since the client no longer reads the file.
- **Notices**: this spec introduces `ServerMsg::Notice { level, text }` (shown as a flash in the status bar, timed and coloured by level as sub-spec 03 says). DECK-04 reuses it for restore problems.

## Sub-specs

| Sub-spec                    | Type      | Crate                          | Summary                                                                 | Depends | Status |
| --------------------------- | --------- | ------------------------------ | ----------------------------------------------------------------------- | ------- | ------ |
| [01](01-theme.md)           | shippable | deck-core, deck-tui            | `[theme]`: colour roles, colour syntax, border style, glyphs; applied everywhere | DECK-02 | draft |
| [02](02-config-commands.md) | shippable | deck-core, deck                | `default.toml` reference, `deck config default`, `check`, `path`         | 01      | draft  |
| [03](03-live-reload.md)     | shippable | deck-daemon, deck-tui, deck-core | Daemon reads and watches the file, pushes config to clients; `Notice`   | 02      | draft  |

## Affected docs

- `docs/features/config.md` (new): where the file lives, live reload, `deck config` commands, and that `deck config default` is the key reference.
- `docs/architecture.md#config`: the daemon as the only reader, `default.toml` as the reference, live reload; `#protocol`: `Notice` and the config in `Welcome`.
- `README.md`: a "Configuration" section with a short example.

Reviewed by spec-reviewer on 22 Nov 2026.
