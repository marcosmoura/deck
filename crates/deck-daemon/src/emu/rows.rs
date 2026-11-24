//! Row ids, and what stands on them: search and text between positions.
//!
//! Every row of a terminal has an id that grows by one per row and never goes back. The
//! emulator does not number its rows, so the ids follow an **anchor**: a tracked grid
//! reference (`Terminal::track_grid_ref`) that `libghostty-vt` keeps on the same row through
//! scrolling, pruning and reflow. After each `feed` and resize the id of the top row of the
//! live screen is the anchor's id plus the anchor's distance from that row, and the anchor
//! moves to that row, so it is never further from the live screen than the rows one feed
//! scrolls. When the anchor's row is gone anyway (one feed scrolled the whole retained
//! history away, or a reset cleared it) the ids start again above every id handed out, and
//! `numbering` grows so clients drop the positions they hold.

use deck_core::protocol::Pos;
use ghostty_vt::Error;
use ghostty_vt::screen::{Screen, TrackedGridRef};
use ghostty_vt::terminal::{Point, PointCoordinate, PointSpace, Terminal};

use super::capture::MAX_BYTES;
use super::{Emu, Glyph};

pub(super) struct RowIds {
    anchor: TrackedGridRef,
    /// The id of the row the anchor sits on.
    anchor_id: u64,
    /// The id of the top row of the live screen as of the last sync; on the alternate screen
    /// a base chosen on entering it.
    top: u64,
    /// Changes when ids stop meaning the same rows.
    numbering: u32,
    cols: u16,
    alternate: bool,
    /// The first id never handed out.
    next: u64,
}

impl RowIds {
    pub(super) fn new(term: &Terminal, cols: u16) -> Result<Self, Error> {
        let anchor = term.track_grid_ref(Point::Active(PointCoordinate { x: 0, y: 0 }))?;
        Ok(Self {
            anchor,
            anchor_id: 0,
            top: 0,
            numbering: 0,
            cols,
            alternate: false,
            next: u64::from(term.rows().unwrap_or(0)),
        })
    }

    pub(super) fn top(&self) -> u64 {
        self.top
    }

    pub(super) fn numbering(&self) -> u32 {
        self.numbering
    }
}

/// One character cell (or a wide character's two) with its text.
struct Unit {
    col: u16,
    /// The last column the unit covers.
    end: u16,
    text: String,
}

/// A row as text units, and whether it is soft-wrapped.
struct RowUnits {
    units: Vec<Unit>,
    wrapped: bool,
}

/// The match of a query in one row, as columns.
type Hit = (u16, u16);

impl Emu {
    /// Brings the ids up to date after the emulator changed.
    pub(super) fn sync_ids(&mut self) {
        let (cols, rows) = self.size();
        let alternate = self.term.active_screen() == Ok(Screen::Alternate);
        let cols_changed = cols != self.ids.cols;
        self.ids.cols = cols;
        let mut renumber = cols_changed;
        if alternate != self.ids.alternate {
            self.ids.alternate = alternate;
            renumber = true;
        }
        if alternate {
            // The alternate screen has no history: its rows get fresh ids when it is entered
            // and when it is resized.
            if renumber {
                self.ids.top = self.ids.next;
            }
            self.ids.numbering = self.ids.numbering.wrapping_add(u32::from(renumber));
            self.ids.next = self.ids.next.max(self.ids.top + u64::from(rows));
            return;
        }

        let live_top = self.term.scrollback_rows().unwrap_or(0);
        // The anchor sits on the top row of the live screen. Not on the last one: a tracked
        // reference on a blank row at the bottom keeps a shrinking resize from dropping that
        // row, so the content is pushed into the history instead.
        let anchor_y = match self.ids.anchor.point(PointSpace::Screen) {
            Ok(Some(c)) if !cols_changed => Some(c.y),
            _ => None,
        };
        let top = match anchor_y {
            Some(y) => {
                let id = i128::from(self.ids.anchor_id)
                    + i128::from(u64::try_from(live_top).unwrap_or(0))
                    - i128::from(y);
                u64::try_from(id.max(0)).unwrap_or(0)
            }
            None => {
                renumber = true;
                self.ids.next
            }
        };
        self.ids.numbering = self.ids.numbering.wrapping_add(u32::from(renumber));
        self.ids.top = top;
        self.ids.next = self.ids.next.max(top + u64::from(rows));
        if anchor_y.is_none_or(|y| usize::try_from(y).ok() != Some(live_top)) {
            let to = Point::Active(PointCoordinate { x: 0, y: 0 });
            if self.ids.anchor.set(&mut self.term, to).is_ok() {
                self.ids.anchor_id = top;
            } else {
                tracing::warn!("cannot move the row anchor");
            }
        }
    }

