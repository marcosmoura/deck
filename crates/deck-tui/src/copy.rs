//! Copy mode, inside scroll mode: the copy cursor and its vi motions, the keyboard selection
//! (`v`, `V`, `o`), the copy (`y`), and the drag that scrolls the view at the screen's edge.
//!
//! Positions are row ids (see `Frame::top_row`), so the cursor and a selection keep their
//! rows while the view scrolls, here or in another client.

use std::time::{Duration, Instant};

use deck_core::keys::{KeyCode, KeyEvent};
use deck_core::model::TerminalId;
use deck_core::protocol::{Pos, Request, ScrollTo};

use crate::app::{App, Out};
use crate::picker::Req;
use crate::select::{Cell, TextSel, Unit, cells, grow, text, wordish};

/// How often a drag past the top or bottom edge scrolls the view one row.
pub(crate) const EDGE_EVERY: Duration = Duration::from_millis(50);

/// The cap on copied text, as the daemon says it.
const TRUNCATED: &str = "Copied the first 8 MiB";

/// The copy cursor of scroll mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CopyState {
    pub terminal: TerminalId,
    /// The generation of the row ids it was set in.
    pub numbering: u32,
    pub cursor: Pos,
    /// The id of the view's first row once the scrolls this client sent have landed; `None`
    /// when none is on its way. Keys typed faster than frames arrive build on it.
    pub expected_top: Option<u64>,
}

/// A drag with the button down past the top or bottom of the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Edge {
    pub terminal: TerminalId,
    pub up: bool,
    /// The column of the pointer.
    pub col: u16,
    pub next: Instant,
}

/// What a copy was started by: the mouse says how many characters, the keyboard lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Copied {
    Mouse,
    Key,
}

/// Where a motion goes: a row relative to the view's first row (it may lie outside the
/// screen: the view scrolls then) and a column.
type Target = (i64, u16);

/// The view as a motion needs it, in signed row ids.
struct View {
    /// The id of the first row shown, counting the scrolls in flight.
    top: i64,
    rows: i64,
    cols: u16,
    /// The oldest and newest rows there are.
    min: i64,
    max: i64,
}

impl View {
    fn bottom(&self) -> i64 {
        self.top + self.rows - 1
    }

    /// The id of the first row when the view is at the bottom.
    fn live_top(&self) -> i64 {
        (self.max - self.rows + 1).max(self.min)
    }
}

fn id(row: i64) -> u64 {
    u64::try_from(row).unwrap_or(0)
}

fn signed(row: u64) -> i64 {
    i64::try_from(row).unwrap_or(i64::MAX)
}

impl App {
    /// Enters scroll mode on the focused terminal, with the copy cursor on the terminal's
    /// cursor cell.
    pub(crate) fn enter_scroll(&mut self) {
        self.scroll_mode = self.ready();
        self.copy = self
            .scroll_mode
            .and_then(|terminal| self.copy_at_cursor(terminal));
    }

    /// The copy cursor on the terminal's cursor cell. When another client has the view
    /// scrolled back, the frame's cursor row is clamped to the screen, so it starts on the
    /// edge row then.
    fn copy_at_cursor(&self, terminal: TerminalId) -> Option<CopyState> {
        let t = self.mirror.terminal(terminal)?;
        Some(CopyState {
            terminal,
            numbering: t.numbering,
            cursor: Pos {
                row: t.top_row + u64::from(t.cursor.row),
                col: t.cursor.col,
            },
            expected_top: None,
        })
    }

    /// Ends scroll mode's own state: the cursor, a selection and a drag past the edge.
    pub(crate) fn end_copy(&mut self) {
        self.copy = None;
        self.edge = None;
        self.search.clear();
        if self.text_sel.take().is_some() {
            self.dirty = true;
        }
    }

