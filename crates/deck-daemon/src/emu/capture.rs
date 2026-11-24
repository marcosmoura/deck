//! A terminal's screen as text: `capture` and the line search of `wait --text`.

use deck_core::model::TerminalId;
use deck_core::protocol::{Captured, Color, Modes, Style};
use ghostty_vt::Error;
use ghostty_vt::screen::{CellWide, GridRef, Screen};
use ghostty_vt::terminal::{Point, PointCoordinate};

use super::{Emu, Glyph, glyph, modes_of};

/// A capture holds at most this many lines.
pub const MAX_LINES: usize = 10_000;
/// A capture holds at most this many bytes of text, so its answer fits one protocol frame.
pub const MAX_BYTES: usize = 8 * 1024 * 1024;

impl Emu {
    /// The modes programs set that decide how keys are encoded.
    pub fn modes(&self) -> Modes {
        modes_of(&self.term)
    }

    /// The point of a cell, `line` rows from the top of the live screen (negative lines are
    /// the history); `history` is the emulator's `scrollback_rows`, read once by the caller.
    fn point_at(col: u16, line: i32, history: usize) -> Point {
        if line >= 0 {
            Point::Active(PointCoordinate {
                x: col,
                y: u32::try_from(line).unwrap_or(0),
            })
        } else {
            let y = i64::try_from(history).unwrap_or(0) + i64::from(line);
            Point::History(PointCoordinate {
                x: col,
                y: u32::try_from(y).unwrap_or(0),
            })
        }
    }

    /// One cell of the grid, `line` rows from the top of the live screen (negative lines are
    /// the history).
    fn glyph_at(&self, col: u16, line: i32) -> Glyph {
        let history = self.term.scrollback_rows().unwrap_or(0);
        self.term
            .grid_ref(Self::point_at(col, line, history))
            .and_then(|at| read_glyph(&at, true))
            .unwrap_or_default()
    }

    /// Every cell of a row (without styles), `line` as in [`Self::glyph_at`], and whether the
    /// row is soft-wrapped (continues on the next one). `libghostty-vt` 0.9.0 reads one cell
    /// per lookup, so this is one lookup per column; `history` is read once by the caller.
    pub(super) fn row_glyphs(&self, line: i32, history: usize) -> (Vec<Glyph>, bool) {
        let cols = self.size().0;
        let wrapped = self
            .term
            .grid_ref(Self::point_at(0, line, history))
            .and_then(|at| at.row())
            .and_then(|row| row.is_wrapped())
            .unwrap_or(false);
        let glyphs = (0..cols)
            .map(|col| {
                self.term
                    .grid_ref(Self::point_at(col, line, history))
                    .and_then(|at| read_glyph(&at, false))
                    .unwrap_or_default()
            })
            .collect();
        (glyphs, wrapped)
    }

    /// One row of the grid as text; negative lines are the history. A wide character is
    /// printed once, combining marks stay attached to their character, trailing blanks are
    /// left out. With `ansi` colours and attributes come as SGR sequences.
    fn row_text(&self, line: i32, ansi: bool) -> String {
        let cols = self.size().0;
        let glyphs: Vec<Glyph> = (0..cols).map(|col| self.glyph_at(col, line)).collect();
        let end = glyphs
            .iter()
            .rposition(|g| {
                if ansi {
                    !g.is_blank()
                } else {
                    !matches!(g.text.as_str(), "" | " ")
                }
            })
            .map_or(0, |i| i + 1);
        let mut out = String::new();
        let mut current = Style::default();
        for glyph in glyphs.iter().take(end) {
            if glyph.spacer {
                continue;
            }
            if ansi && glyph.style != current {
                sgr(&mut out, glyph.style);
                current = glyph.style;
            }
            out.push_str(if glyph.text.is_empty() {
                " "
            } else {
                &glyph.text
            });
        }
        if current != Style::default() {
            out.push_str("\x1b[0m");
        }
        out
    }

    /// The first row of the visible screen that contains `needle`.
    pub fn find_line(&self, needle: &str) -> Option<String> {
        let rows = i32::from(self.size().1);
        (0..rows)
            .map(|row| self.row_text(row, false))
            .find(|text| text.contains(needle))
    }

