---
type: foundation
status: done
---

# 04 — Performance measurements, first round

## Scope

The measuring tools and the core measurements of the performance review, from the code analysis that reviewers ran on 24 Nov 2026 (overview, "Audits run first"; the lead hands its notes to the worker). No performance fixes here ([06](06-performance-fixes.md) makes them), with one exception the user asked for (requirement 6). [05](05-performance-second-round.md) takes the rest of the measurements. Run on Fable 5.2 at `xhigh` (overview).

## Requirements

1. **Where measurements live**: each measurement that can be scripted is a scenario in DECK-23's ignored suite (`crates/deck/tests/stress.rs`, run with `just stress <name>`), writing its numbers to `target/stress/<name>.json`, so it can be rerun after the fixes. Scenarios assert only generous bounds (3 to 5 times the measured value): a missed target is a finding in the report, not a red run. The report records the machine (model, CPU, memory, OS version) and the build profile (there is no `[profile.release]`: say so).
2. **Measuring tools** in the test support code:
   - a cell watch with a timestamp taken in the harness's PTY reader thread right after `read` returns (`crates/deck/tests/support/mod.rs`), since `wait_for_text` polls every 10 ms and the `flood` probe every 2 ms, too coarse for a 5 ms target;
   - per-process CPU time and wake-ups in `crates/deck/tests/support/sys.rs` (macOS `proc_pid_rusage`, Linux `/proc`), since `getrusage(RUSAGE_CHILDREN)` counts only reaped children;
   - a way for `just stress` to run `idle` only when asked (the recipe cannot tell a test its filter: export the name from the recipe, or gate on `DECK_STRESS_IDLE=1` alone).
3. **Measurements** (scenario names and details in the analysis notes):
   - **keystroke latency** (`latency`): 200 samples at rest and 200 with a flood in another terminal of the same panel, and Ctrl-C into a flood in the same terminal; min, p50, p90, p99, max, and the time of the first output after each key;
   - **one slow terminal stalling the others** (`stall`): a terminal printing slow sequences in the active panel, in a background panel, and in a session with no client, while keys and `deck ls` are timed in another session;
   - **throughput** (`throughput`): MiB/s through one terminal with a client attached and active, with no client, and with styled text, against a bare PTY baseline in the same test;
   - **idle cost** (`idle`, 10 minutes): 20 idle terminals and one attached client; daemon and client CPU time and wake-ups per second, and any bytes the client writes;
   - **idle with a working agent** (`idle_spinner`): client CPU and bytes per second with an agent working, in a text terminal and in an image terminal;
   - **memory** (`memory`): per idle terminal, per 10,000 lines of history at 80 and 250 columns and with the history budget full, per attached client;
   - **frame cost**: ignored benchmarks in `deck-daemon` (build, clone and encode a full 200×60 styled frame) and `deck-tui` (decode it, and draw nothing, one cell, or the whole screen, with bytes and write calls).
4. **Profiles** of the throughput and draw paths with the profiler the user installed; none is installed on 25 Nov 2026, so the report says the step was skipped unless one is there when the worker starts.
5. **Targets**: keystroke latency under 5 ms median at rest and under 20 ms median during a flood; idle daemon CPU under 0.5% of a core with 20 terminals; no timer firing more often than once a second while nothing changes. A missed target is a finding.
6. **One fix now (asked by the user on 25 Nov 2026, because CI on `main` is red):** `deck kill-server` during a live upgrade waits 1 s, then sends `SIGTERM` (`crates/deck/src/control.rs`, `ANSWER_TIMEOUT` in `kill_server`); it should wait out the handoff as `Conn::connect` does. The test `kill_server_during_the_handoff_is_answered_by_the_new_daemon_which_then_stops` fails on Linux CI and passes on macOS only because a large `Hello` blocks the write. Fix it, and make that test also run with a minimal environment so it fails on macOS without the fix.
7. **The report** `docs/reviews/<date>-performance.md`, started here: the commit, the machine, each measurement (method and numbers), the profile summary or skip note, the targets met or missed, and the findings table so far. Findings that fit the overview's budget are `planned (sub-spec 06)`; the rest are proposed tasks.

## Acceptance criteria

- [ ] The measuring tools exist and the seven measurements of requirement 3 run (`just stress <name>` for the scenarios, `cargo nextest run --release -p <crate> --run-ignored only <name>` for the benchmarks) and write their numbers.
- [ ] `kill_server_during_the_handoff_is_answered_by_the_new_daemon_which_then_stops` passes with a minimal environment and with the full one; it failed with the minimal one before the fix.
- [ ] The report exists with the content of requirement 7.

## Tests

- The new scenarios and benchmarks; the `kill-server` test.