    fn view(&self, terminal: TerminalId) -> Option<View> {
        let t = self.mirror.terminal(terminal)?;
        let top = signed(t.top_row);
        let live = top + i64::from(t.display_offset);
        let rows = i64::from(t.size.1).max(1);
        let expected = self
            .copy
            .as_ref()
            .filter(|c| c.terminal == terminal)
            .and_then(|c| c.expected_top);
        Some(View {
            top: expected.map_or(top, signed),
            rows,
            cols: t.size.0,
            min: (live - i64::from(t.history)).max(0),
            max: live + rows - 1,
        })
    }

    /// The cells of the row `row` as the mirror shows it (blank when it is not on screen).
    fn row_cells(&self, terminal: TerminalId, row: i64) -> Vec<Cell> {
        let Some(t) = self.mirror.terminal(terminal) else {
            return Vec::new();
        };
        let line = row
            .checked_sub(signed(t.top_row))
            .and_then(|at| usize::try_from(at).ok())
            .and_then(|at| t.lines.get(at));
        cells(line, t.size.0)
    }

    /// What `col` of row `row` grows to with `unit`.
    fn grown(&self, terminal: TerminalId, unit: Unit, row: u64, col: u16) -> (u16, u16) {
        let Some(t) = self.mirror.terminal(terminal) else {
            return (col, col);
        };
        let line = row
            .checked_sub(t.top_row)
            .and_then(|at| usize::try_from(at).ok())
            .and_then(|at| t.lines.get(at));
        grow(unit, line, t.size.0, col)
    }

    /// The copy cursor, moved onto the screen when its row is off it.
    fn cursor_on_screen(&self, terminal: TerminalId, view: &View) -> Option<Pos> {
        let copy = self.copy.as_ref().filter(|c| c.terminal == terminal)?;
        let row = signed(copy.cursor.row)
            .clamp(view.top, view.bottom())
            .clamp(view.min, view.max);
        Some(Pos {
            row: id(row),
            col: copy.cursor.col.min(view.cols.saturating_sub(1)),
        })
    }

    /// A key typed in scroll mode: it moves the cursor, selects, copies or scrolls, or is
    /// dropped; it never reaches the terminal.
    pub(crate) fn scroll_key(&mut self, terminal: TerminalId, key: KeyEvent) -> Vec<Out> {
        let (ctrl, alt) = (key.mods.ctrl, key.mods.alt);
        if alt || self.view(terminal).is_none() {
            return Vec::new();
        }
        if self.copy.as_ref().is_none_or(|c| c.terminal != terminal) {
            self.copy = self.copy_at_cursor(terminal);
        }
        self.dirty = true;
        let motion = match key.code {
            KeyCode::Esc => {
                // A selection first, then a search's highlights; scroll mode's own end
                // comes after.
                self.edge = None;
                if self.text_sel.take().is_some() {
                    return Vec::new();
                }
                if self.search.active.take().is_some() {
                    self.search.current = None;
                    return Vec::new();
                }
                return self.leave_scroll();
            }
            KeyCode::Char('q') if !ctrl => return self.leave_scroll(),
            KeyCode::Char('v') if !ctrl => return self.toggle_selection(terminal, Unit::Cell),
            KeyCode::Char('V') if !ctrl => return self.toggle_selection(terminal, Unit::Line),
            KeyCode::Char('o') if !ctrl => return self.other_end(terminal),
            KeyCode::Char('y') if !ctrl => return self.copy_selection(terminal, true),
            KeyCode::Enter => return self.copy_selection(terminal, false),
            KeyCode::Char('/') if !ctrl => {
                self.search_open(false);
                return Vec::new();
            }
            KeyCode::Char('?') if !ctrl => {
                self.search_open(true);
                return Vec::new();
            }
            KeyCode::Char('n') if !ctrl => return self.search_again(terminal, false),
            KeyCode::Char('N') if !ctrl => return self.search_again(terminal, true),
            KeyCode::Left => Motion::Left,
            KeyCode::Char('h') if !ctrl => Motion::Left,
            KeyCode::Right => Motion::Right,
            KeyCode::Char('l') if !ctrl => Motion::Right,
            KeyCode::Up => Motion::Up,
            KeyCode::Char('k') if !ctrl => Motion::Up,
            KeyCode::Down => Motion::Down,
            KeyCode::Char('j') if !ctrl => Motion::Down,
            KeyCode::Char('w') if !ctrl => Motion::WordForward,
            KeyCode::Char('b') if !ctrl => Motion::WordBack,
            KeyCode::Char('e') if !ctrl => Motion::WordEnd,
            KeyCode::Char('0') if !ctrl => Motion::LineStart,
            KeyCode::Char('^') if !ctrl => Motion::FirstNonBlank,
            KeyCode::Char('$') if !ctrl => Motion::LineEnd,
            KeyCode::Char('H') if !ctrl => Motion::ScreenTop,
            KeyCode::Char('M') if !ctrl => Motion::ScreenMiddle,
            KeyCode::Char('L') if !ctrl => Motion::ScreenBottom,
            KeyCode::Home => Motion::Top,
            KeyCode::Char('g') if !ctrl => Motion::Top,
            KeyCode::End => Motion::Bottom,
            KeyCode::Char('G') if !ctrl => Motion::Bottom,
            KeyCode::PageUp => Motion::Page(1),
            KeyCode::Char('b') if ctrl => Motion::Page(1),
            KeyCode::PageDown => Motion::Page(-1),
            KeyCode::Char('f') if ctrl => Motion::Page(-1),
            KeyCode::Char('u') if ctrl => Motion::Half(1),
            KeyCode::Char('d') if ctrl => Motion::Half(-1),
            _ => return Vec::new(),
        };
        self.motion(terminal, motion)
    }

