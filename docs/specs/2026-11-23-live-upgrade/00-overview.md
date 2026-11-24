---
branch: feat/live-upgrade
backlog: DECK-21
---

# Live upgrade

## Goal

Installing a new `deck` no longer kills what runs inside it. `deck upgrade` (and `deck`, when it meets an older daemon) hands the running daemon over to the new binary in place: the daemon re-executes itself (`execve`, same pid), so every shell, editor, dev server, REPL, ssh session and agent keeps running, with its screen and scrollback, and attached clients reattach by themselves.

Asked by the user on 23 Nov 2026, as part of making deck a daily driver: every protocol bump today means `deck kill-server`, which restores the layout but kills every running program.

## Builds on

- [DECK-01 core-daemon-terminal](../2026-11-22-core-daemon-terminal/00-overview.md): daemon start-up, `deck.lock` with the pid, the hello/`Bye` handshake and its stable variant indices, `kill-server`'s `SIGTERM` fallback.
- [DECK-04 persistence](../2026-11-22-persistence/00-overview.md): `state.json`, the shutdown save, what a restart loses.
- [DECK-06 agent-status](../2026-11-22-agent-status/00-overview.md): per-terminal agent state.
- [DECK-19 kitty-images](../2026-11-23-kitty-images/00-overview.md): the emulator thread behind `with_emu` and `libghostty-vt`. This spec is written against the `Emu` API that DECK-19 keeps; it must land after DECK-19.
- `docs/features/sessions.md` (the shutdown log lines added on 23 Nov 2026).

## Non-goals

- Surviving a daemon crash, `SIGKILL` or a reboot: the PTY masters die with the process. Persistence (DECK-04) still restores the layout then.
- Downgrades: `deck upgrade` refuses a binary whose handoff version is older than the running daemon's.
- Upgrading a daemon that predates this spec: the first daemon with live upgrade is started once with `deck kill-server` + `deck`, as today.
- Keeping kitty images (DECK-19), the emulator's palette changes (OSC 4/10/11 set by a program), scroll regions, tab stops and charset state across the handoff: full-screen programs redraw them after the size nudge (requirement in [02](02-handoff.md)); a plain shell does not need them.
- Keeping attached client connections open across the exec: clients reconnect.
- Changing `kill-server`, `--forget` or `state.json`.

## Decisions

- **In-place `execve`, not a second process.** The daemon keeps its pid, so terminal programs stay its children (`waitpid` keeps working, exit statuses stay exact), and the PTY masters, the listening socket and both lock fds survive by having `FD_CLOEXEC` cleared just before the exec. No connection is refused during the handoff: connections wait in the listen backlog.
- **The trigger is a file and a signal, not a protocol message,** because the client and the daemon speak different protocol versions by definition. `deck upgrade` writes its own executable path to `<runtime>/deck.upgrade` (0600) and sends `SIGUSR2` to the pid in `deck.lock`. A daemon that supports this keeps a marker, `<runtime>/deck.handoff`, with its handoff version, pid, a generation that grows with each adoption and whether a handoff is under way; the client sends `SIGUSR2` only when the marker exists and names the pid in `deck.lock`, since `SIGUSR2` kills a daemon without a handler. The client learns the outcome from the marker's generation or from `deck.upgrade.result`. `deck.lock` keeps holding only the pid, so older clients' `kill-server` still works.
- **The new binary is the requesting client's,** not the daemon's original path, so a Homebrew upgrade (a new Cellar path, the old one removed) works.
- **Preflight before the point of no return.** The daemon runs `<exe> daemon --handoff-check <version>` and execs only when it exits 0. A failed exec restores everything and keeps running; the failure is logged, shown as a notice and written to `deck.upgrade.result` for the client. `state.json` is saved before the exec, so even an adoption that fails outright loses only the programs, not the layout.
- **A frozen daemon during the handoff.** Readers, waiters, actions, launches and tasks stop before the snapshot, so nothing changes between the snapshot and the exec; termination signals are recorded and honoured after the handoff, and the four signals are blocked across the `execve` (the mask survives it).
- **Terminal state crosses as a VT replay, not as emulator internals.** Each terminal's history, screen, alternate screen, cursor and modes are written as escape sequences (`Emu::snapshot`) and fed into a fresh `Emu` by the new daemon. This survives emulator changes (DECK-19's `libghostty-vt` today, anything later) and needs no serialisable emulator.
- **Adopted PTYs plug into the existing trait objects.** `TermHandle` holds `Box<dyn portable_pty::MasterPty>` and `Box<dyn Child>`; the new daemon wraps inherited fds and pids in deck's own types implementing those traits, in `deck-daemon/src/platform/` (the only place with `unsafe`).
- **Readers pause, no output is lost.** Before the snapshot, every PTY reader stops reading; bytes written meanwhile stay in the kernel's PTY buffer and are read by the new daemon after the replay.
- **Handoff file**: JSON (serde_json, already a dependency), in an unlinked 0600 temp file in the runtime directory whose fd is passed by number. It carries a `HANDOFF_VERSION`; a daemon accepts every handoff version from 1 up to its own.
- **Clients**: a new `ByeReason::Upgrading { exe }`, appended after the existing variants. An attached client that gets it restores the outer terminal and executes `<exe> attach <its session>`, so the user sees a short redraw. `PROTOCOL_VERSION` is bumped once, in [03](03-upgrade-command.md).
- **`deck` and `deck attach` upgrade by themselves** when the daemon answers with an older protocol version and supports handoff, printing `upgrading the deck daemon (protocol N → M)`. A client older than the daemon keeps today's mismatch error.

## Sub-specs

| Sub-spec                    | Type       | Crate                     | Summary                                                                                         | Depends | Status |
| --------------------------- | ---------- | ------------------------- | ----------------------------------------------------------------------------------------------- | ------- | ------ |
| [01](01-snapshot-adopt.md)  | foundation | deck-daemon               | `Emu::snapshot` VT replay with round-trip tests; adopted PTY and child types; pausable readers  | DECK-19 | done   |
| [02](02-handoff.md)         | foundation | deck-daemon, deck         | Handoff file, `SIGUSR2`, preflight, CLOEXEC, `execve`, `daemon --adopt`, rollback on failure    | 01      | done   |
| [03](03-upgrade-command.md) | shippable  | deck, deck-tui, deck-core | `deck upgrade`, `ByeReason::Upgrading` and client reattach, automatic upgrade on mismatch, docs | 02      | done   |

## Affected docs

- `docs/features/sessions.md`: `deck upgrade`, automatic upgrade, what survives, the log lines.
- `docs/features/persistence.md`: upgrade versus restart (what each keeps).
- `docs/architecture.md`: the handoff (files in the runtime directory, the exec, fd inheritance, the snapshot), `ByeReason::Upgrading`, the new `platform` code, a Decisions row.
- `README.md` and, when [DECK-25](../2026-11-23-release-homebrew/00-overview.md) has landed, the Homebrew formula's caveats: run `deck upgrade` after upgrading.

Reviewed by spec-reviewer on 23 Nov 2026.
