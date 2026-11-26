---
type: foundation
status: done
---

# 05 — Performance measurements, second round

## Scope

The measurements [04](04-performance.md) did not take, with its tools, from the same code analysis (the lead hands its notes to the worker). No fixes here ([06](06-performance-fixes.md) makes them). Finishes the performance report. Run on Fable 5.2 at `xhigh` (overview).

## Requirements

1. Same rules as 04's requirement 1 (scenarios in `crates/deck/tests/stress.rs`, JSON in `target/stress/`, generous bounds).
2. **Measurements** (details in the analysis notes):
   - **start-up** (`startup`): `deck` to its first frame with 0 and with 30 restored terminals, warm and cold, and with an outer terminal that answers nothing; time until 30 restored terminals are live; 200 × `deck ls` with 30 terminals;
   - **live upgrade** (`upgrade`): 30 terminals × 10,000 lines of 100 columns, one client: `deck upgrade` wall time, the handoff time from the log, when a `deck ls` started during it answers, the re-attached client's first frame, memory before and after;
   - **control connections** (`hello`): 500 × `deck ls` and 500 × `deck agent-event` with the environment minimal and padded to 8, 32 and 128 KiB; `kill-server` during a held handoff (after 04's fix);
   - **one slow terminal stalling the others, with images** (`stall`, a variant added to 04's scenario): terminal A prints the small PNG commands that declare a large image (each makes the emulator allocate and zero up to 64 MiB), so one read's feed takes tens of milliseconds; keys and `deck ls` timed in another session with A in the active panel, a background panel and a session with no client. 04's slow-text variant could not show the stall (each feed lasted about 5 ms, below what its key timing can see), so this variant decides the state-lock finding, which stays planned for [06](06-performance-fixes.md) unless it is refuted here.
   - **paste** (`paste`): 64 KiB, 256 KiB and 1 MiB into `cat > file`, time to the first echo and to completion;
   - **images** (`images`): time from a program's image to the outer terminal's placement; key latency in another terminal while images arrive; bytes re-sent on a panel switch away and back;
   - **client memory** (`client_memory`): the client's resident memory at 200×60, after visiting 20 panels, after paging 100,000 lines in copy mode with a search, with 200 terminals in the sidebar, with 1 and 8 large images and after leaving them;
   - **benchmarks**: in `deck-daemon`, encoding one 4 MiB image piece, the tiny-PNG cost (100 small PNG commands that declare a large image), terminal snapshots for a live upgrade (time and size at 80 and 200 columns, plain and styled, with 10,000 lines and a full budget), feeding a snapshot back, writing and reading 30 snapshot records; in `deck-tui`, building and sending a 64 MiB image canvas, scaled and same-size, and a usual 1920×1080 image into 80×24 cells.
3. **The report** completed: every measurement of 04 and 05, the targets met or missed, and the full findings table (`planned (sub-spec 06)`, proposed task, or accepted).

## Acceptance criteria

- [ ] The six scenarios and the benchmarks of requirement 2 run and write their numbers.
- [ ] `docs/reviews/<date>-performance.md` has every measurement of 04 and 05 and the full findings table.

## Tests

- The new scenarios and benchmarks.