    /// Moves the cursor (and the end of a keyboard selection with it), scrolling the view by
    /// the rows the cursor crosses.
    fn motion(&mut self, terminal: TerminalId, motion: Motion) -> Vec<Out> {
        let Some(view) = self.view(terminal) else {
            return Vec::new();
        };
        let Some(at) = self.cursor_on_screen(terminal, &view) else {
            return Vec::new();
        };
        let row = signed(at.row);
        let rel = row - view.top;
        let here = || self.row_cells(terminal, row);
        let mut scroll = None;
        let target: Target = match motion {
            Motion::Left => (rel, step(&here(), at.col, false)),
            Motion::Right => (rel, step(&here(), at.col, true)),
            Motion::Up => (rel - 1, at.col),
            Motion::Down => (rel + 1, at.col),
            Motion::WordForward | Motion::WordBack | Motion::WordEnd => {
                let rows: Vec<Vec<Cell>> = (view.top..=view.bottom())
                    .map(|r| self.row_cells(terminal, r))
                    .collect();
                let from = (usize::try_from(rel).unwrap_or(0), at.col);
                match motion {
                    Motion::WordForward => word_forward(&rows, from),
                    Motion::WordBack => word_back(&rows, from),
                    _ => word_end(&rows, from),
                }
            }
            Motion::LineStart => (rel, 0),
            Motion::FirstNonBlank => (rel, first_non_blank(&here())),
            Motion::LineEnd => (rel, last_non_blank(&here())),
            Motion::ScreenTop => (0, at.col),
            Motion::ScreenMiddle => ((view.rows - 1) / 2, at.col),
            Motion::ScreenBottom => (view.rows - 1, at.col),
            Motion::Top | Motion::Bottom => {
                let (to, goal) = if motion == Motion::Top {
                    (view.min, view.min)
                } else {
                    (view.max, view.max)
                };
                scroll = Some(ScrollTo::Row(id(to)));
                (goal - view.top, 0)
            }
            Motion::Half(sign) | Motion::Page(sign) => {
                let n = match motion {
                    Motion::Half(_) => i32::try_from((view.rows / 2).max(1)).unwrap_or(1),
                    _ => 1,
                };
                scroll = Some(match motion {
                    Motion::Half(_) => ScrollTo::Lines(sign * n),
                    _ => ScrollTo::Pages(sign),
                });
                (rel, at.col)
            }
        };
        let (rel, col) = target;
        let mut outs = Vec::new();
        let moved_to;
        if let Some(to) = scroll {
            // The scroll carries the cursor with the view, except to the ends of the history.
            let before = view.top;
            let out = self.scroll(terminal, to);
            let after = self
                .copy
                .as_ref()
                .and_then(|c| c.expected_top)
                .map_or(before, signed);
            outs.push(out);
            moved_to = if matches!(motion, Motion::Top | Motion::Bottom) {
                view.top + rel
            } else {
                row + (after - before)
            };
        } else {
            let wanted = (view.top + rel).clamp(view.min, view.max);
            if wanted < view.top {
                outs.push(self.scroll(terminal, ScrollTo::Lines(clip(view.top - wanted))));
            } else if wanted > view.bottom() {
                outs.push(self.scroll(terminal, ScrollTo::Lines(-clip(wanted - view.bottom()))));
            }
            moved_to = wanted;
        }
        let row = moved_to.clamp(view.min, view.max);
        self.set_cursor(terminal, Pos { row: id(row), col });
        outs
    }

