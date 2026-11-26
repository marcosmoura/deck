---
type: foundation
status: todo
---

# 04 — Output read while the emulator works (DECK-46)

## Scope

Each PTY reader hands its bytes to the emulator and waits for the result before reading on; DECK-26 batches reads already waiting, up to 16 KiB. Let the reader keep reading while the emulator feeds the previous batch, without letting one slow terminal delay others. Owning crate: `deck-daemon` (`terminal.rs`, `server.rs`).

## Requirements

1. At most one batch ahead per terminal, so memory stays bounded.
2. The live upgrade's reader pause and held bytes (`pause_reader`, `hold_quiet`) keep their guarantees (overview, Notes).

## Acceptance criteria

- [ ] The `throughput` scenario's ratio to the bare PTY rises against the VM's own "before" (on macOS, DECK-26 measured 0.43: 82 against 190 MiB/s).
- [ ] The `stall` scenario's `deck ls` times do not rise in the VM.
- [ ] The reader-pause and upgrade tests pass.

## Tests

The `throughput` and `stall` scenarios; unit tests of the hand-over.
