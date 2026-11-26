---
type: foundation
status: done
---

# 06 — Performance fixes

## Scope

The fixes from the findings of [04](04-performance.md) and [05](05-performance-second-round.md) that fit the overview's budget, as many as there are, most impactful first, each proved by numbers. Before fixing, the worker rebases onto `main` when it moved since 05 and re-runs the scenarios of 04 and 05 when the changes touch a measured path, so the before numbers describe the code being fixed. Run on Fable 5.2 at `xhigh` (overview).

## Requirements

1. Each fix: the scenario or test from 04 or 05 that shows the problem, run before and after on the same machine, with both numbers in the performance report's findings table (status `fixed (sub-spec 06, test <name>)`).
2. Where a fix removes a regression risk, the matching DECK-23 stress bound is tightened to the new value plus its usual margin.
3. **Decided by the user on 25 Nov 2026, rate changed on 26 Nov 2026:** the "agent is working" spinner draws only while it is visible. On 26 Nov 2026 the user decided that every spinner (the agent's and the running-command one) runs at 8 frames a second in every terminal, the rate of main's eight-spoke loader (it ran at 60 in terminals with images; this spec first chose 10). The image loader's frame is the clock's tick scaled to its frames, so it cannot round to 0.
4. Every finding marked `planned (sub-spec 06)` ends as `fixed` or, over the budget or by the user's choice when they do not fit one session, as a proposed task (overview).

## Acceptance criteria

- [x] Every fix shows before and after numbers in the report (G-7 and F-15 accepted without, overview Notes).
- [x] `just stress` passes, with any tightened bounds: all 19 scenarios on Linux (Ubuntu 24.04 container, 4 CPUs) on 26 Nov 2026, before `main` was merged in. The user chose on 26 Nov 2026 to merge before the macOS run, which follows on `main`.
- [x] The targets of 04 are re-measured and the report says which are now met.

## Tests

- DECK-23's suite and the scenarios of 04 and 05; a unit test per fix where one can catch a regression.
