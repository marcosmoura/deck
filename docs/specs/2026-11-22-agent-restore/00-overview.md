---
branch: feat/agent-restore
backlog: DECK-11
---

# Restore agent sessions

## Goal

The `Restore` button at the right of the sidebar's bottom bar opens a dialog with three columns, Claude Code, OpenCode and Codex, each listing that agent's recent sessions. The user picks one and deck reopens it, resumed with the agent's own command, in a new panel in the right project and directory.

## Builds on

- [DECK-10 sidebar-mouse](../2026-11-22-sidebar-mouse/00-overview.md): the bottom bar and its reserved `Restore` slot, hover state, `Action::NewPanelWith`, `PanelLaunch` and `ProgramNotFound` (sub-specs 02 and 03).
- [DECK-06 agent-status](../2026-11-22-agent-status/00-overview.md): `AgentKind`, the `[agents.<kind>]` tables, the `resume` templates and the session-id rule (sub-spec 05, requirements 3 and 4).
- [DECK-02 sessions-sidebar](../2026-11-22-sessions-sidebar/00-overview.md): overlays and the picker's style ([03-session-picker](../2026-11-22-sessions-sidebar/03-session-picker.md)), projects with root directories.
- [architecture.md](../../architecture.md): daemon off-thread work, client, protocol, platform paths.

## Non-goals

- Restoring a session that is still open in a deck terminal (it is listed like any other; resuming it starts a second copy, as the agent allows).
- Deleting, renaming or exporting agent sessions, or reading conversations (only a title, a directory and a time are read).
- Sessions of other tools, or of agents on another machine.
- Searching message bodies: the filter matches the title and directory only.
- Restore after a daemon restart: that is DECK-06's automatic resume.

## Decisions

