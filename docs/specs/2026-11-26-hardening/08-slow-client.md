---
type: foundation
status: todo
---

# 08 — Queued frames merged for a slow client (DECK-47)

## Scope

The daemon queues every frame for a client one after another (`crates/deck-daemon/src/queue.rs`), so a client that falls behind replays each intermediate frame of a terminal instead of only its latest state. Not measured: no scenario showed a client falling behind. Owning crate: `deck-daemon`.

## Requirements

1. Merge queued frames of the same terminal: damage frames combine their dirty rows, or are replaced by one full frame.
2. The queue's drop and resync rules and its byte cap still hold.

## Acceptance criteria

- [ ] A test with a client that stops reading shows queued frames of one terminal merged, and the screen correct once it reads again.
- [ ] The byte cap and resync tests pass.

## Tests

Unit tests of the queue; one e2e test with a client that stops reading.
