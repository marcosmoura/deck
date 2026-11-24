//! deck's own ratatui `Backend`: crossterm's writes a plain `4` for an underline and resets
//! after it, so it cannot draw curly, double, dotted and dashed underlines.
//!
//! A ratatui `Cell` has no field for the underline kind, so the kind travels in the cell's
//! modifier, in bits deck never uses otherwise (blink is a non-goal): the `UNDERLINED` bit says
//! there is an underline, and [`KIND_BITS`] hold its kind (see [`kind_bits`]). `Buffer::diff`
//! compares whole cells, so a change of kind alone redraws the cell. [`DeckBackend::draw`]
//! writes the kind as `4:n` after the other attributes.

use std::io::{self, Write};

use ratatui::backend::{Backend, ClearType, IntoCrossterm, WindowSize};
use ratatui::buffer::Cell;
use ratatui::crossterm::cursor::{Hide, MoveTo, Show};
use ratatui::crossterm::style::{
    Attribute, Colors, Print, SetAttribute, SetBackgroundColor, SetColors, SetForegroundColor,
    SetUnderlineColor,
};
use ratatui::crossterm::terminal::{self, Clear};
use ratatui::crossterm::{execute, queue};
use ratatui::layout::{Position, Size};
use ratatui::style::{Color, Modifier};

use deck_core::protocol::Underline;

/// The bits of a cell's modifier that carry the underline kind: the two blink bits and the
/// first bit ratatui leaves unused.
const KIND_SLOW: Modifier = Modifier::SLOW_BLINK;
const KIND_RAPID: Modifier = Modifier::RAPID_BLINK;
const KIND_HIGH: Modifier = Modifier::from_bits_retain(1 << 9);
pub const KIND_BITS: Modifier = Modifier::from_bits_retain(
    Modifier::SLOW_BLINK.bits() | Modifier::RAPID_BLINK.bits() | (1 << 9),
);

/// The modifier of a cell with this underline: empty without one, else `UNDERLINED` and the
/// kind. A single underline is kind 0, so it is plain `UNDERLINED`.
pub fn modifier_of(kind: Underline) -> Modifier {
    let (code, high) = match kind {
        Underline::None => return Modifier::empty(),
        Underline::Single => (0, false),
        Underline::Double => (1, false),
        Underline::Curly => (2, false),
        Underline::Dotted => (3, false),
        Underline::Dashed => (4, true),
    };
    let mut out = Modifier::UNDERLINED;
    if code & 1 != 0 {
        out |= KIND_SLOW;
    }
    if code & 2 != 0 {
        out |= KIND_RAPID;
    }
    if high {
        out |= KIND_HIGH;
    }
    out
}

/// The underline a modifier carries.
pub fn kind_bits(modifier: Modifier) -> Underline {
    if !modifier.contains(Modifier::UNDERLINED) {
        return Underline::None;
    }
    let slow = modifier.contains(KIND_SLOW);
    let rapid = modifier.contains(KIND_RAPID);
    match (modifier.contains(KIND_HIGH), rapid, slow) {
        (true, ..) => Underline::Dashed,
        (false, true, true) => Underline::Dotted,
        (false, true, false) => Underline::Curly,
        (false, false, true) => Underline::Double,
        (false, false, false) => Underline::Single,
    }
}

/// The `SGR 4` sub-parameter of an underline kind.
fn sub_parameter(kind: Underline) -> u8 {
    match kind {
        Underline::None => 0,
        Underline::Single => 1,
        Underline::Double => 2,
        Underline::Curly => 3,
        Underline::Dotted => 4,
        Underline::Dashed => 5,
    }
}

/// A [`Backend`] that writes with crossterm, and the underline kind as `4:n`.
#[derive(Debug)]
pub struct DeckBackend<W: Write> {
    writer: W,
}

impl<W: Write> DeckBackend<W> {
    pub const fn new(writer: W) -> Self {
        Self { writer }
    }

    /// What was written so far.
    #[cfg(test)]
    pub fn writer(&self) -> &W {
        &self.writer
    }
}

impl<W: Write> Write for DeckBackend<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.writer.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

