# Specs

Read when you write, build or check the status of a spec.

Substantial work gets a spec directory, written by the lead session ([workflow.md](../workflow.md#spec)). A simple task has no spec: its backlog note describes it. Specs stay here after merge as a record of decisions. The lasting description of how a feature works belongs in `docs/features/`, not in the spec.

An approved spec is committed on `main` before the work; its `branch:` frontmatter names the branch where it is built, and its `backlog:` frontmatter names its row in [../backlog.md](../backlog.md). After the `docs(spec): start <slug>` commit, the spec's files are edited only on that branch (status, notes, clarifications); the squash merge brings them back to `main` ([workflow.md](../workflow.md#finish)). In this repo, a spec or feature "lands" when it is merged into `main`.

## Layout

```text
docs/specs/<yyyy-mm-dd>-<slug>/
  00-overview.md     # goal, non-goals, decisions, sub-specs table
  01-<sub-spec>.md   # one file per sub-spec, in dependency order
  02-<sub-spec>.md
```

A spec that is not split has one sub-spec. Split when the work will not fit one worker session. Each sub-spec can be built, tested, reviewed and committed on its own, and is one of two types:

- **shippable**: once it is committed, the user can try something new or changed with `just run` in the spec's worktree;
- **foundation**: nothing to try yet, but the base that later shippable sub-specs build on (a model, a protocol, an engine).

A spec that builds on an earlier spec names it in the overview and only references its decisions; it does not repeat them. When an earlier spec's work changed something a later `ready` spec relies on, the lead updates the later spec on `main` before its work starts, notes the change in its `Notes`, and commits it (`docs(spec): update <slug>`, authorized by the request for that task).

## Template

Copy the overview and sub-spec skeletons from [TEMPLATE.md](TEMPLATE.md).

## Status rules

- `todo`: not started (the spec itself may still be a draft or already approved; the backlog row says which).
- `in-progress`: its worker has started. The lead sets it in the worktree when it briefs the worker; it is committed with the sub-spec.
- `done`: built, reviewed and committed.
- Status lives in the sub-spec files' frontmatter. The overview's table mirrors it, updated in the same sub-spec commit. Only the lead changes `status`.
- The spec's backlog row says whether it waits for approval, is ready, started or done; the lead updates it on `main` ([backlog](../backlog.md#keeping-it-current)).
- The [merge gate](../workflow.md#merge-gate) blocks merging while a sub-spec is not `done`.
