---
type: shippable
status: todo
---

# 03 — OpenCode sessions without loading repositories (DECK-40)

## Scope

The restore dialog lists OpenCode sessions of the user's repositories again, without OpenCode running code from them. Owning crate: `deck-daemon` (`crates/deck-daemon/src/server/listing.rs`).

Why: before DECK-26, deck ran `opencode session list` in every space root. In OpenCode 2.4.1 (commit `6f05140`) that boots the directory in OpenCode's background service: `session list` calls `location.get` with its working directory, whose location middleware builds the instance with discovery on (`packages/server/src/location.ts:75-85`, `packages/core/src/instance.ts:63-120`); plugins are imported (`packages/core/src/plugin/supervisor.ts:146-173`) and MCP servers started (`packages/core/src/mcp/index.ts:905-926`); read from source, not confirmed by a run. DECK-26 closed it by listing only from `~`, which hides the sessions of repositories.

## Requirements

1. One `opencode api GET "/api/session?parentID=null&order=desc&limit=N"` run from `~` (an endpoint that boots no location), keeping the sessions whose directory is under a space root.
2. A parser for the `{data, cursor}` answer, paging with `cursor` until N sessions match or a page cap is reached, all inside the 8 s deadline (DECK-26's `Reaped` guard and deadline stay).
3. Confirmed by a run against the OpenCode installed on the user's machine first: the field names, that the unfiltered list returns every project's sessions, and what a version without `api` does; those fall back to today's `~`-only list.

## Acceptance criteria

- [ ] A test with a fake `opencode` proves no call runs with a space root as working directory and that sessions are filtered by directory.
- [ ] A fake whose first page has no match still yields the matches of later pages; the page cap and the deadline hold.
- [ ] The fallback for an `opencode` without `api` is tested.
- [ ] `docs/features/privacy.md` (its lines on `opencode session list` in the home directory), `docs/features/agents.md` and the privacy report are updated.

## Tests

`crates/deck-daemon/tests/agent_sessions.rs` with fake `opencode` scripts; an e2e check in `crates/deck/tests/restore.rs` that the restore dialog lists a repository's session.