    /// The ids of the rows the history holds: from the oldest to one past the last row of the
    /// screen.
    fn held_rows(&self) -> (u64, u64) {
        let top = self.ids.top;
        let alternate = self.ids.alternate;
        let oldest = if alternate {
            top
        } else {
            top.saturating_sub(u64::try_from(self.history()).unwrap_or(0))
        };
        (oldest, top + u64::from(self.size().1))
    }

    /// The display offset that shows the row `id`: the current one when it is visible, else
    /// the one that puts the row in the middle; the oldest row's when it is gone.
    pub(super) fn offset_to_show(&self, id: u64, now: i64) -> i64 {
        let (oldest, end) = self.held_rows();
        let history = i64::try_from(self.history()).unwrap_or(0);
        if id < oldest {
            return history;
        }
        if id >= end {
            return 0;
        }
        let rows = i64::from(self.size().1);
        let rel = i64::try_from(i128::from(id) - i128::from(self.ids.top)).unwrap_or(0);
        if rel >= -now && rel < -now + rows {
            return now;
        }
        (rows / 2 - rel).clamp(0, history)
    }

    /// The row with id `id`, which must be held.
    /// `history` is the emulator's `scrollback_rows`, read once per request; `trim` drops the
    /// trailing blanks.
    fn row_units(&self, id: u64, history: usize, trim: bool) -> RowUnits {
        let line = i32::try_from(i128::from(id) - i128::from(self.ids.top)).unwrap_or(0);
        let (glyphs, wrapped) = self.row_glyphs(line, history);
        let mut units: Vec<Unit> = Vec::new();
        for (col, glyph) in glyphs.iter().enumerate() {
            let col = u16::try_from(col).unwrap_or(u16::MAX);
            if glyph.spacer {
                continue;
            }
            let text = unit_text(glyph);
            units.push(Unit {
                col,
                end: if glyph.wide {
                    col.saturating_add(1)
                } else {
                    col
                },
                text,
            });
        }
        // The trailing blanks are not part of the row's text, as in `capture`.
        while trim && units.last().is_some_and(|u| u.text == " ") {
            units.pop();
        }
        RowUnits { units, wrapped }
    }

    /// The first match of `query` strictly after `from` (`up`: before), wrapping around
    /// once, with whether it wrapped. A `from` that fell off the history's old end starts at
    /// that end; one beyond the screen, at the new end.
    pub fn search(&self, query: &str, from: Pos, up: bool) -> Option<(Pos, Pos, bool)> {
        if query.is_empty() {
            return None;
        }
        let (oldest, end) = self.held_rows();
        let history = self.term.scrollback_rows().unwrap_or(0);
        let hits = |row: u64| {
            let row = self.row_units(row, history, true);
            deck_core::search::find(
                row.units.iter().map(|u| (u.col, u.end, u.text.as_str())),
                query,
            )
        };
        let found = |row: u64, (start, last): Hit, wrapped: bool| {
            (Pos { row, col: start }, Pos { row, col: last }, wrapped)
        };
        // The rows of the first pass, and of the second (wrapped) one. Only `from`'s own row
        // is cut at `from`'s column in the first pass.
        let (first, second): (Vec<u64>, Vec<u64>) = if up {
            let upto = from.row.min(end.saturating_sub(1));
            if from.row < oldest {
                (Vec::new(), (oldest..end).rev().collect())
            } else {
                ((oldest..=upto).rev().collect(), (upto..end).rev().collect())
            }
        } else if from.row < oldest {
            ((oldest..end).collect(), Vec::new())
        } else {
            (
                (from.row..end).collect(),
                (oldest..end.min(from.row.saturating_add(1))).collect(),
            )
        };
        let cut = |row: u64, hit: &Hit| {
            row != from.row
                || if up {
                    hit.0 < from.col
                } else {
                    hit.0 > from.col
                }
        };
        for row in first {
            let mut all = hits(row);
            if up {
                all.reverse();
            }
            if let Some(hit) = all.into_iter().find(|h| cut(row, h)) {
                return Some(found(row, hit, false));
            }
        }
        for row in second {
            let mut all = hits(row);
            if up {
                all.reverse();
            }
            if let Some(hit) = all.into_iter().next() {
                return Some(found(row, hit, true));
            }
        }
        None
    }

