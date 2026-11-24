---
type: foundation
status: todo
---

# 04 — Performance fixes

## Scope

The fixes from [03](03-performance.md)'s findings that fit the overview's budget, as many as there are, most impactful first, each proved by numbers. Before fixing, the worker rebases onto `main` when it moved since 03 and re-runs 03's scenarios when the changes touch a measured path, so the before numbers describe the code being fixed. Run on Fable 5.2 at `xhigh` (overview).

## Requirements

1. Each fix: the scenario or test from 03 that shows the problem, run before and after on the same machine, with both numbers in the performance report's findings table (status `fixed (sub-spec 04, test <name>)`).
2. Where a fix removes a regression risk, the matching DECK-23 stress bound is tightened to the new value plus its usual margin.
3. Every finding marked `planned (sub-spec 04)` ends as `fixed` or, over the budget or by the user's choice when they do not fit one session, as a proposed task (overview).

## Acceptance criteria

- [ ] Every fix shows before and after numbers in the report.
- [ ] `just stress` passes, with any tightened bounds.
- [ ] The targets of 03 are re-measured and the report says which are now met.

## Tests

- DECK-23's suite and 03's scenarios; a unit test per fix where one can catch a regression.
