# Agent workflow

Read when a request changes the product or the repository, when you pick up a backlog task, or when you change the agent setup.

The session the user talks to is the **lead**. It runs this flow itself: there is no orchestrator agent. The lead owns decisions, specs, the backlog, git and the conversation with the user. Lead sessions start in the main checkout (the repository root, on `main`). Its subagents:

| Agent           | Does                                                                          | Writes            |
| --------------- | ----------------------------------------------------------------------------- | ----------------- |
| `spec-reviewer` | Checks a written spec against the request, the code, the docs and the backlog | nothing           |
| `worker`        | Implements one sub-spec with tests, then fixes the review findings for it     | code, tests, docs |
| `reviewer`      | Reviews finished work against its sub-spec or task description                | nothing           |

Subagents never start other agents and never commit. Role definitions live in `.agents/roles/<agent>.md`. The harness files only add model, effort, tools and turn limits: edit the role, not the wrapper. Every brief still tells the subagent to read its role file first, so a harness that fails to load a wrapper falls back to the role text.

| Harness     | Wrappers                                                                       | Lead               | Start a subagent                | Send it fixes                                                                       | Turn limit |
| ----------- | ------------------------------------------------------------------------------ | ------------------ | ------------------------------- | ----------------------------------------------------------------------------------- | ---------- |
| Claude Code | `.claude/agents/*.md`                                                          | the normal session | Agent tool, `subagent_type`     | SendMessage to the agent's ID                                                       | `maxTurns` |
| Codex       | `.codex/agents/*.toml` (the repository must be trusted)                        | the normal session | `spawn_agent` with `agent_type` | `followup_task` to the same agent; do not close it before its sub-spec is committed | none       |
| OpenCode    | `.opencode/agents/*.md`, permissions and `steps` in `.opencode/opencode.jsonc` | the `build` agent  | the `subagent` tool             | the `subagent` tool with the same `sessionID`                                       | `steps`    |

Read-only and "never start another agent" are enforced only partly: Claude Code reviewers have Bash, OpenCode denies only file edits to reviewers, and Codex cannot restrict a role at all. Everywhere else they hold by the role text. When a harness cannot reach a finished subagent again, start a new `worker` for the fixes and brief it with the findings and the `git diff --stat` of the work.

## Triage

A question, an explanation or a fix of a line or two: answer or do it, nothing below applies.

**A request for a backlog task** ("the next task", `DECK-04`) skips triage: it is the user's yes to the repo flow, and it authorizes the commits this workflow names for that task. Go to [Picking the next task](backlog.md#picking-the-next-task), then [Work](#work) (or [Spec](#spec) for `needs spec` and `draft spec`).

For any other request:

1. **Think it through.** Restate it in a sentence or two, then read [index.md](index.md), the docs it routes the request to, and just enough code to size it.
2. **Size it.**
   - **Simple task**: fits one focused session of under about 500k tokens. As a rule of thumb: one or two crates, a handful of files, no new protocol message or persisted field, no product decision left open, no new dependency.
   - **Substantial**: anything else. It gets a spec.
