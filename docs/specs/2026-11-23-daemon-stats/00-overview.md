---
branch: feat/daemon-stats
backlog: DECK-23
---

# Daemon stats and stress tests

## Goal

deck can tell how healthy its daemon is and proves it stays healthy under load. The daemon measures its own memory, file descriptors, threads and CPU time, shows them with `deck stats` and logs them every 10 minutes, so two weeks of daily use leave a trail in `daemon.log`. A stress suite (`just stress`, also run nightly in CI) floods terminals, opens and closes hundreds of them, attaches and detaches in a loop and runs for a long time, and fails when memory, descriptors or responsiveness drift.

Asked by the user on 23 Nov 2026, as part of making deck a daily driver. [DECK-26](../2026-11-23-reviews/00-overview.md)'s performance review uses both.

## Builds on

- [DECK-01 core-daemon-terminal](../2026-11-22-core-daemon-terminal/00-overview.md): the daemon, its threads per terminal, the control requests.
- [DECK-05 control-api](../2026-11-22-control-api/00-overview.md): control commands, `--json` output.
- The daemon-level test harness (`crates/deck-daemon/tests/support/`) and the e2e harness.

## Non-goals

- Metrics export (Prometheus, OpenTelemetry) or a live dashboard.
- Client-side stats (the TUI's memory and frame times): the performance review measures them by hand.
- Fixing what the stress suite finds beyond this spec's budget: findings become backlog tasks (see Decisions).
- Benchmarks with statistical rigour (criterion): the suite asserts bounds, the review profiles.

## Decisions

- **Measured by the daemon itself**, in `deck-daemon::platform`: resident memory, open file descriptors, threads, and user and system CPU time, on macOS (`proc_pidinfo` with `PROC_PIDTASKINFO` and `PROC_PIDLISTFDS`, `getrusage`) and Linux (`/proc/self/statm`, `/proc/self/fd`, `/proc/self/status`, `getrusage`).
- **`deck stats`** asks the daemon (a new `Request::Stats`, one protocol bump) and prints the numbers plus per-terminal history lines and bytes read; `--json` for scripts. It never starts a daemon.
- **A log line every 10 minutes** (`stats: rss=… fds=… threads=… cpu=…s sessions=… terminals=… clients=…`), and one at shutdown, at INFO, so `daemon.log` shows drift over days. The interval is fixed (a hidden test variable shortens it).
- **Stress tests are ignored tests** in `crates/deck/tests/stress.rs`, run by a new `just stress` recipe and by a new nightly GitHub Actions workflow on macOS and Linux. They are too slow for `just check`.
- **Bounds are set from measurements, per OS**: the worker runs each scenario on macOS (and on Linux where it can), sets time bounds at 3 to 5 times the measured value (the rule for end-to-end tests in `docs/architecture.md#testing`), memory bounds at twice the measured value, and descriptor and thread checks at the baseline plus a slack of 2 after the counts settle. After merge, the lead dispatches the nightly workflow once and adjusts any bound the hosted runners exceed (a `fix(repo)` commit), recording the runners' numbers next to the worker's in `docs/features/stats.md`.
- **Release build**: `just stress` builds and runs with `--release`, so the numbers mean what the user will feel; one scenario at a time (`--test-threads 1`), so scenarios do not skew each other's timings.
- **24-hour soak**: the user asked for it, and hosted runners stop at 6 hours. The suite's soak takes its duration from `DECK_SOAK_SECS`; after merge, the lead runs `DECK_SOAK_SECS=86400 just stress soak` once on the user's machine, in the background, and records the summary in `docs/features/stats.md`. Daily use with the 10-minute log line is the long-term record.
- **Findings**: a leak or a stall the suite exposes is fixed in this spec when the fix is small; otherwise it becomes a backlog task and the scenario's bound is set to today's value with a comment naming the task.

## Sub-specs

| Sub-spec                 | Type      | Crate                        | Summary                                                                      | Depends | Status |
| ------------------------ | --------- | ---------------------------- | ---------------------------------------------------------------------------- | ------- | ------ |
| [01](01-stats.md)        | shippable | deck-daemon, deck-core, deck | Self-measurement, `Request::Stats`, `deck stats`, the periodic log line      | —       | todo   |
| [02](02-stress-suite.md) | shippable | deck (tests), repo           | Stress and soak scenarios, `just stress`, nightly workflow, bounds, findings | 01      | todo   |

## Affected docs

- `docs/features/stats.md` (new): `deck stats`, the log line, the stress suite and its measured numbers.
- `docs/features/README.md`: link to it.
- `docs/architecture.md`: the platform functions, `Request::Stats`, Testing (stress suite, nightly workflow), CI.
- `AGENTS.md`: `just stress` in the commands table.

Reviewed by spec-reviewer on 23 Nov 2026.