    /// Puts the cursor on `at` and takes the keyboard selection's moving end with it.
    pub(crate) fn set_cursor(&mut self, terminal: TerminalId, at: Pos) {
        let Some(copy) = self.copy.as_mut() else {
            return;
        };
        copy.cursor = at;
        let unit = self
            .text_sel
            .as_ref()
            .filter(|s| s.terminal == terminal && !s.dragging)
            .map(|s| s.unit);
        if let Some(unit) = unit {
            let cols = self.grown(terminal, unit, at.row, at.col);
            if let Some(sel) = &mut self.text_sel {
                sel.set_head(at, cols);
            }
        }
    }

    /// `v` and `V`: starts a selection of that kind at the cursor, ends one of the same
    /// kind, switches one of the other kind.
    fn toggle_selection(&mut self, terminal: TerminalId, unit: Unit) -> Vec<Out> {
        let Some(view) = self.view(terminal) else {
            return Vec::new();
        };
        let Some(at) = self.cursor_on_screen(terminal, &view) else {
            return Vec::new();
        };
        self.set_cursor(terminal, at);
        self.edge = None;
        match self.text_sel.take().filter(|s| s.terminal == terminal) {
            Some(sel) if sel.unit == unit => {}
            Some(mut sel) => {
                sel.set_unit(unit, |p| self.grown(terminal, unit, p.row, p.col));
                self.text_sel = Some(sel);
            }
            None => {
                let cols = self.grown(terminal, unit, at.row, at.col);
                self.text_sel = Some(TextSel::new(terminal, at, unit, cols));
            }
        }
        Vec::new()
    }

    /// `o`: the cursor goes to the other end of the selection.
    fn other_end(&mut self, terminal: TerminalId) -> Vec<Out> {
        let Some(sel) = self.text_sel.as_mut().filter(|s| s.terminal == terminal) else {
            return Vec::new();
        };
        sel.swap();
        let head = sel.head;
        if let Some(copy) = self.copy.as_mut() {
            copy.cursor = head;
        }
        let Some(view) = self.view(terminal) else {
            return Vec::new();
        };
        if signed(head.row) >= view.top && signed(head.row) <= view.bottom() {
            return Vec::new();
        }
        vec![self.scroll(terminal, ScrollTo::Row(head.row))]
    }

    /// `y` (and `Enter`, which needs a selection): copies the selection, or with `line_ok`
    /// and none, the cursor's line.
    fn copy_selection(&mut self, terminal: TerminalId, line_ok: bool) -> Vec<Out> {
        let sel = self.text_sel.take().filter(|s| s.terminal == terminal);
        self.edge = None;
        self.dirty = true;
        let (from, to, lines) = match sel {
            Some(sel) => match sel.range() {
                Some((from, to)) => (from, to, sel.unit == Unit::Line),
                // A character selection that never left its cell is that cell.
                None => (sel.anchor, sel.anchor, false),
            },
            None if line_ok => {
                let Some(view) = self.view(terminal) else {
                    return Vec::new();
                };
                let Some(at) = self.cursor_on_screen(terminal, &view) else {
                    return Vec::new();
                };
                (at, at, true)
            }
            None => return Vec::new(),
        };
        vec![self.request(
            Request::Text {
                terminal,
                from,
                to,
                lines,
            },
            Req::Copy(Copied::Key),
        )]
    }
}

