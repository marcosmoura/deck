//! One terminal emulator (`alacritty_terminal::Term`) and its conversion to protocol frames.

use std::fmt;
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::{ClipboardType, Config, Osc52, Term, TermDamage, TermMode};
use alacritty_terminal::vte::ansi::{
    Color as AnsiColor, CursorShape as AnsiCursorShape, NamedColor, Processor, Rgb,
};
use deck_core::model::TerminalId;
use deck_core::protocol::{
    Color, Cursor, CursorShape, Frame, Line as FrameLine, Modes, Run, ScrollTo, Style,
};

use crate::lock;

mod capture;

/// Collects the events a `Term` raises while it processes bytes.
#[derive(Clone, Default)]
struct Listener(Arc<Mutex<Vec<Event>>>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        lock(&self.0).push(event);
    }
}

struct Size {
    cols: usize,
    rows: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// The emulator needs at least two columns (a wide character does not fit in one) and a
/// huge size would allocate gigabytes of cells.
pub fn clamp_size(cols: u16, rows: u16) -> (u16, u16) {
    (cols.clamp(2, 1000), rows.clamp(1, 1000))
}

/// The largest scroll delta handed to the grid; the history is far smaller.
const LIMIT: i32 = 1 << 24;

type ColorFormat = Arc<dyn Fn(Rgb) -> String + Send + Sync>;

/// What the daemon does about an emulator event.
pub enum EmuEvent {
    /// A title change; `None` resets it.
    Title(Option<String>),
    /// Bytes to send to the program (answers to queries).
    Reply(String),
    /// OSC 4/10/11: the answer needs the client's colours, or the palette's entry.
    Color {
        index: usize,
        palette: Option<Rgb>,
        format: ColorFormat,
    },
    /// OSC 52 copy; `selection` is `"c"` or `"p"`.
    ClipboardStore {
        selection: &'static str,
        text: String,
    },
    /// OSC 52 read, which the daemon denies.
    ClipboardLoad,
}

impl fmt::Debug for EmuEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Title(t) => write!(f, "Title({t:?})"),
            Self::Reply(s) => write!(f, "Reply({s:?})"),
            Self::Color { index, .. } => write!(f, "Color({index})"),
            Self::ClipboardStore { selection, text } => {
                write!(f, "ClipboardStore({selection}, {text:?})")
            }
            Self::ClipboardLoad => write!(f, "ClipboardLoad"),
        }
    }
}

pub struct Emu {
    term: Term<Listener>,
    parser: Processor,
    events: Listener,
}

impl fmt::Debug for Emu {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Emu")
            .field("size", &self.size())
            .finish_non_exhaustive()
    }
}

impl Emu {
    pub fn new(cols: u16, rows: u16, scrollback: usize) -> Self {
        let events = Listener::default();
        let config = Config {
            scrolling_history: scrollback,
            osc52: Osc52::CopyPaste,
            ..Config::default()
        };
        let size = Size {
            cols: usize::from(clamp_size(cols, rows).0),
            rows: usize::from(clamp_size(cols, rows).1),
        };
        Self {
            term: Term::new(config, &size, events.clone()),
            parser: Processor::new(),
            events,
        }
    }

    pub fn size(&self) -> (u16, u16) {
        (
            u16::try_from(self.term.columns()).unwrap_or(u16::MAX),
            u16::try_from(self.term.screen_lines()).unwrap_or(u16::MAX),
        )
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.term.resize(Size {
            cols: usize::from(clamp_size(cols, rows).0),
            rows: usize::from(clamp_size(cols, rows).1),
        });
    }

