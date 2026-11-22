---
type: shippable
status: done
---

# 03 — Hooks for Claude Code and Codex

## Scope

After `deck hooks install claude codex`, deck knows exactly when Claude Code and Codex are working, waiting for the user or done, because their own hooks report it. Also lays down `deck agent-event` and the install machinery that sub-spec 04 reuses for OpenCode. Owning crates: `deck` (`agent-event`, `hooks`), `deck-core` (event mapping); `deck-daemon` already handles `AgentEvent` (sub-spec 01).

## Requirements

1. **`deck agent-event --agent <claude|codex|opencode> [--event <name>]`** (hidden from the main help, documented in `docs/features/agents.md`): reads the hook's JSON payload from stdin (at most 1 MiB), takes the event name from `--event` or the payload's `hook_event_name`, maps it (requirement 2; for `opencode`, sub-spec 04 sends the raw event instead), and sends `AgentEvent` for `DECK_TERMINAL_ID` over `DECK_SOCKET`. It never writes to stdout, always exits 0, and gives up after 500 ms in total, reading stdin included. It does nothing when `DECK_TERMINAL_ID` is unset or the socket is unreachable, and never starts a daemon.
2. **Mapping** (pure, in `deck-core::agents`, unit-tested). Both agents' payloads carry `hook_event_name` and `session_id`; prompts come from `prompt`.

   | Agent  | Event                                                                           | Status    |
   | ------ | ------------------------------------------------------------------------------- | --------- |
   | Claude | `SessionStart`                                                                  | `idle`    |
   | Claude | `UserPromptSubmit`                                                              | `working` |
   | Claude | `PostToolUse`                                                                   | `working` |
   | Claude | `PermissionRequest`                                                             | `waiting` |
   | Claude | `Notification`, `notification_type` `permission_prompt` or `elicitation_dialog` | `waiting` |
   | Claude | `Notification`, `notification_type` `idle_prompt`                               | `idle`    |
   | Claude | `Stop`                                                                          | `idle`    |
   | Claude | `SessionEnd`                                                                    | `none`    |
   | Codex  | `SessionStart`                                                                  | `idle`    |
   | Codex  | `UserPromptSubmit`                                                              | `working` |
   | Codex  | `PostToolUse`                                                                   | `working` |
   | Codex  | `PermissionRequest`                                                             | `waiting` |
   | Codex  | `Stop`, `Interrupt`                                                             | `idle`    |
   | Codex  | `SessionEnd`                                                                    | `none`    |

   Every event records the payload's `session_id`; only `UserPromptSubmit` also records the `prompt`. Other events and other `notification_type` values are ignored. A missing payload field is not recorded. `PreToolUse` is not used: `PostToolUse` keeps the state fresh at half the cost per tool call. An answered permission prompt turns back to `working` through sub-spec 01's input rule. Field names were taken from the agents' docs on 22 Nov 2026; the worker checks them again (Notes).

3. **`deck hooks install <agent>... [--print]`**, **`deck hooks uninstall <agent>...`**, **`deck hooks status`** (OpenCode is added by sub-spec 04):
   - **Claude Code**: `$CLAUDE_CONFIG_DIR/settings.json` (default `~/.claude/settings.json`), `hooks` key: one entry per event of the table (no matcher, so nothing is filtered; `Notification` included), each running `deck agent-event --agent claude`, marked to run asynchronously when Claude Code's hook schema supports it so the agent never waits for deck.
   - **Codex**: `$CODEX_HOME/hooks.json` (default `~/.codex/hooks.json`), always written there. Its schema (Codex rejects unknown fields): `{ "description": "…" (optional), "hooks": { "<Event>": [ { "matcher": "", "hooks": [ { "type": "command", "command": "deck agent-event --agent codex", "timeout": 2 } ] } ] } }`, with one entry per Codex event of the table (`SessionEnd` and `Interrupt` included). `status` also reports deck entries found under `[hooks]` in `$CODEX_HOME/config.toml` (Codex loads both and warns about duplicates). When Codex needs a trust or enable step for user hooks, `install` prints it.
   - **Writing safely**: the target path is canonicalized first (a file symlinked into a dotfiles repo is written in place, and the symlink survives); the new content is written to a temp file in the target's own directory and renamed over it. Before any write, the current file is copied to `$XDG_STATE_HOME/deck/backups/<agent>-<file>-<unix-seconds>`. JSON is parsed and rewritten with `serde_json` with the `preserve_order` feature (key order kept) and 2-space indentation; other keys and hooks are kept. The first install may still reformat the file; the docs say so.
   - **Idempotent**: deck's entries are recognised by their `deck agent-event` command and replaced, never duplicated. `--print` prints what would be written and writes nothing. `uninstall` removes only deck's entries. `status` prints one line per agent: `installed`, `not installed`, or `stale`; Codex's trust cannot be checked, so Codex reads `installed (trust them in /hooks if Codex asks)` (an installed command points at a `deck` binary that no longer exists).
   - The `deck` path in the hook commands is `deck` when the running binary is the one found on `PATH`, else its absolute path, and `install` then warns that a rebuild or move will make the hooks stale.

## Acceptance criteria

- [ ] Mapping unit tests cover every row of the table with sample payloads (captured from the real agents where available, otherwise written from their docs and named as such in the test), including ignored `notification_type` values.
- [ ] `echo '{"hook_event_name":"Stop","session_id":"abc"}' | deck agent-event --agent claude` inside deck sets the terminal to `idle` with session `abc`, prints nothing and exits 0.
- [ ] The same command outside deck, or with the daemon stopped, exits 0 well within 2 s and prints nothing; a unit test with an injected clock checks that the whole run gives up at 500 ms, a stdin that never closes included.
- [ ] `deck hooks install claude` on a `settings.json` with existing hooks keeps them and their key order, adds deck's entries, writes a backup under `$XDG_STATE_HOME/deck/backups/`, and a second run changes nothing (byte-identical file).
- [ ] With `settings.json` a symlink to another directory, install writes the target and the symlink is still a symlink.
- [ ] `CLAUDE_CONFIG_DIR` and `CODEX_HOME` are honoured.
- [ ] `deck hooks uninstall claude` restores the hooks to what they were apart from formatting.
- [ ] `deck hooks install codex` writes `$CODEX_HOME/hooks.json` with the `hooks` wrapper above, and the result deserializes with a strict (unknown-fields-denied) copy of Codex's schema in the test; `status` reports deck entries found in `config.toml` too.
- [ ] `status` reports `stale` after the hook's binary path is removed; `--print` writes nothing.
- [ ] Each install test runs against a temp `HOME` (and temp `XDG_STATE_HOME`), never the real one.
- [ ] By hand (listed in the worker's report as not automated), for Claude Code and Codex where available on the machine: with hooks installed, a prompt shows the spinner, a permission prompt shows `!`, and the end of the turn shows `●`. The report says which agents were verified.

## Tests

- `deck-core`: the mapping table.
- `deck`: the 500 ms budget with an injected clock.
- `crates/deck/tests/hooks.rs`: `agent-event` inside and outside deck; install, uninstall, status and `--print` against a temp `HOME` for both agents, including the symlink and env-var cases.

## Notes

- Check each agent's current hook docs (Context7: `/openai/codex`; Claude Code's hooks reference, including whether hooks can run asynchronously) before writing the install logic and the mapping; event names and payload fields drift. Record the versions checked in `docs/features/agents.md`.
