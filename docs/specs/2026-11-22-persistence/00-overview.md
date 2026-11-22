---
branch: feat/persistence
backlog: DECK-04
---

# Persistence across daemon restarts

## Goal

Sessions survive the daemon stopping: after `deck kill-server`, a crash, a logout or a reboot, the next `deck` brings back every session with its projects, panels, terminal grid, names, ratios and focus, with each shell started in the directory it was last in. Commands started explicitly are relaunched. Scrollback is not kept.

## Builds on

- [DECK-01 core-daemon-terminal](../2026-11-22-core-daemon-terminal/00-overview.md): the daemon's start-up, signal handling and `kill-server` (which waits up to 5 s for the daemon to release the lock and socket), the client's environment carried in `Hello`.
- [DECK-08 config-theming](../2026-11-22-config-theming/00-overview.md): `ServerMsg::Notice`.
- [DECK-03 grid-layout](../2026-11-22-grid-layout/00-overview.md): the full model, including ratios and focus.
- [architecture.md#persistence](../../architecture.md#persistence) and [#paths](../../architecture.md#paths).

## Non-goals

- Restoring scrollback or screen contents.
- Relaunching whatever happened to run in a shell (an editor, a dev server): only explicit commands and, in DECK-06, agents.
- Starting the daemon at login (launchd or systemd units). The first `deck` after a reboot restores.
- Sharing or syncing state between machines.
- Durability against power loss beyond a best effort (see the decisions).

## Decisions

- **One file**, `state.json`, rewritten whole and atomically after changes ([architecture.md#persistence](../../architecture.md#persistence)). A daemon killed with `SIGKILL` loses at most the last 500 ms of changes. The write is safe against process crashes; against power loss it is best effort (the file and its directory are fsynced, but macOS's `F_FULLFSYNC` is not used).
- **What restores**: every session, project, panel and terminal with its id, name, name source, order, ratios, collapsed state and focus; the ID counter. Every terminal, shell or explicit program, starts in the first directory that exists of: its saved cwd, its project root, the home dir. Terminals with an explicit `program` are relaunched with the same program, args and env. The `DECK_*` variables are regenerated, never saved. Exited terminals kept by `keep` (DECK-05) are not restored.
- **Shutdown never loses terminals**: the daemon marks itself as shutting down before anything else, so terminals that end because of the shutdown stay in the saved state.
- **`deck kill-server`** saves and stops; the next start restores. `deck kill-server --forget` deletes `state.json` once the daemon is gone (also when none is running), for a clean slate. The protocol does not change for it.
- **Notices**: problems found while restoring (an unreadable file, a terminal that could not be relaunched) reach the user as a `ServerMsg::Notice` shown by the next client that attaches.
- **cwd tracking**: from OSC 7 when the shell reports it, else by asking the OS for the cwd of the terminal's own child process (the shell or the program) every 5 s. The foreground job is never used: its cwd is not where the shell is. Default zsh and bash on macOS emit OSC 7 only in some terminals, so the fallback matters.
- **New terminal in a panel** starts in that panel's focused terminal's cwd (replacing DECK-03's project-root default). New panels still start in the project root.
- **Protocol**: this spec bumps `PROTOCOL_VERSION` once, for the terminal cwd in `SessionView`. `Notice` comes from DECK-08.

## Sub-specs

| Sub-spec                 | Type      | Crate                        | Summary                                                                     | Depends | Status |
| ------------------------ | --------- | ---------------------------- | --------------------------------------------------------------------------- | ------- | ------ |
| [01](01-save-restore.md) | shippable | deck-daemon, deck-core, deck | Save `state.json`, restore on start, shutdown safety, notices, `--forget`   | DECK-03 | draft  |
| [02](02-cwd-tracking.md) | shippable | deck-daemon, deck-core       | Live cwd from OSC 7 and the OS; restore into it; new terminals inherit it    | 01      | draft  |

## Limitations

- After `kill -9` of the daemon, child processes that ignore `SIGHUP` (`nohup`, their own `setsid`) keep running, and the restored daemon relaunches explicit commands next to them (two dev servers on one port, for example).
- Restored terminals start at 80×24, so their first prompt is drawn at that size and reflowed when a client attaches.

## Affected docs

- `docs/features/persistence.md` (new): what survives, what does not, `--forget`, where the file is, the limitations.
- `docs/features/sessions.md`: `kill-server` keeps sessions.
- `docs/architecture.md#persistence`: the file format version and fields, the shutdown flag, the restore order.

Reviewed by spec-reviewer on 22 Nov 2026.