    /// The screen as text, with the history when asked (never on the alternate screen, which
    /// has none): trailing empty lines dropped, then the last `lines` kept, then the size cap
    /// applied (which counts what it leaves out in `truncated`).
    pub fn capture(
        &self,
        terminal: TerminalId,
        lines: Option<u32>,
        history: bool,
        ansi: bool,
    ) -> Captured {
        let alternate = self.term.active_screen() == Ok(Screen::Alternate);
        let rows = i32::from(self.size().1);
        let earlier = if history && !alternate {
            i32::try_from(self.history()).unwrap_or(0)
        } else {
            0
        };
        let limit = lines.map_or(usize::MAX, |n| usize::try_from(n).unwrap_or(usize::MAX));

        // Walk up from the bottom: the empty lines there are dropped, then lines are taken
        // until the wanted count or the cap.
        let mut line = rows - 1;
        let mut text = None;
        while line >= -earlier {
            let row = self.row_text(line, ansi);
            if !row.is_empty() {
                text = Some(row);
                break;
            }
            line -= 1;
        }
        let total = usize::try_from(line + earlier + 1).unwrap_or(0);
        let mut taken: Vec<String> = Vec::new();
        let mut bytes = 0usize;
        let mut capped = false;
        while let Some(row) = text.take() {
            if taken.len() >= limit {
                break;
            }
            if taken.len() >= MAX_LINES || bytes + row.len() + 1 > MAX_BYTES {
                capped = true;
                break;
            }
            bytes += row.len() + 1;
            taken.push(row);
            line -= 1;
            if line >= -earlier {
                text = Some(self.row_text(line, ansi));
            }
        }
        let truncated = if capped {
            u32::try_from(total.min(limit) - taken.len()).unwrap_or(u32::MAX)
        } else {
            0
        };
        taken.reverse();

        let (cols, screen) = self.size();
        Captured {
            terminal,
            size: (cols, screen),
            cursor: (
                self.term
                    .cursor_y()
                    .unwrap_or(0)
                    .min(screen.saturating_sub(1)),
                self.term
                    .cursor_x()
                    .unwrap_or(0)
                    .min(cols.saturating_sub(1)),
            ),
            alternate_screen: alternate,
            truncated,
            lines: taken,
        }
    }
}

/// The cell a grid reference points at; with `styled` also its style (a search needs only
/// the text).
fn read_glyph(at: &GridRef<'_>, styled: bool) -> Result<Glyph, Error> {
    let raw = at.cell()?;
    let mut text = String::new();
    if raw.has_text().unwrap_or(false) {
        let mut buf = ['\0'; 8];
        let len = match at.graphemes(&mut buf) {
            Err(Error::OutOfSpace { required }) => {
                let mut big = vec!['\0'; required];
                let len = at.graphemes(&mut big)?;
                text.extend(big.iter().take(len));
                0
            }
            other => other?,
        };
        text.extend(buf.iter().take(len));
    }
    if !styled {
        let wide = raw.wide();
        return Ok(Glyph {
            text,
            style: Style::default(),
            spacer: matches!(wide, Ok(CellWide::SpacerTail | CellWide::SpacerHead)),
            wide: matches!(wide, Ok(CellWide::Wide)),
        });
    }
    let style = if raw.has_styling().unwrap_or(false) {
        at.style().ok()
    } else {
        None
    };
    Ok(glyph(raw, style, text))
}

/// The SGR sequence that sets exactly `style`.
fn sgr(out: &mut String, style: Style) {
    out.push_str("\x1b[0");
    for (flag, code) in [
        (Style::BOLD, 1),
        (Style::DIM, 2),
        (Style::ITALIC, 3),
        (Style::UNDERLINE, 4),
        (Style::REVERSE, 7),
        (Style::HIDDEN, 8),
        (Style::STRIKETHROUGH, 9),
    ] {
        if style.has(flag) {
            out.push_str(&format!(";{code}"));
        }
    }
    sgr_color(out, style.fg, 30, 90, 38);
    sgr_color(out, style.bg, 40, 100, 48);
    out.push('m');
}

