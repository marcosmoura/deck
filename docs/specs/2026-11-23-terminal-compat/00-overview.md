---
branch: feat/terminal-compat
backlog: DECK-22
---

# Terminal compatibility

## Goal

Programs look and behave the same inside deck as in a good terminal. A differential test suite feeds the same bytes to a reference emulator and to deck and compares the screens cell by cell; recordings of real programs (neovim, less, htop, lazygit, fzf, git, …) are replayed through it; the gaps found on the way are fixed. Two known gaps that matter every day are closed outright: underline styles and colours (editor diagnostics), and the kitty keyboard protocol (Shift+Enter, Ctrl+Enter and friends in Claude Code, Codex and editors).

Asked by the user on 23 Nov 2026, as part of making deck a daily driver.

## Builds on

- [DECK-19 kitty-images](../2026-11-23-kitty-images/00-overview.md): `libghostty-vt` as the daemon's emulator, `alacritty_terminal` kept as the e2e harness's screen, mode 2026. This spec must land after DECK-19; testing the old emulator would be wasted.
- [DECK-01 core-daemon-terminal](../2026-11-22-core-daemon-terminal/00-overview.md): `Frame`, `Style`, `Modes`, key encoding in `deck-core::keys`, the client's crossterm input.
- The e2e harness (`crates/deck/tests/support/`, [architecture.md#testing](../../architecture.md#testing)).

## Non-goals

- OSC 8 hyperlinks, blink, Sixel and iTerm2 images, and the bright/dim distinctions of named colours: a later task can add them; the compat suite lists them as known differences.
- `modifyOtherKeys` (xterm's alternative to the kitty keyboard protocol).
- The base-layout key of kitty flag 4 (crossterm gives the shifted key but not the base-layout one): the flag is honoured without that field, which the protocol allows.
- Recording programs that need an account or show private data (Claude Code, Codex, OpenCode sessions): they get a manual checklist in the compatibility doc, not a recording.
- Performance: [DECK-23](../2026-11-23-daemon-stats/00-overview.md) and [DECK-26](../2026-11-23-reviews/00-overview.md).

## Decisions

- **Raw replay.** Both sides replay a case through a tty in raw mode with echo off (`stty raw -echo` before `cat`), so the emulators' answers to queries in the bytes are not echoed onto the screen and a bare `\n` stays a line feed, as it was when a program in raw mode wrote it.
- **Differential, not golden.** The reference is the harness's `alacritty_terminal`, fed the bytes directly (`Harness::spawn`). deck gets the same bytes from a program in its only terminal (sidebar off, one terminal, so terminal cell (0,0) is outer cell (0,0)), and the harness reads deck's outer screen with the same `alacritty_terminal`. The screens and cursors must be equal after normalisation (named colours 0–15 as indexed, the known differences). No hand-written expected screens.
- **When the two disagree,** the worker decides which is right from xterm's ctlseqs and Ghostty's behaviour. A deck bug is fixed. A reference quirk, or a difference by design, goes into the known-differences table with its reason. The table lives in the test code; every entry names a reason, and those that deck should fix later name a backlog task.
- **Fix budget per sub-spec:** a difference whose fix fits the sub-spec (a day of work, no new protocol message beyond this spec's) is fixed in it. A bigger one becomes a backlog task and a known difference.
- **Underline styles and colour travel in `Style`:** an underline kind (single, double, curly, dotted, dashed) instead of the one bit, and an optional underline colour. ratatui's cells have no field for the kind and its crossterm backend writes plain `4`, so the client carries the kind inside each ratatui `Cell` (where `Buffer::diff` sees it) and draws with deck's own `Backend`, which writes `4:n` itself ([03](03-underline-styles.md)).
- **Kitty keyboard protocol end to end:** the emulator tracks each terminal's flag stack and answers the program's query, which undoes DECK-19's deliberate drop of that reply ([04](04-kitty-keyboard.md) says how); `Frame.modes` carries the active flags. So programs inside deck always see kitty support, whatever the outer terminal; when the outer terminal lacks it, keys it cannot tell apart arrive as their plain form. While the focused terminal has flags set and the outer terminal supports the protocol (detected with the client's existing start-up query), the client pushes flags to the outer terminal and encodes keys for the program with the kitty encoding; otherwise legacy encoding stays. The prefix key keeps working in both.
- **Recordings** are made with a new `just record-compat` recipe (a harness test run with `--run-ignored`) from scenario files, in a scratch `HOME` with a fixed environment allowlist and a fake identity, at 100×30, and committed as raw output. They are output only, so replaying them needs none of the programs.
- **Protocol**: one bump, for `Style` and the kitty flags, in [03](03-underline-styles.md) (04 adds to it on the same unmerged branch).

## Sub-specs

| Sub-spec                         | Type       | Crate                            | Summary                                                                                      | Depends | Status |
| -------------------------------- | ---------- | -------------------------------- | -------------------------------------------------------------------------------------------- | ------- | ------ |
| [01](01-differential-harness.md) | foundation | deck (tests)                     | Differential compare, normalisation, known-differences table, first corpus                   | DECK-19 | done   |
| [02](02-escape-corpus.md)        | shippable  | deck-daemon, deck-tui, tests     | Full escape-sequence corpus through deck, resize cases, fixes                                | 01      | done   |
| [03](03-underline-styles.md)     | shippable  | deck-core, deck-daemon, deck-tui | Underline kinds and colour in `Style`, written to the outer terminal                         | 02      | done   |
| [04](04-kitty-keyboard.md)       | shippable  | deck-core, deck-daemon, deck-tui | Kitty keyboard protocol from the program through deck to the outer terminal                  | 03      | done   |
| [05](05-app-recordings.md)       | shippable  | deck (tests), repo               | `just record-compat`, recordings of real programs replayed differentially, fixes, compat doc | 02      | done   |

## Affected docs

- `docs/features/compatibility.md` (new): what the suite covers, the programs recorded, the manual checklist (Claude Code, Codex, OpenCode, ssh, tmux inside deck), the known differences.
- `docs/features/keys.md`: the kitty keyboard protocol, Shift+Enter and friends.
- `docs/architecture.md`: `Style`'s underline, `Modes`' kitty flags, the client's backend wrapper, the compat tests in Testing, the `record-compat` recipe.
- `AGENTS.md`: the `just record-compat` line in the commands table.

Reviewed by spec-reviewer on 23 Nov 2026.