fn clip(rows: i64) -> i32 {
    i32::try_from(rows).unwrap_or(i32::MAX)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Motion {
    Left,
    Right,
    Up,
    Down,
    WordForward,
    WordBack,
    WordEnd,
    LineStart,
    FirstNonBlank,
    LineEnd,
    ScreenTop,
    ScreenMiddle,
    ScreenBottom,
    Top,
    Bottom,
    /// Half a screen, up for 1 and down for -1.
    Half(i32),
    /// A screen, up for 1 and down for -1.
    Page(i32),
}

/// The column a step left or right of `col` lands on, over the second cell of a wide
/// character.
fn step(row: &[Cell], col: u16, right: bool) -> u16 {
    let last = u16::try_from(row.len().saturating_sub(1)).unwrap_or(u16::MAX);
    let mut to = if right {
        col.saturating_add(1).min(last)
    } else {
        col.saturating_sub(1)
    };
    if row.get(usize::from(to)) == Some(&Cell::Spacer) {
        to = if right {
            to.saturating_add(1).min(last)
        } else {
            to.saturating_sub(1)
        };
    }
    to
}

fn col_of(i: usize) -> u16 {
    u16::try_from(i).unwrap_or(u16::MAX)
}

fn row_of(i: usize) -> i64 {
    i64::try_from(i).unwrap_or(i64::MAX)
}

/// The first cell of the next word (`w`); past the last row when there is none.
pub(crate) fn word_forward(rows: &[Vec<Cell>], (row, col): (usize, u16)) -> Target {
    let (mut r, mut c) = (row, usize::from(col));
    while rows.get(r).and_then(|x| x.get(c)).is_some_and(wordish) {
        c += 1;
    }
    loop {
        let Some(cells) = rows.get(r) else {
            return (row_of(rows.len()), 0);
        };
        while cells.get(c).is_some_and(|cell| !wordish(cell)) {
            c += 1;
        }
        if c < cells.len() {
            return (row_of(r), col_of(c));
        }
        r += 1;
        c = 0;
    }
}

/// The first cell of the previous word (`b`); above the first row when there is none.
pub(crate) fn word_back(rows: &[Vec<Cell>], (row, col): (usize, u16)) -> Target {
    let (mut r, mut c) = (row_of(row), i64::from(col) - 1);
    loop {
        while c < 0 {
            r -= 1;
            let Some(len) = usize::try_from(r)
                .ok()
                .and_then(|r| rows.get(r))
                .map(Vec::len)
            else {
                return (-1, 0);
            };
            c = row_of(len) - 1;
        }
        let cell = usize::try_from(r)
            .ok()
            .and_then(|r| rows.get(r))
            .and_then(|x| x.get(usize::try_from(c).unwrap_or(0)));
        if cell.is_some_and(wordish) {
            break;
        }
        c -= 1;
    }
    let cells = usize::try_from(r)
        .ok()
        .and_then(|r| rows.get(r))
        .map_or(&[][..], Vec::as_slice);
    let mut c = usize::try_from(c).unwrap_or(0);
    while c > 0 && cells.get(c - 1).is_some_and(wordish) {
        c -= 1;
    }
    (r, col_of(c))
}

/// The last cell of the word (`e`): of the current one when the cursor is not on its end,
/// else of the next; past the last row when there is none.
pub(crate) fn word_end(rows: &[Vec<Cell>], (row, col): (usize, u16)) -> Target {
    let (mut r, mut c) = (row, usize::from(col) + 1);
    loop {
        let Some(cells) = rows.get(r) else {
            return (row_of(rows.len()), 0);
        };
        while cells.get(c).is_some_and(|cell| !wordish(cell)) {
            c += 1;
        }
        if c < cells.len() {
            while cells.get(c + 1).is_some_and(wordish) {
                c += 1;
            }
            return (row_of(r), col_of(c));
        }
        r += 1;
        c = 0;
    }
}

fn blank(cell: &Cell) -> bool {
    match cell {
        Cell::Blank => true,
        Cell::Text(t) => t.trim().is_empty(),
        Cell::Spacer => false,
    }
}

/// The first cell with something in it (`^`).
pub(crate) fn first_non_blank(row: &[Cell]) -> u16 {
    row.iter().position(|c| !blank(c)).map_or(0, col_of)
}

/// The last cell with something in it (`$`); a wide character counts by its first cell.
pub(crate) fn last_non_blank(row: &[Cell]) -> u16 {
    row.iter()
        .rposition(|c| !blank(c) && *c != Cell::Spacer)
        .map_or(0, col_of)
}

impl App {
    /// The daemon's answer to a `Request::Text`: the text goes to the outer clipboard.
    pub(crate) fn copied(
        &mut self,
        how: Copied,
        text: &str,
        truncated: bool,
        now: Instant,
    ) -> Vec<Out> {
        if text.is_empty() {
            return Vec::new();
        }
        let chars = text.chars().count();
        let lines = text.split('\n').count();
        let message = if truncated {
            TRUNCATED.to_string()
        } else if how == Copied::Key && lines > 1 {
            format!("Copied {lines} lines")
        } else {
            let unit = if chars == 1 {
                "character"
            } else {
                "characters"
            };
            format!("Copied {chars} {unit}")
        };
        self.set_flash(message, now, false);
        vec![crate::app::osc52("c", text)]
    }

    /// The release of a mouse selection: the text comes from the mirror when every row is on
    /// screen, else from the daemon.
    pub(crate) fn copy_mouse_selection(&mut self, sel: &TextSel, now: Instant) -> Vec<Out> {
        let Some((from, to)) = sel.range() else {
            return Vec::new();
        };
        let terminal = sel.terminal;
        let Some(term) = self.mirror.terminal(terminal) else {
            return Vec::new();
        };
        let top = term.top_row;
        let rows = u64::from(term.size.1);
        if from.row >= top && to.row < top + rows {
            let at = |p: Pos| (p.col, u16::try_from(p.row - top).unwrap_or(0));
            let text = text(&term.lines, term.size.0, at(from), at(to));
            return self.copied(Copied::Mouse, &text, false, now);
        }
        vec![self.request(
            Request::Text {
                terminal,
                from,
                to,
                lines: sel.unit == Unit::Line,
            },
            Req::Copy(Copied::Mouse),
        )]
    }

    /// A drag that is past the top or bottom of the screen: the view scrolls one row now
    /// and again every [`EDGE_EVERY`] while the pointer stays there.
    pub(crate) fn edge_drag(
        &mut self,
        terminal: TerminalId,
        up: bool,
        col: u16,
        now: Instant,
    ) -> Vec<Out> {
        let fresh = self.edge.is_none();
        self.edge = Some(Edge {
            terminal,
            up,
            col,
            next: self
                .edge
                .as_ref()
                .filter(|e| e.up == up)
                .map_or(now, |e| e.next),
        });
        if fresh || self.edge.as_ref().is_some_and(|e| e.next <= now) {
            return self.edge_step(now);
        }
        Vec::new()
    }

    /// One row of scrolling in the direction of the edge drag, with the selection's end on the
    /// row that came into view.
    fn edge_step(&mut self, now: Instant) -> Vec<Out> {
        let Some(edge) = self.edge.clone() else {
            return Vec::new();
        };
        let terminal = edge.terminal;
        // Only a selection still being dragged is scrolled for.
        let dragging = self
            .text_sel
            .as_ref()
            .is_some_and(|s| s.dragging && s.terminal == terminal);
        let Some(view) = self.view(terminal).filter(|_| dragging) else {
            self.edge = None;
            return Vec::new();
        };
        let mut outs = Vec::new();
        let row = if edge.up {
            if view.top > view.min {
                outs.push(self.scroll(terminal, ScrollTo::Lines(1)));
            }
            (view.top - 1).max(view.min)
        } else {
            if view.top < view.live_top() {
                outs.push(self.scroll(terminal, ScrollTo::Lines(-1)));
            }
            (view.bottom() + 1).min(view.max)
        };
        let at = Pos {
            row: id(row),
            col: edge.col,
        };
        let unit = self.text_sel.as_ref().map(|s| s.unit);
        if let Some(unit) = unit {
            let cols = self.grown(terminal, unit, at.row, at.col);
            if let Some(sel) = self.text_sel.as_mut() {
                sel.set_head(at, cols);
            }
        }
        if let Some(e) = &mut self.edge {
            e.next = now + EDGE_EVERY;
        }
        self.dirty = true;
        outs
    }

    /// Runs the edge drag's scroll when it is due.
    pub(crate) fn edge_tick(&mut self, now: Instant) -> Vec<Out> {
        if self.edge.as_ref().is_some_and(|e| now >= e.next) {
            return self.edge_step(now);
        }
        Vec::new()
    }

    /// Whether a frame is on the way of the scrolls this client sent: its view is between
    /// the mirror's and the one they bring about, and neither.
    pub(crate) fn scrolls_in_flight(&self, frame: &deck_core::protocol::Frame) -> bool {
        let expected = self
            .copy
            .as_ref()
            .filter(|c| c.terminal == frame.terminal)
            .and_then(|c| c.expected_top);
        let old = self.mirror.terminal(frame.terminal).map(|t| t.top_row);
        let (Some(expected), Some(old)) = (expected, old) else {
            return false;
        };
        // A frame that did not move the view is not on the way.
        frame.top_row != expected
            && frame.top_row != old
            && (old.min(expected)..=old.max(expected)).contains(&frame.top_row)
    }

    /// The scroll of a frame that came, for the copy cursor of its terminal: drops the
    /// cursor's state when the ids it is in no longer mean the same rows, and the expected
    /// view once it was reached or something else moved it.
    pub(crate) fn copy_follows(&mut self, frame: &deck_core::protocol::Frame) {
        if self
            .copy
            .as_ref()
            .is_none_or(|c| c.terminal != frame.terminal)
        {
            return;
        }
        if self
            .copy
            .as_ref()
            .is_some_and(|c| c.numbering != frame.numbering)
        {
            let cursor = Pos {
                row: frame.top_row + u64::from(frame.cursor.row),
                col: frame.cursor.col,
            };
            if let Some(copy) = self.copy.as_mut() {
                copy.numbering = frame.numbering;
                copy.expected_top = None;
                copy.cursor = cursor;
            }
            self.text_sel = None;
            self.edge = None;
            self.search.current = None;
            return;
        }
        let old = self.mirror.terminal(frame.terminal).map(|t| t.top_row);
        if let (Some(copy), Some(old)) = (self.copy.as_mut(), old)
            && let Some(expected) = copy.expected_top
        {
            let (lo, hi) = (old.min(expected), old.max(expected));
            let between = (lo..=hi).contains(&frame.top_row);
            if frame.top_row == expected || !between {
                copy.expected_top = None;
            }
        }
    }
}

/// The copy cursor's cell in a terminal's screen, for drawing: its row index and column, moved
/// to the nearest row when its row is off the screen.
pub(crate) fn cursor_cell(app: &App, terminal: TerminalId) -> Option<(u16, u16)> {
    let copy = app.copy.as_ref().filter(|c| c.terminal == terminal)?;
    app.scroll_mode.filter(|t| *t == terminal)?;
    let t = app.mirror.terminal(terminal)?;
    let last = u64::from(t.size.1.saturating_sub(1));
    let row = copy
        .cursor
        .row
        .saturating_sub(t.top_row)
        .min(last)
        .try_into()
        .ok()?;
    Some((row, copy.cursor.col.min(t.size.0.saturating_sub(1))))
}
