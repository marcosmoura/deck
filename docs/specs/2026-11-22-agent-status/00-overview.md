---
branch: feat/agent-status
backlog: DECK-06
---

# Agent status, hooks and resume

## Goal

The sidebar shows what every coding agent (Claude Code, Codex, OpenCode) is doing: working, waiting for the user, or done. deck tells the user when an agent needs them, jumps to the next one waiting, and after a daemon restart reopens each agent's conversation where it left off.

## Builds on

- [DECK-01 core-daemon-terminal](../2026-11-22-core-daemon-terminal/00-overview.md): focus events from the outer terminal (sub-spec 06), the harness's `raw_output()` (sub-spec 04) and the daemon's `platform` module (sub-spec 03).
- [DECK-02 sessions-sidebar](../2026-11-22-sessions-sidebar/00-overview.md): the sidebar's reserved status column.
- [DECK-04 persistence](../2026-11-22-persistence/00-overview.md): the `shutting_down` flag, state fields and restore (used by sub-spec 05 only). Detection has its own 1 s tick and does not use DECK-04's cwd tick.
- [DECK-05 control-api](../2026-11-22-control-api/00-overview.md): requests from processes inside deck, `deck ls --json`, the `DECK_*` targeting.
- [DECK-08 config-theming](../2026-11-22-config-theming/00-overview.md): the status glyphs (spinner frames, waiting, idle, quiet) and the unseen marker (`•`) are added to `[theme.glyphs]`, their colours use the theme roles, and every new key (`notify`, `[agents.*]`, `resume_agents`, `next_attention`) is documented in `default.toml`. DECK-08 owns `ServerMsg::Notice`; this spec's agent alert is a separate `ServerMsg::AgentAlert`.
- [architecture.md#agents](../../architecture.md#agents).

## Non-goals

- Launch presets or a menu to start agents (not requested). Agents are started by typing their command or through the control API.
- A git worktree per agent.
- Reading or showing the agents' conversations, costs or token counts.
- Panel auto-naming: that is [DECK-07](../../backlog.md#deck-07), which uses the title and first prompt this spec records.

## Decisions

- **Two sources.** Hooks are the truth: each agent's hooks run `deck agent-event`, which reports precise states. Without hooks, deck still recognises an agent by the PTY's foreground process (checked every second; wrapper scripts and `node`/`bun`/`deno` launchers included) and shows only whether it is printing (`active`) or not (`quiet`).
- **Answered prompts**: typing into a terminal whose agent is `waiting` sets it back to `working`, since neither Claude Code nor Codex fires a hook between a permission request and the next tool result.
- **States** per terminal: `none`, `working`, `waiting` (needs the user: a permission prompt or a question), `idle` (finished its turn, ready for the next prompt), and for process-only detection `active` and `quiet`. A panel shows its most urgent terminal: waiting > working > active > idle > quiet > none.
- **Glyphs** in the sidebar status column and the terminal title: working and active: an animated braille spinner (accent; active is muted); waiting: `!` (warning, bold); idle: `●` (ok colour); quiet: `○` (muted).
- **Attention** comes from hook-sourced states only (process-only activity pauses all the time and would flap). When a terminal goes from `working` to `waiting` or `idle` while the user is not looking at it (its panel is not the active panel of any client whose outer terminal has focus), its panel row is marked unseen (bold name) until the panel is focused, and the daemon sends one `ServerMsg::AgentAlert` to the session's most recently active client, which sends a desktop notification through the outer terminal: `notify = "osc9"` (default; OSC 9, understood by iTerm2, Ghostty and WezTerm, ignored by terminals that do not know it; kitty uses OSC 99 and is not targeted), `"bell"` or `"none"`. Inside tmux or over ssh, OSC 9 reaches the desktop only if those pass it through; the docs say so. A session with no attached client alerts nobody, and the session picker shows no agent status (limitations).
- **Jump**: `prefix a` focuses the next terminal in `waiting`, then `idle` and unseen, in sidebar order after the current one.
- **Hook install** writes only the user's own agent config files (honouring `CLAUDE_CONFIG_DIR` and `CODEX_HOME`), only when the user runs `deck hooks install`, follows symlinks to the real file, keeps a backup under `$XDG_STATE_HOME/deck/backups/`, and is idempotent. `deck agent-event` outside deck does nothing and exits 0, so global hooks are harmless.
- **OpenCode logic lives in Rust**: the OpenCode plugin only forwards raw events; a pure per-terminal state machine in `deck-core::agents`, held by the daemon, filters subagent sessions and finds the first user prompt, so it is unit-tested in Rust and no JavaScript test runner is needed.
- **New dependency feature**: `serde_json` with `preserve_order`, so rewriting an agent's JSON config keeps its key order (sub-spec 03). It brings in `indexmap` as a new transitive crate (`toml` 1.2 does not pull it in by default); approved by this spec.
- **Resume**: a shell terminal whose foreground process was an agent when the state was saved is relaunched as `/bin/sh -c '<resume command>; exec "$0" "$@"' <shell> <shell_args…>`: the agent reopens its conversation, quitting it leaves the user's shell with its configured arguments, and nothing is typed into a shell or its history. Resume commands are configurable per agent and validated so no shell syntax or untrusted id reaches that command line. Once the daemon is shutting down it ignores agent events and detection changes, so the info resume needs survives the shutdown.

## Sub-specs

| Sub-spec                      | Type      | Crate                            | Summary                                                                       | Depends               | Status |
| ----------------------------- | --------- | -------------------------------- | ----------------------------------------------------------------------------- | --------------------- | ------ |
| [01](01-detection-state.md)   | shippable | deck-daemon, deck-tui, deck-core | Process detection on a 1 s tick, state model, `AgentEvent`, glyphs, `ls --json` | DECK-05               | todo   |
| [02](02-attention.md)         | shippable | deck-daemon, deck-tui, deck-core | Unseen panels, `AgentAlert`, notifications, `ClientMsg::Focus`, waiting count, `prefix a` | 01           | todo   |
| [03](03-hooks.md)             | shippable | deck, deck-core                  | `deck agent-event`, mapping and `deck hooks` for Claude Code and Codex         | 01                    | todo   |
| [04](04-opencode.md)          | shippable | deck-core, deck-daemon, deck     | OpenCode plugin forwarding raw events; the OpenCode state machine              | 03                    | todo   |
| [05](05-resume.md)            | shippable | deck-daemon, deck-core           | Save agent kind and session id; relaunch with the resume command               | 03, DECK-04           | todo   |

02 and 03 both build on 01 only, but share `deck-core::agents` and the end-to-end tests, so they run one after the other.

## Affected docs

- `docs/features/agents.md` (new): states, glyphs, attention, notifications, hooks, resume, config keys.
- `docs/features/keys.md`: `prefix a`.
- `docs/features/control-api.md`: the `agent` field in `ls --json`, `agent-event`.
- `README.md`: `deck hooks install`.

Reviewed by spec-reviewer on 22 Nov 2026.
