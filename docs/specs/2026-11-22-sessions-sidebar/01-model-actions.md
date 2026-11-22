---
type: foundation
status: draft
---

# 01 — Model actions and lifecycle

## Scope

The daemon side of projects, panels and sessions: every operation the sidebar and the session picker need, as protocol actions with validation, plus the lifecycle rules and frame streaming that follows the active panel. Sub-specs 02 and 03 build the UI on it. Owning crates: `deck-core` (types, validation), `deck-daemon`; `deck`'s existing end-to-end tests are updated for the lifecycle change.

## Requirements

1. **Protocol** (`PROTOCOL_VERSION` becomes 2): `ClientMsg::Action(Action)` for an attached client, answered by `ServerMsg::ActionResult(Result<(), ActionError>)` in order, and `Request` variants for control clients. Actions:
   - `NewProject { name, root }`, `RenameProject { project, name }`, `CloseProject { project }`, `SetProjectCollapsed { project, collapsed }`, `FocusProject { project }`;
   - `NewPanel { project, name: Option<String> }`, `RenamePanel { panel, name }`, `ClosePanel { panel }`, `FocusPanel { panel }`, `CyclePanel { delta: i32 }`;
   - `RenameSession { name }`, `KillSession` (both act on the attached session).
   Requests (control and attach clients): `NewSession { name: Option<String>, cwd }`, `KillSession { name }`, `RenameSession { from, to }`, plus `ListSessions` from DECK-01. `SessionSummary` gains `projects` (project count) and a `last_attached` order key in this version bump, so sub-spec 03 needs no further protocol change. `Role::Attach` (from DECK-01, with `create`, `cols`, `rows`) is unchanged.
2. **Validation** in `deck-core` (pure, unit-tested): names are trimmed, 1–64 characters, no control characters, no `/` for project and session names. Session names are unique in the daemon, project names unique in their session; panel names are free. Errors are `ActionError` variants with a human-readable `Display` (`a project named "api" already exists`, `"/nope" is not a directory`, `name must not be empty`, …). The daemon checks that a root exists and is a directory.
3. **New panel**: one terminal launched with `program: None` in the project's root; default name as in the overview's decisions; it becomes the active panel and its project the active project.
4. **New project**: appended to the session; creates its first panel as in requirement 3 and focuses it.
5. **Close**: `ClosePanel` and `CloseProject` hang up the processes of their terminals (`SIGHUP`, `SIGKILL` after 2 s), remove them, and move focus as in requirement 7. Confirmation is the client's job.
6. **Lifecycle**: a terminal whose process exits is removed; a panel left with no terminals is removed; projects and sessions stay. `KillSession` hangs up every terminal of the session, sends `Bye { SessionEnded }` to its clients and removes it; the daemon exits when no session is left. This replaces DECK-01's temporary rule (a session ends with its last terminal); the worker updates the DECK-01 daemon test and attach criterion that relied on it, and their spec text, as listed in the overview.
7. **Focus**: each session stores its active project and panel. After a panel is removed, focus goes to the next panel of the same project, else the previous one, else stays on the project with no active panel. After a project is removed, focus goes to the next project, else the previous one, else none. `CyclePanel` walks panels in sidebar order (projects in order, their panels in order, skipping collapsed projects), wrapping around. `FocusProject` focuses the project and its last active panel, or none.
8. **Frames follow the active panel**: on a focus change, the daemon stops streaming the old panel's terminals, resizes the new panel's terminals to the latest viewport, and sends `full` frames for them. Terminals of inactive panels keep running and parsing output.
9. **SessionView** carries the session's id and name, and gains: project `root` and `collapsed`, panel `name_source`, per-terminal program name (for titles), and the session's active project and panel (`Option`s). It is sent to every client of the session after every change that alters it, including `RenameSession` from a control client.
10. **Empty states** are data only: a session with no projects or a project with no panels is a valid `SessionView`.

## Acceptance criteria

- [ ] Every action and request round-trips through the protocol, and validation returns the documented error for an empty name, a duplicate project or session name, a name with `/`, and a missing directory.
- [ ] `NewProject` creates a project, its panel `1` with a shell in the root (`pwd` prints the root), and focuses it.
- [ ] `NewPanel` twice in a project gives panels `1`, `2`, `3`; closing `2` then adding one gives `2` again.
- [ ] `ClosePanel` kills the panel's process (its pid no longer exists within 3 s) and focus moves as in requirement 7, for each case (next, previous, none).
- [ ] `exit` in a panel's only terminal removes the panel and keeps the project and session.
- [ ] `CyclePanel` order matches the sidebar order and skips collapsed projects.
- [ ] After `FocusPanel`, the client receives `full` frames for the new panel only, sized to its last viewport.
- [ ] Two clients on one session both receive the new `SessionView` after any action of either, and after a `RenameSession` request from a control client.
- [ ] `KillSession` on the last session stops the daemon; on another, the remaining sessions keep running.
- [ ] `NewSession` with an existing name fails; with none picks `main`, `main-2`, ….

## Tests

- `deck-core`: validation and default-name tests.
- `crates/deck-daemon/tests/`: one protocol-level test per daemon criterion above.
- `crates/deck/tests/attach.rs` from DECK-01 still passes, with the "session ended" scenario changed: `exit` in the last terminal now leaves the session alive with an empty project (the client stays attached). The DECK-01 daemon test for the same rule changes the same way.

## Notes

- Bumping `PROTOCOL_VERSION` is expected for every protocol change until 1.0; DECK-04 makes a restart keep the sessions.
