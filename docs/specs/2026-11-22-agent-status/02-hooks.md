---
type: shippable
status: draft
---

# 02 — Agent hooks

## Scope

After `deck hooks install claude codex opencode`, deck knows exactly when each agent is working, waiting for the user or done, because the agents' own hooks report it. Owning crates: `deck` (`agent-event`, `hooks`), `deck-core` (event mapping), `deck-daemon` (already handles `AgentEvent` from sub-spec 01).

## Requirements

1. **`deck agent-event --agent <claude|codex|opencode> [--event <name>]`** (hidden from the main help, documented in `docs/features/agents.md`): reads the hook's JSON payload from stdin (at most 1 MiB, 1 s), takes the event name from `--event` or the payload's `hook_event_name`, maps it (requirement 2), and sends `AgentEvent` for `DECK_TERMINAL_ID` over `DECK_SOCKET`. It never writes to stdout, always exits 0, gives up after 500 ms in total, and does nothing when `DECK_TERMINAL_ID` is unset or the socket is unreachable. It never starts a daemon.
2. **Mapping** (pure, in `deck-core::agents`, unit-tested). Claude Code and Codex payloads carry `hook_event_name` and `session_id`; prompts come from `prompt`.

   | Agent    | Event                                                        | Status    | Also records           |
   | -------- | ------------------------------------------------------------ | --------- | ---------------------- |
   | Claude   | `SessionStart`                                               | `idle`    | `session_id`           |
   | Claude   | `UserPromptSubmit`                                           | `working` | `session_id`, `prompt` |
   | Claude   | `PostToolUse`                                                | `working` |                        |
   | Claude   | `PermissionRequest`                                          | `waiting` |                        |
   | Claude   | `Notification`, `notification_type` `permission_prompt` or `elicitation_dialog` | `waiting` |   |
   | Claude   | `Notification`, `notification_type` `idle_prompt`            | `idle`    |                        |
   | Claude   | `Stop`                                                       | `idle`    |                        |
   | Claude   | `SessionEnd`                                                 | `none`    |                        |
   | Codex    | `SessionStart`                                               | `idle`    | `session_id`           |
   | Codex    | `UserPromptSubmit`                                           | `working` | `session_id`, `prompt` |
   | Codex    | `PostToolUse`                                                | `working` |                        |
   | Codex    | `PermissionRequest`                                          | `waiting` |                        |
   | Codex    | `Stop`, `Interrupt`                                          | `idle`    |                        |
   | Codex    | `SessionEnd`                                                 | `none`    |                        |
   | OpenCode | `session.status`, `properties.status.type` `busy` / `idle`   | `working` / `idle` | `session_id` from `properties.sessionID` |
   | OpenCode | `session.idle` (deprecated upstream, still handled)          | `idle`    | `session_id`           |
   | OpenCode | `permission.asked`                                           | `waiting` | `session_id`           |
   | OpenCode | `permission.replied`                                         | `working` |                        |
   | OpenCode | `message.part.updated` of type `text` in a message whose `message.updated` had `properties.info.role` `user` | — | `prompt` |

   Other events and other `notification_type` values are ignored. A payload field that is missing is simply not recorded. `PreToolUse` is not used: `PostToolUse` keeps the state fresh at half the cost per tool call. Field names were taken from the agents' docs on 22 Nov 2026; the worker checks them again (Notes).
