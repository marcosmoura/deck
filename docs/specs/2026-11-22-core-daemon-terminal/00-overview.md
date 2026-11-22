---
branch: feat/core-daemon-terminal
backlog: DECK-01
---

# Core: daemon, PTYs, attach and detach

## Goal

Lay down the repository and the client/daemon core, and ship the first thing the user can run: `deck` opens a session with one shell in a rounded frame, `Ctrl+Space d` detaches, the shell keeps running, and `deck` again brings it back exactly as it was. Then scrollback, the mouse and focus events work inside it.

## Builds on

None. Uses the decisions in [architecture.md](../../architecture.md): process model, crates, protocol, paths, testing and dependencies.

## Non-goals

- Several sessions, projects, panels, the sidebar (DECK-02). This spec creates exactly one project, one panel and one terminal per session, with the real model types.
- Several terminals per panel (DECK-03), persistence across a daemon restart (DECK-04), control commands beyond `ls` and `kill-server` (DECK-05), agents (DECK-06).
- Selecting and copying text inside deck. The outer terminal's own selection works with its bypass modifier (usually Shift or Option while dragging).
- The kitty keyboard protocol, images (sixel, kitty graphics), Windows.

## Decisions

- `deck` with no arguments attaches to the most recently attached session, or creates one named `main` (then `main-2`, …) whose single project is the current directory (named after its basename).
- Default prefix `Ctrl+Space`; pressing it twice sends it to the terminal. While armed, the status bar shows `PREFIX`; `Esc` disarms. An unbound key after the prefix disarms and flashes `no binding for <key>` for 2 s.
- The terminal gets `TERM=xterm-256color` and `COLORTERM=truecolor`, plus the `DECK_*` variables of [architecture.md](../../architecture.md#agents).
- The shell is the config's `shell`, else the client's `$SHELL`, else `/bin/sh`, started with `shell_args` (default `["-l"]`, a login shell). No argv0 tricks: portable-pty sets argv0 to the program.
- **Environment**: the terminals of a session inherit the environment of the client that created the session (sent in `Hello`), minus `DECK_*`, `TMUX`, `TMUX_PANE`, `COLUMNS`, `LINES` and `TERM_PROGRAM*`; deck then sets `TERM`, `COLORTERM` and the `DECK_*` variables. The locale is passed through untouched.
- **First size**: the creating client sends its panel area in `Hello`, so the first shell starts at the right size.
- **Lifecycle until DECK-02**: a session whose last terminal exits ends ([architecture.md#model](../../architecture.md#model) records this as temporary; DECK-02 replaces it).
- Scrollback: 10,000 lines per terminal by default (`scrollback_lines`).
- A version mismatch between client and daemon is reported, never worked around ([architecture.md](../../architecture.md#protocol)).

## Sub-specs

| Sub-spec                          | Type       | Crate                         | Summary                                                              | Depends | Status |
| --------------------------------- | ---------- | ----------------------------- | -------------------------------------------------------------------- | ------- | ------ |
| [01](01-workspace-tooling.md)     | foundation | root, all crates (skeletons)  | Cargo workspace, four crates, mise, just, Lefthook, lints            | —       | draft  |
| [02](02-core-types.md)            | foundation | deck-core                     | Model, protocol and framing, paths, config, `KeySpec`, bindings, `keys::encode` | 01 | draft |
| [03](03-daemon.md)                | foundation | deck-daemon, deck             | Daemon: socket, lock, sessions, PTYs, emulation, frames, `deck daemon` | 02    | draft  |
| [04](04-attach-client.md)         | shippable  | deck-tui, deck                | `deck` attaches, draws, types, resizes, detaches; CLI; e2e harness    | 03      | draft  |
| [05](05-scrollback-mouse.md)      | shippable  | deck-core, deck-daemon, deck-tui | Scroll mode, mouse wheel, mouse reporting, focus events            | 04      | draft  |

02 and the harness part of 04 do not depend on 03's internals beyond the protocol, but 04's end-to-end tests need the daemon, so the sub-specs run in order.

## Affected docs

- `README.md` (new): what deck is, install from source, `deck`, detach and reattach.
- `docs/features/sessions.md` (new): attach, detach, the default session, `deck ls`, `deck kill-server`.
- `docs/features/keys.md` (new): the prefix and every binding, kept current by later specs.
- `docs/architecture.md`: anything this spec settles differently; when DECK-02 lands, the temporary lifecycle note goes.

Reviewed by spec-reviewer on 22 Nov 2026.
