---
type: shippable
status: done
---

# 03 — OpenCode activity

## Scope

OpenCode sessions get the same line as Claude Code and Codex. The user tries it with `deck hooks install opencode` and an OpenCode session in `just run`. Crates: `deck` (the plugin template `opencode_plugin.js.tmpl`, `agent_event.rs`), `deck-core` (`agents/opencode.rs`). The daemon already passes `OpenCodeUpdate.activity` on (01).

## Requirements

- The plugin also forwards OpenCode v2's event for a tool call that starts (in v1's shape: `message.part.updated` with a `tool` part whose `state.status` is `running`), for its own sessions only, under the existing filter. Find the v2 event names and fields in the OpenCode source (`packages/schema/src/`), as DECK-13 did.
- Bounded processes: the plugin starts a `deck agent-event` at most once per tool call (it remembers the call ids it sent, at most 256) and never for a streamed text chunk. For the end of a turn it keeps only the latest assistant text part of its root session in memory and sends its first line once, with the idle status, as one small event of its own shape (documented in the template and the tracker).
- Only the line reaches the daemon: `deck agent-event --agent opencode` passes the raw event through the pure `deck-core::agents::reduce_opencode_event`, which keeps the event's type, session ids, status fields and, for a tool part, only the tool name, `description`, the first line of a command and a file name (never the rest of `state.input`, a file's content or an output), and for a text only its first line, cut to 80 characters. Other events are forwarded as today.
- `OpenCodeTracker` turns them into an `Activity` with the same words as 01: a tool by its name (OpenCode's `bash`, `read`, `edit`, `write`, `grep`, `glob`, `webfetch`, `task`, `todowrite` map to the Claude Code rows; others `using <tool>`), `bash` by its `description` else its command, `permission.asked` as `needs permission: <tool line>`, `session.status` `busy` as `thinking` when no tool ran yet in the turn, `idle` as the first line of the last assistant text, else `done`. Subagent sessions' tool events are ignored, like their status.
- The tracker never keeps a tool's whole input: only the line.
- `deck hooks status` reports a plugin without the new event as `stale` (the existing "lacks some events" check).

## Acceptance criteria

- [ ] Tracker tests: each tool row, `bash` with and without a description, a permission, a busy turn without a tool, an idle turn with and without text, subagent events ignored; v1 and v2 shapes where both exist.
- [ ] The plugin forwards the new events, only for its sessions (plugin test or the documented manual check of DECK-13).
- [ ] An older plugin is `stale`.
- [ ] `reduce_opencode_event` drops a marker string placed in a tool's `state.input` content, its output and in the lines after the first of a text; the request the CLI sends does not contain it.
- [ ] A streamed answer of 500 text chunks and 3 tool calls starts at most 3 + 1 `deck agent-event` processes beyond today's events (plugin test, or a documented count with a fake `deck`).
- [ ] In `just run` with OpenCode, the sidebar line follows the session (screen captures, or a recorded event stream if no model is available; say which).

## Tests

- `deck-core`: `OpenCodeTracker` table tests; `reduce_opencode_event` with marker strings.
- `deck`: plugin template contents and `status` staleness.

## Notes

- If v2 has no event for a running tool that the TUI plugin can see, the line shows `thinking`/`ready`/`needs you` from the existing events, and the worker reports it; that is acceptable.
- Built without a live OpenCode run (no model available to the worker): tool names and input keys were checked against `anomalyco/opencode@dev` `packages/core/src/tool/plugin` and the installed binary on 26 Nov 2026. The live acceptance criterion stays open until the user tries it.