3. **OpenCode subagents**: the plugin ignores every event of a session that has a parent. It remembers the ids of child sessions from `session.created` and `session.updated` (`properties.info.parentID` set) and drops events whose `sessionID` is one of them, so a subagent finishing never marks the terminal `idle` or replaces the resumable session id.
4. **`deck hooks install <agent>... [--print]`**, **`deck hooks uninstall <agent>...`**, **`deck hooks status`**, for the user-level config of each agent:
   - **Claude Code**: `$CLAUDE_CONFIG_DIR/settings.json` (default `~/.claude/settings.json`), `hooks` key: one entry per event of the table (no matcher, so nothing is filtered; `Notification` included), each running `deck agent-event --agent claude`, marked to run asynchronously when Claude Code's hook schema supports it so the agent never waits for deck.
   - **Codex**: `$CODEX_HOME/hooks.json` (default `~/.codex/hooks.json`), always written there, with the schema `{ "<Event>": [{ "matcher": "", "hooks": [{ "type": "command", "command": "deck agent-event --agent codex", "timeout": 2 }] }] }`. `status` also reports deck entries found under `[hooks]` in `$CODEX_HOME/config.toml` (Codex loads both and warns about duplicates). When Codex needs a trust or enable step for user hooks, `install` prints it.
   - **OpenCode**: a plugin file `deck.js` in the user plugin directory OpenCode loads (`~/.config/opencode/plugin/` or `plugins/`, whichever the current docs name), subscribing to the events of the table and running `deck agent-event --agent opencode --event <type>` with the event as JSON on stdin, through the plugin's shell helper, without awaiting a result that could slow OpenCode.
   - **Writing safely**: the target path is canonicalized first (a `settings.json` symlinked into a dotfiles repo is written in place, and the symlink survives); the new content is written to a temp file in the target's own directory and renamed over it. Before any write, the current file is copied to `$XDG_STATE_HOME/deck/backups/<agent>-<file>-<unix-seconds>`. JSON is parsed and rewritten with `serde_json` with the `preserve_order` feature (key order kept) and 2-space indentation; other keys and hooks are kept. The first install may still reformat the file; the docs say so.
   - **Idempotent**: deck's entries are recognised by their `deck agent-event` command and replaced, never duplicated. `--print` prints what would be written and writes nothing. `uninstall` removes only deck's entries or file. `status` prints one line per agent: `installed`, `not installed`, `needs a step` (Codex trust) or `stale` (an installed command points at a `deck` binary that no longer exists).
   - The `deck` path in the hook commands is `deck` when the running binary is the one found on `PATH`, else its absolute path, and `install` then warns that a rebuild or move will make the hooks stale.

## Acceptance criteria

- [ ] Mapping unit tests cover every row of the table with sample payloads (captured from the real agents where available, otherwise written from their docs and named as such in the test), including ignored `notification_type` values and OpenCode child-session filtering.
- [ ] `echo '{"hook_event_name":"Stop","session_id":"abc"}' | deck agent-event --agent claude` inside deck sets the terminal to `idle` with session `abc`, prints nothing and exits 0.
- [ ] The same command outside deck, or with the daemon stopped, exits 0 in under 600 ms and prints nothing.
- [ ] `deck hooks install claude` on a `settings.json` with existing hooks keeps them and their key order, adds deck's entries, writes a backup under `$XDG_STATE_HOME/deck/backups/`, and a second run changes nothing (byte-identical file).
- [ ] With `settings.json` a symlink to another directory, install writes the target and the symlink is still a symlink.
- [ ] `CLAUDE_CONFIG_DIR` and `CODEX_HOME` are honoured.
- [ ] `deck hooks uninstall claude` restores the hooks to what they were apart from formatting.
- [ ] `deck hooks install codex` writes `$CODEX_HOME/hooks.json` with the schema above; `status` reports deck entries found in `config.toml` too.
- [ ] `deck hooks install opencode` writes the plugin file; `status` reports each agent correctly, including `stale` after the hook's binary path is removed; `--print` writes nothing.
- [ ] Each install test runs against a temp `HOME` (and temp `XDG_STATE_HOME`), never the real one.
- [ ] By hand (listed in the worker's report as not automated), for each agent available on the machine: with hooks installed, a prompt shows the spinner, a permission prompt shows `!`, and the end of the turn shows `●`. The report says which agents were verified; the others are listed as not verified.

## Tests

- `deck-core`: the mapping table, OpenCode child-session filtering.
- `crates/deck/tests/hooks.rs`: `agent-event` inside and outside deck; install, uninstall, status and `--print` against a temp `HOME` for each agent, including the symlink and env-var cases.

## Notes

- Check each agent's current hook and plugin docs (Context7: `/openai/codex`, `/anomalyco/opencode`; Claude Code's hooks reference, including whether hooks can run asynchronously) before writing the install logic and the mapping; event names and payload fields drift. Record the versions checked in `docs/features/agents.md`.
- This sub-spec is the largest of the spec (three installers). If the worker expects to run out of budget, the lead may move the OpenCode plugin into its own sub-spec.
