---
branch: feat/sidebar-titles
backlog: DECK-09
---

# Sidebar titles with icons

## Goal

Every panel row in the sidebar shows what is running in it: an icon for the program and a live title. A coding agent (Claude Code, Codex, OpenCode) shows its own icon followed by what it is doing (`✻ Fix login bug`), never its bare product name. A shell shows a terminal icon and the title the shell sets, a known program (an editor, git, a monitor…) a fitting icon, and anything else falls back to the terminal icon. A panel the user renamed keeps its name, with the live icon.

This spec takes over [DECK-07](../../backlog.md#deck-07) (panels named after what their agent is doing): its rules for agents are requirement 2 of sub-spec 01, extended to every program.

## Builds on

- [DECK-02 sessions-sidebar](../2026-11-22-sessions-sidebar/00-overview.md): the sidebar rows, `NameSource`, the status-bar title of a single-terminal panel.
- [DECK-03 grid-layout](../2026-11-22-grid-layout/00-overview.md): terminals in grid order, the focused terminal of a panel.
- [DECK-05 control-api](../2026-11-22-control-api/00-overview.md): `deck rename-panel`, `deck ls --json`.
- [DECK-06 agent-status](../2026-11-22-agent-status/00-overview.md): `platform::foreground_process` and the 1 s detection tick, `match_argv`, `AgentState` with `first_prompt`, the sidebar status column.
- [DECK-08 config-theming](../2026-11-22-config-theming/00-overview.md): `[theme.glyphs]` and the one-cell glyph rule, `default.toml` and its sync test.

## Non-goals

- User-defined program-to-icon rules (a fixed table in v1; only the glyph of each icon is configurable).
- Icons in terminal borders of multi-terminal panels (DECK-03 titles stay `1 · nvim`) and in the session picker.
- Brand logos drawn as images; icons are text glyphs.

## Decisions

- **One name, kept on the panel.** A panel whose `name_source` is not `manual` gets its name from its _source terminal_; the daemon writes it into `Panel.name` with `name_source = auto`, so persistence (DECK-04), `ls --json` and every client see the same name. A manual rename (`prefix ,`, `deck rename-panel`) locks it; `deck rename-panel --auto` unlocks it.
- **Source terminal**: the panel's first terminal in grid order that has an agent; otherwise its focused terminal.
- **Program**: the daemon records each terminal's foreground program name from DECK-06's detection tick (`program_name(argv)`, shared with `match_argv`: login dash removed, an interpreter unwrapped only when its first argument is not an option, extension removed), keeping the last value when the process cannot be read, so the client can choose an icon without knowing processes.
- **Icons** are a fixed set of kinds (`claude`, `codex`, `opencode`, `terminal`, `editor`, `git`, `monitor`, `remote`, `build`, `pager`, `repl`) chosen by a pure table in `deck-core`; the glyph of each kind lives in `[theme.icons]`, one cell wide like every theme glyph. `[theme] icons = "unicode"` (default) uses plain Unicode glyphs that every font has; `icons = "nerd"` uses Nerd Font glyphs; `icons = "none"` draws no icons.
- **Status bar** shows the same icon and name as the sidebar row; DECK-02's ` · <title>` suffix stays only for a manually named panel, where the name no longer follows the title.
- **Changed from DECK-07**: when an agent exits, the panel's name follows the program that is left (usually the shell), instead of keeping the agent's last name.

## Sub-specs

| Sub-spec                | Type      | Crate                        | Summary                                                                             | Depends | Status |
| ----------------------- | --------- | ---------------------------- | ----------------------------------------------------------------------------------- | ------- | ------ |
| [01](01-auto-titles.md) | shippable | deck-core, deck-daemon, deck | Foreground program per terminal; panels named after their source terminal; `--auto` | DECK-06 | todo   |
| [02](02-icons.md)       | shippable | deck-core, deck-tui          | Icon kinds, `[theme.icons]` and `icons`, icons in the sidebar and status bar        | 01      | todo   |

## Affected docs

- `docs/features/sidebar.md`: panel names that follow their program, icons.
- `docs/features/agents.md` (created by DECK-06): agent titles in the sidebar.
- `docs/features/control-api.md`: `rename-panel --auto`, `program` in `ls --json`.
- `docs/features/config.md` or wherever DECK-08 documents `[theme]`: `icons` and `[theme.icons]`.
- `docs/architecture.md#model`: `NameSource::Auto` and the naming rule.
- `docs/backlog.md`: DECK-07 withdrawn in favour of this spec (done by the lead when the user approves).

Reviewed by spec-reviewer on 22 Nov 2026.
