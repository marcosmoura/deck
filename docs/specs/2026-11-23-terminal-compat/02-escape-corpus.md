---
type: shippable
status: done
---

# 02 — Escape-sequence corpus

## Scope

The corpus grows to cover what full-screen programs and shells use, and deck is fixed where it disagrees with the reference. The user gets programs that draw correctly in cases that were broken. Owning crates: tests in `deck`; fixes wherever they fall (`deck-daemon`, `deck-tui`, `deck-core`).

## Requirements

1. Cases, at least one each:
   - scrolling regions (`DECSTBM`) with `IND`, `RI`, `SU`, `SD`, and `IL`/`DL` inside and outside the region;
   - `ICH`, `DCH`, `ECH`, `REP`;
   - tab stops (`HTS`, `TBC`, default stops, `CHT`, `CBT`);
   - autowrap on and off (`DECAWM`), the pending-wrap state at the last column, origin mode (`DECOM`);
   - `DECSC`/`DECRC` and `CSI s`/`CSI u`, with attributes;
   - the alternate screen (`?47`, `?1047`, `?1049`) entered and left, with the primary screen's content and cursor restored;
   - DEC special graphics (`ESC ( 0`, `SO`/`SI`), as `tmux`, `htop` and `mc` draw boxes;
   - wide characters and emoji at the last column, over wide characters, and erased by half; variation selectors (`❤️`), ZWJ sequences (`👩‍💻`), CJK;
   - `DECSCUSR` shapes and `?25` visibility;
   - a full-screen redraw inside synchronized output (`?2026`);
   - `\r\n` versus `\n` only, backspace at column 0, `NEL`, `IND` at the bottom;
   - `ED 3` (clear scrollback) and its effect on the history, compared through `deck capture --history` and the reference's scrollback, read with a new harness helper `Harness::history_text()`.
2. Resize cases: a reflowed long line growing and shrinking, the cursor on a wrapped line, the alternate screen resized (no reflow there).
3. Every difference is fixed or recorded per the overview's rules.
4. A test of the floating bar's interaction: the corpus runs with no mode or flash active, so the bar never covers the last row; a case asserts that.

## Acceptance criteria

- [ ] Every case in 1 and 2 passes, or is a known difference with a reason (and a backlog task when deck should fix it).
- [ ] The report lists each fix made, with the case that shows it.
- [ ] `docs/features/compatibility.md` exists with the covered areas and the known differences.

## Tests

- The cases themselves; a unit test next to each fix in the crate it touches.

## Notes

- Grapheme widths differ between emulators (emoji with variation selectors, ZWJ). Where Ghostty and alacritty disagree, deck follows its own emulator (Ghostty's): the case is a known difference with that reason, not a bug.