    /// Feeds program output and returns what it asked of the daemon.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<EmuEvent> {
        self.parser.advance(&mut self.term, bytes);
        let raised = std::mem::take(&mut *lock(&self.events.0));
        raised
            .into_iter()
            .filter_map(|event| self.convert(event))
            .collect()
    }

    fn convert(&self, event: Event) -> Option<EmuEvent> {
        Some(match event {
            Event::Title(title) => EmuEvent::Title(Some(title)),
            Event::ResetTitle => EmuEvent::Title(None),
            Event::PtyWrite(text) => EmuEvent::Reply(text),
            Event::ColorRequest(index, format) => EmuEvent::Color {
                index,
                palette: self.term.colors()[index],
                format,
            },
            Event::TextAreaSizeRequest(format) => {
                let (cols, rows) = self.size();
                EmuEvent::Reply(format(WindowSize {
                    num_lines: rows,
                    num_cols: cols,
                    cell_width: 8,
                    cell_height: 16,
                }))
            }
            Event::ClipboardStore(kind, text) => EmuEvent::ClipboardStore {
                selection: match kind {
                    ClipboardType::Clipboard => "c",
                    ClipboardType::Selection => "p",
                },
                text,
            },
            Event::ClipboardLoad(..) => EmuEvent::ClipboardLoad,
            _ => return None,
        })
    }

    /// Moves the display through the history, clamped to it.
    pub fn scroll(&mut self, to: ScrollTo) {
        let rows = i32::try_from(self.term.screen_lines()).unwrap_or(i32::MAX);
        // The grid adds the delta to the offset unchecked, so keep it far from the limits.
        let delta = |n: i32| Scroll::Delta(n.clamp(-LIMIT, LIMIT));
        self.term.scroll_display(match to {
            ScrollTo::Lines(n) => delta(n),
            ScrollTo::Pages(n) => delta(n.saturating_mul(rows)),
            ScrollTo::Top => Scroll::Top,
            ScrollTo::Bottom => Scroll::Bottom,
        });
        // Scrolling raises a cursor-dirty event nobody needs.
        lock(&self.events.0).clear();
    }

    /// Returns the display to the live screen; whether it was scrolled back.
    pub fn scroll_to_bottom(&mut self) -> bool {
        let scrolled = self.term.grid().display_offset() > 0;
        if scrolled {
            self.scroll(ScrollTo::Bottom);
        }
        scrolled
    }

    /// Every row of the screen.
    pub fn full_frame(&self, terminal: TerminalId) -> Frame {
        let rows = self.term.screen_lines();
        let lines = (0..rows).map(|row| self.line(row)).collect();
        self.frame(terminal, lines, true)
    }

    /// The rows changed since the last call, or every row when the emulator says so or the
    /// display is scrolled back (its damage is of the live screen, not of what shows).
    pub fn damage_frame(&mut self, terminal: TerminalId) -> Frame {
        let rows = self.term.screen_lines();
        let scrolled = self.term.grid().display_offset() > 0;
        let damaged: Option<Vec<usize>> = match self.term.damage() {
            TermDamage::Full => None,
            TermDamage::Partial(iter) => Some(iter.map(|d| d.line).filter(|l| *l < rows).collect()),
        };
        self.term.reset_damage();
        match damaged {
            None => self.full_frame(terminal),
            Some(_) if scrolled => self.full_frame(terminal),
            Some(rows) => {
                let lines = rows.into_iter().map(|row| self.line(row)).collect();
                self.frame(terminal, lines, false)
            }
        }
    }

    pub fn reset_damage(&mut self) {
        let _ = self.term.damage();
        self.term.reset_damage();
    }

    fn frame(&self, terminal: TerminalId, lines: Vec<(u16, FrameLine)>, full: bool) -> Frame {
        let mode = self.term.mode();
        let point = self.term.grid().cursor.point;
        let (cols, rows) = self.size();
        let shape = self.term.cursor_style().shape;
        // The cursor sits on the live screen: scrolled back, it moves down with the
        // content and is hidden once it falls below the last row.
        let offset = i32::try_from(self.term.grid().display_offset()).unwrap_or(0);
        let row = point.line.0.max(0).saturating_add(offset);
        let on_screen = row < i32::from(rows);
        Frame {
            terminal,
            size: (cols, rows),
            cursor: Cursor {
                row: u16::try_from(row).unwrap_or(0).min(rows.saturating_sub(1)),
                col: u16::try_from(point.column.0)
                    .unwrap_or(0)
                    .min(cols.saturating_sub(1)),
                visible: on_screen
                    && mode.contains(TermMode::SHOW_CURSOR)
                    && shape != AnsiCursorShape::Hidden,
                shape: match shape {
                    AnsiCursorShape::Beam => CursorShape::Bar,
                    AnsiCursorShape::Underline => CursorShape::Underline,
                    _ => CursorShape::Block,
                },
            },
            modes: modes(*mode),
            display_offset: u32::try_from(self.term.grid().display_offset()).unwrap_or(0),
            history: u32::try_from(self.term.grid().history_size()).unwrap_or(0),
            lines,
            full,
        }
    }

    fn line(&self, row: usize) -> (u16, FrameLine) {
        let grid = self.term.grid();
        let offset = i32::try_from(grid.display_offset()).unwrap_or(0);
        let cells = &grid[Line(i32::try_from(row).unwrap_or(0) - offset)];
        let cols = self.term.columns();
        let end = (0..cols)
            .rev()
            .find(|c| !is_blank(&cells[Column(*c)]))
            .map_or(0, |c| c + 1);
        let mut runs: Vec<Run> = Vec::new();
        for col in 0..end {
            let cell = &cells[Column(col)];
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            let style = style_of(cell);
            let run = match runs.last_mut() {
                Some(run) if run.style == style => run,
                _ => runs.push_mut(Run {
                    col: u16::try_from(col).unwrap_or(u16::MAX),
                    text: String::new(),
                    style,
                }),
            };
            run.text.push(if cell.c == '\0' { ' ' } else { cell.c });
            if let Some(extra) = cell.zerowidth() {
                run.text.extend(extra);
            }
        }
        (u16::try_from(row).unwrap_or(u16::MAX), FrameLine { runs })
    }
}