3. **Ask the user, once,** whether to run it through the repo flow. Give your sizing (simple task or spec) and put any open scope question in the same message, each with a recommended answer.
   - **No**: do the work as a normal session. No backlog row, no reviewer.
   - **Yes, simple task**: add a `todo` row to [backlog.md](backlog.md) with a note (what, why, acceptance) and its "Depends on", and commit it on `main` at once: `docs(spec): add DECK-NN`. If a dependency is not met, or the user only wanted it recorded, place it in the queue and stop. Otherwise go to [Work](#work).
   - **Yes, substantial**: go to [Spec](#spec).

A new task's ID is the highest ID ever used, in the Tasks and Withdrawn tables, plus one.

## Spec

1. **Write** `docs/specs/<yyyy-mm-dd>-<slug>/` in the format of [specs/README.md](specs/README.md), from [specs/TEMPLATE.md](specs/TEMPLATE.md). Add a backlog row with status `draft spec` (or turn the `needs spec` row into one), name it in the overview's `backlog:` line, and place it in the queue.
2. **Split** it when it is too big for one worker session. Every sub-spec is either **shippable** (the user can try something new when it lands) or **foundation** (nothing to try yet, but the base that the next shippable sub-specs build on). A sub-spec that adds or changes something on screen draws it in its `Screen` section, as a text mockup.
3. **Review.** Run `spec-reviewer` on the spec directory, and record it at the end of the overview (`Reviewed by spec-reviewer on <date>.`). Brief it with the spec path, the user's request and the backlog row. Fix its BLOCKER and MAJOR findings yourself, and take the MINOR ones that are cheap. Run it again only when a fix changed the scope or the split. Several specs can be reviewed at the same time, one `spec-reviewer` each.
4. **Commit the draft** on `main` (the spec directory and `docs/backlog.md` only): `docs(spec): draft <slug>`. Other sessions now see the row and the ID is taken.
5. **Summarize** for the user: the goal, the key decisions, the non-goals, each sub-spec on one line (shippable or foundation, crate, depends on), the open questions with a recommendation, and the spec's path. Change the spec as the user asks until they approve it. Small edits need no new `spec-reviewer` run. A later session asked to approve a `draft spec` runs `spec-reviewer` first when the overview records no review, or when `git log` shows the spec changed after the commit that recorded it.
6. **Commit on approval,** before anything else, even when the same message says to start the work: set the row to `ready`, stage only the spec directory and `docs/backlog.md` (other sessions may have work in the checkout), and commit on `main`: `docs(spec): add <slug> spec`. Then ask whether to start the work now or stop here, unless the user already said. A stop ends the session; a later session starts from the backlog task.

## Work

Work starts with "start now" after a spec, with a simple task, or in a new session asked for a backlog task (`DECK-03`, "the next task"; how to pick is in [backlog.md](backlog.md#picking-the-next-task)).

1. **Read** the backlog row on `main`, and for a spec its overview and every sub-spec that is not `done`. Once a spec has started, its sub-spec status lives on its branch: read it from the worktree (`git worktree list`). Check what is already built (`git log`, the code). Think it through: what each sub-spec needs and which decisions are still open. Ask the user only about a decision that blocks the work.
2. **Choose how:**
   - **Inline**: a simple task, or a spec whose remaining work is one small sub-spec of simple-task size. The lead does it.
   - **Delegated**: anything bigger. Each sub-spec goes to one `worker`, one sub-spec at a time, in the spec's order. Start the next as soon as the previous one is reviewed and committed. Parallel work happens across tasks, in separate sessions and worktrees, when the backlog's dependencies allow it, never inside one spec.
3. **Choose where:**
   - A simple task runs in the main checkout on `main`.
   - A spec runs in its own worktree ([Worktrees](#worktrees)). If `git worktree add` fails because the branch already exists, another session has claimed the task: re-read the backlog on `main` and ask the user. Right after creating it, set the row to `in-progress` and commit that on `main` from the main checkout: `docs(spec): start <slug>`. From then on the spec's files are edited only on its branch.
4. **Brief each worker** in a few lines: read `.agents/roles/worker.md` first; the absolute path of the checkout, the sub-spec file, the owning crate, decisions that are not in the spec, the files to start from, and the checks to run. The worker reads the spec itself; do not paste it. Set the sub-spec's `status: in-progress` in the worktree when its worker starts. Keep each worker's handle (agent ID, or OpenCode `sessionID`): its fixes go back to it.
5. **When a worker reports,** read `git diff --stat`, then only the diffs that carry the acceptance criteria, and check output filtered to the failures. A gap goes back to the same worker. If a worker stops at its turn limit, resume it; if it stops again at once, start a new worker with the diff summary and what is left.

**Waiting.** While a worker or reviewer runs, end your turn with one line saying what you are waiting for: the harness wakes you when it reports. Never sleep, poll, or read a subagent's transcript or output file. In Codex, use one long `wait_agent` instead of repeated short ones.

**Keep going.** Once work has started, do not stop between steps: continue until every sub-spec is done, reviewed and committed. Stop only for a question that blocks everything left, the fix-loop limit in [Review](#review), the choice at [Finish](#finish), or a checkpoint: after any committed sub-spec, when your context has grown large, you may stop and recommend that the user continue in a fresh session ("continue DECK-NN"), which resumes from the worktree.

## Review

After the work of a simple task, or of each sub-spec:

1. Run `just check` in the checkout first; a failure goes back to the worker (or the lead fixes inline work) before any review.
2. Run `reviewer` with "read `.agents/roles/reviewer.md` first", the checkout path, what to review (uncommitted work: `git diff HEAD` plus untracked files), and the sub-spec file or the task's description.
3. BLOCKER and MAJOR findings: when the work was inline, the lead fixes them. When it was delegated, send them to the same worker, so it fixes them with its context. Read the fix diff. Run `reviewer` again only after a BLOCKER fix. At most two fix rounds per sub-spec or task; if findings remain after that, stop and ask the user.
4. MINOR findings: fix the cheap ones, and list the rest in the summary.
5. Run `just check` again, then commit: one commit per sub-spec or simple task (`feat(<scope>): …` or `fix(<scope>): …`), holding its code, tests, docs, the sub-spec's `status: done` and the overview's Status column. Stage only its paths, since other sessions may have work in the same checkout. A simple task's commit also sets its row to `done` and removes it from the queue.

## Finish

- **Simple task**: summarize in a few lines; the row turned `done` in the task's commit.
- **Spec**: once every sub-spec is `done`, summarize: the sub-specs and their commits, what was verified and what was not, the MINOR findings left, and how the user can try it: `cd .worktrees/<slug> && just run`. Then ask what to do with the branch: merge it into `main`, keep it for later, or discard it (only with explicit confirmation). Push and open a pull request only when a remote is configured and the user asks.
- **Changes after trying it**: before the merge, a change the user wants that stays inside the spec's scope is a fix on the branch: the same worker (or the lead, inline), then `reviewer`, then a `fix(<scope>): …` commit. Anything outside the scope becomes a new task.
- **Merge** (the [merge gate](#merge-gate) applies):
  1. In the worktree: `git merge main`, resolve conflicts, `just check`, commit the merge.
  2. In the main checkout: `git merge --squash feat/<slug>`, set the row to `done` and remove it from the queue in `docs/backlog.md`, run `just check`, and commit: `feat: <spec title> (DECK-NN)`.
  3. `git worktree remove .worktrees/<slug>` and `git branch -D feat/<slug>` (a squash merge leaves the branch unmerged in git's eyes, so `-d` refuses).
- **Discard**: with explicit confirmation, remove the worktree, `git branch -D` the branch, and set the row back to `ready` on `main` (`docs(spec): stop <slug>`).
- Never push, merge or delete a branch without the user's go-ahead in this session. Never bypass hooks.

One task or spec per lead session. When it is done and the user asks for something else, recommend a new session: every turn re-reads the whole session.

## Commits the flow may make

The user's yes to the repo flow, the approval of a spec, or a request for a backlog task authorizes exactly these commits for that task, and no others:

| When                                                                                                     | Where             | Message                                |
| -------------------------------------------------------------------------------------------------------- | ----------------- | -------------------------------------- |
| A new simple task or `needs spec` row                                                                    | `main`            | `docs(spec): add DECK-NN`              |
| A spec drafted and reviewed                                                                              | `main`            | `docs(spec): draft <slug>`             |
| A spec approved                                                                                          | `main`            | `docs(spec): add <slug> spec`          |
| A `ready` spec updated after earlier work changed what it relies on ([specs/README.md](specs/README.md)) | `main`            | `docs(spec): update <slug>`            |
| Work on a spec starts                                                                                    | `main`            | `docs(spec): start <slug>`             |
| A sub-spec or simple task done                                                                           | worktree / `main` | `feat(<scope>): …` / `fix(<scope>): …` |
| A fix after the user tried it                                                                            | worktree          | `fix(<scope>): …`                      |
| Merging `main` into the branch                                                                           | worktree          | git's merge message                    |
| The squash merge                                                                                         | `main`            | `feat: <spec title> (DECK-NN)`         |
| A spec discarded                                                                                         | `main`            | `docs(spec): stop <slug>`              |

## Keeping sessions cheap

The cost of a session is its turns times its context.

- **Lead.** Reads diff stats and targeted diffs, not whole diffs, and check output filtered to failures. Does not paste specs into briefs. Waits without polling.
- **Workers.** Focused tests while iterating (`cargo nextest run -p <crate> <filter>`), crate checks once at the end, never `just check`, no background processes left running, no `sleep` polling, code that passes Clippy the first time.
- **Reviewers.** Read the diff once, and the code around it only where a finding depends on it.
- **Turn limits.** Claude Code's `maxTurns` and OpenCode's `steps` cap each subagent (see the harness table); Codex has none, so its subagents rely on the role text. At the limit a subagent returns its partial work, and the lead resumes it or starts a fresh one with a summary.

## Checks

| When                        | Command                                                                                                                                  |
| --------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| While iterating on a crate  | `cargo nextest run -p <crate> <filter>`, `cargo clippy -p <crate>`                                                                       |
| A worker, at the end        | for each crate touched: `cargo fmt -p <crate>`, `cargo clippy -p <crate> --all-targets -- -D warnings`, `just test-crate <crate>`        |
| The lead, per sub-spec      | `just check`, before the reviewer and again before the commit                                                                            |
| Only docs or config changed | none beyond the hooks                                                                                                                    |
| On commit (automatic)       | Lefthook: `cargo fmt --check` on staged Rust files, `oxfmt --check` on staged Markdown, TOML, JSON and YAML files, commit message format |

Until DECK-01 sub-spec 01 is merged into `main` with the `justfile`, `.mise.toml` and `lefthook.yml` (inside the DECK-01 worktree, from the moment sub-spec 01 is committed there), use plain Cargo instead: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo nextest run --workspace` (or `cargo test`), and `cargo nextest run -p <crate>` for `just test-crate`. For sub-spec 01 itself, its acceptance criteria are the check.

## Trying a shippable sub-spec

A shippable sub-spec ends with something the user can run. The worker proves it with the end-to-end harness (`crates/deck/tests/`, which drives the real `deck` binary in a PTY and reads its screen; see [architecture.md](architecture.md#testing)) and lists the screen captures in its report. The lead puts the captures in the summary and tells the user how to try it from the worktree: `cd .worktrees/<slug> && just run`. Changes the user wants follow [Finish](#finish).

## Merge gate

A branch with a spec may only be merged, or proposed for merge, when every sub-spec file (`NN-*.md`, not the overview) in its spec directory has `status: done`. If one is `todo` or `in-progress`, the lead refuses and names it. Keeping or discarding the branch is always allowed.

This gate applies to implementation branches, not to landing an approved spec on `main` before the work.

## Worktrees

- Location: `.worktrees/<slug>` (gitignored), on branch `feat/<slug>`, from `main` after the spec is committed there.
- Create it from the main checkout:

  ```bash
  git worktree add .worktrees/<slug> -b feat/<slug> main
  cp -c -R target .worktrees/<slug>/target   # macOS APFS clone; skip without target/ or on Linux
  ```

  The `cp -c` line clones the build output in seconds, so Cargo rebuilds only the workspace crates. The main checkout gets a `target/` from the `just check` of each merge.

- The backlog is read and edited only on `main`, in the main checkout; a worktree's copy of `docs/backlog.md` is stale and never edited.
- After the merge or a discard: see [Finish](#finish).
