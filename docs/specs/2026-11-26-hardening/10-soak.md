---
type: foundation
status: todo
---

# 10 — 24-hour soak (DECK-28)

## Scope

DECK-23's bounds come from one macOS machine and a 600 s soak; its spec asks for a 24-hour soak. Run on the finished fixes so it describes the final code. Owning crate: `deck` (tests and docs).

## Requirements

1. `DECK_SOAK_SECS=86400 just stress soak` in the VM at 2 CPUs (overview): the VM is recreated with `just vm-down`, then `just vm-up 2`, and nothing else is measured in it during the 24 hours. Started by the worker, recorded by the lead.
2. Record in `docs/features/stats.md` with the date and machine: the rss and fds slopes, idle CPU, and whether CPU under load kept climbing (one 600 s run hinted it); read the rss slope by hand, since the 50 MiB/h bound does not tighten with length.
3. Optional: make `attach_loop` record its JSON before asserting, like the other scenarios.

## Acceptance criteria

- [ ] `docs/features/stats.md` has the 24-hour soak's summary with date and machine.
