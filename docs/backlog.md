# Backlog

Read when you decide what to build next, pick up a task, or add, start or finish one.

Every task has an ID, a status and the tasks it depends on. A task is either a **simple task**, described by its note below, or **substantial**, which links its spec ([workflow.md](workflow.md#triage) says how to tell them apart). A spec's own progress lives in its sub-spec files ([specs/README.md](specs/README.md#status-rules)); the task's status summarizes it.

## Queue

Every open task, in the order the user set on 22 Nov 2026. "The next task" is picked from here ([Picking the next task](#picking-the-next-task)).

| #   | Task              | Work                                                          |
| --- | ----------------- | ------------------------------------------------------------- |
| 1   | [DECK-01](#tasks) | Core: workspace, daemon, PTYs, attach and detach one terminal |
| 2   | [DECK-02](#tasks) | Sessions, projects and panels, with the sidebar               |
| 3   | [DECK-08](#tasks) | Config file: theme, live reload, `deck config` commands       |
| 4   | [DECK-03](#tasks) | Several terminals per panel in the automatic grid, resizable  |
| 5   | [DECK-04](#tasks) | Sessions survive a daemon restart                             |
| 6   | [DECK-05](#tasks) | Control API: the `deck` CLI for agents and scripts            |
| 7   | [DECK-06](#tasks) | Agent status in the sidebar, hooks, resume after restart      |
| 8   | [DECK-07](#deck-07) | Panels named after what their agent is doing                |

### Not ordered yet

Open tasks the user has not placed in the queue: none.

## Tasks

| ID      | Title                                     | Status     | Depends on       | Spec                                                                  |
| ------- | ----------------------------------------- | ---------- | ---------------- | --------------------------------------------------------------------- |
| DECK-01 | Core daemon and terminal                  | ready      | —                | [core-daemon-terminal](specs/2026-11-22-core-daemon-terminal/00-overview.md) |
| DECK-02 | Sessions, projects, panels and sidebar    | ready      | DECK-01          | [sessions-sidebar](specs/2026-11-22-sessions-sidebar/00-overview.md)  |
| DECK-03 | Grid layout                               | ready      | DECK-02          | [grid-layout](specs/2026-11-22-grid-layout/00-overview.md)            |
| DECK-04 | Persistence across daemon restarts        | ready      | DECK-03, DECK-08 | [persistence](specs/2026-11-22-persistence/00-overview.md)            |
| DECK-05 | Control API                               | ready      | DECK-03          | [control-api](specs/2026-11-22-control-api/00-overview.md)            |
| DECK-06 | Agent status, hooks and resume            | ready      | DECK-05, DECK-04 | [agent-status](specs/2026-11-22-agent-status/00-overview.md)          |
| DECK-07 | Auto-named panels                         | todo       | DECK-06          | — (simple task, [note](#deck-07))                                     |
| DECK-08 | Config and theming                        | ready      | DECK-02          | [config-theming](specs/2026-11-22-config-theming/00-overview.md)      |

### Withdrawn

None.

## Notes

### DECK-07

**Auto-named panels.** Asked on 22 Nov 2026: when an agent takes over a terminal, the panel's name in the sidebar follows what the agent is doing.

- What: a panel whose `name_source` is not `manual` takes its name from its first terminal (in grid order) that has an agent. Source, in order: the agent's terminal title (OSC 0/2) once it differs from the bare program name, with leading spinner or status glyphs (any non-alphanumeric prefix) stripped; otherwise the first prompt of the agent's session from its `UserPromptSubmit` hook event (DECK-06), first line only. Names are trimmed to 32 characters with `…`. The `name_source` becomes `auto`. A manual rename (`prefix ,` or `deck rename-panel`) sets `manual` and is never overwritten; `deck rename-panel --auto` (a new flag this task adds to DECK-05's command) returns the panel to `auto`. When the agent exits, the auto name stays until another agent renames it.
- Why: panels full of agents all read "claude" otherwise.
- Acceptance: unit tests in `deck-daemon` for the title cleanup, the source order, the 32-character cut and the manual lock; an end-to-end test where a fake agent script sets its title with OSC 2 and its panel's sidebar row shows the new name, and where a manual rename then survives another title change; `docs/features/agents.md` (created by DECK-06) describes it.

## Statuses

A status says where the task stands. Dependencies are a separate column and never change the status.

| Status        | Meaning                                                                                       |
| ------------- | --------------------------------------------------------------------------------------------- |
| `todo`        | A simple task, described in its note. The next step is to do it.                              |
| `needs spec`  | Agreed substantial work with no spec yet. The next step is to write one.                      |
| `draft spec`  | A spec is written and waits for the user's approval.                                          |
| `ready`       | The spec is approved and no work on it has started.                                           |
| `in-progress` | Work has started: a worktree or branch exists, or a sub-spec is committed.                    |
| `done`        | A simple task is committed; a spec has every sub-spec `done` and its branch merged.           |

## Dependencies

"Depends on" lists task IDs. A dependency is met when that task is `done`. `<ID> sub-spec NN` is met when that sub-spec of the task's spec is `done`.

## Picking the next task

The [queue](#queue) is the priority order, set by the user.

- **"The next task"**: walk the queue from the top. An `in-progress` task is resumed first (after the check in [workflow.md](workflow.md#work) that no other session is on it). Otherwise pick the first task whose status is `todo`, `ready`, `draft spec` or `needs spec` and whose dependencies are all met. Say which earlier tasks were skipped and which dependency holds each back.
- Tasks under [Not ordered yet](#not-ordered-yet) are never picked as "the next task"; ask the user to place them.
- **A named task** (`DECK-04`) is built as named. If a dependency is not met, report it and ask before going on.
- For `todo`, the lead does the task ([workflow.md](workflow.md#work)). For `needs spec`, it writes the spec from the task's note and asks for approval. For `draft spec`, it summarizes the written spec and asks for approval. For `ready` and `in-progress`, it builds or resumes it.
- Tasks can run at the same time, in separate sessions, when neither depends on the other.

## Keeping it current

Only the lead session changes a task's status, and always in a commit on `main`, so every checkout sees it:

- New simple task: a `todo` row and a note (what, why, acceptance). It turns `done` in the task's own commit.
- New agreed substantial work with no spec: a `needs spec` row and a note on what the user asked for and when.
- A new spec: a `draft spec` row (or the `needs spec` row turned into one), and the `backlog:` line in its overview.
- Spec approved: `ready`, in the `docs(spec): add <slug> spec` commit.
- Work starts: `in-progress`, in a `docs(spec): start <slug>` commit on `main` when the worktree is created.
- Merge: `done`.

The queue changes in the same commits. A new task enters the queue where the user says; when the user names no place, it goes under [Not ordered yet](#not-ordered-yet) and the lead asks. A task leaves the queue in the commit that sets it `done` or withdraws it. Only the user reorders the queue. IDs never change and are never reused.
