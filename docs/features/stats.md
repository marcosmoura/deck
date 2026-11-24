# Stats

Read when you need to know how much the daemon uses, or what `deck stats` and the `stats:` lines of `daemon.log` mean.

The daemon measures itself: resident memory, open file descriptors, threads and CPU time (user and system). `deck stats` shows them, with what the daemon holds, and the daemon logs a summary line every 10 minutes, so a few weeks of use leave a trail in `daemon.log` of any drift.

## `deck stats`

```text
deck daemon pid 4312, up 3d 4h, version 0.4.0 (protocol 22)
memory 84.2 MiB  fds 61  threads 38  cpu 312.4s user, 41.0s system
2 sessions, 14 terminals (1 exited), 1 client

terminal  session  size    history  read
4         main     120x40  10000    48.1 MiB
…
```

- The first line is the daemon's pid, how long it has run, its version and its protocol version.
- `memory` is resident memory, `fds` the open descriptors, `threads` the threads of the daemon process (every terminal has a few), `cpu` the CPU time since the daemon started. A figure the daemon cannot read shows as `?`.
- `terminals` counts the live ones and the ones whose program ended and that stay on screen (`exited`). `clients` counts the TUI clients attached to a session; the control connections, including the one asking, are not counted.
- The table has one row per terminal: its id, its session, its size in columns and rows, the lines of history it can scroll through, and the bytes read from its PTY since it started (`read`).
- `--json` prints one JSON document with the same figures: `running` (always `true`), `pid`, `uptime_secs`, `version`, `protocol`, `rss_bytes`, `fds`, `threads`, `cpu_user_secs`, `cpu_system_secs`, `sessions`, `terminals` (live and exited), `exited`, `clients` and `terminal_list` (`id`, `session`, `history`, `cols`, `rows`, `bytes_read`). Sizes are in bytes and durations in seconds; a figure that could not be read is `null`.
- With no daemon, `deck stats` prints `no deck daemon running` and exits 0 (with `--json`: `{"running":false}`). It never starts a daemon. Inside deck it asks the daemon through `DECK_SOCKET`, like `deck ls`.

## The log line

Every 10 minutes, and once when the daemon shuts down, the daemon logs at INFO:

```text
stats: rss=84.2 fds=61 threads=38 cpu=353.4s sessions=2 terminals=14 clients=1
```

`rss` is in MiB, `cpu` is user plus system, `terminals` counts live and exited ones. A figure the daemon cannot read is left out of the line. `DECK_TEST_STATS_LOG_MS` shortens the interval, for tests.

## How it is measured

| Figure      | macOS                             | Linux                             |
| ----------- | --------------------------------- | --------------------------------- |
| memory      | `proc_pidinfo` `PROC_PIDTASKINFO` | `/proc/self/statm` (pages)        |
| descriptors | `proc_pidinfo` `PROC_PIDLISTFDS`  | entries of `/proc/self/fd`        |
| threads     | `proc_pidinfo` `PROC_PIDTASKINFO` | `Threads:` in `/proc/self/status` |
| CPU time    | `getrusage(RUSAGE_SELF)`          | `getrusage(RUSAGE_SELF)`          |

The daemon measures on a blocking thread, never on an async worker. The bytes read from a PTY are counted by the terminal's reader thread. Threads of tokio's blocking pool stay for 10 seconds after their work, so the thread count settles back about that long after terminals close.

## The stress suite

`just stress [filter]` runs the ignored tests of `crates/deck/tests/stress.rs` in a release build, one at a time, with the open-file limit raised to 4096. `just check` does not run them. Each scenario starts its own daemon on a temp `DECK_HOME`, measures it with `deck stats --json`, writes `target/stress/<scenario>.json` and prints a one-line summary. `DECK_SOAK_SECS` sets the soak's length (default 600); the soak also writes `target/stress/soak.csv`, one row every 10 seconds.

