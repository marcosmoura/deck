//! Selecting text in a terminal with the mouse, from the client's mirror of its screen.

use deck_core::model::TerminalId;
use deck_core::protocol::{Frame, Line};

use crate::draw::width;

/// How far two presses on the same cell may be apart to count as a double or triple click.
pub(crate) const MULTI_CLICK_MS: u128 = 400;

/// What a selection grows by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unit {
    Cell,
    Word,
    Line,
}

/// A selection on one terminal's screen, in 0-based `(col, row)` cells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TextSel {
    pub terminal: TerminalId,
    /// The press cell.
    pub anchor: (u16, u16),
    /// The pointer's cell.
    pub head: (u16, u16),
    pub unit: Unit,
    /// The button is still down.
    pub dragging: bool,
}

impl TextSel {
    /// The first and last selected cells, both included, in reading order; `None` when a
    /// cell selection has not moved off its press cell. `lines` and `cols` are the screen's.
    pub fn range(&self, lines: &[Line], cols: u16) -> Option<((u16, u16), (u16, u16))> {
        let (a, b) = if (self.anchor.1, self.anchor.0) <= (self.head.1, self.head.0) {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        };
        let last = cols.saturating_sub(1);
        match self.unit {
            Unit::Cell => (a != b).then_some((a, b)),
            Unit::Line => Some(((0, a.1), (last, b.1))),
            Unit::Word => {
                let row = |r: u16| cells(lines.get(usize::from(r)), cols);
                let start = word_bounds(&row(a.1), a.0).0;
                let end = word_bounds(&row(b.1), b.0).1;
                Some(((start, a.1), (end, b.1)))
            }
        }
    }

    /// True when a frame of its terminal moves the selected text: another size or scroll
    /// offset, or a selected row that differs from `lines`, the screen before the frame. A
    /// full frame (sent on every change while scrolled back) leaves out blank rows.
    pub fn stale(&self, frame: &Frame, size: (u16, u16), offset: u32, lines: &[Line]) -> bool {
        if frame.terminal != self.terminal {
            return false;
        }
        if frame.size != size || frame.display_offset != offset {
            return true;
        }
        let blank = Line::default();
        (self.anchor.1.min(self.head.1)..=self.anchor.1.max(self.head.1)).any(|row| {
            let new = frame.lines.iter().find(|(r, _)| *r == row).map(|(_, l)| l);
            let new = match new {
                Some(line) => line,
                None if frame.full => &blank,
                None => return false,
            };
            lines.get(usize::from(row)).unwrap_or(&blank) != new
        })
    }
}

/// One cell of a screen row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Cell {
    /// Nothing was written there.
    Blank,
    /// A character and its combining marks.
    Text(String),
    /// The second column of a wide character.
    Spacer,
}

/// A row as `cols` cells.
pub(crate) fn cells(line: Option<&Line>, cols: u16) -> Vec<Cell> {
    let mut out = vec![Cell::Blank; usize::from(cols)];
    let Some(line) = line else { return out };
    for run in &line.runs {
        let mut col = usize::from(run.col);
        let mut last: Option<usize> = None;
        for c in run.text.chars() {
            let mut buf = [0; 4];
            let w = usize::from(width(c.encode_utf8(&mut buf)));
            if w == 0 {
                if let Some(Cell::Text(t)) = last.and_then(|i| out.get_mut(i)) {
                    t.push(c);
                }
                continue;
            }
            let Some(slot) = out.get_mut(col) else {
                break;
            };
            *slot = Cell::Text(c.to_string());
            last = Some(col);
            for spacer in out.iter_mut().skip(col + 1).take(w - 1) {
                *spacer = Cell::Spacer;
            }
            col += w;
        }
    }
    out
}

/// Part of a word: anything but blanks and the brackets and quotes around words.
fn wordish(cell: &Cell) -> bool {
    match cell {
        Cell::Blank => false,
        Cell::Spacer => true,
        Cell::Text(t) => !t
            .chars()
            .all(|c| c.is_whitespace() || "()[]{}<>\"'`,;|│".contains(c)),
    }
}

/// The first and last columns of the word under `col`; just `col` (and its wide
/// character's spacer) when it is not on a word.
pub(crate) fn word_bounds(row: &[Cell], col: u16) -> (u16, u16) {
    let i = usize::from(col).min(row.len().saturating_sub(1));
    let to_u16 = |i: usize| u16::try_from(i).unwrap_or(u16::MAX);
    let Some(cell) = row.get(i) else {
        return (col, col);
    };
    if !wordish(cell) {
        let mut start = i;
        if *cell == Cell::Spacer {
            start = i.saturating_sub(1);
        }
        let mut end = start;
        while row.get(end + 1) == Some(&Cell::Spacer) {
            end += 1;
        }
        return (to_u16(start), to_u16(end));
    }
    let mut start = i;
    while start > 0 && row.get(start - 1).is_some_and(wordish) {
        start -= 1;
    }
    let mut end = i;
    while row.get(end + 1).is_some_and(wordish) {
        end += 1;
    }
    (to_u16(start), to_u16(end))
}