/// Writes what changes between two modifiers: the attributes, without the underline.
fn queue_modifier_diff(w: &mut impl Write, from: Modifier, to: Modifier) -> io::Result<()> {
    let strip = |m: Modifier| m - Modifier::UNDERLINED - KIND_BITS;
    let (from, to) = (strip(from), strip(to));
    let removed = from - to;
    if removed.contains(Modifier::REVERSED) {
        queue!(w, SetAttribute(Attribute::NoReverse))?;
    }
    // Bold and dim are both reset by the normal intensity, so what stays is set again.
    let reset_intensity = removed.intersects(Modifier::BOLD | Modifier::DIM);
    if reset_intensity {
        queue!(w, SetAttribute(Attribute::NormalIntensity))?;
        if to.contains(Modifier::DIM) {
            queue!(w, SetAttribute(Attribute::Dim))?;
        }
        if to.contains(Modifier::BOLD) {
            queue!(w, SetAttribute(Attribute::Bold))?;
        }
    }
    if removed.contains(Modifier::ITALIC) {
        queue!(w, SetAttribute(Attribute::NoItalic))?;
    }
    if removed.contains(Modifier::CROSSED_OUT) {
        queue!(w, SetAttribute(Attribute::NotCrossedOut))?;
    }
    if removed.contains(Modifier::HIDDEN) {
        queue!(w, SetAttribute(Attribute::NoHidden))?;
    }
    let added = to - from;
    if added.contains(Modifier::REVERSED) {
        queue!(w, SetAttribute(Attribute::Reverse))?;
    }
    if added.contains(Modifier::BOLD) && !reset_intensity {
        queue!(w, SetAttribute(Attribute::Bold))?;
    }
    if added.contains(Modifier::ITALIC) {
        queue!(w, SetAttribute(Attribute::Italic))?;
    }
    if added.contains(Modifier::DIM) && !reset_intensity {
        queue!(w, SetAttribute(Attribute::Dim))?;
    }
    if added.contains(Modifier::CROSSED_OUT) {
        queue!(w, SetAttribute(Attribute::CrossedOut))?;
    }
    if added.contains(Modifier::HIDDEN) {
        queue!(w, SetAttribute(Attribute::Hidden))?;
    }
    Ok(())
}

/// Writes the change of underline kind: `24` to remove it, plain `4` for a single underline
/// and `4:n` for the others.
fn queue_underline(w: &mut impl Write, from: Underline, to: Underline) -> io::Result<()> {
    if from == to {
        return Ok(());
    }
    match to {
        Underline::None => queue!(w, SetAttribute(Attribute::NoUnderline)),
        Underline::Single => queue!(w, SetAttribute(Attribute::Underlined)),
        other => queue!(w, Print(format!("\x1b[4:{}m", sub_parameter(other)))),
    }
}

