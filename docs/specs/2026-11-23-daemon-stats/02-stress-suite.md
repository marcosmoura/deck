---
type: shippable
status: todo
---

# 02 — Stress and soak suite

## Scope

A suite that loads the daemon hard and for long, and fails when it leaks, stalls or slows down; run on demand and nightly. Owning crate: `deck` (tests); repo (`justfile`, `.github/workflows/`).

## Requirements

1. **Scenarios**, each an ignored test in `crates/deck/tests/stress.rs`, driving a `Home` daemon through `deck` commands and an attached harness client, measuring with `deck stats --json` ([01](01-stats.md)):
   - **flood**: one terminal prints about 270 MiB of 100-column lines (`head -c 209715200 /dev/urandom | base64 | fold -w 100`, the same shape on macOS and Linux) while attached; bounds on the time to finish, the daemon's peak rss, and **responsiveness**: during the flood, a key typed into a second terminal of the same panel echoes on screen within a bound.
   - **many terminals**: 64 terminals across 16 panels, each printing 10,000 lines; bounds on rss (per terminal and total) and on the time for `deck ls` to answer.
   - **churn**: open and close 500 terminals one after the other; at the end, polling for up to 15 s, fds and threads are back to the baseline (within 2) and rss is within a bound of the baseline.
   - **attach loop**: attach and detach a client 300 times; fds back to baseline, no client left in `deck stats`.
   - **control loop**: 2,000 `deck send` plus `deck capture` calls; fds back to baseline.
   - **soak**: for `DECK_SOAK_SECS` (default 600), with `scrollback_lines = 1000` so histories fill within the first minute, a mixed load (a terminal printing a line every 10 ms, terminals opened and closed every 5 s, a client attaching and detaching every 30 s); samples `deck stats` every 10 s into `target/stress/soak.csv`. After a 120 s warm-up, with at least 30 samples, it fails if the least-squares slope of rss is above a bound in MiB per hour or that of fds above 1 per hour. A quiet minute at the end (no load) checks idle CPU: daemon CPU seconds per minute below a bound.
2. Each scenario writes its measurements to `target/stress/<scenario>.json` and prints a one-line summary.
3. **`just stress [filter]`** runs the scenarios in a release build, one at a time (`cargo nextest run --release -p deck --test stress --run-ignored only --test-threads 1 {{filter}}`), after raising the open-file limit (`ulimit -n 4096`); `DECK_SOAK_SECS` passes through.
4. **Nightly workflow** `.github/workflows/stress.yml`: on a schedule (once a day) and on manual dispatch, `just stress` on `macos-15` and `ubuntu-24.04` with `DECK_SOAK_SECS=1800`, uploading `target/stress/` as an artifact even when a scenario fails (`if: always()`).
5. **Bounds** set as the overview says, each written next to its scenario with the measured value it came from.
6. Everything the suite exposes is fixed or filed (overview, Findings).

## Acceptance criteria

- [ ] `just stress` passes on the worker's machine, and every scenario writes its JSON.
- [ ] A committed ignored test, `churn_catches_a_leak`, runs churn with a hidden `DECK_TEST_LEAK_FD=1` in the `Home`'s environment (the daemon then leaks one descriptor per terminal opened, like the existing `DECK_TEST_PANIC` hook) and asserts that churn's descriptor check fails.
- [ ] The soak scenario at its default length (600 s: a 120 s warm-up, then at least 30 samples) passes and writes the CSV.
- [ ] `docs/features/stats.md` has a table of each scenario's measured values and bounds, with the date and machine.
- [ ] Every finding is fixed or a backlog task named in the report.

## Tests

- The scenarios are the tests. `just check` does not run them.

## Notes

- After merge, the lead dispatches the workflow once, adjusts bounds from its artifacts if needed, and runs the 24-hour soak (overview).
- If [DECK-25](../2026-11-23-release-homebrew/00-overview.md) has landed first, `just lint-ci` checks the new workflow; otherwise its first dispatched run is the check.
- Responsiveness is the bound that matters most to the user: a flood in one terminal must not freeze the others.