    /// The text between two positions, inclusive and in either order (`lines`: whole rows);
    /// with whether the size cap cut it. `None` when a row is no longer held.
    pub fn text(&self, from: Pos, to: Pos, lines: bool) -> Option<(String, bool)> {
        let (oldest, end) = self.held_rows();
        let (a, b) = if (from.row, from.col) <= (to.row, to.col) {
            (from, to)
        } else {
            (to, from)
        };
        if a.row < oldest || b.row >= end {
            return None;
        }
        let mut out = String::new();
        let mut previous_wrapped = false;
        let history = self.term.scrollback_rows().unwrap_or(0);
        for row in a.row..=b.row {
            let RowUnits { units, wrapped } = self.row_units(row, history, false);
            let lo = if lines || row != a.row { 0 } else { a.col };
            let hi = if lines || row != b.row {
                u16::MAX
            } else {
                b.col
            };
            let mut text: String = units
                .iter()
                .filter(|u| u.end >= lo && u.col <= hi)
                .map(|u| u.text.as_str())
                .collect();
            // A row that continues on the next one keeps its trailing blanks (they are real
            // spaces at the join); a row ended by a newline, and the last one, lose them.
            if !wrapped || row == b.row {
                text.truncate(text.trim_end_matches(' ').len());
            }
            let join = usize::from(row != a.row && !previous_wrapped);
            if out.len() + join + text.len() > MAX_BYTES {
                return Some((out, true));
            }
            if join == 1 {
                out.push('\n');
            }
            out.push_str(&text);
            previous_wrapped = wrapped;
        }
        Some((out, false))
    }
}

fn unit_text(glyph: &Glyph) -> String {
    if glyph.text.is_empty() {
        " ".to_string()
    } else {
        glyph.text.clone()
    }
}

#[cfg(test)]
mod tests {
    use deck_core::model::TerminalId;
    use deck_core::protocol::{Frame, ScrollTo};

    use super::*;

    fn new(cols: u16, rows: u16, scrollback: usize) -> Emu {
        Emu::new(cols, rows, scrollback).expect("emulator")
    }

    fn frame(e: &mut Emu) -> Frame {
        e.full_frame(TerminalId(1))
    }

    fn lines(e: &mut Emu, from: usize, to: usize) {
        for n in from..to {
            e.feed(format!("line {n}\r\n").as_bytes());
        }
    }

    fn pos(row: u64, col: u16) -> Pos {
        Pos { row, col }
    }

    /// The id of the oldest row held, as the top of the view scrolled to the history's end.
    fn oldest(e: &mut Emu) -> u64 {
        let now = frame(e).display_offset;
        e.scroll(ScrollTo::Top);
        let top = frame(e);
        e.scroll(ScrollTo::Lines(-i32::try_from(top.history - now).unwrap()));
        top.top_row
    }

    #[test]
    fn ids_grow_by_the_lines_printed_and_scrolling_leaves_them() {
        let mut e = new(10, 4, 100);
        let first = frame(&mut e);
        assert_eq!((first.top_row, first.numbering), (0, 0));
        lines(&mut e, 0, 20);
        let live = frame(&mut e);
        // 20 lines in a screen of 4 rows: 17 scrolled away.
        assert_eq!(live.top_row, 17);
        assert_eq!(live.numbering, 0);
        e.scroll(ScrollTo::Lines(5));
        let back = frame(&mut e);
        assert_eq!((back.top_row, back.display_offset), (12, 5));
        // Output while scrolled back moves the offset, not the id of a row.
        lines(&mut e, 20, 23);
        let after = frame(&mut e);
        assert_eq!((after.top_row, after.display_offset), (12, 8));
        assert_eq!(after.numbering, 0);
    }

    #[test]
    fn a_full_history_ends_exactly_its_cap_below_the_screen() {
        let mut e = new(10, 4, 100);
        lines(&mut e, 0, 500);
        let live = frame(&mut e);
        assert_eq!(live.history, 100);
        let first = oldest(&mut e);
        assert_eq!(first, live.top_row - 100);
        assert!(first > 0);
        assert_eq!(e.held_rows(), (first, live.top_row + 4));
        // The tracked reference never lost its row on the way.
        assert_eq!(live.numbering, 0);
    }

