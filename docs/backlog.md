# Backlog

Read when you decide what to build next, pick up a task, or add, start or finish one.

Every task has an ID, a status and the tasks it depends on. A task is either a **simple task**, described by its note below, or **substantial**, which links its spec ([workflow.md](workflow.md#triage) says how to tell them apart). A spec's own progress lives in its sub-spec files ([specs/README.md](specs/README.md#status-rules)); the task's status summarizes it.

## Queue

Every open task, in the order the user set on 22 Nov 2026. "The next task" is picked from here ([Picking the next task](#picking-the-next-task)). The list order is the priority; adding or removing an item needs no renumbering.

1. [DECK-01](#tasks): core: workspace, daemon, PTYs, attach and detach one terminal
1. [DECK-02](#tasks): sessions, projects and panels, with the sidebar
1. [DECK-08](#tasks): config file: theme, live reload, `deck config` commands
1. [DECK-03](#tasks): several terminals per panel in the automatic grid, resizable
1. [DECK-04](#tasks): sessions survive a daemon restart
1. [DECK-05](#tasks): control API: the `deck` CLI for agents and scripts
1. [DECK-06](#tasks): agent status in the sidebar, hooks, resume after restart
1. [DECK-07](#deck-07): panels named after what their agent is doing

### Not ordered yet

Open tasks the user has not placed in the queue: none.

## Tasks

| ID      | Title                                     | Status     | Depends on       | Spec                                                                  |
| ------- | ----------------------------------------- | ---------- | ---------------- | --------------------------------------------------------------------- |
| DECK-01 | Core daemon and terminal                  | ready      | —                | [core-daemon-terminal](specs/2026-11-22-core-daemon-terminal/00-overview.md) |
| DECK-02 | Sessions, projects, panels and sidebar    | ready      | DECK-01          | [sessions-sidebar](specs/2026-11-22-sessions-sidebar/00-overview.md)  |
| DECK-03 | Grid layout                               | ready      | DECK-02, DECK-08 | [grid-layout](specs/2026-11-22-grid-layout/00-overview.md)            |
| DECK-04 | Persistence across daemon restarts        | ready      | DECK-03, DECK-08 | [persistence](specs/2026-11-22-persistence/00-overview.md)            |
| DECK-05 | Control API                               | ready      | DECK-03, DECK-04 | [control-api](specs/2026-11-22-control-api/00-overview.md)            |
| DECK-06 | Agent status, hooks and resume            | ready      | DECK-05, DECK-04 | [agent-status](specs/2026-11-22-agent-status/00-overview.md)          |
| DECK-07 | Auto-named panels                         | todo       | DECK-06          | — (simple task, [note](#deck-07))                                     |
| DECK-08 | Config and theming                        | ready      | DECK-02          | [config-theming](specs/2026-11-22-config-theming/00-overview.md)      |

### Withdrawn

A task that will not be built moves here from the Tasks table, with the date and the reason, in a commit on `main` (`docs(backlog): withdraw DECK-NN`, made only when the user asks). It keeps its ID and is never picked.

None.

## Notes

### DECK-07

**Auto-named panels.** Asked on 22 Nov 2026: when an agent takes over a terminal, the panel's name in the sidebar follows what the agent is doing.

- What: a panel whose `name_source` is not `manual` takes its name from its first terminal (in grid order) that has an agent. Source, in order: the agent's terminal title (OSC 0/2), with leading spinner or status glyphs (any non-alphanumeric prefix) stripped, once it differs (case-insensitively) from the bare program name and the product names (`claude`, `Claude Code`, `codex`, `Codex`, `opencode`, `OpenCode`); otherwise the first prompt of the agent's session from its `UserPromptSubmit` hook event (DECK-06), first line only. Names are cut to 32 display cells (by Unicode width) with `…`. The naming logic is pure and lives in `deck-core`. The `name_source` becomes `auto`. A manual rename (`prefix ,` or `deck rename-panel`) sets `manual` and is never overwritten; `deck rename-panel --auto` (a new flag this task adds to DECK-05's command) returns the panel to `auto`. When the agent exits, the auto name stays until another agent renames it.
- Why: panels full of agents all read "claude" otherwise.
- Acceptance: unit tests in `deck-core` for the title cleanup (including product-name titles), the source order, the 32-cell cut with wide characters and the manual lock; an end-to-end test where a fake agent script sets its title with OSC 2 and its panel's sidebar row shows the new name, and where a manual rename then survives another title change; `docs/features/agents.md` (created by DECK-06) describes it.

## Statuses

A status says where the task stands. Dependencies are a separate column and never change the status.

| Status        | Meaning                                                                                       |
| ------------- | --------------------------------------------------------------------------------------------- |
| `todo`        | A simple task, described in its note. The next step is to do it.                              |
| `needs spec`  | Agreed substantial work with no spec yet. The next step is to write one.                      |
| `draft spec`  | A spec is written and waits for the user's approval.                                          |
| `ready`       | The spec is approved and no work on it has started.                                           |
| `in-progress` | Work has started: a worktree or branch exists, or a sub-spec is committed. Another session may own it. |
| `done`        | A simple task is committed; a spec has every sub-spec `done` and its branch merged.           |

## Dependencies

"Depends on" lists task IDs. A dependency is met when that task is `done` on `main` (merged). `<ID> sub-spec NN` is met only once that task is merged too: a sub-spec committed on an unmerged branch does not count.

## Picking the next task

The [queue](#queue) is the priority order, set by the user.

- **"The next task"**: read the backlog on `main` and walk the queue from the top. When a task is `in-progress`, ask the user one question: resume it here, or leave it (another session may own it) and take the next eligible task. Otherwise pick the first task whose status is `todo`, `ready`, `draft spec` or `needs spec` and whose dependencies are all met. Say which earlier tasks were skipped and which dependency holds each back.
- Tasks under [Not ordered yet](#not-ordered-yet) are never picked as "the next task"; ask the user to place them.
- **A named task** (`DECK-04`) is built as named. If a dependency is not met, report it and ask before going on.
- For `todo`, the lead does the task ([workflow.md](workflow.md#work)). For `needs spec`, it writes the spec from the task's note and asks for approval. For `draft spec`, it summarizes the written spec and asks for approval. For `ready` and `in-progress`, it builds or resumes it.
- Tasks can run at the same time, in separate sessions, when neither depends on the other.

## Keeping it current

Only the lead session changes a task's status, always in the main checkout and in a commit on `main` made at once, so every session sees it ([the commits](workflow.md#commits-the-flow-may-make)):

- New simple task: a `todo` row, its "Depends on" and a note (what, why, acceptance), committed as `docs(backlog): add DECK-NN`. It turns `done` in the task's own commit.
- New agreed substantial work with no spec: a `needs spec` row and a note on what the user asked for and when (`docs(backlog): add DECK-NN`).
- A new spec: a `draft spec` row (or the `needs spec` row turned into one) and the `backlog:` line in its overview, committed with the draft (`docs(spec): draft <slug>`).
- Spec approved: `ready`, in the `docs(spec): add <slug> spec` commit.
- Work starts: `in-progress`, in a `docs(spec): start <slug>` commit on `main` when the worktree is created.
- Merge: `done`, in the squash-merge commit. Discard: back to `ready` (`docs(spec): stop <slug>`).

A new ID is the highest ID ever used (Tasks and Withdrawn) plus one.

The queue changes in the same commits. A new task enters the queue where the user says; when the user names no place, it goes under [Not ordered yet](#not-ordered-yet) and the lead asks. A task leaves the queue in the commit that sets it `done` or withdraws it. Only the user reorders the queue. IDs never change and are never reused.
