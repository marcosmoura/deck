//! Selecting text in a terminal with the mouse, from the client's mirror of its screen.

use deck_core::model::TerminalId;
use deck_core::protocol::{Frame, Line, Pos};

use crate::diacritics::{DIACRITICS, PLACEHOLDER};
use crate::draw::width;
use crate::mirror::TermMirror;

/// How far two presses on the same cell may be apart to count as a double or triple click.
pub(crate) const MULTI_CLICK_MS: u128 = 400;

/// What a selection grows by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unit {
    Cell,
    Word,
    Line,
}

/// A selection in one terminal, by row id (see `Frame::top_row`): it follows its rows when
/// the view scrolls and can reach rows that are off screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TextSel {
    pub terminal: TerminalId,
    /// The press cell, or where the keyboard selection started.
    pub anchor: Pos,
    /// The pointer's or the copy cursor's cell.
    pub head: Pos,
    pub unit: Unit,
    /// The columns the anchor's and the head's cells grow to with `unit`: the cell itself,
    /// its word, or the whole row. Fixed when they are set, so rows that scroll off screen
    /// keep them.
    pub anchor_cols: (u16, u16),
    pub head_cols: (u16, u16),
    /// The button is still down.
    pub dragging: bool,
}

/// The columns `col` of a row grows to with `unit`.
pub(crate) fn grow(unit: Unit, line: Option<&Line>, cols: u16, col: u16) -> (u16, u16) {
    match unit {
        Unit::Cell => (col, col),
        Unit::Line => (0, u16::MAX),
        Unit::Word => word_bounds(&cells(line, cols), col),
    }
}

impl TextSel {
    /// A selection that has not moved off `at`; `at_cols` is what `at` grows to.
    pub fn new(terminal: TerminalId, at: Pos, unit: Unit, at_cols: (u16, u16)) -> Self {
        Self {
            terminal,
            anchor: at,
            head: at,
            unit,
            anchor_cols: at_cols,
            head_cols: at_cols,
            dragging: false,
        }
    }

    pub fn set_head(&mut self, head: Pos, cols: (u16, u16)) {
        self.head = head;
        self.head_cols = cols;
    }

    /// Changes what the selection grows by, keeping its cells; `grown` gives the columns a
    /// cell grows to.
    pub fn set_unit(&mut self, unit: Unit, grown: impl Fn(Pos) -> (u16, u16)) {
        self.unit = unit;
        self.anchor_cols = grown(self.anchor);
        self.head_cols = grown(self.head);
    }

    /// Swaps the two ends.
    pub fn swap(&mut self) {
        std::mem::swap(&mut self.anchor, &mut self.head);
        std::mem::swap(&mut self.anchor_cols, &mut self.head_cols);
    }

    /// The first and last selected cells, both included, in reading order; `None` when a
    /// cell selection has not moved off its press cell. A whole-row selection ends at
    /// `u16::MAX`.
    pub fn range(&self) -> Option<(Pos, Pos)> {
        let key = |p: Pos| (p.row, p.col);
        let ((a, a_cols), (b, b_cols)) = if key(self.anchor) <= key(self.head) {
            ((self.anchor, self.anchor_cols), (self.head, self.head_cols))
        } else {
            ((self.head, self.head_cols), (self.anchor, self.anchor_cols))
        };
        if self.unit == Unit::Cell && a == b {
            return None;
        }
        Some((
            Pos {
                row: a.row,
                col: a_cols.0,
            },
            Pos {
                row: b.row,
                col: b_cols.1,
            },
        ))
    }

    /// The first and last row ids it touches.
    pub fn rows(&self) -> (u64, u64) {
        (
            self.anchor.row.min(self.head.row),
            self.anchor.row.max(self.head.row),
        )
    }

