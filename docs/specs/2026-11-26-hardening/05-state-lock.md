---
type: foundation
status: todo
---

# 05 — State lock never held across emulator calls (DECK-43)

## Scope

The daemon holds its state lock while waiting on a terminal's emulator thread in `input`, `scroll`, `apply_sizes`, `resync`, `send_owed`, attach and the handoff's `collect` (`crates/deck-daemon/src/server.rs`; `pump` was fixed in DECK-26). Each wait is bounded by one feed, about 5 ms on macOS, possibly longer on Linux with larger reads. One terminal with slow output must never delay keys, frames or `deck ls` in others. Owning crate: `deck-daemon`.

## Requirements

1. A test seam that blocks one terminal's emulator.
2. Every path named above stops holding the state lock across an emulator call.

## Acceptance criteria

- [ ] With one emulator blocked, `deck ls` and a key into another terminal stay within 2x their at-rest p99 measured in the same VM run.
- [ ] A test or debug assertion covers each path named above.

## Tests

The seam and latency checks in `crates/deck-daemon/tests/`.
