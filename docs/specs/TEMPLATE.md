# Spec template

Read when you write a spec. Layout and status rules are in [README.md](README.md).

## `00-overview.md`

```markdown
---
branch: feat/<slug>
backlog: DECK-NN
---

# <Title>

## Goal

## Builds on

<!-- Earlier specs and docs this one relies on; "none" for the first. -->

## Non-goals

## Decisions

## Sub-specs

| Sub-spec           | Type       | Crate | Summary | Depends | Status |
| ------------------ | ---------- | ----- | ------- | ------- | ------ |
| [01](01-<name>.md) | foundation | …     | …       | —       | draft  |
| [02](02-<name>.md) | shippable  | …     | …       | 01      | draft  |

## Affected docs
```

## Sub-spec file (`NN-<name>.md`)

```markdown
---
type: shippable # shippable | foundation
status: draft # draft | in-progress | done
---

# NN — <Title>

## Scope

<!-- What the user can try when it lands (shippable), or what it lays down
     and which sub-specs build on it (foundation). Owning crate(s). -->

## Requirements

## Screen

<!-- Sub-specs that add or change something on screen only; delete otherwise.
     Text mockups of every new state, with sizes, colours by role and key hints. -->

## Acceptance criteria

- [ ] …

## Tests

<!-- Unit tests per crate, and the end-to-end scenarios (crates/deck/tests/)
     for shippable sub-specs. -->

## Notes
```
