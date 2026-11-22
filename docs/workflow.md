# Agent workflow

Read when a request changes the product or the repository, when you pick up a backlog task, or when you change the agent setup.

The session the user talks to is the **lead**. It runs this flow itself: there is no orchestrator agent. The lead owns decisions, specs, the backlog, git and the conversation with the user. Its subagents:

| Agent           | Does                                                                          | Writes            |
| --------------- | ----------------------------------------------------------------------------- | ----------------- |
| `spec-reviewer` | Checks a written spec against the request, the code, the docs and the backlog | nothing           |
| `worker`        | Implements one sub-spec with tests, then fixes the review findings for it     | code, tests, docs |
| `reviewer`      | Reviews finished work against its sub-spec or task description                | nothing           |

Subagents never start other agents and never commit. Role definitions live in `.agents/roles/<agent>.md`. The harness files only add model, effort, tools and turn limits: edit the role, not the wrapper.

| Harness     | Wrappers                                                    | Lead                  | Delegation                       |
| ----------- | ----------------------------------------------------------- | --------------------- | -------------------------------- |
| Claude Code | `.claude/agents/*.md`                                       | the normal session    | Agent tool; fixes via SendMessage |
| Codex       | `.codex/agents/*.toml`                                      | the normal session    | its subagent tool                |
| OpenCode    | `.opencode/agents/*.md`, permissions in `.opencode/opencode.jsonc` | the `build` agent | the task tool                    |

Codex cannot make a role read-only, so its reviewers are read-only by their role text only. When a harness cannot send a follow-up message to a finished subagent, start a new `worker` for the fixes and brief it with the findings and the `git diff --stat` of the work.

## Triage

A question, an explanation or a fix of a line or two: answer or do it, nothing below applies. For any other request:

1. **Think it through.** Restate it in a sentence or two, then read [index.md](index.md), the docs it routes the request to, and just enough code to size it.
2. **Size it.**
   - **Simple task**: fits one focused session of under about 500k tokens. As a rule of thumb: one or two crates, a handful of files, no new protocol message or persisted field, no product decision left open, no new dependency.
   - **Substantial**: anything else. It gets a spec.
