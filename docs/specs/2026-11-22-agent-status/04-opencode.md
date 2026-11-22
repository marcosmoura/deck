---
type: shippable
status: todo
---

# 04 — OpenCode

## Scope

After `deck hooks install opencode`, deck knows exactly when OpenCode is working, waiting or done, ignoring its subagents, and records the user's first prompt. Owning crates: `deck-core` (the OpenCode state machine), `deck-daemon` (holding it per terminal), `deck` (the plugin file and its install).

## Requirements

1. **Plugin** `deck.js`, written by `deck hooks install opencode` to the user plugin directory OpenCode loads (`~/.config/opencode/plugin/` or `plugins/`, whichever the current docs name). It subscribes to `session.created`, `session.updated`, `session.status`, `session.idle`, `permission.asked`, `permission.replied`, `message.updated` and `message.part.updated`, and for each runs `deck agent-event --agent opencode --event <type>` with the event as JSON on stdin, through the plugin's shell helper, without awaiting a result that could slow OpenCode. The plugin holds no logic and no state: it forwards raw events.
2. **`agent-event` for OpenCode** sends `AgentEvent { kind: OpenCode, raw: Some(<event JSON>) }` (sub-spec 01, requirement 7), with no mapping in the CLI.
3. **State machine** (pure, `deck-core::agents::OpenCodeTracker`), one per terminal, held by the daemon, fed each raw event, returning the status, `session_id` and prompt to apply:
   - **Child sessions**: `session.created` and `session.updated` with `properties.info.parentID` set record that session id as a child; every later event whose `properties.sessionID` (or `properties.info.sessionID`) is a child is dropped, so a subagent finishing never marks the terminal `idle` or replaces the resumable session id.
   - **Status**: `session.status` with `properties.status.type` `busy` → `working`, `idle` → `idle`; `session.idle` (deprecated upstream, still handled) → `idle`; `permission.asked` → `waiting`; `permission.replied` → `working`. `session_id` comes from `properties.sessionID`.
   - **First prompt**: `message.updated` with `properties.info.role` `user` records that message id as a user message; the first `message.part.updated` of type `text` belonging to it gives the prompt.
   - Unknown events and missing fields are ignored. The tracker's memory is bounded (at most 256 child ids and 64 user message ids, oldest dropped first).
4. **Install, uninstall, status** follow sub-spec 03's rules (canonicalized path, backup, atomic write, idempotent, `--print`, `stale`); `uninstall` removes the plugin file.

## Acceptance criteria

- [ ] `OpenCodeTracker` unit tests, from event samples taken from the OpenCode docs (named as such in the test): busy/idle via `session.status`, `session.idle`, `permission.asked` and `permission.replied`, child-session events dropped (status and session id unchanged), the first user prompt found from `message.updated` plus `message.part.updated`, assistant messages ignored, memory bounds.
- [ ] Piping a `session.status` busy event into `deck agent-event --agent opencode --event session.status` inside deck shows the spinner for that terminal.
- [ ] `deck hooks install opencode` writes the plugin file (temp `HOME`), a second run changes nothing, `uninstall` removes it, `status` reports `installed`, `not installed` and `stale` correctly, `--print` writes nothing.
- [ ] The plugin file contains no filtering logic (it forwards every subscribed event).
- [ ] By hand (listed in the worker's report as not automated), when OpenCode is available: a prompt shows the spinner, a permission prompt shows `!`, the end of the turn shows `●`, and a subagent finishing does not.

## Tests

- `deck-core`: `OpenCodeTracker`.
- `crates/deck-daemon/tests/`: one tracker per terminal fed through `AgentEvent { raw }`.
- `crates/deck/tests/hooks.rs`: the OpenCode install cases.

## Notes

- Check the current OpenCode plugin API and event payloads (Context7: `/anomalyco/opencode`) before writing the plugin and the tracker; record the version checked in `docs/features/agents.md`.
