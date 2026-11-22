---
type: shippable
status: done
---

# 05 — Resume agents after a restart

## Scope

After `deck kill-server` or a reboot, every terminal that had an agent running comes back with that agent reopened on its conversation. Owning crates: `deck-daemon` (save and relaunch), `deck-core` (state field, config, command templates).

## Requirements

1. **State**: `TerminalState` gains `agent: Option<AgentResume { kind, session_id: Option<String> }>` (`#[serde(default)]`, DECK-04 sub-spec 01), saved when the terminal has an agent state with the agent in the foreground (any source). An agent state change (kind, or a new `session_id`) marks the state dirty.
2. **Shutdown**: once DECK-04's `shutting_down` flag is set, the daemon ignores `AgentEvent` requests and detection changes, so an agent's `SessionEnd` hook or the hang-up's exits cannot clear the info that resume needs; the agent info is captured in the final save before the terminals are hung up.
3. **Config** (`[agents.<kind>]`, documented in `default.toml`): `resume` and `resume_latest` templates, `resume_agents = true` at the top level. Defaults (check each agent's current `--help` before relying on them; OpenCode's flags were not verified when this spec was written):

   | Agent    | `resume`                          | `resume_latest`       |
   | -------- | --------------------------------- | --------------------- |
   | Claude   | `claude --resume {session_id}`    | `claude --continue`   |
   | Codex    | `codex resume {session_id}`       | `codex resume --last` |
   | OpenCode | `opencode --session {session_id}` | `opencode --continue` |

   A template is split on whitespace into program and arguments; `{session_id}` may appear only as a whole argument. A template containing a quote (`'` or `"`), a backslash or `$` is a config error naming the key.

4. **Session ids** are used only when they match `^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$` (the first character can never make the id an option); otherwise the id is dropped, logged, and `resume_latest` is used. The `resume` template is used when a valid id is known, else `resume_latest`.
5. **Shell terminals** (`program: None`) whose saved state has `agent` and `resume_agents = true` are relaunched as `/bin/sh -c '<resume command>; exec "$0" "$@"' <shell> <shell_args…>`, where `<resume command>` is the rendered template joined with single spaces (safe because requirements 3 and 4 forbid quotes and shell syntax). POSIX `sh` runs the agent, then replaces itself with the user's shell and its configured `shell_args` (DECK-01), whatever that shell is. Quitting the agent leaves the user in their shell in the same cwd, and nothing is typed into a shell or written to its history. The terminal's `LaunchSpec` stays the plain shell, so a later restore without an agent starts a plain shell.
6. **Explicit-program terminals** whose launch spec runs the agent itself (created with `deck new-terminal -- claude`, for example: `match_argv` of sub-spec 01 on the program and arguments gives the agent) are relaunched with the rendered resume command as program and arguments, keeping the original cwd and env. No shell is involved.
7. **Several agents in one directory** each resume their own session when session ids are known; with no ids, every one of them runs `resume_latest`, which the docs call out as a limitation.
8. **Staleness** (documented in `docs/features/agents.md`): after a crash or `kill -9`, the restore uses the last debounced save, so an agent quit within the last 500 ms may be relaunched, and a `session_id` changed in that window may point at the previous conversation.

## Acceptance criteria

- [ ] A fake `claude` (a `#!/bin/sh` script that records its argv to a file, then waits on stdin) running in a terminal with hook state `session_id = "abc"`: after `deck kill-server` and `deck`, the argv file shows `--resume abc`, and after the fake agent exits the terminal shows a shell prompt.
- [ ] With `shell_args = ["-i"]` in the config, the shell after the agent is started with `-i` (a fake shell script records its argv).
- [ ] With no session id, `claude --continue` runs.
- [ ] Session ids `abc; rm -rf ~` and `-p` are dropped and `resume_latest` runs instead.
- [ ] A template containing a quote or `$` is rejected at config load with the key named.
- [ ] `resume_agents = false` restores a plain shell.
- [ ] A custom `resume` template in the config is used.
- [ ] A terminal created with `deck new-terminal -- claude` and session id `xyz` restores as `claude --resume xyz` launched directly, in its original cwd.
- [ ] An agent running when `deck kill-server` is called is resumed, even when its fake hook sends `SessionEnd` during the shutdown.
- [ ] Old `state.json` files without the `agent` field still load.

## Tests

- `deck-core`: template parsing, rendering and rejection; session-id validation.
- `crates/deck-daemon/tests/`: save of agent info, including across the shutdown hang-up with a late `AgentEvent`; both relaunch paths.
- `crates/deck/tests/resume.rs`: the end-to-end criteria with fake agent scripts on `PATH`.

## Notes

- Checked on 23 Nov 2026: `claude --help` lists `-c, --continue` and `-r, --resume [value]` (a session id), `codex resume --help` shows `codex resume [SESSION_ID]` and `--last`, and the OpenCode docs (Context7, `/anomalyco/opencode`, CLI flags of the TUI) list `--continue` and `--session <id>`; running `opencode --help` in the sandbox failed because it writes under `~/.local/share/opencode`, so its flags rest on the docs. All six defaults of the table are right as written.
- Requirement 3 is implemented more strictly than the spec: besides quotes, backslash and `$`, a template word (other than a whole `{session_id}`) may not hold a backtick or any of `; & | < > ( ) * ? [ ] { } # ~ !` or a control character, `{session_id}` cannot be the program, `resume_latest` cannot hold `{session_id}`, and an empty template is an error. `resume_command` checks the template again at relaunch, so a config built in code cannot reach the shell unchecked.
- `Config` gained `resume_agents` and `AgentConfig.resume` / `resume_latest`, which the clients receive, so `PROTOCOL_VERSION` went from 10 to 11.
- A terminal that resumes its agent gets its agent state (process source, `quiet`, the saved kind and session id) when the state is restored, before any save, and `program_name` looks through deck's own `sh -c '<command>; exec "$0" "$@"'` wrapper (which has no job control, so the agent stays in `sh`'s process group) so detection keeps finding it. Only that wrapper form is looked through: other `sh -c` scripts keep the shell as their program, as sub-spec 01's tests require.
- The late-`SessionEnd` ordering between `request_shutdown` and the final save cannot be driven from a test without a production hook, so no test fails by itself without the `shutting_down` guard; the guard is covered by the E2E trap test and by reading.
- The `deck new-terminal` CLI has no directory option: the terminal starts in its project's directory, which is the "original cwd" the e2e test checks.
- The shutdown test in `crates/deck-daemon/tests/resume.rs` sends the late `SessionEnd` after the clients got `Bye`; the daemon may already have closed the connection, and either way the saved agent must stay. The end-to-end test with a trapped `SIGHUP` that runs `deck agent-event` covers the hang-up itself.
