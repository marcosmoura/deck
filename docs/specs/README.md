# Specs

Read when you write, build or check the status of a spec.

Substantial work gets a spec directory, written by the lead session ([workflow.md](../workflow.md#spec)). A simple task has no spec: its backlog note describes it. Specs stay here after merge as a record of decisions. The lasting description of how a feature works belongs in `docs/features/`, not in the spec.

An approved spec lands on `main` before the work; its `branch:` frontmatter names the branch where it is built, and its `backlog:` frontmatter names its row in [../backlog.md](../backlog.md). When that branch merges later, keep `main`'s spec files and take only the code and status updates.

## Layout

```text
docs/specs/<yyyy-mm-dd>-<slug>/
  00-overview.md     # goal, non-goals, decisions, sub-specs table
  01-<sub-spec>.md   # one file per sub-spec, in dependency order
  02-<sub-spec>.md
```

A spec that is not split has one sub-spec. Split when the work will not fit one worker session. Each sub-spec can be built, tested, reviewed and committed on its own, and is one of two types:

- **shippable**: when it lands, the user can try something new or changed with `just run`;
- **foundation**: nothing to try yet, but the base that later shippable sub-specs build on (a model, a protocol, an engine).

A spec that builds on an earlier spec names it in the overview and only references its decisions; it does not repeat them. When an earlier spec's work changed something a later `ready` spec relies on, the lead updates the later spec before its work starts and notes the change in its `Notes`.

## Template

Copy the overview and sub-spec skeletons from [TEMPLATE.md](TEMPLATE.md).

## Status rules

- `draft`: written, not started.
- `in-progress`: being built.
- `done`: built, reviewed and committed.
- Status lives in the sub-spec files' frontmatter. The overview's table mirrors it, and the lead keeps both in sync. Only the lead changes `status`.
- The spec's backlog row says whether it waits for approval, is ready, started or done; the lead updates it on `main` ([backlog](../backlog.md#keeping-it-current)).
- The [merge gate](../workflow.md#merge-gate) blocks merging while a sub-spec is not `done`.