| Scenario               | What it does                                                                                                                                   |
| ---------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| `flood`                | One terminal prints about 270 MiB of 100-column lines while a client is attached; a key typed into a second terminal of the panel must echo    |
| `many_terminals`       | 64 terminals in 16 panels print 10,000 lines each; then `deck ls` is timed                                                                     |
| `churn_500_terminals`  | Opens and closes 500 terminals; within 15 s descriptors and threads return to the baseline (within 2) and memory to within 16 MiB of it        |
| `churn_catches_a_leak` | The same with `DECK_TEST_LEAK_FD=1` (the daemon leaks a descriptor per terminal it starts); the descriptor check must fail                     |
| `attach_loop`          | Attaches and detaches a client 300 times; descriptors return to the baseline, no client is left                                                |
| `control_loop`         | 2,000 `deck send` plus `deck capture`; descriptors return to the baseline                                                                      |
| `soak`                 | A mixed load (a line every 10 ms, a terminal opened and closed every 5 s, a client attached for 1 s every 30 s) with `scrollback_lines = 1000` |

The soak samples every 10 s. After a 120 s warm-up, with at least 30 samples, it fails when the least-squares slope of memory is above 50 MiB per hour or that of descriptors above 1 per hour. Terminals and clients come and go between the samples, so every sample sees the same quiet state. A final quiet minute (only the first shell left) fails when the daemon uses more than 0.2 CPU seconds.

### Measured values and bounds

Measured on 24 Nov 2026 on a MacBookPro18,3 (Apple M1 Pro, 10 cores), macOS 27.3, release build. Time bounds are 3 to 5 times the measured value, memory bounds twice, descriptor and thread checks the baseline plus 2 (the rule of the [spec](../specs/2026-11-23-daemon-stats/00-overview.md)). Linux and the hosted runners are filled in after the nightly workflow's first run.

| Scenario       | Figure                                               | Measured                        | Bound        |
| -------------- | ---------------------------------------------------- | ------------------------------- | ------------ |
| flood          | time to print 270 MiB                                | 12.8 s                          | 60 s         |
| flood          | peak memory (sampled every 250 ms)                   | 34 MiB                          | 70 MiB       |
| flood          | key echo during the flood, median / max              | 7 ms / 20 ms                    | 0.1 s max    |
| flood          | `deck stats` during the flood, max                   | 9 ms                            | not bounded  |
| many terminals | memory with 65 terminals                             | 276 MiB                         | 560 MiB      |
| many terminals | memory per terminal                                  | 4.1 MiB                         | 8.5 MiB      |
| many terminals | `deck ls`                                            | 3 ms                            | 0.25 s       |
| churn          | descriptors after 500 cycles                         | 17, baseline 17                 | baseline + 2 |
| churn          | threads after 500 cycles                             | 18, baseline 18                 | baseline + 2 |
| churn          | memory above the baseline                            | +0.7 MiB                        | 16 MiB       |
| attach loop    | descriptors after 300 attaches                       | 17, baseline 17                 | baseline + 2 |
| attach loop    | time (each attach waits out the 200 ms colour query) | 88.6 s                          | not bounded  |
| control loop   | descriptors after 2,000 send + capture               | 20, baseline 20                 | baseline + 2 |
| control loop   | time                                                 | 13.8 s                          | not bounded  |
| soak (600 s)   | memory slope                                         | -1.6 and -11.4 MiB/h (two runs) | 50 MiB/h     |
| soak (600 s)   | descriptor slope                                     | 0.0 /h                          | 1 /h         |
| soak (600 s)   | idle CPU, quiet minute                               | 0.03 s                          | 0.2 s        |

The churn baseline is taken after three warm-up cycles and 12 s, because the daemon keeps two or three threads for good once it has started its first terminals. The 24-hour soak (`DECK_SOAK_SECS=86400 just stress soak`) is run by hand after merge. The soak's 50 MiB/h bound is the same whatever its length, and a slope over 10 minutes is far noisier than one over 24 hours, so read the 24-hour run's memory slope by hand against the 10-minute values above instead of relying on the bound.