impl<W: Write> Backend for DeckBackend<W> {
    type Error = io::Error;

    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        let mut fg = Color::Reset;
        let mut bg = Color::Reset;
        let mut underline_color = Color::Reset;
        let mut modifier = Modifier::empty();
        let mut last_pos: Option<Position> = None;
        for (x, y, cell) in content {
            // Move the cursor unless the previous cell was just before this one.
            if !matches!(last_pos, Some(p) if x == p.x + 1 && y == p.y) {
                queue!(self.writer, MoveTo(x, y))?;
            }
            last_pos = Some(Position { x, y });
            if cell.modifier != modifier {
                queue_modifier_diff(&mut self.writer, modifier, cell.modifier)?;
                queue_underline(
                    &mut self.writer,
                    kind_bits(modifier),
                    kind_bits(cell.modifier),
                )?;
                modifier = cell.modifier;
            }
            if cell.fg != fg || cell.bg != bg {
                queue!(
                    self.writer,
                    SetColors(Colors::new(
                        cell.fg.into_crossterm(),
                        cell.bg.into_crossterm()
                    ))
                )?;
                fg = cell.fg;
                bg = cell.bg;
            }
            // Only an underlined cell has an underline colour to show.
            let wanted = if kind_bits(cell.modifier) == Underline::None {
                Color::Reset
            } else {
                cell.underline_color
            };
            if wanted != underline_color {
                queue!(self.writer, SetUnderlineColor(wanted.into_crossterm()))?;
                underline_color = wanted;
            }
            queue!(self.writer, Print(cell.symbol()))?;
        }
        queue!(
            self.writer,
            SetForegroundColor(Color::Reset.into_crossterm()),
            SetBackgroundColor(Color::Reset.into_crossterm()),
            SetUnderlineColor(Color::Reset.into_crossterm()),
            SetAttribute(Attribute::Reset),
        )
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        execute!(self.writer, Hide)
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        execute!(self.writer, Show)
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        ratatui::crossterm::cursor::position()
            .map(|(x, y)| Position { x, y })
            .map_err(io::Error::other)
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        let Position { x, y } = position.into();
        execute!(self.writer, MoveTo(x, y))
    }

    fn clear(&mut self) -> io::Result<()> {
        self.clear_region(ClearType::All)
    }

    fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
        execute!(
            self.writer,
            Clear(match clear_type {
                ClearType::All => terminal::ClearType::All,
                ClearType::AfterCursor => terminal::ClearType::FromCursorDown,
                ClearType::BeforeCursor => terminal::ClearType::FromCursorUp,
                ClearType::CurrentLine => terminal::ClearType::CurrentLine,
                ClearType::UntilNewLine => terminal::ClearType::UntilNewLine,
            })
        )
    }

    fn append_lines(&mut self, n: u16) -> io::Result<()> {
        for _ in 0..n {
            queue!(self.writer, Print("\n"))?;
        }
        self.writer.flush()
    }

    fn size(&self) -> io::Result<Size> {
        let (width, height) = terminal::size()?;
        Ok(Size { width, height })
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        let terminal::WindowSize {
            columns,
            rows,
            width,
            height,
        } = terminal::window_size()?;
        Ok(WindowSize {
            columns_rows: Size {
                width: columns,
                height: rows,
            },
            pixels: Size { width, height },
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::Style;

    const KINDS: [Underline; 6] = [
        Underline::None,
        Underline::Single,
        Underline::Double,
        Underline::Curly,
        Underline::Dotted,
        Underline::Dashed,
    ];

    fn output(cells: &[Cell]) -> String {
        let mut b = DeckBackend::new(Vec::new());
        let content: Vec<(u16, u16, &Cell)> = cells
            .iter()
            .enumerate()
            .map(|(i, c)| (u16::try_from(i).unwrap(), 0, c))
            .collect();
        b.draw(content.into_iter()).unwrap();
        String::from_utf8(b.writer().clone()).unwrap()
    }

    fn cell(kind: Underline, color: Color) -> Cell {
        let mut c = Cell::new("x");
        c.modifier = modifier_of(kind);
        c.underline_color = color;
        c
    }

    #[test]
    fn the_kind_bit_is_not_a_ratatui_modifier() {
        assert!(!Modifier::all().intersects(KIND_HIGH));
    }

    #[test]
    fn a_colour_without_an_underline_is_not_written() {
        let out = output(&[cell(Underline::None, Color::Rgb(1, 2, 3))]);
        assert!(!out.contains("58;2;1;2;3"), "{out:?}");
    }

    #[test]
    fn every_kind_survives_the_modifier_bits() {
        for kind in KINDS {
            assert_eq!(kind_bits(modifier_of(kind)), kind);
        }
        let all: Vec<Modifier> = KINDS.iter().map(|k| modifier_of(*k)).collect();
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn a_curly_underline_and_its_colour_are_written() {
        let out = output(&[cell(Underline::Curly, Color::Rgb(255, 0, 0))]);
        assert!(out.contains("\x1b[4:3m"), "{out:?}");
        assert!(out.contains("\x1b[58;2;255;0;0m"), "{out:?}");
        assert!(out.ends_with("\x1b[0m"), "{out:?}");
    }

    #[test]
    fn each_kind_is_written_in_its_own_form() {
        let want = [
            (Underline::Single, "\x1b[4mx"),
            (Underline::Double, "\x1b[4:2mx"),
            (Underline::Curly, "\x1b[4:3mx"),
            (Underline::Dotted, "\x1b[4:4mx"),
            (Underline::Dashed, "\x1b[4:5mx"),
        ];
        for (kind, text) in want {
            let out = output(&[cell(kind, Color::Reset)]);
            assert!(out.contains(text), "{kind:?}: {out:?}");
        }
        let out = output(&[
            cell(Underline::Curly, Color::Indexed(9)),
            cell(Underline::None, Color::Reset),
        ]);
        assert!(out.contains("\x1b[58;5;9m"), "{out:?}");
        assert!(out.contains("\x1b[24m"), "{out:?}");
    }

    #[test]
    fn a_change_of_kind_alone_redraws_the_cell() {
        let area = Rect::new(0, 0, 3, 1);
        let mut old = Buffer::empty(area);
        let mut new = Buffer::empty(area);
        old[(1, 0)].set_symbol("w").set_style(Style::new());
        new[(1, 0)].set_symbol("w").set_style(Style::new());
        old[(1, 0)].modifier = modifier_of(Underline::Single);
        new[(1, 0)].modifier = modifier_of(Underline::Curly);
        let diff = old.diff(&new);
        assert_eq!(diff.len(), 1);
        assert_eq!((diff[0].0, diff[0].1), (1, 0));
        assert!(old.diff(&old.clone()).is_empty());
    }
}