    /// True when a frame of its terminal moves the selected text: another size or numbering,
    /// or a selected row whose text differs from what the screen `old` showed for that id.
    /// A row of the frame that it does not list is what the screen showed at that place (a
    /// full frame leaves out blank rows).
    pub fn stale(&self, frame: &Frame, old: &TermMirror) -> bool {
        if frame.terminal != self.terminal {
            return false;
        }
        if frame.size != old.size || frame.numbering != old.numbering {
            return true;
        }
        let blank = Line::default();
        let (first, last) = self.rows();
        let on = |top: u64, id: u64| {
            let at = id.checked_sub(top)?;
            u16::try_from(at).ok().filter(|at| *at < frame.size.1)
        };
        (first..=last).any(|id| {
            let (Some(was), Some(now)) = (on(old.top_row, id), on(frame.top_row, id)) else {
                return false;
            };
            let new = match frame.lines.iter().find(|(r, _)| *r == now) {
                Some((_, line)) => line,
                None if frame.full => &blank,
                None => old.lines.get(usize::from(now)).unwrap_or(&blank),
            };
            old.lines.get(usize::from(was)).unwrap_or(&blank) != new
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
        let mut marks = false;
        for c in run.text.chars() {
            // A program's image placeholder (and its diacritics) is never copied: a space.
            if c == PLACEHOLDER || (marks && DIACRITICS.binary_search(&c).is_ok()) {
                if c == PLACEHOLDER {
                    marks = true;
                    if let Some(slot) = out.get_mut(col) {
                        *slot = Cell::Text(" ".into());
                        last = None;
                        col += 1;
                    }
                }
                continue;
            }
            marks = false;
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
pub(crate) fn wordish(cell: &Cell) -> bool {
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

/// The text from cell `from` to cell `to`, both included and in reading order, as the daemon
/// builds it for `Request::Text`: a row ended by a newline and the last row lose their
/// trailing blanks, a row that wraps onto the next keeps them, and rows are joined with
/// newlines except after a wrapped one.
pub(crate) fn text(lines: &[Line], cols: u16, from: (u16, u16), to: (u16, u16)) -> String {
    let mut out = String::new();
    let mut previous_wrapped = false;
    for r in from.1..=to.1 {
        let line = lines.get(usize::from(r));
        let row = cells(line, cols);
        let mut start = if r == from.1 { usize::from(from.0) } else { 0 };
        // A start on the second cell of a wide character keeps the character, as the daemon.
        if start > 0 && row.get(start) == Some(&Cell::Spacer) {
            start -= 1;
        }
        let end = if r == to.1 {
            usize::from(to.0)
        } else {
            usize::MAX
        }
        .min(row.len().saturating_sub(1));
        let mut s = String::new();
        for cell in row.iter().take(end + 1).skip(start) {
            match cell {
                Cell::Blank => s.push(' '),
                Cell::Text(t) => s.push_str(t),
                Cell::Spacer => {}
            }
        }
        let wrapped = line.is_some_and(|l| l.wrapped);
        if !wrapped || r == to.1 {
            s.truncate(s.trim_end_matches(' ').len());
        }
        if r != from.1 && !previous_wrapped {
            out.push('\n');
        }
        out.push_str(&s);
        previous_wrapped = wrapped;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mirror::tests::line;
    use deck_core::protocol::{Pos, Run, Style};

    fn runs(parts: &[(u16, &str)]) -> Line {
        Line {
            wrapped: false,
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

    fn text_of(lines: &[Line]) -> String {
        text(lines, 10, (0, 0), (9, 0))
    }

    /// A selection from `anchor` to `head`, in `(col, row id)`, grown by `unit` over `lines`.
    fn sel(anchor: (u16, u64), head: (u16, u64), unit: Unit, lines: &[Line]) -> TextSel {
        let at = |(col, row): (u16, u64)| Pos { row, col };
        let cols =
            |(col, row): (u16, u64)| grow(unit, lines.get(usize::try_from(row).unwrap()), 10, col);
        let mut s = TextSel::new(TerminalId(4), at(anchor), unit, cols(anchor));
        s.set_head(at(head), cols(head));
        s
    }

    #[test]
    fn program_placeholders_are_copied_as_spaces() {
        let p = crate::diacritics::PLACEHOLDER;
        let text = format!("a{p}\u{0305}\u{030D}{p}b\u{0305}");
        let lines = vec![line(&text)];
        // The mark after `b` is a real combining mark of `b`.
        assert_eq!(text_of(&lines), "a  b\u{0305}");
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
        assert_eq!(text(&lines, 6, (3, 0), (3, 0)), "b");
    }

    #[test]
    fn a_start_on_the_second_cell_of_a_wide_character_keeps_it() {
        let lines = vec![line("a界b")];
        assert_eq!(text(&lines, 6, (2, 0), (3, 0)), "界b");
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
        let pos = |row: u64, col: u16| Pos { row, col };
        assert_eq!(sel((3, 0), (3, 0), Unit::Cell, &lines).range(), None);
        assert_eq!(
            sel((2, 1), (5, 0), Unit::Cell, &lines).range(),
            Some((pos(0, 5), pos(1, 2)))
        );
        assert_eq!(
            sel((5, 0), (7, 1), Unit::Word, &lines).range(),
            Some((pos(0, 4), pos(1, 9)))
        );
        let whole = sel((5, 1), (1, 1), Unit::Line, &lines).range().unwrap();
        assert_eq!((whole.0, whole.1.row), (pos(1, 0), 1));
        assert_eq!(whole.1.col, u16::MAX);
    }

    #[test]
    fn a_wrapped_row_keeps_its_blanks_and_joins_the_next() {
        let mut first = line("abc ");
        first.wrapped = true;
        let lines = vec![first, line("def  "), line("ghi")];
        // Cells 0..3 of row 0 are `abc ` and the join adds no newline.
        assert_eq!(text(&lines, 4, (0, 0), (3, 2)), "abc def\nghi");
        // The last row of the range loses its trailing blanks, wrapped or not.
        let mut only = line("abc ");
        only.wrapped = true;
        assert_eq!(text(&[only], 4, (0, 0), (3, 0)), "abc");
    }

    #[test]
    fn a_frame_is_stale_when_it_moves_the_selection() {
        use crate::mirror::tests::frame;
        let screen = vec![line("a"), line("b"), line("c"), line("d"), line("e")];
        let mut old = TermMirror::default_for_test((10, 5));
        old.lines = screen.clone();
        old.top_row = 100;
        let s = sel((0, 102), (4, 103), Unit::Cell, &screen);
        let partial = |top: u64, rows: &[(u16, &str)]| {
            let mut f = frame(
                false,
                (10, 5),
                rows.iter().map(|(r, t)| (*r, line(t))).collect(),
            );
            f.top_row = top;
            f
        };
        assert!(!s.stale(&partial(100, &[(0, "x"), (4, "x")]), &old));
        assert!(s.stale(&partial(100, &[(3, "x")]), &old));
        // The same text again is no change.
        assert!(!s.stale(&partial(100, &[(3, "d")]), &old));
        // Rows that moved up with the view are the same rows: row 102 is now row 1.
        assert!(!s.stale(
            &partial(101, &[(0, "b"), (1, "c"), (2, "d"), (4, "z")]),
            &old
        ));
        assert!(s.stale(&partial(101, &[(2, "q")]), &old));
        // Scrolled out of the view: nothing to compare.
        assert!(!s.stale(&partial(110, &[(0, "x"), (1, "y")]), &old));
        let mut resized = partial(100, &[]);
        resized.size = (10, 6);
        assert!(s.stale(&resized, &old));
        let mut renumbered = partial(100, &[]);
        renumbered.numbering = 1;
        assert!(s.stale(&renumbered, &old));
        let mut other = partial(100, &[(3, "x")]);
        other.terminal = TerminalId(9);
        assert!(!s.stale(&other, &old));
    }

    #[test]
    fn a_full_frame_clears_only_when_a_selected_row_changes() {
        use crate::mirror::tests::frame;
        let screen = vec![line("a"), line("b"), line("c")];
        let mut old = TermMirror::default_for_test((10, 3));
        old.lines = screen.clone();
        let s = sel((0, 1), (4, 2), Unit::Cell, &screen);
        let full = |rows: Vec<(u16, Line)>| frame(true, (10, 3), rows);
        let same = full(vec![(0, line("new")), (1, line("b")), (2, line("c"))]);
        assert!(!s.stale(&same, &old));
        let changed = full(vec![(1, line("b")), (2, line("z"))]);
        assert!(s.stale(&changed, &old));
        // A selected row left out of a full frame is blank now.
        let dropped = full(vec![(1, line("b"))]);
        assert!(s.stale(&dropped, &old));
    }
}
