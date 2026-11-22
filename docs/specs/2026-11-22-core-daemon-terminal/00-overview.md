---
branch: feat/core-daemon-terminal
backlog: DECK-01
---

# Core: daemon, PTYs, attach and detach

## Goal

Lay down the repository and the client/daemon core, and ship the first thing the user can run: `deck` opens a session with one shell in a rounded frame, `Option+Space d` detaches, the shell keeps running, and `deck` again brings it back exactly as it was. Then scrollback, the mouse and focus events work inside it.

## Builds on

None. Uses the decisions in [architecture.md](../../architecture.md): process model, crates, protocol, paths, testing and dependencies.

## Non-goals

- Several sessions, projects, panels, the sidebar (DECK-02). This spec creates exactly one project, one panel and one terminal per session, with the real model types.
- Several terminals per panel (DECK-03), persistence across a daemon restart (DECK-04), control commands beyond `ls` and `kill-server` (DECK-05), agents (DECK-06).
- Selecting and copying text inside deck (a copy mode). Programs that copy through OSC 52 (Claude Code, nvim, …) do reach the clipboard (decisions), and the outer terminal's own selection works with its bypass modifier (usually Shift or Option while dragging), though in a grid it also selects borders and neighbouring terminals.
- The kitty keyboard protocol (so `Shift+Enter` arrives as `Enter`, which matters to agents that use it for new lines), images (sixel, kitty graphics), Windows.

## Decisions

- `deck` with no arguments attaches to the most recently attached session, or creates one named `main` (then `main-2`, …) whose single project is the current directory (named after its basename; the home dir is `~` and the filesystem root `root`, since names may not contain `/`).
- Default prefix `Option+Space` (`M-Space`; macOS reserves `Option+Space` for input-source switching); pressing it twice sends the key as received to the terminal. The non-breaking space macOS terminals send for Option+Space when Option is not Alt also counts as the prefix. While armed, the status bar shows `PREFIX`; `Esc` disarms. An unbound key after the prefix disarms and flashes `no binding for <key>` for 2 s.
- The terminal gets `TERM=xterm-256color` and `COLORTERM=truecolor`, plus the `DECK_*` variables of [architecture.md](../../architecture.md#agents).
- The shell is the config's `shell`, else the client's `$SHELL`, else `/bin/sh`, started with `shell_args` (default `["-l"]`, a login shell). No argv0 tricks: portable-pty sets argv0 to the program.
- **Environment**: the terminals of a session inherit the environment of the client that created the session (sent in `Hello`), minus `DECK_*`, `TMUX`, `TMUX_PANE`, `COLUMNS`, `LINES` and `TERM_PROGRAM*`; deck then sets `TERM`, `COLORTERM` and the `DECK_*` variables. The locale is passed through untouched. Each attach refreshes `SSH_AUTH_SOCK`, `SSH_CONNECTION`, `SSH_TTY`, `DISPLAY`, `WAYLAND_DISPLAY` and `XAUTHORITY` from the attaching client for terminals started afterwards ([architecture.md#model](../../architecture.md#model)).
- **Clipboard**: OSC 52 copies from programs are forwarded to the outer terminal through the most recently active client; OSC 52 reads are denied (user decision, 22 Nov 2026).
- **Colours**: the client asks the outer terminal for its foreground and background (OSC 10/11) at attach, so programs inside deck that ask the same question (Codex, OpenCode, nvim) pick a matching light or dark theme.
- **Upgrades**: `deck kill-server` stops a daemon of any protocol version (the pid is in `deck.lock`; `SIGTERM` when the daemon cannot answer), so the advice printed on a version mismatch always works.
- **First size**: the creating client sends its panel area in `Hello`, so the first shell starts at the right size.
- **Lifecycle until DECK-02**: a session whose last terminal exits ends ([architecture.md#model](../../architecture.md#model) records this as temporary; DECK-02 replaces it).
- Scrollback: 10,000 lines per terminal by default (`scrollback_lines`).
- A version mismatch between client and daemon is reported, never worked around ([architecture.md](../../architecture.md#protocol)).

## Sub-specs

| Sub-spec                          | Type       | Crate                         | Summary                                                              | Depends | Status |
| --------------------------------- | ---------- | ----------------------------- | -------------------------------------------------------------------- | ------- | ------ |
| [01](01-workspace-tooling.md)     | foundation | root, all crates (skeletons)  | Cargo workspace, four crates, mise, just, Lefthook, lints            | —       | done   |
| [02](02-core-types.md)            | foundation | deck-core                     | Model, protocol and framing, paths, config, `KeySpec`, bindings, `keys::encode` | 01 | done   |
| [03](03-daemon.md)                | foundation | deck-daemon, deck             | Daemon: socket, lock, sessions, PTYs, emulation, frames, `deck daemon` | 02    | done   |
| [04](04-cli-harness.md)           | foundation | deck                          | CLI (`ls`, `kill-server`, daemon start-up, guards), e2e harness       | 03      | done   |
| [05](05-attach-client.md)         | shippable  | deck-tui, deck                | `deck` attaches, draws, types, resizes, detaches; colours; clipboard  | 04      | done   |
| [06](06-scrollback-mouse.md)      | shippable  | deck-core, deck-daemon, deck-tui | Scroll mode, mouse wheel, mouse reporting, focus events            | 05      | done   |

The sub-specs run in order: each one's tests need the one before.

## Affected docs

- `README.md` (new): what deck is, install from source, `deck`, detach and reattach.
- `docs/features/sessions.md` (new): attach, detach, the default session, `deck ls`, `deck kill-server`.
- `docs/features/keys.md` (new): the prefix and every binding, kept current by later specs.
- `docs/architecture.md`: anything this spec settles differently; when DECK-02 lands, the temporary lifecycle note goes.

Reviewed by spec-reviewer on 22 Nov 2026.
