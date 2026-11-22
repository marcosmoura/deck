---
type: shippable
status: todo
---

# 03 — Session picker and session CLI

## Scope

The user switches between sessions without leaving deck, creates, renames and kills sessions from a picker, and does the same from the shell with `deck new`, `deck attach`, `deck kill-session` and `deck rename-session`. Owning crates: `deck-tui` (picker, re-attach), `deck` (CLI).

## Requirements

1. **Binding** `sessions = "s"` opens the picker; `rename_session = "$"` prompts `Rename session:`.
2. **Picker** (see [Screen](#screen)): a centered rounded overlay listing every session (from `ListSessions`, refreshed when it opens) with its name, project count, terminal count and `attached` when another client is attached; the current session is marked `●`. Typing filters by substring (case-insensitive); `Up`/`Down` (and `Ctrl+p`/`Ctrl+n`) move; `Enter` switches; `Ctrl+o` creates a new session from the filter text (or prompts for a name when the filter is empty) with the client's start cwd and switches to it; `Ctrl+r` renames the selected session; `Ctrl+x` kills it after a confirmation; `Esc` closes. These picker keys are fixed in v1 (not under `[keys]`).
3. **Switching** re-attaches the same client process: it sends `Detach`, then a new `Hello { role: Attach { session: Some(name), create: false } }` on a new connection, and redraws from the new `SessionView` and frames. The outer terminal is not restored in between (no flicker of the main screen).
4. **Killing the current session**, from the picker or by `deck kill-session` in another shell, makes the client switch to the most recently attached other session as in requirement 3; when there is none, the client exits with `[session <name> ended]`. The client does this on `Bye { SessionEnded }`.
5. The picker orders sessions by most recently attached first (from `SessionSummary`, extended by sub-spec 01); `deck ls` keeps sorting by name and gains the project count column. This sub-spec makes no protocol change.
6. **CLI**:
   - `deck new [<name>] [-c <dir>] [-d]`: sends `NewSession` (name defaults as in sub-spec 01; first project `<dir>`, default the current dir, resolved to an absolute path), then attaches to the created session by the name in `Response::Session`; it never attaches to an existing session. Inside deck (`DECK` set) it refuses like `deck attach` unless `-d` is given. `-d` creates it without attaching and prints its name. A duplicate name or a missing directory fails with the daemon's error text, exit 1.
   - `deck attach <name>` (DECK-01) gains `-c/--create`: create the session when it does not exist.
   - `deck kill-session <name>` and `deck rename-session <old> <new>`; errors print the daemon's message and exit 1.
   - All of them start the daemon when needed, except `kill-session` and `rename-session`, which print `no deck daemon running` and exit 1.

## Screen

```text
            ╭─ Sessions ─────────────────────────────────────────╮
            │ > wo█                                              │
            │                                                    │
            │ ● work          3 projects   7 terminals           │  ← selected, reversed
            │   workshop      1 project    1 terminal   attached │
            │                                                    │
            │ ⏎ switch  ^o new  ^r rename  ^x kill  esc close    │
            ╰────────────────────────────────────────────────────╯
```

The overlay is 60% of the width (at least 40 columns) and as tall as its rows, at most 70% of the height; it scrolls beyond that.

## Acceptance criteria

- [ ] `deck new other -d` prints `other`; `⌥Space s` then lists `main` and `other` (capture `picker`).
- [ ] Typing `oth` filters to `other`; `Enter` switches and the sidebar header shows `other`, with no main-screen flash: the harness's `raw_output()` (DECK-01) contains no `ESC[?1049l` between the two sessions.
- [ ] `^o` with the filter `third` creates and switches to `third`.
- [ ] `^r` renames the selected session; a duplicate name shows the error.
- [ ] `^x` on the current session, after `y`, switches to the previous one; on the last session the client exits with `[session <name> ended]`.
- [ ] `⌥Space $` renames the current session and the status bar updates.
- [ ] `deck attach nope` fails; `deck attach -c nope` creates and attaches.
- [ ] `deck kill-session other` and `deck rename-session main primary` work from a second shell, and an attached client sees the rename at once.
- [ ] `deck kill-session` of the session a client is attached to makes that client switch to another session, as the picker does.
- [ ] `deck new x` inside deck refuses with the nesting message, and `deck new -d x` inside deck works.
- [ ] `deck new main` while `main` exists fails and does not attach; `deck new -c /nope x` fails with the daemon's directory error.

## Tests

- `deck-tui`: picker drawing, filtering and key handling with `TestBackend`.
- `crates/deck/tests/sessions.rs`: one scenario per acceptance criterion.