3. **Ask the user, once,** whether to run it through the repo flow. Give your sizing (simple task or spec) and put any open scope question in the same message, each with a recommended answer.
   - **No**: do the work as a normal session. No backlog row, no reviewer.
   - **Yes, simple task**: add a `todo` row to [backlog.md](backlog.md) with a note (what, why, acceptance), then go to [Work](#work). If the user only wanted it recorded, place it in the queue and stop.
   - **Yes, substantial**: go to [Spec](#spec).

## Spec

1. **Write** `docs/specs/<yyyy-mm-dd>-<slug>/` in the format of [specs/README.md](specs/README.md), from [specs/TEMPLATE.md](specs/TEMPLATE.md). Add a backlog row with status `draft spec` (or turn the `needs spec` row into one), name it in the overview's `backlog:` line, and place it in the queue.
2. **Split** it when it is too big for one worker session. Every sub-spec is either **shippable** (the user can try something new when it lands) or **foundation** (nothing to try yet, but the base that the next shippable sub-specs build on). A sub-spec that adds or changes something on screen draws it in its `Screen` section, as a text mockup.
3. **Review.** Run `spec-reviewer` on the spec directory, and record it at the end of the overview (`Reviewed by spec-reviewer on <date>.`). A later session asked to approve a `draft spec` runs `spec-reviewer` first when the overview records no review, or the spec changed since. Brief it with the spec path, the user's request and the backlog row. Fix its BLOCKER and MAJOR findings yourself, and take the MINOR ones that are cheap. Run it again only when a fix changed the scope or the split. Several specs can be reviewed at the same time, one `spec-reviewer` each.
4. **Summarize** for the user: the goal, the key decisions, the non-goals, each sub-spec on one line (shippable or foundation, crate, depends on), the open questions with a recommendation, and the spec's path. Change the spec as the user asks until they approve it. Small edits need no new `spec-reviewer` run.
5. **Commit on approval,** before anything else, even when the same message says to start the work: set the row to `ready`, stage only the spec directory and `docs/backlog.md` (other sessions may have work in the checkout), and commit on `main`: `docs(spec): add <slug> spec`. Then ask whether to start the work now or stop here, unless the user already said. A stop ends the session; a later session starts from the backlog task.

## Work

Work starts with "start now" after a spec, with a simple task, or in a new session asked for a backlog task (`DECK-03`, "the next task"; how to pick is in [backlog.md](backlog.md#picking-the-next-task)).

1. **Read** the backlog row, and for a spec its overview and every sub-spec that is not `done`. Sub-spec status is committed on the spec's branch: read it from the worktree when one exists (`git worktree list`). Check what is already built (`git log`, the code). If a task is `in-progress` and its worktree has uncommitted changes, another session may be working on it: ask the user before resuming. Think it through: what each sub-spec needs, which can run at the same time, which decisions are still open. Ask the user only about a decision that blocks the work.
2. **Choose how:**
   - **Inline**: a simple task, or a spec whose remaining work is one small sub-spec of simple-task size. The lead does it.
   - **Delegated**: anything bigger. Each sub-spec goes to one `worker`. Sub-specs that do not depend on each other and touch different crates run at the same time, up to three workers; tell each that others share the checkout. Sub-specs in the same crate, or that depend on each other, run one after the other: start the next as soon as the previous one is reviewed and committed.
3. **Choose where:**
   - A simple task runs in the current checkout, on its current branch.
   - A spec runs in its own worktree ([Worktrees](#worktrees)). Right after creating it, set the row to `in-progress` and commit that on `main` from the main checkout (`git -C <main checkout> …`): `docs(spec): start <slug>`. Parallel workers of one spec share its worktree: they touch different crates and never commit.
4. **Brief each worker** in a few lines: the absolute path of the checkout, the sub-spec file, the owning crate, decisions that are not in the spec, the files to start from, and the checks to run. The worker reads the spec itself; do not paste it. Keep each worker's agent ID: its fixes go back to it.
5. **When a worker reports,** read `git diff --stat`, then only the diffs that carry the acceptance criteria, and check output filtered to the failures. A gap goes back to the same worker. If a worker stops at its turn limit, resume it; if it stops again at once, start a new worker with the diff summary and what is left.

Once work has started, keep going until every sub-spec is done, reviewed and committed. A message without a tool call ends your turn and stops the work. Stop only for a question that blocks everything left, or the choice at [Finish](#finish).

## Review

After the work of a simple task, or of each sub-spec:

1. Run `reviewer` with the checkout path, what to review (uncommitted work: `git diff HEAD` plus untracked files, limited to the sub-spec's paths when parallel workers share the worktree), and the sub-spec file or the task's description. Reviews of parallel sub-specs can run at the same time.
2. BLOCKER and MAJOR findings: when the work was inline, the lead fixes them. When it was delegated, send them to the same worker, so it fixes them with its context. Read the fix diff. Run `reviewer` again only after a BLOCKER fix.
3. MINOR findings: fix the cheap ones, and list the rest in the summary.
4. Run `just check`, then commit: one commit per sub-spec or simple task, holding its code, tests, docs and the sub-spec's `status: done`. Stage only its paths, since other sessions may have work in the same checkout.

## Finish

- **Simple task**: set its row to `done` in the task's commit, remove it from the queue in the same commit, and summarize in a few lines.
- **Spec**: once every sub-spec is `done`, summarize: the sub-specs and their commits, what was verified and what was not, the MINOR findings left, and how the user can try the shippable parts (`just run`). Then ask what to do with the branch: squash-merge into `main`, push and open a PR, keep it for later, or discard it (only with explicit confirmation). The [merge gate](#merge-gate) applies. Set the row to `done` and remove it from the queue in the merge commit; remove the worktree and branch once the user agrees.
- Never push, merge or delete a branch without the user's go-ahead in this session. Never bypass hooks.

One task or spec per lead session. When it is done and the user asks for something else, recommend a new session: every turn re-reads the whole session.

## Keeping sessions cheap

The cost of a session is its turns times its context.

- **Lead.** Reads diff stats and targeted diffs, not whole diffs, and check output filtered to failures. Does not paste specs into briefs.
- **Workers.** Focused tests while iterating (`cargo nextest run -p <crate> <filter>`), crate checks once at the end, never `just check`, no background processes left running, no `sleep` polling, code that passes Clippy the first time.
- **Reviewers.** Read the diff once, and the code around it only where a finding depends on it.
- **Turn limits.** Where the harness supports it (Claude Code's `maxTurns`), each subagent wrapper sets a turn limit. At the limit it returns its partial work, and the lead resumes it or starts a fresh one with a summary.

## Checks

| When                         | Command                                                          |
| ---------------------------- | ---------------------------------------------------------------- |
| While iterating on a crate   | `cargo nextest run -p <crate> <filter>`, `cargo clippy -p <crate>` |
| A worker, at the end         | `just fmt`, then `just lint` and `just test-crate <crate>` for each crate touched |
| Before a commit (the lead)   | `just check`                                                     |
| Only docs or config changed  | none beyond the hooks                                            |
| On commit (automatic)        | Lefthook: `cargo fmt --check` on staged Rust files, commit message format |

Until DECK-01 sub-spec 01 lands the `justfile`, `.mise.toml` and `lefthook.yml`, use plain Cargo instead: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo nextest run --workspace` (or `cargo test`). For sub-spec 01 itself, its acceptance criteria are the check.

## Trying a shippable sub-spec

A shippable sub-spec ends with something the user can run. The worker proves it with the end-to-end harness (`crates/deck/tests/`, which drives the real `deck` binary in a PTY and reads its screen; see [architecture.md](architecture.md#testing)) and lists the screen captures in its report. The lead puts the captures in the summary and tells the user how to try it with `just run`. What the user wants changed after trying it becomes a new task.

## Merge gate

A branch with a spec may only be merged, or proposed for merge, when every sub-spec file (`NN-*.md`, not the overview) in its spec directory has `status: done`. If one is `draft` or `in-progress`, the lead refuses and names it. Keeping or discarding the branch is always allowed.

This gate applies to implementation branches, not to landing an approved spec on `main` before the work.

## Worktrees

- Location: `.worktrees/<slug>` (gitignored), on branch `feat/<slug>` or `fix/<slug>`, from `main` after the spec is committed there.
- Create it from the main checkout:

  ```bash
  git worktree add .worktrees/<slug> -b feat/<slug> main
  cp -c -R target .worktrees/<slug>/target   # macOS APFS clone; skip without target/ or on Linux
  ```

  The `cp -c` line clones the build output in seconds, so Cargo rebuilds only the workspace crates.

- After the merge: `git worktree remove .worktrees/<slug>`, and delete the branch if the user agrees.
