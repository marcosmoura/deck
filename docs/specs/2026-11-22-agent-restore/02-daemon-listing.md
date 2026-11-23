---
type: foundation
status: done
---

# 02 — Listing sessions and resuming in the daemon

## Scope

Nothing to try yet: the daemon lists each agent's recent sessions on request and can start a panel that resumes one. Sub-spec 03's dialog uses both. Owning crates: `deck-core` (protocol, `PanelLaunch::Resume`), `deck-daemon` (directory walks, the OpenCode command, the launch).

## Requirements

1. **Protocol** (version bumps): `Request::ListAgentSessions { kind: AgentKind, limit: u16 }` answered by `Response::AgentSessions { kind, sessions: Vec<AgentSession>, error: Option<String> }`. `limit` is clamped to 1–100; the dialog sends 50. The request needs an attached session (it uses its environment and projects); from a control client it fails with `no session`. **Replies are unordered**: today `dispatch` awaits each message in turn and the client's picker matches `Response`s first in, first out, so `ListAgentSessions` must not go through either. The daemon spawns one task per request and `dispatch` returns at once; the task pushes `Response::AgentSessions` onto the client's queue when done, and the client routes it by `kind`, never through the picker's FIFO `requests`. Every failure (timeout, missing command, unreadable root) comes back as `AgentSessions { error }`, never as `Response::Error`. Round-trip tests.
2. **Claude**: the root is `$CLAUDE_CONFIG_DIR/projects` from the session environment, else `$HOME/.claude/projects`. The daemon lists `projects/*/*.jsonl` (one level, no recursion, so subagent directories are not walked), keeps only files whose stem is a UUID (`is_claude_session_file`, sub-spec 01), takes the newest `limit × 2` of them by modification time, reads at most 256 KiB from the head of each, and gives them to `parse_claude`; the result goes through `recent`. A missing root is an empty list, not an error.
3. **Codex**: the root is `$CODEX_HOME/sessions` from the session environment, else `$HOME/.codex/sessions`. The daemon lists the year, month and day directories in descending name order and the `rollout-*.jsonl` files in each in descending name order, and stops once it has `limit × 2` candidates. Candidates are chosen by file name (creation) but sorted by modification time, so an old session resumed recently can be missed when more than `limit × 2` newer rollouts exist (documented); it reads each rollout line by line, keeping only the lines `parse_codex` needs (the first line, then up to the first `user_message`), stops at the first `user_message` that gives a title, and reads at most 4 MiB per file (a fixed 256 KiB head would drop rollouts whose first line carries large `instructions` or that have response items before the first prompt). It calls `parse_codex`, then `recent`. A missing root is an empty list.
4. **OpenCode**: the daemon runs `opencode session list --max-count <limit> --format json` (the program from `[agents.opencode] command`, first word only) with each distinct project root of the session and the home directory as the working directory, at most 2 children at a time, all inside one overall 8 s timeout (children still running are killed when it ends, and their directories contribute nothing). The program is found by the operating system's own search, run with the session's `PATH` set on the child (not the daemon's), and the child runs with the session's environment (`HOME`, `XDG_DATA_HOME`, `PATH`), not the daemon's; stdout is capped at 4 MiB, stderr is discarded and there is no input. The outputs go through `parse_opencode_list`, are merged and passed to `recent`. A command counts as successful only when it exits with status 0, prints less than the 4 MiB cap and prints a JSON array; one that is missing, fails, times out or prints anything else contributes nothing. The two-children limit holds daemon-wide, across concurrent requests; when all of them failed, `error` is `could not run opencode session list` (or `timed out`), and the dialog shows it in that column.
5. **Where the work runs**: each request runs in its own spawned task. Every directory walk and file read inside it runs on `spawn_blocking`; the OpenCode children run through `tokio::process` with the timeout inside that task. Nothing runs under the state lock or inside `dispatch`. Reads are synchronous and bounded (256 KiB per Claude file, 4 MiB per Codex file, at most `limit × 2` files, 4 MiB per command); a file that fails to open or read is skipped. The daemon does not cache results; each request lists again.
6. **Resume launch**: `PanelLaunch::Resume { kind, session_id, cwd }` (a new variant of DECK-10's `PanelLaunch`, sent in `Action::NewPanelWith`). The daemon checks `session_id` with DECK-06's id rule, renders the agent's `resume` template (DECK-06 sub-spec 05, requirement 3) into a `LaunchSpec { program: Some(..), args, cwd }` and calls DECK-05's `NewPanel` function with it, so the terminal is saved and restored as an explicit-program terminal and DECK-06 sub-spec 05, requirement 6 resumes it again after a daemon restart (the template's program is matched by `match_argv`). The terminal starts in `cwd` when it is an existing directory, else in the project root, and the daemon then sends a `Warning` notice `<dir> no longer exists; started in <project root>`. The program lookup and `ProgramNotFound` (DECK-10 sub-spec 03) apply to the template's program. An invalid id returns `ActionError::InvalidSessionId`.
7. **Privacy**: titles and directories are sent only to the client that asked, never logged, and never written to `state.json`. The daemon logs counts and timings only.
8. **No change to the persistence of panels**: a resumed panel is saved like any panel with an agent (DECK-06 sub-spec 05).

## Acceptance criteria

- [ ] With a temp `CLAUDE_CONFIG_DIR` holding three fixture transcripts in two project directories, the Claude response lists them newest first with the right titles and directories; a transcript touched later moves to the top.
- [ ] With a temp `CODEX_HOME` holding rollouts in three day directories, the Codex response is newest first and stops reading older day directories once `limit × 2` candidates exist (checked by a counter in a test hook that counts files opened).
- [ ] The directories come from the session's environment: a session created by a client with a different `HOME` and `CODEX_HOME` than the daemon's reads the session's directories.
- [ ] A fake `opencode` found only on the session's `PATH`, which also records its `HOME` and `XDG_DATA_HOME`, runs with the session's values.
- [ ] A fake `opencode` script on `PATH` that prints a JSON array only when its cwd is a project root is run for each project root and the home directory, and its sessions are merged without duplicates.
- [ ] While a 30 s fake `opencode` listing is pending, an `Input` and an `Action` on the same connection are answered at once, and the Claude and Codex answers arrive before it; the OpenCode request yields `error: "timed out"` within about 8 s (never `Response::Error`); a fake that prints garbage yields an empty list and the error text.
- [ ] A 10 MiB Claude transcript is read only up to 256 KiB, and a 10 MiB Codex rollout up to its first prompt or 4 MiB, whichever comes first, even when its first line holds 600 KiB of `instructions` (test hook counting bytes read).
- [ ] With 6 project roots, never more than 2 fake `opencode` children run at once.
- [ ] Claude candidates exclude an `agent-<id>.jsonl` file even when it is the newest.
- [ ] Missing roots give empty lists with no error.
- [ ] `PanelLaunch::Resume` with a fake `claude` that records its argv and cwd: argv is `--resume <id>`, cwd is the session's directory; with the directory removed, cwd is the project root and the notice is sent.
- [ ] A session id of `-p` or `x; ls` returns `InvalidSessionId` and creates no panel; a missing program returns `ProgramNotFound` naming `resume`.
- [ ] The daemon log after a listing contains no title or directory text.
- [ ] A list request from a control client returns `no session`.

## Tests

- `deck-core`: protocol and action round trips.
- `crates/deck-daemon/tests/`: every criterion with temp directories and fake `opencode`, `claude` scripts, plus the test hooks for the file, byte and child counters, which go through `Options.listing` (like the existing `Options.waiters`) rather than `DECK_TEST_` environment variables, since the daemon runs in-process in the tests.

## Notes

- Codex is read line by line, not as a fixed head, because newer versions put large `instructions` text in the `session_meta` line and response items can precede the first `user_message`; only the first line and the first titled `user_message` are kept in memory.
- Walking Codex's date directories in name order avoids a `stat` per file; Claude's layout has no date, so it sorts by modification time over at most a few thousand `stat` calls on the blocking pool.
- Several agents of the same kind in one directory are all listed; nothing de-duplicates by directory.