fn modes(mode: TermMode) -> Modes {
    let table = [
        (TermMode::APP_CURSOR, Modes::APPLICATION_CURSOR),
        (TermMode::BRACKETED_PASTE, Modes::BRACKETED_PASTE),
        (TermMode::MOUSE_REPORT_CLICK, Modes::MOUSE_CLICK),
        (TermMode::MOUSE_DRAG, Modes::MOUSE_DRAG),
        (TermMode::MOUSE_MOTION, Modes::MOUSE_MOTION),
        (TermMode::SGR_MOUSE, Modes::SGR_MOUSE),
        (TermMode::FOCUS_IN_OUT, Modes::FOCUS_REPORTING),
        (TermMode::ALT_SCREEN, Modes::ALTERNATE_SCREEN),
        (TermMode::ALTERNATE_SCROLL, Modes::ALTERNATE_SCROLL),
    ];
    Modes(
        table
            .iter()
            .filter(|(m, _)| mode.contains(*m))
            .fold(0, |acc, (_, bit)| acc | bit),
    )
}

fn is_blank(cell: &Cell) -> bool {
    matches!(cell.c, ' ' | '\0') && cell.zerowidth().is_none() && style_of(cell) == Style::default()
}

fn style_of(cell: &Cell) -> Style {
    let table = [
        (Flags::BOLD, Style::BOLD),
        (Flags::DIM, Style::DIM),
        (Flags::ITALIC, Style::ITALIC),
        (Flags::ALL_UNDERLINES, Style::UNDERLINE),
        (Flags::INVERSE, Style::REVERSE),
        (Flags::STRIKEOUT, Style::STRIKETHROUGH),
        (Flags::HIDDEN, Style::HIDDEN),
    ];
    let flags = table
        .iter()
        .filter(|(f, _)| cell.flags.intersects(*f))
        .fold(0, |acc, (_, bit)| acc | bit);
    Style {
        fg: color(cell.fg),
        bg: color(cell.bg),
        flags,
    }
}

