---
type: foundation
status: todo
---

# 09 — Fuzz target for the emulator feed (DECK-37)

## Scope

All terminal output passes through `Emu::feed` and the young `ghostty-vt` bindings, whose Zig code is built without runtime safety checks; the security review found its high-severity PNG allocation (B-1) by reading code, and nothing fuzzes it. Owning crate: `deck-daemon`, plus the `justfile`.

## Requirements

1. A `cargo-fuzz` target feeding arbitrary bytes, kitty graphics and OSC sequences to `Emu::feed` with the daemon's configuration.
2. A `just fuzz [seconds]` recipe that runs the target for a bounded time, run inside the VM as `just vm-run just fuzz <seconds>` (one entry point), with nightly as the overview says.
3. Check whether `ghostty-vt-sys` can build its Zig code with safety checks for the fuzz build; use it if it can, say so if it cannot.
4. `cargo-fuzz` and the nightly recorded in `docs/architecture.md#dependencies`.

## Acceptance criteria

- [ ] The target builds and runs for a bounded time.
- [ ] A run of several hours in the VM (started by the worker, recorded by the lead): every crash fixed with a regression test, or added to the backlog by the lead.
- [ ] The recipe and tools are documented.

## Tests

The fuzz target; a unit test per crash fixed.
