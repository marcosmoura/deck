---
branch: feat/hardening
backlog: DECK-49
---

# Hardening for daily use

## Goal

Make deck safe to use as the only terminal multiplexer, day in, day out: close the last known privacy hole, make heavy output and large panels cheap, fuzz the emulator input, and prove it with long runs on both platforms. Asked by the user on 26 Nov 2026 ("all follow-ups in one spec") after DECK-26 merged; it takes over DECK-28, DECK-37, DECK-40 and DECK-42 to DECK-47, whose notes it carries, and the macOS `just stress` run DECK-26 merged without.

## Builds on

- [DECK-26 reviews](../2026-11-23-reviews/00-overview.md): the reports in `docs/reviews/`, whose findings these sub-specs close.
- [DECK-23 daemon stats](../2026-11-23-daemon-stats/00-overview.md): the stress scenarios and bounds that prove them ([docs/features/stats.md](../../features/stats.md), `just stress`).

## Non-goals

- New features. Every sub-spec is a fix with a measured before and after, or a verification run.
- DECK-32 to DECK-36 and DECK-38: they stay separate tasks.
- Moving a stress bound to make a run pass: a bound changes only with a recorded reason.
- A protocol change for client images (06 stays in the client; see Decisions).

## Decisions

- **The machine is shared (the user, 25–26 Nov 2026).** Long or repeated runs (test loops, the fuzzer, the soak, Linux stress, every before/after measurement) run in the Linux VM of 01, never uncapped on the Mac. Builds and tests on the Mac run under `nice` with at most 4 build jobs and 3 test threads.
- **The Linux VM** is Apple's `container` (a macOS tool the user already uses; approved by this spec as a developer tool, not a build dependency), set up by 01's `just` recipes: Ubuntu 24.04, **4 CPUs and 8 GB, fixed for every measurement** (the soak: 2 CPUs), the repository mounted read-only with writable build directories.
- **Before and after** for a perf sub-spec: "before" is the branch tip just before the sub-spec, "after" its finished state, both measured in the VM in one session, each the median of 3 runs. A figure taken on macOS is compared only with another macOS figure.
- **macOS runs need the user.** 02 and 12 run `just stress` on macOS in windows the user names; a sub-spec whose problem does not show in the VM (06 may not: memory not given back can be allocator behaviour) takes its numbers on macOS in such a window.
- **Fuzzing** uses `cargo-fuzz` (DECK-37 approves it) with a nightly toolchain installed through rustup inside the VM and used only as `cargo +nightly fuzz` by the fuzz recipe; the pinned stable toolchain in `.mise.toml` does not change.
- **Client images (06)** stay a client change: the client drops the pixels of images no live terminal shows and accepts a re-send when a terminal comes back. If a fix needs the daemon to resend on demand (a protocol message), the worker stops and the lead amends this spec.
- **Long runs.** A worker builds and starts a run of hours (the fuzzer, the soak) and stops; the lead records the results. A crash or finding that is not fixed in its sub-spec becomes a backlog row, added by the lead on `main`.
- **Agents.** One worker per sub-spec, on its wrapper's model; reviewers on theirs. The lead does not change code inline.
- **Order.** 02 waits for the user's window and blocks nothing; the rest run in the table's order.

## Sub-specs

| Sub-spec                     | Type       | Crate        | Summary                                                                      | Depends | Status |
| ---------------------------- | ---------- | ------------ | ---------------------------------------------------------------------------- | ------- | ------ |
| [01](01-linux-vm.md)         | foundation | repo         | `just` recipes for the capped Linux VM; the tool recorded                    | —       | todo   |
| [02](02-macos-baseline.md)   | foundation | deck, daemon | `just stress` on macOS at `9aedd08`; the macOS baseline                      | —       | todo   |
| [03](03-opencode-listing.md) | shippable  | daemon       | OpenCode sessions of repositories listed without loading them (DECK-40)      | 01      | todo   |
| [04](04-reader-ahead.md)     | foundation | daemon       | Output read while the emulator works (DECK-46)                               | 01      | todo   |
| [05](05-state-lock.md)       | foundation | daemon       | State lock never held across emulator calls (DECK-43)                        | 04      | todo   |
| [06](06-client-images.md)    | foundation | tui          | Client image memory released; images kept across a switch (DECK-44, DECK-45) | 01      | todo   |
| [07](07-history-width.md)    | foundation | daemon       | Terminal history sized by width (DECK-42)                                    | 01      | todo   |
| [08](08-slow-client.md)      | foundation | daemon       | Queued frames merged for a slow client (DECK-47)                             | 05      | todo   |
| [09](09-fuzzing.md)          | foundation | daemon, repo | Fuzz target for the emulator feed (DECK-37)                                  | 01      | todo   |
| [10](10-soak.md)             | foundation | deck         | 24-hour soak in the VM (DECK-28)                                             | 03–09   | todo   |
| [11](11-hosted-stress.md)    | foundation | deck         | The hosted stress workflow's numbers (DECK-28)                               | 10      | todo   |
| [12](12-macos-final.md)      | foundation | deck         | `just stress` on macOS on the finished branch                                | 02, 11  | todo   |

## Affected docs

`docs/reviews/2026-11-25-performance.md` and `docs/reviews/2026-11-24-privacy.md` (findings closed), `docs/features/privacy.md`, `docs/features/agents.md`, `docs/features/images.md`, `docs/features/stats.md`, `docs/architecture.md` (the VM and fuzz tools, the reader, the state lock, the emulator's history budget), `docs/workflow.md` (where long runs go), the `justfile`.

## Notes

- 04 changes the PTY reader, the same code as DECK-41 (the reader-pause test); the reader-pause tests must keep passing.

Reviewed by spec-reviewer on 26 Nov 2026 (two rounds).
