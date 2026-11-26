---
type: foundation
status: todo
---

# 02 — macOS baseline

## Scope

DECK-26 merged after `just stress` passed on Linux only (the user's choice, 26 Nov 2026). This runs `just stress` on macOS at `9aedd08` (DECK-26 as merged) in a temporary worktree, whatever the branch's progress, and records the figures, which are also the macOS baseline that 12 compares with. Owning crates: `deck` (tests); `deck-daemon` or `deck-tui` if a real regression is found.

## Requirements

1. Run in a window the user names: about 25 minutes, uncapped.
2. Record each scenario's figures in `docs/reviews/2026-11-25-performance.md` ("after the DECK-26 merge").
3. A failure is diagnosed: a real regression is fixed on the branch with a test; a bound the merged code legitimately moves (main's running-command spinner at 8 frames a second) changes with its reason written next to it.

## Acceptance criteria

- [ ] `just stress` passes on macOS at `9aedd08`, or each failure is fixed or its bound changed with a reason.
- [ ] The figures are in the performance report.

## Notes

The merge review expected `latency`'s at-rest figures to move: `start_cat` leaves `cat` in the foreground, so the running-command spinner draws during those samples.
