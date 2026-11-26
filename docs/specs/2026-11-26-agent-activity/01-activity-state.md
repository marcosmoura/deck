---
type: foundation
status: todo
---

# 01 — Activity state from Claude Code and Codex hooks

## Scope

Lays down the activity line: the field in the agent state and the protocol, the pure functions that describe a hook payload, the CLI that sends it, the daemon that keeps and clears it, and the new `PreToolUse` hook. It also adds the panel id to `AgentAlert` for 04, so the protocol changes once. `deck ls --json` shows the activity, which is how this sub-spec is checked; the sidebar draws it in 02, OpenCode feeds it in 03. Crates: `deck-core` (model, protocol, `agents`), `deck` (`agent_event.rs`), `deck-daemon` (`agents.rs`, `server/agents.rs`).

## Requirements

### State and protocol

- `AgentState.activity: Option<String>`: one line, at most `ACTIVITY_MAX` (80) characters, control characters removed, trimmed; `None` when blank.
- `pub enum Activity { Keep, Clear, Set(String) }` in `deck-core::agents`. `Request::AgentEvent` gains `activity: Activity` and `at_ms: u64` (Unix milliseconds when `deck agent-event` started; 0 from a caller that has none, such as a test, which is never older than anything). The daemon cleans and cuts a `Set` again: a hook is not trusted to have done it.
- `HookUpdate` and `OpenCodeUpdate` gain `activity: Activity` (`OpenCodeUpdate`'s stays `Keep` until 03), and `apply_opencode` (`deck-daemon/src/agents.rs`) copies it into the daemon's `Event` with the rest.
- `ServerMsg::AgentAlert` gains `panel_id: PanelId` (the names stay, for the notification text). `raise_attention` (`deck-daemon/src/server/agents.rs`) fills it. Update the round-trip test.
- `PROTOCOL_VERSION` becomes the next free number (29 today), recorded in `docs/architecture.md`'s version list.
- `AgentState` is not saved (`TerminalState.agent` holds only `AgentResume`); keep it so. A live upgrade rebuilds terminals with no agent state (`handoff/file.rs`), so the lines start empty after one; that is fine. The activity is never written to `daemon.log`.

### Describing a payload (`deck-core::agents`, pure)

`describe_hook(kind, event, payload: &serde_json::Value) -> Activity`. Rules for Claude Code (Codex uses the same where its payload has the field; a missing field gives the fallback in brackets):

| Event                                                    | Line                                                                                                                           |
| -------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| `SessionStart`                                           | `Clear`                                                                                                                        |
| `UserPromptSubmit`                                       | `thinking`                                                                                                                     |
| `PreToolUse`                                             | the tool line, below; from a subagent (`agent_type` in the payload) `<agent_type>: <tool line>`                                |
| `PostToolUse`                                            | `Keep`                                                                                                                         |
| `PermissionRequest`                                      | `needs permission: <tool line>`                                                                                                |
| `Notification` `permission_prompt`, `elicitation_dialog` | `message`, first line [`needs you`]                                                                                            |
| `Notification` `idle_prompt`                             | `Keep`                                                                                                                         |
| `Stop` with a non-empty `background_tasks`               | `waiting for <n> background task(s)`                                                                                           |
| `Stop` (Claude), `Stop` (Codex)                          | the first non-blank line of `last_assistant_message`, Markdown marks (`#`, `*`, `` ` ``, `>`, a leading `- `) removed [`done`] |
| `Interrupt` (Codex)                                      | `interrupted`                                                                                                                  |
| `SessionEnd`                                             | `Clear`                                                                                                                        |

The tool line, from `tool_name` and `tool_input`:

| Tool                                                              | Line                                                                                                                     |
| ----------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| `Bash` (Codex: `shell`, `exec_command`)                           | `tool_input.description` when present, lower-cased first letter; else `running <command>`, the first line of the command |
| `Read`                                                            | `reading <file name>`                                                                                                    |
| `Edit`, `MultiEdit`, `Write`, `NotebookEdit`; Codex `apply_patch` | `editing <file name>` (`apply_patch`: the first file the patch names, else `editing files`)                              |
| `Grep`, `Glob`                                                    | `searching <pattern>`                                                                                                    |
| `WebFetch`                                                        | `fetching <host>`                                                                                                        |
| `WebSearch`                                                       | `searching the web`                                                                                                      |
| `Agent`, `Task`                                                   | `dispatching <subagent_type>: <description>` (`dispatching agent` without either)                                        |
| `AskUserQuestion`                                                 | `asking: <header of the first question>` [`asking a question`]                                                           |
| `TodoWrite`                                                       | `planning`                                                                                                               |
| `Skill`                                                           | `using skill <name>`                                                                                                     |
| `mcp__<server>__<tool>`                                           | `using <server> <tool>`                                                                                                  |
| anything else                                                     | `using <tool name>`                                                                                                      |

- A file name is the last path component only (never the directory). A host is the URL's host only (never the path or query).
- Every line is cut to `ACTIVITY_MAX` characters at a character boundary; the sidebar cuts it again to its width.
- `describe_status(status) -> &'static str`: `working`, `needs you` (`waiting`), `ready` (`idle`), `printing` (`active`), `quiet`, `""` (`none`); clients show it when `activity` is `None`. It is the sidebar's list only; the notification text (`alert_body` in `deck-tui/src/agent.rs`) keeps its own words.

### CLI (`deck agent-event`)

- For Claude Code and Codex it calls `describe_hook` on the payload it already parses and sends the result as `activity`, with `at_ms` taken when it starts. Nothing else of the payload is sent; the 1 MiB, 500 ms and always-exit-0 rules stay.

### Hooks installed

- `hook_events` adds `PreToolUse` for Claude Code and Codex, mapped to status `working` like `PostToolUse`. `deck hooks install` writes it like the others (Claude: `"async": true`, no matcher; Codex: `"matcher": ""`, `"timeout": 2`). `deck hooks status` already reports `stale: some events have no hook; run "deck hooks install" again` for an install that lacks an event of `hook_events` (`crates/deck/src/hooks.rs`), so nothing else changes there. The test that expects `(Claude, "PreToolUse")` to be ignored (`deck-core/src/agents.rs`) changes.

### Daemon

- `apply_event` sets `activity` from the event (`Keep` leaves it, `Clear` empties it), unless the event's `at_ms` is older than the last applied activity's for that terminal, in which case only the status is applied (as today). The last `at_ms` lives in the terminal's `AgentRt`.
- A state that is cleared (agent left the foreground, `SessionEnd`, a hook state falling back to the process state after 60 s) loses its activity. Typing into a `waiting` terminal (it becomes `working`) sets `activity` to `None`.
- A change of activity alone pushes a `SessionView` through the throttle that title changes use (`push_view_throttled`, `TITLE_VIEW_INTERVAL`); a change of status keeps its immediate push.
- `deck ls --json` adds `"activity"` (string or null) to the agent object.

## Acceptance criteria

- [ ] `describe_hook` gives the lines in the tables for Claude Code and Codex payloads, including the subagent prefix, the fallbacks for missing fields, a path given only as its file name and a URL only as its host.
- [ ] A line longer than 80 characters, or with control characters or several lines, arrives cut, clean and on one line, both from the CLI and from a raw `AgentEvent` request.
- [ ] `deck hooks install claude codex` writes `PreToolUse`; an older install without it is `stale` in `deck hooks status`.
- [ ] With a fake hook sequence (`UserPromptSubmit`, `PreToolUse` Bash with a description, `PostToolUse`, `Stop` with `last_assistant_message`), `deck ls --json` shows `thinking`, the description, the same description, then the message's first line.
- [ ] A `PreToolUse` with an older `at_ms` than an applied `Stop` does not change the activity.
- [ ] `SessionEnd`, a cleared state and a typed answer to `waiting` clear the activity.
- [ ] 100 `PreToolUse` events within 1 s send an attached client at most 11 `SessionView`s for activity changes.
- [ ] `state.json` and `daemon.log` never contain the activity text (test with a marker string).
- [ ] `AgentAlert` carries the alerted panel's id.
- [ ] The protocol round-trip tests cover the new fields; `PROTOCOL_VERSION` is bumped.

## Tests

- `deck-core`: table tests for `describe_hook` per row of both tables, Claude and Codex, with and without `agent_type`; cleaning and cutting; `describe_status`; protocol round trips.
- `deck`: `agent_event` request building with an activity and `at_ms`; hooks install and status with `PreToolUse`.
- `deck-daemon`: `apply_event` with `Keep`/`Clear`/`Set` and an old `at_ms`; clearing paths; the throttle; the marker string absent from `state.json` and the log; `AgentAlert.panel_id`.

## Notes

- Payload fields checked on 26 Nov 2026 against the Claude Code hook types (`mods/types/claude-code.d.ts` in `anthropics/claude-code`, via Context7): `Stop` has `stop_hook_active`, `last_assistant_message?` and `background_tasks?`. The worker confirms there `PreToolUse` (`tool_name`, `tool_input`), `Notification` (`message`, `notification_type`) and whether a subagent's tool events carry `agent_type` (or `agent_id`); if they carry neither, subagent events get no prefix and the doc says so. The worker checks Codex's `PreToolUse` and `Stop` payloads in `codex-rs/hooks` (`codex-rs/protocol/src/hooks.rs`); a field Codex does not send gives the fallback. Record what was checked, with the date, in `docs/features/agents.md` as the existing "Formats checked" sentence does.
- A Codex hook is synchronous with a 2 s timeout, so `PreToolUse` adds one `deck agent-event` run (a few ms, 500 ms at most) before each Codex tool call. Say so in the agents doc.
