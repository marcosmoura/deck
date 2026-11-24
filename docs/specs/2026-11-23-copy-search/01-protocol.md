---
type: foundation
status: done
---

# 01 — Row ids, search and text requests

## Scope

The protocol and daemon pieces copy mode and search stand on. Owning crates: `deck-core` (protocol), `deck-daemon` (emulator access). Nothing changes on screen.

## Requirements

1. **Row ids.** Each terminal numbers its rows with ids that grow by one per row, from a counter that never goes back. The daemon keeps an **anchor**: a `libghostty-vt` tracked grid reference (`Terminal::track_grid_ref`) on a row with a known id, which follows that row through scrolling and reflow. After each `feed` and resize, the id of the screen's top row is the anchor's id plus the anchor's distance from the top, and the anchor moves to the screen's top row (not its last row: a tracked reference on a blank bottom row keeps a shrinking resize from dropping that row, pushing the screen's content into the history). When the anchor's row is lost anyway (one feed scrolled past everything `libghostty-vt` retains, a reset), the daemon cannot know how many rows went by: it starts the ids again above every id handed out and bumps `numbering` (a deviation from re-anchoring on the oldest row, which would need the dropped count). Leaving the alternate screen returns to the primary screen's earlier ids, which is safe because the numbering changes then. The worker documents the mechanism in `docs/architecture.md`; if tracked references turn out unusable, the worker stops and reports before choosing another mechanism.
2. **The history is deck's**: ids, "the oldest row", search and text all follow DECK-19's line cap on the history (`scrollback_lines`), not rows `libghostty-vt` still holds beyond it.
3. **`Frame.top_row: u64`** (the id of the frame's first row) and **`Frame.numbering: u32`**, a generation that only grows. It changes when ids stop meaning the same rows: a reflowing resize, a switch to or from the alternate screen, a full reset (`RIS`). A client drops every position it holds when the generation changes; since it is carried by every frame, a frame dropped by the client queue (`queue.rs`) cannot hide it. Clearing the history (`CSI 3 J`) does not change the generation: its rows are gone and their ids with them, the counter keeps growing.
4. **`protocol::Line` gains `wrapped: bool`**: the row continues on the next one (soft wrap), so the client can join wrapped rows the same way the daemon does.
5. **`ScrollTo::Row(u64)`**: scrolls so the row is visible, leaving the view unchanged when it already is, else placing it in the middle of the screen (clamped to the history's ends). A row no longer held scrolls to the top. Copy-mode edge scrolling uses the existing `ScrollTo::Lines`; `Row` is for search results and long jumps.
6. **`Request::Search { terminal, query, from: Pos, up: bool }`**, `Pos { row: u64, col: u16 }`: the next occurrence of `query` strictly after `from` (`up`: before), in reading order, over the history and the screen (on the alternate screen, just its screen), wrapping around once. Smart case: case-insensitive unless the query has an uppercase letter. Matching is on each row's text as `capture` builds it, by grapheme, within one row. Answers `Response::Found { start: Pos, end: Pos, wrapped: bool }` (end inclusive) or `Response::NotFound`. A `from` row no longer held starts from the end it fell off. It reads rows through the same row reader `capture` uses. `ghostty-vt` 0.9.0 offers no row or page read that leaves the viewport alone, only one grid lookup per cell, so the reader makes one lookup per column (its per-row invariants read once per request); measured in a release build, a full search of 10,024 rows of 80 columns takes about 96 ms, twice the goal of item 9.
7. **`Request::Text { terminal, from: Pos, to: Pos, lines: bool }`**: the text between two positions (inclusive, either order); `lines` selects whole rows. A row ended by a newline and the last row of the range lose their trailing blanks; a wrapped row keeps them (they are real spaces at the join); rows are joined with `\n` except where the earlier row is wrapped. Capped at 8 MiB (`Response::Text { text, truncated }`). Positions whose rows are gone answer `Response::Error("the selection is no longer in the history")`.
8. **Session check**: for an attached client, both requests (and `ScrollTo::Row`) apply only to terminals of its own session. This check is new (`Capture` has none today) and lives with the request dispatch in `conn.rs`; control clients reach any session, like `Capture`.
9. Both requests run on the terminal's emulator thread through `with_emu`, called from a blocking task and do not hold up frames of other terminals. Goal (not an acceptance criterion): a 10,000-row history searched end to end in under 50 ms in a release build.
10. **Protocol**: one bump for everything in this spec, including the theme roles of [03](03-search.md) (`Theme` travels in `Welcome` and `ServerMsg::Config`), to the next number free at merge (DECK-19 and DECK-20 also bump it).

## Acceptance criteria

- [ ] `top_row` grows by the number of lines printed; scrolling changes `display_offset`, not a row's id; with a full history (scrollback 100, 500 lines printed), the oldest row held has an id exactly 100 below the screen's top row, and above 0.
- [ ] `numbering` changes on a reflowing resize, on entering and leaving the alternate screen, and on `RIS`; not on `CSI 3 J`, after which the oldest row held has a larger id than before.
- [ ] `wrapped` is set on a soft-wrapped row and not on a row ended by a newline.
- [ ] `ScrollTo::Row` of a visible row does not scroll; of a row 300 back centres it; of a dropped row scrolls to the top.
- [ ] Search finds a word 5,000 lines up (`up: true` from the bottom), the next one from its start, wraps with `wrapped: true`, respects smart case, matches a CJK query, and answers `NotFound`.
- [ ] Text between a position 3 screens up and the bottom equals the same range of `capture --history`'s rows with the wrapped pair joined without a newline.
- [ ] An attached client's request for a terminal of another session is refused.
- [ ] Protocol tests pin the new messages and fields.

## Tests

- `deck-core` protocol tests; `crates/deck-daemon/tests/` for ids, the generation, scroll, search and text.
