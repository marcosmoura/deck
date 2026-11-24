---
branch: feat/copy-search
backlog: DECK-24
---

# Scrollback search and copy mode

## Goal

In scroll mode, the keyboard can search the whole history and select and copy text, as in tmux's copy mode: `/` and `?` search with `n`/`N`, every match on screen is highlighted, `v`/`V` start a selection at a cursor moved with vi keys, and `y` copies it, however many screens it spans. A mouse selection can span scrolled lines too.

Asked by the user on 23 Nov 2026, as part of making deck a daily driver. Reverses the 22 Nov 2026 decision "No keyboard copy mode in v1" in `docs/architecture.md`.

## Builds on

- [DECK-01 core-daemon-terminal](../2026-11-22-core-daemon-terminal/00-overview.md): scroll mode (`prefix [`), `ClientMsg::Scroll`, the daemon-owned scroll position shared by the session's clients, `Frame.display_offset` and `history`.
- [DECK-15](../../backlog.md#deck-15) mouse selection (`deck-tui/src/select.rs`), OSC 52 copy, the selection colours.
- [DECK-17 floating-bar-menus](../2026-11-23-floating-bar-menus/00-overview.md): the bar's modes, badge and hints.
- [DECK-19 kitty-images](../2026-11-23-kitty-images/00-overview.md): `libghostty-vt` and the emulator thread. This spec reads the history through the `Emu` that DECK-19 leaves, so it lands after it.

## Non-goals

- Regular expressions (no new dependency): plain text, smart case.
- Matches spanning two rows, including soft-wrapped ones.
- Rectangular (block) selection.
- Searching a terminal outside the focused one, or across terminals.
- Emacs key bindings for copy mode; configurable copy-mode keys (scroll mode's keys are fixed today too).
- Pasting from deck's own buffer (`prefix ]`): copy goes to the system clipboard through OSC 52, as today.

## Decisions

- **The daemon searches and extracts; the client draws.** The history lives in the daemon's emulator and is never streamed. `Request::Search` returns the next match from a position; `Request::Text` returns the text between two positions. The client only finds matches on the rows it has, for highlighting.
- **Positions are stable row ids.** Every `Frame` carries `top_row: u64`, the id of its first row, and `numbering`, a generation that changes when ids stop meaning the same rows (reflow, alternate screen, reset). Ids grow by one per row and a row keeps its id while it stays in the history, so a position survives scrolling and new output. A row dropped from a full history takes its id with it. The daemon follows rows with `libghostty-vt`'s tracked grid references ([01](01-protocol.md)).
- **Copy mode lives inside scroll mode.** Entering scroll mode puts a copy cursor on the terminal's cursor cell. Moving it past the top or bottom of the screen scrolls the view by the rows crossed. `Esc` clears a selection, then a search's highlights, then leaves scroll mode; `q` leaves at once. The scroll position stays shared by the session's clients; [02](02-copy-mode.md) says how a copy cursor behaves when another client moves the view.
- **Keys are vi's**, matching scroll mode's `j`/`k`/`g`/`G`: `h j k l`, `w b e`, `0 $ ^`, `H M L`, `g G`, `C-u C-d C-b C-f`, `v` (characters), `V` (lines), `o` (other end), `y` or `Enter` (copy and leave the selection), `/` (search down, toward newer), `?` (search up, toward older), `n` (same direction), `N` (other direction). A search wraps around and says `search wrapped` in the bar.
- **Copied text** follows the mouse selection's rules: rows joined with `\n` and each row's trailing blanks dropped; but a soft-wrapped row keeps its trailing blanks and is joined to the next without a newline. Capped at 8 MiB, like `capture`.
- **Colours**: two new theme roles, `search_match_bg`/`search_match_fg` (default yellow on black) and `search_current_bg`/`search_current_fg` (default bright yellow on black, bold), documented in `default.toml`.
- **Protocol**: one bump for the whole spec, in [01](01-protocol.md), covering the theme roles too.

## Sub-specs

| Sub-spec              | Type       | Crate                  | Summary                                                                                             | Depends | Status |
| --------------------- | ---------- | ---------------------- | --------------------------------------------------------------------------------------------------- | ------- | ------ |
| [01](01-protocol.md)  | foundation | deck-core, deck-daemon | `Frame.top_row`, `ScrollTo::Row`, `Request::Search`, `Request::Text`, daemon tests                  | DECK-19 | done   |
| [02](02-copy-mode.md) | shippable  | deck-tui               | Copy cursor, vi motions, `v`/`V`/`o`/`y`, selections by row id (keyboard and mouse), edge scrolling | 01      | done   |
| [03](03-search.md)    | shippable  | deck-tui, deck-core    | `/` and `?` prompt, `n`/`N`, highlights, theme roles, docs                                          | 02      | done   |

## Affected docs

- `docs/features/keys.md`: Scroll mode gains Copy and Search subsections; Selecting and copying says a selection can span scrolled rows.
- `docs/features/config.md` and `crates/deck-core/src/config/default.toml`: the four new theme roles.
- `docs/architecture.md`: `top_row`, the new requests and `ScrollTo::Row`; the Decisions row "No keyboard copy mode in v1" is replaced.

Reviewed by spec-reviewer on 23 Nov 2026.
