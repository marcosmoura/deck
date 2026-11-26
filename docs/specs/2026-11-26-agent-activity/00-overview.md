---
branch: feat/agent-activity
backlog: DECK-50
---

# Agent activity in the sidebar

## Goal

See at a glance what every agent is doing, without opening its panel. Each sidebar row of a panel or terminal with an agent gets a second, muted line that says what the agent does now (`running cargo test`, `editing sidebar.rs`, `dispatching worker: implement sub-spec 02`, `worker: reading sidebar.rs`), what it needs (`needs permission: Bash`, `asking: Repo flow`) or how it finished (the first line of its last message, or `waiting for 1 background task`). The line follows the agent in real time, from the hook events it already sends.

Clicking a desktop notification of deck (`needs you`, `finished`) brings you to the panel it is about.

## Builds on

- Agent status, hooks and resume ([agent-status](../2026-11-22-agent-status/00-overview.md), DECK-06): `AgentState`, `Request::AgentEvent`, `deck agent-event`, `deck hooks install`, attention and `AgentAlert`.
- OpenCode v2 hooks (DECK-13): the TUI plugin and `OpenCodeTracker`.
- Sidebar titles (DECK-09), sidebar mouse (DECK-10), terminals in the sidebar (DECK-27) and the collapsed sidebar (DECK-48): the row layout this spec extends.
- Docs: [agents](../../features/agents.md), [sidebar](../../features/sidebar.md), [privacy](../../features/privacy.md).

## Non-goals

- A model-written summary of intent ("analyzing requirements"). The line describes the agent's actions from its hook events; no model is called and no new dependency is added.
- Changing the panel's name. It already follows the agent's title (Claude Code titles its session after the task).
- New statuses, or a change to the urgency order, the status slot, alerts' wording or when alerts fire.
- The activity in grid border titles, the collapsed sidebar, the session picker or `state.json`.
- Activity for agents found only by their process (no hooks): their line shows the status word.
- Notifications that report a click (OSC 99): sub-spec 04 only records which terminals support it.

## Decisions

- **Source: hook events only.** Claude Code and Codex `PreToolUse` (newly installed), `PostToolUse`, `PermissionRequest`, `Notification`, `Stop`, `UserPromptSubmit`, `SessionStart`; OpenCode's tool and permission events through its plugin. The user chose this over a model summary (real time, no cost, nothing leaves the machine).
- **Only the line reaches the daemon.** `deck agent-event` turns a Claude Code or Codex payload into one line of at most 80 characters with the pure `deck-core::agents::describe_hook`. For OpenCode, which forwards raw events to the daemon's `OpenCodeTracker`, `deck agent-event` first reduces a tool event to the few fields the tracker needs (tool name, description, first line of a command, file name, the first line of a text) with the pure `reduce_opencode_event`. No tool input, file content or whole message reaches the daemon. `AgentState.activity: Option<String>` carries the line to clients in `SessionView` and to `deck ls --json`. It is never saved, logged or put in a notification.
- **Wire shape.** `Request::AgentEvent` gains `activity: Activity` (`Keep`, `Clear`, `Set(String)`, defined in `deck-core::agents`) and `at_ms: u64` (when `deck agent-event` started, Unix milliseconds). The protocol is postcard, so the three cases are an enum, not optional fields.
- **Order.** Claude Code runs its hooks asynchronously, so events can arrive out of order. The daemon keeps, per terminal, the `at_ms` of the last activity it applied and ignores the activity of an older event (its status is applied as today).
- **Subagents.** A Claude Code tool event from a subagent (its payload has `agent_type`) sets `<agent_type>: <tool line>` (`worker: reading sidebar.rs`), so the line shows what the delegated work does. Parallel subagents take turns on the line; the newest event wins.
- **Fallback.** With no activity known, the line shows the status in words: `working`, `needs you`, `ready`, `printing`, `quiet`.
- **One line per row that has an agent.** A panel row with one terminal shows its agent's line; a panel with terminal rows shows the line under each agent terminal row and none under the panel row. Rows without an agent stay one line. The line is a separate sidebar row (`Row::Activity`), not selectable, that belongs to the row above it: hover, click, the band and drag dimming cover both. Scrolling and hit-testing keep counting one screen line per row.
- **Notification click by focus return.** OSC 9 tells deck nothing about a click. `AgentAlert` gains the panel's id (names are not unique). When the outer terminal regains focus within 10 s of an alert this client wrote while unfocused, and the alerted panel is still unseen or still waits, the client focuses that panel and its most urgent terminal. It works in every terminal that reports focus; switching back by hand within 10 s jumps too.
- **Protocol**: `PROTOCOL_VERSION` 29 (`AgentState.activity`, `Request::AgentEvent.activity` and `at_ms`, `AgentAlert.panel_id`), all in sub-spec 01; the number is the next free one at merge.

## Sub-specs

| Sub-spec                      | Type       | Crate              | Summary                                                                           | Depends | Status |
| ----------------------------- | ---------- | ------------------ | --------------------------------------------------------------------------------- | ------- | ------ |
| [01](01-activity-state.md)    | foundation | core, daemon, deck | `activity` in the state and protocol, Claude Code and Codex hooks, `ls`, alert id | —       | done   |
| [02](02-sidebar-line.md)      | shippable  | tui                | The second line under agent panel and terminal rows                               | 01      | done   |
| [03](03-opencode-activity.md) | shippable  | core, deck         | OpenCode tool and permission events feed the line                                 | 01      | done   |
| [04](04-notification-jump.md) | shippable  | tui                | Returning to the terminal after a notification focuses the panel it is about      | 01      | done   |

## Affected docs

- `docs/features/agents.md`: Hook events (the activity, order, subagents), the Claude Code and Codex event lists (`PreToolUse`), OpenCode's forwarded events and their reduction, the "Formats checked" sentence, Codex's per-tool hook cost, and the jump on focus return (Attention).
- `docs/features/sidebar.md`: the activity line, its mockup and rules.
- `docs/features/privacy.md`: the "never logs" list and the in-memory "Per agent" row gain the activity; commands, file names and the agent's last message are on screen and in `deck ls --json` (any control client can read them, a secret typed in a command included); never saved or notified.
- `docs/features/control-api.md`: `activity` in `ls --json`.
- `docs/architecture.md`: the Agents section, `AgentAlert`'s fields and the protocol version list.

Reviewed by spec-reviewer on 26 Nov 2026.