    #[test]
    fn the_numbering_changes_when_ids_stop_meaning_the_same_rows() {
        let mut e = new(10, 4, 100);
        lines(&mut e, 0, 30);
        let n0 = frame(&mut e).numbering;
        // Rows only: no reflow.
        e.resize(10, 6);
        assert_eq!(frame(&mut e).numbering, n0);
        // A reflowing resize.
        e.resize(20, 6);
        let n1 = frame(&mut e).numbering;
        assert!(n1 > n0);
        // The alternate screen, in and out.
        e.feed(b"\x1b[?1049h");
        let n2 = frame(&mut e).numbering;
        assert!(n2 > n1);
        e.feed(b"\x1b[?1049l");
        let n3 = frame(&mut e).numbering;
        assert!(n3 > n2);
        // A full reset.
        e.feed(b"\x1bc");
        let n4 = frame(&mut e).numbering;
        assert!(n4 > n3, "{n3} {n4}");
        // Ids keep growing across all of it.
        lines(&mut e, 0, 10);
        let n5 = frame(&mut e);
        assert_eq!(n5.numbering, n4);
    }

    #[test]
    fn clearing_the_history_keeps_the_numbering_and_raises_the_oldest_row() {
        let mut e = new(10, 4, 100);
        lines(&mut e, 0, 50);
        let before = frame(&mut e);
        let oldest_before = oldest(&mut e);
        e.feed(b"\x1b[3J");
        let after = frame(&mut e);
        assert_eq!(after.numbering, before.numbering);
        assert_eq!(after.history, 0);
        assert!(oldest(&mut e) > oldest_before);
        assert!(after.top_row >= before.top_row);
        // Output goes on counting from there.
        lines(&mut e, 0, 5);
        assert_eq!(frame(&mut e).top_row, after.top_row + 5);
    }

    #[test]
    fn a_wrapped_row_is_marked_and_a_newline_ends_the_mark() {
        let mut e = new(10, 4, 100);
        e.feed(b"abcdefghijklmno\r\nxy");
        let f = frame(&mut e);
        let wrapped: Vec<bool> = f.lines.iter().map(|(_, l)| l.wrapped).collect();
        assert_eq!(wrapped, [true, false, false, false]);
    }

    #[test]
    fn a_row_scrolls_into_view_centred_or_not_at_all() {
        let mut e = new(10, 10, 1_000);
        lines(&mut e, 0, 600);
        let live = frame(&mut e);
        let top = live.top_row;
        // Visible: no scroll.
        e.scroll(ScrollTo::Row(top + 3));
        assert_eq!(frame(&mut e).display_offset, 0);
        // 300 back: centred.
        let row = top - 300;
        e.scroll(ScrollTo::Row(row));
        let f = frame(&mut e);
        assert_eq!(f.top_row, row - 5);
        assert_eq!(f.display_offset, 305);
        // Visible now: unchanged, even at the view's edge.
        e.scroll(ScrollTo::Row(f.top_row + 9));
        assert_eq!(frame(&mut e).display_offset, 305);
        // Near the top of the history: clamped.
        let first = oldest(&mut e);
        e.scroll(ScrollTo::Bottom);
        e.scroll(ScrollTo::Row(first + 1));
        let f = frame(&mut e);
        assert_eq!((f.top_row, f.display_offset), (first, f.history));
        // A dropped row: the top.
        e.scroll(ScrollTo::Bottom);
        e.scroll(ScrollTo::Row(0));
        assert_eq!(frame(&mut e).display_offset, frame(&mut e).history);
    }

    #[test]
    fn search_finds_words_far_up_steps_and_wraps() {
        let mut e = new(20, 5, 10_000);
        for n in 0..6_000 {
            let word = if n == 1_000 || n == 1_010 {
                "needle"
            } else {
                "hay"
            };
            e.feed(format!("{word} {n}\r\n").as_bytes());
        }
        let live = frame(&mut e);
        let bottom = pos(live.top_row + 4, 0);
        let (start, end, wrapped) = e.search("needle", bottom, true).expect("a match");
        assert!(!wrapped);
        assert_eq!((start.col, end.col), (0, 5));
        // Row ids follow the lines: line n is row n.
        assert_eq!(start.row, 1_010);
        let (next, _, wrapped) = e.search("needle", start, true).expect("the next");
        assert_eq!((next.row, wrapped), (1_000, false));
        // Down from there; and the search wraps past the end of the history.
        let (down, _, wrapped) = e.search("needle", next, false).expect("down");
        assert_eq!((down.row, wrapped), (1_010, false));
        let (around, _, wrapped) = e.search("needle", down, false).expect("around");
        assert_eq!((around.row, wrapped), (1_000, true));
        // Upwards from the first match wraps to the last.
        let (around, _, wrapped) = e.search("needle", next, true).expect("around up");
        assert_eq!((around.row, wrapped), (1_010, true));
        assert!(e.search("absent", bottom, true).is_none());
        assert!(e.search("", bottom, true).is_none());
    }

