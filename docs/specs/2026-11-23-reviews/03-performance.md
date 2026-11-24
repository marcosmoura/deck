---
type: foundation
status: todo
---

# 03 — Performance measurements

## Scope

How fast deck feels and how much it costs, measured and profiled, with a report and findings; no fixes here ([04](04-performance-fixes.md) makes them). Run on Fable 5.2 at `xhigh` (overview).

## Requirements

1. **Where measurements live**: each measurement that can be scripted is a new scenario in DECK-23's ignored suite (`crates/deck/tests/stress.rs`, run with `just stress <name>`), writing its numbers to `target/stress/<name>.json`, so it can be rerun after the fixes and later. These scenarios record numbers and assert only generous bounds (3 to 5 times the measured value), so a missed target is a finding in the report, not a red nightly run. `idle` lasts 10 minutes and is skipped by `just stress` unless named or `DECK_STRESS_IDLE=1` is set. The report records the machine (model, CPU, memory, OS version) and the build profile.
2. **Measurements**, each with its method and numbers in the report. This list is the minimum: a feature present at the reviewed commit that adds work on a hot path, a timer, or memory per terminal or client gets its own measurement (overview, "Review the project at execution time"):
   - **keystroke latency** (scenario `latency`): the time from a key written to the harness client's PTY to its echo appearing in the client's output, 200 samples, median and 99th percentile, at rest and during a flood in another terminal of the same panel;
   - **throughput** (scenario `throughput`): MiB/s through one terminal with a client attached, and with none;
   - **idle cost** (scenario `idle`): daemon CPU seconds (from `deck stats`) over 10 minutes with 20 idle terminals and one attached client, and the client's CPU time (from `getrusage` of the child, read by the test); every wake-up source of both processes, found by reading the code at the reviewed commit, listed with its period (on 23 Nov 2026 these were the daemon's 1 s config and agent polls, 5 s cwd poll and 16 ms frame pump, and the client's 16 ms `REDRAW` throttle in `deck-tui/src/lib.rs`; later features may have added more, such as cursor blinking);
   - **memory**: per idle terminal, per 10,000 lines of history, per attached client (from `deck stats` deltas), and the client's own rss;
   - **start-up**: `deck` to first frame with 0 and with 30 restored terminals; `deck ls` round trip;
   - **frame cost**: encode in the daemon, decode and draw in the client, for a full 200×60 frame of styled text (unit benchmarks as ignored tests in `deck-daemon` and `deck-tui`, timed with `std::time::Instant` over 1,000 iterations);
   - **live upgrade** (DECK-21): handoff time with 30 terminals and 10,000 lines of history each.
3. **Profiles** of the throughput and draw hot paths, with the profiler the user installed (overview); the top costs summarised in the report. Without a profiler, the report says the step was skipped.
4. **Targets**: keystroke latency under 5 ms median at rest and under 20 ms median during a flood; idle daemon CPU under 0.5% of a core with 20 terminals; no timer firing more often than once a second while nothing changes (the frame pump only runs on output). A missed target is a finding.
5. **Findings** in the report's table, each with the measurement that shows it and a proposed fix; the fixes that fit the overview's budget are marked `planned (sub-spec 04)` and are the input of [04](04-performance-fixes.md), the rest are proposed tasks.

## Acceptance criteria

- [ ] `docs/reviews/<date>-performance.md` exists with the commit reviewed, the machine, every measurement of 2 (and every added one) (method and numbers), the profiles' summary or the skip note, the targets met or missed, and the findings table.
- [ ] The `latency`, `throughput` and `idle` scenarios run with `just stress <name>` and write their JSON.

## Tests

- The new scenarios and the ignored frame-cost tests.
