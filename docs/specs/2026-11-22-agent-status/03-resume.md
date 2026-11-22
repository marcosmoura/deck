---
type: shippable
status: draft
---

# 03 — Resume agents after a restart

## Scope

After `deck kill-server` or a reboot, every terminal that had an agent running comes back with that agent reopened on its conversation. Owning crates: `deck-daemon` (save and relaunch), `deck-core` (state field, config, command templates).

## Requirements

1. **State**: `TerminalState` gains `agent: Option<AgentResume { kind, session_id: Option<String> }>` (`#[serde(default)]`, DECK-04 sub-spec 01), saved when the terminal has an agent state with the agent in the foreground (any source). An agent state change (kind, or a new `session_id`) marks the state dirty. On shutdown the agent info is captured before the terminals are hung up: DECK-04's `shutting_down` flag stops the hang-up's exits from clearing it.
2. **Config** (`[agents.<kind>]`): `resume` and `resume_latest` templates, `resume_agents = true` at the top level. Defaults (check each agent's current `--help` before relying on them; OpenCode's flags were not verified when this spec was written):

   | Agent    | `resume`                          | `resume_latest`       |
   | -------- | --------------------------------- | --------------------- |
   | Claude   | `claude --resume {session_id}`    | `claude --continue`   |
   | Codex    | `codex resume {session_id}`       | `codex resume --last` |
   | OpenCode | `opencode --session {session_id}` | `opencode --continue` |

   A template is split on whitespace into program and arguments; `{session_id}` may appear only as a whole argument. A template containing a quote (`'` or `"`), a backslash or `$` is a config error naming the key.
3. **Session ids** are used only when they match `^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$` (the first character can never make the id an option); otherwise the id is dropped, logged, and `resume_latest` is used. The `resume` template is used when a valid id is known, else `resume_latest`.
4. **Shell terminals** (`program: None`) whose saved state has `agent` and `resume_agents = true` are relaunched as `<shell> -l -c '<resume command>; exec <shell> -l'`, where `<resume command>` is the rendered template joined with single spaces (safe because requirement 2 and 3 forbid quotes and shell syntax). Quitting the agent leaves the user in a login shell in the same cwd, and nothing is typed into a shell or written to its history. The terminal's `LaunchSpec` stays the plain shell, so a later restore without an agent starts a plain shell.
5. **Explicit-program terminals** whose launch spec runs the agent itself (created with `deck new-terminal -- claude`, for example: the program's basename matches the agent's `process` list) are relaunched with the rendered resume command as program and arguments, keeping the original cwd and env. No shell is involved.
6. **Several agents in one directory** each resume their own session when session ids are known; with no ids, every one of them runs `resume_latest`, which the docs call out as a limitation.
7. **Staleness** (documented in `docs/features/agents.md`): after a crash or `kill -9`, the restore uses the last debounced save, so an agent quit within the last 500 ms may be relaunched, and a `session_id` changed in that window may point at the previous conversation.

## Acceptance criteria

- [ ] A fake `claude` script (records its argv to a file, then waits on stdin) running in a terminal with hook state `session_id = "abc"`: after `deck kill-server` and `deck`, the argv file shows `--resume abc`, and after the fake agent exits the terminal shows a shell prompt.
- [ ] With no session id, `claude --continue` runs.
- [ ] Session ids `abc; rm -rf ~` and `-p` are dropped and `resume_latest` runs instead.
- [ ] A template containing a quote or `$` is rejected at config load with the key named.
- [ ] `resume_agents = false` restores a plain shell.
- [ ] A custom `resume` template in the config is used.
- [ ] A terminal created with `deck new-terminal -- claude` and session id `xyz` restores as `claude --resume xyz` launched directly, in its original cwd.
- [ ] An agent running when `deck kill-server` is called is resumed (the shutdown hang-up did not clear it).
- [ ] Old `state.json` files without the `agent` field still load.

## Tests

- `deck-core`: template parsing, rendering and rejection; session-id validation.
- `crates/deck-daemon/tests/`: save of agent info, including across the shutdown hang-up; both relaunch paths.
- `crates/deck/tests/resume.rs`: the end-to-end criteria with fake agent scripts on `PATH`.