    #[test]
    fn search_respects_smart_case_and_matches_wide_characters() {
        let mut e = new(30, 6, 100);
        e.feed("Hello world\r\nhello again\r\n日本語 の 文字\r\ne\u{301}clair\r\n".as_bytes());
        let top = pos(frame(&mut e).top_row, 0);
        let rows = |q: &str| {
            let mut found: Vec<(u64, u16)> = Vec::new();
            let mut at = pos(top.row, 0);
            while let Some((s, ..)) = e.search(q, at, false) {
                if found.contains(&(s.row - top.row, s.col)) {
                    break;
                }
                found.push((s.row - top.row, s.col));
                at = s;
            }
            found.sort_unstable();
            found
        };
        // Lower case: both; with a capital: only the matching one.
        assert_eq!(rows("hello"), [(0, 0), (1, 0)]);
        assert_eq!(rows("Hello"), [(0, 0)]);
        assert_eq!(rows("HELLO"), Vec::<(u64, u16)>::new());
        // CJK: columns count the wide cells, the end is inclusive.
        let (s, end, _) = e.search("本語", pos(top.row + 1, 20), false).expect("cjk");
        assert_eq!((s.row - top.row, s.col, end.col), (2, 2, 5));
        // A combining mark belongs to its base character: `e` alone is not a match.
        assert!(
            e.search("e", pos(top.row + 2, 29), false)
                .is_none_or(|(s, ..)| s.row - top.row != 3)
        );
        let (s, end, _) = e
            .search("e\u{301}c", pos(top.row, 0), false)
            .expect("graphemes");
        assert_eq!((s.row - top.row, s.col, end.col), (3, 0, 1));
    }

    #[test]
    fn text_joins_wrapped_rows_and_drops_trailing_blanks() {
        let mut e = new(10, 4, 100);
        e.feed(b"abcdefghijklmno\r\nxy   \r\nlast");
        let top = frame(&mut e).top_row;
        let (all, truncated) = e.text(pos(top, 0), pos(top + 3, 9), false).expect("text");
        assert_eq!(
            (all.as_str(), truncated),
            ("abcdefghijklmno\nxy\nlast", false)
        );
        // Either order; characters inside the first and last rows.
        let (part, _) = e.text(pos(top + 1, 1), pos(top, 8), false).expect("part");
        assert_eq!(part, "ijkl");
        // Whole rows.
        let (rows, _) = e.text(pos(top, 5), pos(top + 2, 0), true).expect("lines");
        assert_eq!(rows, "abcdefghijklmno\nxy");
        // Rows that are gone.
        lines(&mut e, 0, 300);
        assert!(e.text(pos(top, 0), pos(top + 1, 0), false).is_none());
    }

    #[test]
    fn a_space_at_a_soft_wrap_is_kept() {
        let mut e = new(10, 4, 100);
        // The space is the tenth cell of the first row, which wraps.
        e.feed(b"abcdefghi jklm\r\nxy  \r\n");
        let top = frame(&mut e).top_row;
        let (text, _) = e.text(pos(top, 0), pos(top + 2, 9), false).expect("text");
        assert_eq!(text, "abcdefghi jklm\nxy");
        // Cut inside the wrapped row, its last cell is the end of the range: trimmed.
        let (cut, _) = e.text(pos(top, 5), pos(top, 9), false).expect("cut");
        assert_eq!(cut, "fghi");
    }

    #[test]
    fn text_over_three_screens_equals_the_capture_with_wrapped_rows_joined() {
        let mut e = new(20, 5, 1_000);
        for n in 0..200 {
            // Every tenth line is longer than the screen is wide.
            let tail = if n % 10 == 0 {
                "-wrapped-over-the-edge"
            } else {
                ""
            };
            e.feed(format!("row {n}{tail}\r\n").as_bytes());
        }
        let live = frame(&mut e);
        let top = live.top_row;
        let from = pos(top - 15, 0);
        let to = pos(top + 4, 19);
        let (text, _) = e.text(from, to, false).expect("text");
        let capture = e.capture(TerminalId(1), None, true, false).lines;
        // Rows of the capture from `from` on (the capture ends at the last non-empty row).
        let held = e.held_rows().0;
        let skip = usize::try_from(from.row - held).unwrap();
        let mut want = String::new();
        for (row, line) in (from.row..).zip(capture.iter().skip(skip)) {
            want.push_str(line);
            if !e
                .row_units(row, e.term.scrollback_rows().unwrap(), true)
                .wrapped
            {
                want.push('\n');
            }
        }
        assert_eq!(text.trim_end_matches('\n'), want.trim_end_matches('\n'));
    }
}
