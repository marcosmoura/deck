---
type: foundation
status: done
---

# 03 — Launching panels with a program

## Scope

Nothing to try yet: this lays down how a new panel starts an agent, and how a program that is not installed is reported. Sub-spec 04's menu, and DECK-11's resume, build on it. Owning crates: `deck-core` (action, config, command parsing), `deck-daemon` (resolution, lookup).

## Requirements

1. **Action**: a new `Action::NewPanelWith { project, launch: PanelLaunch }` with `PanelLaunch::Agent(AgentKind)` (`AgentKind` from DECK-06; DECK-11 adds `Resume`). The protocol version bumps. `Action::NewPanel` and DECK-05's `Request::NewPanel { launch: Option<LaunchSpec>, .. }` are unchanged (DECK-05, requirement 1a keeps the attach actions as they are), and `prefix c` and the menu's `Terminal` item keep sending `NewPanel`. The daemon resolves `Agent(kind)` into `LaunchSpec { program: Some(<first word of command>), args: <the rest>, cwd: <project root> }` and calls DECK-05's `NewPanel` function with `focus: true`, so everything after it (environment, names, persistence) is that function's.
2. **Config**: each `[agents.<kind>]` table (DECK-06) gains `command`, defaults `"claude"`, `"opencode"`, `"codex"`, documented in `default.toml` (the DECK-08 sync test applies). The value is parsed by the same template parser as DECK-06 sub-spec 05, requirement 3 (split on whitespace into program and arguments; no quote, backslash or `$`), with no placeholders allowed. An empty value, a violation, or a program that contains `/` without being an absolute path is a config error naming the key (`agents.claude.command`).
3. **Launch** of `Agent(kind)`: the new panel runs `command` directly (no shell) as an explicit-program terminal, with the project root as cwd and the environment and `DECK_*` variables DECK-05's function gives programs. The panel is named at once by DECK-09's rule (after its program). The panel is added, becomes active and focused like `prefix c`. The daemon does not wait for the program: an agent that starts and then exits closes its panel as any exit does.
4. **Lookup**: before creating anything, the daemon checks that the program exists. A program containing `/` must be an executable file; otherwise each directory of the `PATH` in the session's environment (from the client that created the session; `PATH` is not refreshed on attach; never the daemon's) is searched for an executable file with that name. The lookup runs on tokio's blocking pool. A miss returns `ActionError::ProgramNotFound { program, kind, key }`, no panel is created and focus does not change. Text: `"<program>" not found in PATH (set [agents.<kind>] <key>)`, where `key` is the setting that supplied the program: `command` here, `resume` for DECK-11's launches.
5. **Spawn failure** after a successful lookup (for example a project directory that no longer exists) returns `ActionError::LaunchFailed(text)` with the OS error, and creates nothing.
6. **No room checks** apply (a new panel has one terminal).
7. **Persistence**: the terminal is saved and restored as an explicit-program terminal (DECK-04). When the program's name is in the kind's `process` list (the default `command` always is), DECK-06 detects the agent and sub-spec 05, requirement 6 resumes it after a daemon restart; for a `command` outside the list the agent is not detected and a restart relaunches it fresh. The docs say so.
8. **`deck new-panel -- <program>`** (DECK-05) is unchanged: it passes its own program and does not use `command`.

## Acceptance criteria

- [ ] `NewPanelWith { launch: Agent(Claude) }` with a fake `claude` script on `PATH` (it prints `$PWD` and waits) creates a panel whose terminal runs that script in the project root; `deck ls --json` shows the program, and the new panel is active.
- [ ] With `[agents.claude] command = "claude --model fast"` the fake script receives `--model fast`.
- [ ] `command` with `$HOME`, quotes or a backslash, or empty, is rejected at config load naming `agents.claude.command`; `deck config default` output keeps matching the sync test.
- [ ] `Agent(OpenCode)` with no `opencode` on the session's `PATH` returns `ProgramNotFound` with the exact text above (`command` as the key), and the panel count is unchanged.
- [ ] The `PATH` searched is the session's (the creating client's), not the daemon's and not a later attaching client's: a fake program only on the creating client's `PATH` is found, one only on the daemon's or an attacher's is not.
- [ ] A `command = "bin/claude"` (relative with a `/`) is rejected at config load naming `agents.claude.command`.
- [ ] After `deck kill-server` and `deck`, a panel started with `Agent(Claude)` (fake `claude`, hook state with a session id) is resumed per DECK-06 sub-spec 05; with `command = "/tmp/x/myclaude"` (not in `process`) it is relaunched as the plain command.
- [ ] A `command` with an absolute path to a non-executable file returns `ProgramNotFound`.
- [ ] `NewPanel` behaves exactly as before (existing DECK-02, DECK-05 tests pass unchanged).
- [ ] An old `state.json` and an old config still load.

## Tests

- `deck-core`: `command` parsing and rejection table, `Action` and `ActionError` round trips, `default.toml` sync.
- `crates/deck-daemon/tests/`: each case above through the protocol with fake programs in a temp directory, including the session-environment `PATH`.

## Notes

- Running the agent directly keeps DECK-06's process detection working (`match_argv` sees `claude`) and avoids quoting problems; it means the panel closes when the agent quits. A user who wants a shell afterwards uses the `Terminal` item and types the command.
- A script with a bad `#!` line is not a spawn failure: portable-pty closes the exec-error pipe in the child, so the spawn reports success, the process dies at once and its panel closes like any exit.
- `ProgramNotFound` is a user-facing message; the client shows it as the usual 4 s error flash (DECK-02).