- **Read in the daemon, off the connection.** It knows the agent config (`resume` templates) and reads each agent's directory with the session's environment (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `HOME`; [architecture.md#model](../../architecture.md#model), Environment). One request per agent (`ListAgentSessions { kind, limit }`), each run in its own spawned task with file I/O on `spawn_blocking`, so a slow agent never delays the other two columns nor the connection's other messages. `AgentSessions` replies are unordered and routed by `kind`.
- **Parsing is pure and in `deck-core`**, over bytes and file metadata passed in, tested with fixture files; the daemon only walks directories and reads at most 256 KiB from the head of each file.
- **Bounded work**: the newest 50 sessions per agent (by file modification time, or the agent's own `updated`), no age limit, and no file is read in full. A file that cannot be read or parsed is skipped, never an error for the whole list.
- **Titles**: the agent's own title or summary when its file has one, else the first user prompt, first line, cut to 80 cells; sessions with neither are not listed (nothing to resume into).
- **Where it opens**: the project whose root contains the session's directory (the longest matching root, ties to the active project); no match means the active project; no project at all means the flash `no project to restore into` and nothing opens. The terminal starts in the session's directory when it exists, else the project root (with a flash). The launch is a new panel (`Action::NewPanelWith` with DECK-10's `PanelLaunch::Resume`), run directly with the agent's `resume` template, no shell.
- **Session ids** that fail DECK-06's id rule are not listed (the id goes into a command line).
- **Sources** (Codex and OpenCode confirmed with DeepWiki; Claude Code's layout is a decision, below):

  | Agent       | Location                                                                                       | How it is read                                                                                   |
  | ----------- | ---------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
  | Claude Code | `$CLAUDE_CONFIG_DIR/projects/<dir>/<id>.jsonl`, default `~/.claude/projects/`                  | JSONL transcripts: directory from a line's `cwd`, id is the file stem (a UUID)                   |
  | Codex       | `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<timestamp>-<id>.jsonl`, default `~/.codex/sessions/` | JSONL rollouts: first line `session_meta` (`id`, `cwd`, `timestamp`); first `user_message` event |
  | OpenCode    | SQLite database in `~/.local/share/opencode/` (`opencode db path`)                             | not read directly: the `opencode session list --format json` command (open question)             |

## Sub-specs

| Sub-spec                    | Type       | Crate                  | Summary                                                                                          | Depends          | Status |
| --------------------------- | ---------- | ---------------------- | ------------------------------------------------------------------------------------------------ | ---------------- | ------ |
| [01](01-session-parsers.md) | foundation | deck-core              | Pure parsers for the three sources, ordering, filter, project choice, age text; fixtures         | DECK-06          | todo   |
| [02](02-daemon-listing.md)  | foundation | deck-core, deck-daemon | `ListAgentSessions`, directory walks on the blocking pool, OpenCode via its CLI, `Resume` launch | 01, DECK-10 (03) | todo   |
| [03](03-restore-dialog.md)  | shippable  | deck-tui, deck-core    | The `Restore` button and the three-column dialog; resume into a new panel                        | 02, DECK-10 (02) | todo   |

## Affected docs

- `docs/features/sidebar.md`: the `Restore` button.
- `docs/features/agents.md` (created by DECK-06): restoring sessions, where each agent's sessions are read from, the 50-session bound.
- `docs/features/keys.md`: the dialog's keys and the `restore` binding.
- `docs/architecture.md#agents`: session listing in the daemon; `#protocol`: `ListAgentSessions` and its response, which is the one reply that arrives out of order (routed by `kind`, answered from a spawned task).
- `docs/features/agents.md`: `resume` (DECK-06) and `command` (DECK-10) are separate settings; a custom `command` path also needs a matching `resume`.

## Decisions on Claude Code's layout

The layout is not in anything public that deck can read (DeepWiki could not confirm it), so it is decided here, and the spec-reviewer confirmed it matches current Claude Code: `projects/<cwd with `/`and`.`replaced by`-`>/<session-uuid>.jsonl`, one JSON object per line with `type` (`user`, `assistant`, `summary`), `sessionId`, `cwd`, `timestamp`, `isSidechain`, `isMeta`, and for user lines `message.content` as a string or a list of `{type: "text", text}` blocks. The parser never decodes the directory name (it reads `cwd` from the lines), accepts every shape above and skips what it does not understand. Fixtures are hand-written to this shape; nobody runs `claude` for them (no login, no network).

## Open questions

1. **OpenCode stores sessions in SQLite** (`opencode.db`, a `session` table with `id`, `title`, `directory`, `parent_id`, `time_created`, `time_updated`), not in JSON files, in current versions. Reading it directly needs a SQLite crate (`rusqlite`, with the bundled library) and is tied to a schema that changes. `opencode session list --max-count <n> --format json` exists and prints an array of `{ id, title, updated, created, projectId, directory }`, newest first, but only for the project of its working directory. **Recommendation:** use the command, no new dependency: run it in the root of every project of the session (deduplicated by directory) and in the home directory, at most 2 at a time inside one overall 8 s timeout, with the session's environment, and merge by `id`. Sessions of a project that is not a deck project root are therefore missing; the docs say so. The unit of `updated` and `created` is unconfirmed: the parser accepts epoch seconds or milliseconds (values above 10¹¹ are milliseconds). Revisit with `rusqlite` only if the user wants OpenCode sessions from directories deck does not know.
2. **Codex's rollout files** are confirmed to live under `$CODEX_HOME/sessions/YYYY/MM/DD/` as `rollout-<timestamp>-<id>.jsonl`, the first line having `type: "session_meta"` with a `payload` holding `id`, `cwd` and `timestamp`, and the first user prompt being an `event_msg` whose payload has `type: "user_message"` and `message`. **Recommendation:** use exactly that, take the id from the payload and fall back to the file name's trailing UUID. Newer Codex versions may keep an index elsewhere; deck does not read it. Candidates are chosen by file name (creation date) but sorted by modification time, so an old session resumed recently can be missed when more than `limit × 2` newer rollouts exist; the docs say so.
3. **New dependency:** none under the recommendations above. `time` formatting ("2h ago") is done by hand in `deck-core`.

Reviewed by spec-reviewer on 22 Nov 2026.

Approved on 22 Nov 2026 by the user's instruction to build every remaining backlog item, including this one, without a separate approval round.