/// One colour: the 8 and 8 bright colours by their own codes, the rest after `extended`.
fn sgr_color(out: &mut String, color: Color, base: u8, bright: u8, extended: u8) {
    match color {
        Color::Default => {}
        Color::Indexed(n @ 0..=7) => out.push_str(&format!(";{}", base + n)),
        Color::Indexed(n @ 8..=15) => out.push_str(&format!(";{}", bright + n - 8)),
        Color::Indexed(n) => out.push_str(&format!(";{extended};5;{n}")),
        Color::Rgb(r, g, b) => out.push_str(&format!(";{extended};2;{r};{g};{b}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new(cols: u16, rows: u16, scrollback: usize) -> Emu {
        Emu::new(cols, rows, scrollback).expect("emulator")
    }

    fn capture(e: &Emu, lines: Option<u32>, history: bool, ansi: bool) -> Captured {
        e.capture(TerminalId(1), lines, history, ansi)
    }

    #[test]
    fn the_screen_is_text_with_trailing_blanks_and_empty_lines_dropped() {
        let mut e = new(20, 6, 100);
        e.feed(b"one   \r\n\r\ntwo\x1b[5;1H");
        let c = capture(&e, None, false, false);
        assert_eq!(c.lines, ["one", "", "two"]);
        assert_eq!((c.size, c.cursor, c.truncated), ((20, 6), (4, 0), 0));
        assert!(!c.alternate_screen);
        assert_eq!(capture(&e, Some(2), false, false).lines, ["", "two"]);
        assert!(capture(&new(10, 3, 0), None, false, false).lines.is_empty());
        assert_eq!(e.find_line("tw").as_deref(), Some("two"));
        assert_eq!(e.find_line("three"), None);
    }

    #[test]
    fn wide_and_combining_characters_are_printed_once() {
        let mut e = new(20, 3, 0);
        e.feed("日本 🙂 e\u{301}x".as_bytes());
        assert_eq!(capture(&e, None, false, false).lines, ["日本 🙂 e\u{301}x"]);
    }

    #[test]
    fn history_is_prepended_but_not_on_the_alternate_screen() {
        let mut e = new(10, 4, 100);
        for n in 1..=10 {
            e.feed(format!("{n}\r\n").as_bytes());
        }
        e.feed(b"$");
        let screen = capture(&e, None, false, false).lines;
        assert_eq!(screen, ["8", "9", "10", "$"]);
        let all = capture(&e, None, true, false).lines;
        assert_eq!(all.len(), 11);
        assert_eq!(all.first().map(String::as_str), Some("1"));
        assert_eq!(capture(&e, Some(3), true, false).lines, ["9", "10", "$"]);
        e.feed(b"\x1b[?1049h\x1b[Hless");
        let alt = capture(&e, None, true, false);
        assert!(alt.alternate_screen);
        assert_eq!(alt.lines, ["less"]);
    }

    #[test]
    fn ansi_keeps_colours_and_attributes() {
        let mut e = new(20, 2, 0);
        e.feed(b"a\x1b[1;31mb\x1b[0;38;2;1;2;3mc\x1b[0m d\x1b[44m  ");
        let c = capture(&e, None, false, true);
        assert_eq!(
            c.lines,
            ["a\x1b[0;1;31mb\x1b[0;38;2;1;2;3mc\x1b[0m d\x1b[0;44m  \x1b[0m"]
        );
        assert_eq!(capture(&e, None, false, false).lines, ["abc d"]);
    }

    #[test]
    fn the_line_cap_counts_what_it_leaves_out() {
        let mut e = new(10, 4, 30_000);
        for n in 1..=12_000 {
            e.feed(format!("{n}\r\n").as_bytes());
        }
        let c = capture(&e, None, true, false);
        assert_eq!(c.lines.len(), MAX_LINES);
        // 12000 numbered lines and the empty line the cursor is on, which is dropped.
        assert_eq!(c.truncated, 2_000);
        assert_eq!(c.lines.last().map(String::as_str), Some("12000"));
        assert_eq!(c.lines.first().map(String::as_str), Some("2001"));
        // Asking for fewer lines is not a truncation.
        let few = capture(&e, Some(5), true, false);
        assert_eq!((few.lines.len(), few.truncated), (5, 0));
    }

    #[test]
    fn the_byte_cap_leaves_a_whole_number_of_lines() {
        // The scrollback budget is sized in bytes for 250 columns (sub-spec 03), so a
        // 1000-column terminal needs more lines to pass 8 MiB.
        let mut e = new(1000, 2, 40_000);
        let line = "x".repeat(1000);
        for _ in 0..(MAX_BYTES / 1001 + 50) {
            e.feed(format!("{line}\r\n").as_bytes());
        }
        let c = capture(&e, None, true, false);
        let bytes: usize = c.lines.iter().map(|l| l.len() + 1).sum();
        assert!(bytes <= MAX_BYTES && c.truncated > 0, "{bytes}");
        assert_eq!(c.lines.len(), MAX_BYTES / 1001);
    }
}