/// The text from cell `from` to cell `to`, both included and in reading order: each row's
/// trailing blanks are dropped and rows are joined with newlines.
pub(crate) fn text(lines: &[Line], cols: u16, from: (u16, u16), to: (u16, u16)) -> String {
    let mut rows = Vec::new();
    for r in from.1..=to.1 {
        let row = cells(lines.get(usize::from(r)), cols);
        let start = if r == from.1 { usize::from(from.0) } else { 0 };
        let end = if r == to.1 {
            usize::from(to.0)
        } else {
            row.len().saturating_sub(1)
        };
        let mut s = String::new();
        for cell in row.iter().take(end + 1).skip(start) {
            match cell {
                Cell::Blank => s.push(' '),
                Cell::Text(t) => s.push_str(t),
                Cell::Spacer => {}
            }
        }
        rows.push(s.trim_end().to_string());
    }
    rows.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mirror::tests::line;
    use deck_core::protocol::{Run, Style};

    fn runs(parts: &[(u16, &str)]) -> Line {
        Line {
            runs: parts
                .iter()
                .map(|(col, text)| Run {
                    col: *col,
                    text: (*text).into(),
                    style: Style::default(),
                })
                .collect(),
        }
    }

    fn sel(anchor: (u16, u16), head: (u16, u16), unit: Unit) -> TextSel {
        TextSel {
            terminal: TerminalId(4),
            anchor,
            head,
            unit,
            dragging: false,
        }
    }

    #[test]
    fn text_fills_gaps_and_drops_trailing_blanks() {
        let lines = vec![runs(&[(0, "ab"), (5, "cd")]), line("next   ")];
        assert_eq!(text(&lines, 10, (0, 0), (9, 0)), "ab   cd");
        assert_eq!(text(&lines, 10, (3, 0), (2, 1)), "  cd\nnex");
    }

    #[test]
    fn a_wide_character_is_copied_once_and_keeps_later_columns_in_place() {
        let lines = vec![line("a界b")];
        let row = cells(lines.first(), 6);
        assert_eq!(row[1], Cell::Text("界".into()));
        assert_eq!(row[2], Cell::Spacer);
        assert_eq!(row[3], Cell::Text("b".into()));
        assert_eq!(text(&lines, 6, (0, 0), (3, 0)), "a界b");
        assert_eq!(text(&lines, 6, (2, 0), (3, 0)), "b");
    }

    #[test]
    fn combining_marks_stay_with_their_character() {
        let lines = vec![line("e\u{301}x")];
        assert_eq!(text(&lines, 4, (0, 0), (0, 0)), "e\u{301}");
    }

    #[test]
    fn word_bounds_stop_at_blanks_and_brackets() {
        let row = cells(Some(&line("ls (src/main.rs) x")), 20);
        assert_eq!(word_bounds(&row, 6), (4, 14));
        assert_eq!(word_bounds(&row, 0), (0, 1));
        // On a blank: the cell alone.
        assert_eq!(word_bounds(&row, 2), (2, 2));
        assert_eq!(word_bounds(&row, 19), (19, 19));
    }

    #[test]
    fn ranges_by_unit_and_in_reading_order() {
        let lines = vec![line("one two"), line("three four")];
        assert_eq!(sel((3, 0), (3, 0), Unit::Cell).range(&lines, 10), None);
        assert_eq!(
            sel((2, 1), (5, 0), Unit::Cell).range(&lines, 10),
            Some(((5, 0), (2, 1)))
        );
        assert_eq!(
            sel((5, 0), (7, 1), Unit::Word).range(&lines, 10),
            Some(((4, 0), (9, 1)))
        );
        assert_eq!(
            sel((5, 1), (1, 1), Unit::Line).range(&lines, 10),
            Some(((0, 1), (9, 1)))
        );
    }

    #[test]
    fn a_frame_is_stale_when_it_moves_the_selection() {
        use crate::mirror::tests::frame;
        let s = sel((0, 2), (4, 3), Unit::Cell);
        let screen = vec![line("a"), line("b"), line("c"), line("d"), line("e")];
        let partial = |rows: &[(u16, &str)]| {
            frame(
                false,
                (10, 5),
                rows.iter().map(|(r, t)| (*r, line(t))).collect(),
            )
        };
        assert!(!s.stale(&partial(&[(0, "x"), (4, "x")]), (10, 5), 0, &screen));
        assert!(s.stale(&partial(&[(3, "x")]), (10, 5), 0, &screen));
        // The same text again is no change.
        assert!(!s.stale(&partial(&[(3, "d")]), (10, 5), 0, &screen));
        assert!(s.stale(&partial(&[]), (10, 6), 0, &screen));
        assert!(s.stale(&partial(&[]), (10, 5), 1, &screen));
        let mut other = partial(&[(3, "x")]);
        other.terminal = TerminalId(9);
        assert!(!s.stale(&other, (10, 5), 0, &screen));
    }

    #[test]
    fn a_full_frame_clears_only_when_a_selected_row_changes() {
        use crate::mirror::tests::frame;
        let s = sel((0, 1), (4, 2), Unit::Cell);
        let screen = vec![line("a"), line("b"), line("c")];
        let full = |rows: Vec<(u16, Line)>| frame(true, (10, 3), rows);
        let same = full(vec![(0, line("new")), (1, line("b")), (2, line("c"))]);
        assert!(!s.stale(&same, (10, 3), 0, &screen));
        let changed = full(vec![(1, line("b")), (2, line("z"))]);
        assert!(s.stale(&changed, (10, 3), 0, &screen));
        // A selected row left out of a full frame is blank now.
        let dropped = full(vec![(1, line("b"))]);
        assert!(s.stale(&dropped, (10, 3), 0, &screen));
    }
}
