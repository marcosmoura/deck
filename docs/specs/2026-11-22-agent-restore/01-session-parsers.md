---
type: foundation
status: done
---

# 01 — Agent session parsers

## Scope

Nothing to try yet: the pure code that turns an agent's files into a list of sessions, and the helpers the dialog needs. Sub-spec 02 reads the files and calls it; sub-spec 03 draws it. Owning crate: `deck-core` (module `agent_sessions`).

## Requirements

1. **Type**: `AgentSession { kind: AgentKind, id: String, title: String, cwd: PathBuf, updated: u64 }` (`updated` in epoch seconds), serde, with `cwd` on the wire as bytes through `deck-core::wire` (a non-UTF-8 path round-trips), with a protocol round-trip test (the protocol itself is sub-spec 02's).
2. **Claude Code**: `parse_claude(file_stem: &str, head: &[u8], mtime: u64) -> Option<AgentSession>`. `head` is the first bytes of a `.jsonl` transcript; the last line is dropped when it is incomplete. Lines that are not JSON objects are skipped. `id` is `file_stem`, which is what `claude --resume` takes, and `parse_claude` returns `None` when the stem is not a UUID (older subagent files are named `agent-<id>.jsonl`; `is_claude_session_file(name)` is the pure check the daemon uses before choosing candidates); `cwd` is the first non-empty `cwd` string; `title` is the first line with `type: "summary"` and a `summary` string, else the first `type: "user"` line's `message.content` (a string, or the text of the first `{type: "text"}` block whose trimmed text is not empty and does not start with `<`), first line only. User text that starts with `<` (command and system wrappers, including IDE blocks) or is empty is skipped. A line with `isSidechain: true` or `isMeta: true` is skipped for the title. `None` when there is no id, no cwd or no title. `updated` is `mtime`.
3. **Codex**: `parse_codex(file_name: &str, head: &[u8], mtime: u64) -> Option<AgentSession>`. The first line must be `type: "session_meta"`; `id` and `cwd` come from its `payload`, the id falling back to the UUID at the end of `rollout-<timestamp>-<id>.jsonl`. `title` is the `payload.message` of the first line with `type: "event_msg"` and `payload.type: "user_message"`, first line only. `None` without id, cwd or title. `updated` is `mtime`.
4. **OpenCode**: `parse_opencode_list(json: &[u8]) -> Vec<AgentSession>` for the array printed by `opencode session list --format json` (`id`, `title`, `updated`, `directory`; others ignored). Entries with an empty title, id or directory are skipped; `updated` is accepted in seconds or milliseconds (above 10¹¹ is milliseconds). A body that is not a JSON array gives an empty list.
5. **Titles** are cut to 80 display cells (Unicode width) with `…`, control characters replaced by spaces, runs of whitespace collapsed.
6. **Ids** failing DECK-06's session-id rule (`^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$`, one shared function) are dropped by every parser.
7. **Ordering and bound**: `recent(sessions, limit)` sorts by `updated` descending (ties by id), removes duplicate ids keeping the newest, and keeps `limit`.
8. **Filter**: `matches(session, query)`: every whitespace-separated word of `query` must be a case-insensitive substring of the title or of the directory. An empty query matches everything.
9. **Project choice**: `project_for(cwd, projects: &[(ProjectId, PathBuf)], active) -> Option<ProjectId>`: the project whose root is `cwd` or an ancestor of it (component-wise, never a string prefix), the longest root wins, a tie goes to `active`; none matching gives `active`; no projects gives `None`.
10. **Age text**: `age(now, updated) -> String`: `now` under a minute, `5m`, `3h`, `2d`, `6w`, `8mo`, `2y` (each with ` ago` added by the caller), clamped at `0` for a future time.
11. **Display directory**: `short_dir(cwd, home) -> String`: `~` for the home prefix, otherwise the path; the caller truncates from the left with `…`.

## Acceptance criteria

- [ ] Claude fixtures: an `agent-<id>.jsonl` file (`None`), a transcript whose `sessionId` differs from its stem (the stem wins), a transcript whose first user line has `isMeta: true`, a transcript with a leading `summary` line, one without (user prompt title), one whose first user lines are `<command-name>` wrappers, one with a sidechain line first, one with a string `content` and one with a block list; an empty file; a file with only assistant lines; a file whose last line is cut mid-JSON.
- [ ] Codex fixtures: a normal rollout, one without a user message (`None`), one with the id only in the file name, one whose `session_meta` is not first (`None`).
- [ ] OpenCode fixtures: seconds and milliseconds timestamps, an empty title, a non-array body, an extra unknown field.
- [ ] A title of 200 characters, one with East Asian wide characters, one with newlines and tabs, all come out within 80 cells on one line.
- [ ] An id such as `-p` or `a b;rm` is dropped; a UUID and `ses_0123abc` pass.
- [ ] `recent` removes a duplicate id and keeps the newest; `limit` bounds the result.
- [ ] `matches("fix login", …)` matches a title `Fix the login bug` and a directory filter `budget`; case does not matter; `""` matches all.
- [ ] `project_for`: `/home/u/work/api/src` picks root `/home/u/work/api` over `/home/u/work`; `/home/u/work2` does not match root `/home/u/work`; no match returns the active project.
- [ ] `age` table: 0 s, 59 s, 60 s, 3599 s, 3 h, 47 h, 6 d, 13 d, 400 d, a future time.

## Tests

- `deck-core` unit tests for each requirement, with the fixtures in `crates/deck-core/tests/fixtures/agent-sessions/` (small hand-written files in the documented shapes, never copies of a user's real sessions; no secrets or real paths).

## Notes

- Fixtures are hand-written to the shapes in the overview's decisions on Claude Code's layout and Codex's confirmed format. If a real file ever shows a different shape, the parser accepts both and keeps the old fixture.
- Parsers take bytes, not paths, so they never do I/O and cannot panic on arbitrary input (the workspace denies `unwrap` and indexing).
