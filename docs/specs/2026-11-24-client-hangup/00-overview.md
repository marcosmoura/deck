---
branch: feat/client-hangup
backlog: DECK-39
---

# Client exits when its terminal closes

## Goal

An attach client whose terminal goes away (the tab or window is closed, an ssh connection drops, the terminal app quits) exits within about a second and never burns CPU. Today it spins at 100% of a core forever: on 24 Nov 2026 the user found three such clients, 5 to 11 hours old, each at about 98% CPU, which ignored `SIGTERM` and needed `SIGKILL`.

Asked by the user on 24 Nov 2026: "create a spec to act on that bug and prioritize it".

## What happens today

Found by sampling a stuck client (`target/debug/deck` from `just run`) and reading the code. Line numbers are at `main` `50c9bd2`.

- The client's input reader is a thread in `run_async` (`crates/deck-tui/src/lib.rs:311-329`, `let reader = { … }`) that loops on `crossterm::event::poll(INPUT_POLL)` (50 ms) and `read()`, and stops on an error or when the `stop` flag is set.
- When the terminal is revoked, crossterm 0.29.0 never returns. In `UnixInternalEventSource::try_read` (`crossterm-0.29.0/src/event/source/unix/mio.rs:95-120`) the inner read loop leaves only on parsed data, `WouldBlock` or `Interrupted`. On a revoked terminal `read` returns `0` (end of file) or an error such as `EIO`: both fall through to `parser.next()`, which gives `None`, and the loop runs again without ever checking the timeout, so the thread spins at full speed.
- The main loop does notice the hang-up (a `SIGHUP`, a failed draw, or a `SIGTERM`) and leaves `'run`, then sets `stop` and calls `reader.join()` (`lib.rs:451`). The join waits forever for the spinning thread. The sample showed exactly that: the main thread in `pthread_join`, the reader thread in crossterm's `try_read`.
- That is also why `SIGTERM` did nothing: the handler only breaks the loop, which then blocks in the same join.
- The daemon and its sessions are not affected; only the client process is stuck.

## Builds on

- DECK-01's attach client and end-to-end harness (`crates/deck/tests/support/`); the `read_stdin` closure pattern by which `crates/deck/src/platform.rs` gives `deck-tui` its colour-query `poll` and `read` (`Options.read_stdin`, `crates/deck/src/control.rs`), and the `unsafe` placement rules in `docs/architecture.md`.

## Non-goals

- Patching, vendoring or replacing crossterm, or changing its version. Reporting the bug upstream is the user's call.
- Changing what the daemon does when a client goes away (sessions stay, as today).
- Daemons left by `just run` after its client exits: a daemon outliving its client is how deck works.

## Decisions

- **Two fixes, both required.**
  1. **Never block exit on the reader.** After the main loop ends, the client waits for the reader thread at most 200 ms; a reader that has not finished is left running and ends with the process. Every exit path benefits (`SIGTERM`, `SIGHUP`, `Lost`, upgrade re-exec), and it also bounds the race fix 2 cannot close.
  2. **The reader never waits inside crossterm.** The reader waits for input on descriptor 0 itself, with `poll(2)` through a closure built in `crates/deck/src/platform.rs` (so `deck-tui` stays free of `unsafe`), and calls crossterm only with a zero timeout to drain what is ready. A hang-up is detected without consuming input: `POLLHUP` or `POLLERR`, or `POLLIN` with `ioctl(FIONREAD)` reporting 0 bytes or failing (on macOS a revoked tty reads as end of file and polls readable; deadfs ioctls fail), or `tcgetattr` on descriptor 0 failing. On a hang-up the reader stops and the main loop ends through the existing `Exit::Error("lost the terminal input")`. A hang-up that lands between the check and crossterm's own `read` can still enter crossterm's loop; fix 1 bounds that case to a 200 ms wait before exit, and the spinning thread dies with the process.
- **Only a terminal stdin is checked.** crossterm reads descriptor 0 only when stdin is a tty (otherwise it opens `/dev/tty`); the attach client already requires a tty stdin (`control.rs`, "attach needs a terminal"), and the check runs only when `isatty(0)`.
- **Bound.** The client is gone within 1 s of the hang-up (tests allow 5 s), with no output from the daemon needed: an idle session must also end its client.
- **No new dependency, no protocol change.** `deck-tui` and `deck`.
- **Conflicts with DECK-26** (`feat/reviews`, in progress): that branch rewrites the `unsafe` placement paragraph of `docs/architecture.md` and the module doc of `crates/deck/src/platform.rs`, and changes `run`, `run_async`'s signature and `Link::open` in `crates/deck-tui/src/lib.rs` (not the reader or the join). Whichever merges second rebases onto `main` and merges those by hand.

## Sub-specs

| Sub-spec           | Type      | Crate          | Summary                                                                                                          | Depends | Status |
| ------------------ | --------- | -------------- | ---------------------------------------------------------------------------------------------------------------- | ------- | ------ |
| [01](01-hangup.md) | shippable | deck-tui, deck | Bounded reader join, a reader that never waits in crossterm, hang-up tests with a harness that can close its PTY | —       | todo   |

## Affected docs

- `docs/features/sessions.md`: closing the terminal a client runs in detaches it within a second; the session stays.
- `docs/architecture.md`: the new stdin calls (`poll`, `ioctl(FIONREAD)`, `tcgetattr`, `isatty`) in `crates/deck/src/platform.rs` in the `unsafe` placement list; the client's input reader; the two new `DECK_TEST_*` seams with the others; the harness's `close_master`.

Reviewed by spec-reviewer on 24 Nov 2026.