fn color(color: AnsiColor) -> Color {
    match color {
        AnsiColor::Spec(Rgb { r, g, b }) => Color::Rgb(r, g, b),
        AnsiColor::Indexed(i) => Color::Indexed(i),
        AnsiColor::Named(named) => {
            let n = named as usize;
            let dim = NamedColor::DimBlack as usize;
            if n < 16 {
                Color::Indexed(u8::try_from(n).unwrap_or(0))
            } else if (dim..dim + 8).contains(&n) {
                Color::Indexed(u8::try_from(n - dim).unwrap_or(0))
            } else {
                Color::Default
            }
        }
    }
}

/// The standard xterm colour for an index below 256, for an OSC 4 query the program made
/// before setting the entry itself.
pub fn default_palette(index: usize) -> Rgb {
    const ANSI: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (205, 0, 0),
        (0, 205, 0),
        (205, 205, 0),
        (0, 0, 238),
        (205, 0, 205),
        (0, 205, 205),
        (229, 229, 229),
        (127, 127, 127),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (92, 92, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    if let Some(&(r, g, b)) = ANSI.get(index) {
        return Rgb { r, g, b };
    }
    let (r, g, b) = match index {
        16..=231 => {
            let i = u8::try_from(index - 16).unwrap_or(0);
            let level = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
            (level(i / 36), level(i / 6 % 6), level(i % 6))
        }
        232..=255 => {
            let v = u8::try_from(8 + 10 * (index - 232)).unwrap_or(238);
            (v, v, v)
        }
        _ => (0, 0, 0),
    };
    Rgb { r, g, b }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emu(cols: u16, rows: u16) -> Emu {
        Emu::new(cols, rows, 100)
    }

    fn text(frame: &Frame, row: u16) -> String {
        frame
            .lines
            .iter()
            .find(|(r, _)| *r == row)
            .map(|(_, l)| l.runs.iter().map(|r| r.text.as_str()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn wide_and_combining_characters_keep_their_columns() {
        let mut e = emu(20, 3);
        e.feed("日本 🙂 e\u{301}x".as_bytes());
        // The emulator's own grid: where each character sits and which cells are spacers.
        let grid = e.term.grid();
        let cell = |c: usize| &grid[Line(0)][Column(c)];
        for (col, ch) in [(0, '日'), (2, '本'), (5, '🙂')] {
            assert_eq!(cell(col).c, ch);
            assert!(cell(col).flags.contains(Flags::WIDE_CHAR), "{ch}");
            assert!(
                cell(col + 1).flags.contains(Flags::WIDE_CHAR_SPACER),
                "{ch}"
            );
        }
        assert_eq!((cell(4).c, cell(7).c), (' ', ' '));
        assert_eq!(cell(8).c, 'e');
        assert_eq!(cell(8).zerowidth(), Some(&['\u{301}'][..]));
        assert_eq!(cell(9).c, 'x');

        // The frame is one run from column 0 with the spacers left out, so every character
        // keeps its emulator column.
        let frame = e.full_frame(TerminalId(1));
        let runs = &frame.lines[0].1.runs;
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].col, 0);
        assert_eq!(runs[0].text, "日本 🙂 e\u{301}x");
        assert_eq!(frame.cursor.col, 10);
    }

    #[test]
    fn the_smallest_emulator_survives_wide_output() {
        assert_eq!(clamp_size(0, 0), (2, 1));
        assert_eq!(clamp_size(u16::MAX, u16::MAX), (1000, 1000));
        let mut e = emu(1, 1);
        assert_eq!(e.size(), (2, 1));
        e.feed("日本🙂\r\n日".as_bytes());
        let _ = e.full_frame(TerminalId(1));
        e.resize(0, 0);
        e.feed("日本".as_bytes());
        e.resize(u16::MAX, u16::MAX);
        assert_eq!(e.size(), (1000, 1000));
    }

    #[test]
    fn style_changes_split_runs_and_trailing_blanks_are_dropped() {
        let mut e = emu(20, 2);
        e.feed(b"a\x1b[1;31mb\x1b[0m   ");
        let frame = e.full_frame(TerminalId(1));
        let runs = &frame.lines[0].1.runs;
        assert_eq!(runs.len(), 2, "{runs:?}");
        assert_eq!(runs[0].text, "a");
        assert_eq!((runs[1].col, runs[1].text.as_str()), (1, "b"));
        assert!(runs[1].style.has(Style::BOLD));
        assert_eq!(runs[1].style.fg, Color::Indexed(1));
        assert!(frame.lines[1].1.runs.is_empty());
        assert!(frame.full);
    }

    #[test]
    fn damage_frames_carry_only_changed_rows() {
        let mut e = emu(10, 4);
        e.feed(b"one\r\ntwo\r\nthree");
        let first = e.damage_frame(TerminalId(1));
        assert!(first.full, "a new terminal is fully damaged");
        e.feed(b"\x1b[2;1Hxx");
        let second = e.damage_frame(TerminalId(1));
        assert!(!second.full);
        assert!(second.lines.iter().any(|(r, _)| *r == 1));
        assert!(!second.lines.iter().any(|(r, _)| *r == 0));
        assert_eq!(text(&second, 1), "xxo");
        e.resize(12, 5);
        let resized = e.damage_frame(TerminalId(1));
        assert!(resized.full);
        assert_eq!(resized.size, (12, 5));
    }

    #[test]
    fn modes_and_cursor_are_reported() {
        let mut e = emu(10, 4);
        e.feed(b"\x1b[?1h\x1b[?2004h\x1b[?25l\x1b[6 q\x1b[3;4H");
        let frame = e.full_frame(TerminalId(1));
        assert!(frame.modes.has(Modes::APPLICATION_CURSOR));
        assert!(frame.modes.has(Modes::BRACKETED_PASTE));
        assert!(!frame.cursor.visible);
        assert_eq!(frame.cursor.shape, CursorShape::Bar);
        assert_eq!((frame.cursor.row, frame.cursor.col), (2, 3));
    }

    #[test]
    fn scrolling_clamps_to_the_history_and_frames_say_where_it_is() {
        let mut e = Emu::new(10, 4, 100);
        for n in 1..=20 {
            e.feed(format!("line {n}\r\n").as_bytes());
        }
        let live = e.full_frame(TerminalId(1));
        assert_eq!((live.display_offset, live.history), (0, 17));

        e.scroll(ScrollTo::Lines(2));
        assert_eq!(e.full_frame(TerminalId(1)).display_offset, 2);
        e.scroll(ScrollTo::Pages(1));
        assert_eq!(e.full_frame(TerminalId(1)).display_offset, 6);
        e.scroll(ScrollTo::Lines(i32::MAX));
        e.scroll(ScrollTo::Pages(i32::MAX));
        let top = e.full_frame(TerminalId(1));
        assert_eq!((top.display_offset, top.history), (17, 17));
        assert_eq!(text(&top, 0), "line 1");
        e.scroll(ScrollTo::Lines(i32::MIN));
        assert_eq!(e.full_frame(TerminalId(1)).display_offset, 0);
        e.scroll(ScrollTo::Top);
        assert_eq!(e.full_frame(TerminalId(1)).display_offset, 17);
        e.scroll(ScrollTo::Pages(-2));
        assert_eq!(e.full_frame(TerminalId(1)).display_offset, 9);
        assert!(e.scroll_to_bottom());
        assert!(!e.scroll_to_bottom());
        assert_eq!(e.full_frame(TerminalId(1)).display_offset, 0);
    }

    #[test]
    fn output_while_scrolled_back_keeps_the_view_and_grows_the_history() {
        let mut e = Emu::new(10, 4, 100);
        for n in 1..=10 {
            e.feed(format!("line {n}\r\n").as_bytes());
        }
        e.reset_damage();
        e.scroll(ScrollTo::Lines(2));
        let before = e.damage_frame(TerminalId(1));
        let shown = text(&before, 0);
        e.feed(b"more\r\n");
        let after = e.damage_frame(TerminalId(1));
        assert!(after.full, "a scrolled display is sent in full");
        assert_eq!(after.history, before.history + 1);
        assert_eq!(after.display_offset, before.display_offset + 1);
        assert_eq!(text(&after, 0), shown);
    }

    #[test]
    fn the_cursor_follows_the_content_when_scrolled_back_and_hides_below_the_screen() {
        let mut e = Emu::new(10, 4, 100);
        for n in 1..=10 {
            e.feed(format!("line {n}\r\n").as_bytes());
        }
        // The cursor is on the last row of the live screen.
        let live = e.full_frame(TerminalId(1)).cursor;
        assert_eq!((live.row, live.visible), (3, true));
        e.scroll(ScrollTo::Lines(1));
        assert!(!e.full_frame(TerminalId(1)).cursor.visible);
        e.scroll(ScrollTo::Top);
        assert!(!e.full_frame(TerminalId(1)).cursor.visible);
        // With the cursor higher on the screen it is still shown, one row lower.
        e.scroll(ScrollTo::Bottom);
        e.feed(b"\x1b[2;1H");
        assert_eq!(e.full_frame(TerminalId(1)).cursor.row, 1);
        e.scroll(ScrollTo::Lines(1));
        let c = e.full_frame(TerminalId(1)).cursor;
        assert_eq!((c.row, c.visible), (2, true));
        e.scroll(ScrollTo::Lines(5));
        assert!(!e.full_frame(TerminalId(1)).cursor.visible);
    }

    #[test]
    fn alternate_scroll_is_on_by_default_and_a_program_can_turn_it_off() {
        let mut e = emu(10, 4);
        assert!(
            e.full_frame(TerminalId(1))
                .modes
                .has(Modes::ALTERNATE_SCROLL)
        );
        e.feed(b"\x1b[?1007l");
        assert!(
            !e.full_frame(TerminalId(1))
                .modes
                .has(Modes::ALTERNATE_SCROLL)
        );
        e.feed(b"\x1b[?1007h");
        assert!(
            e.full_frame(TerminalId(1))
                .modes
                .has(Modes::ALTERNATE_SCROLL)
        );
    }

    #[test]
    fn events_are_converted() {
        let mut e = emu(10, 4);
        let events = e
            .feed(b"\x1b]0;hello\x07\x1b[6n\x1b]11;?\x07\x1b]52;c;aGk=\x07\x1b]52;c;?\x07\x1b[18t");
        let mut seen = String::new();
        for ev in &events {
            seen.push_str(&format!("{ev:?};"));
        }
        assert!(seen.contains("Title(Some(\"hello\"))"), "{seen}");
        assert!(seen.contains("Reply(\"\\u{1b}[1;1R\")"), "{seen}");
        assert!(seen.contains("Color(257)"), "{seen}");
        assert!(seen.contains("ClipboardStore(c, \"hi\")"), "{seen}");
        assert!(seen.contains("ClipboardLoad"), "{seen}");
        assert!(seen.contains("[8;4;10t"), "{seen}");
        let color = events.iter().find_map(|ev| match ev {
            EmuEvent::Color { format, .. } => Some(format(Rgb { r: 1, g: 2, b: 3 })),
            _ => None,
        });
        assert_eq!(color.unwrap(), "\x1b]11;rgb:0101/0202/0303\x07");
    }

    #[test]
    fn default_palette_has_the_xterm_cube_and_greys() {
        assert_eq!(default_palette(1), Rgb { r: 205, g: 0, b: 0 });
        assert_eq!(default_palette(16), Rgb { r: 0, g: 0, b: 0 });
        assert_eq!(
            default_palette(231),
            Rgb {
                r: 255,
                g: 255,
                b: 255
            }
        );
        assert_eq!(default_palette(232), Rgb { r: 8, g: 8, b: 8 });
        assert_eq!(
            default_palette(255),
            Rgb {
                r: 238,
                g: 238,
                b: 238
            }
        );
    }
}
