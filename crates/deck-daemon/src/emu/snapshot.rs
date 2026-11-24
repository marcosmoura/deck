//! `Emu::snapshot`: the state of a terminal as escape sequences, for the live upgrade.
//!
//! The text of the screens comes from `libghostty-vt`'s VT formatter; this module adds what
//! the formatter leaves out (the alternate screen next to the primary one, cursor shape and
//! visibility, the modes of `protocol::Modes`, autowrap, insert and origin modes, the title)
//! and keeps out what would make the emulator answer (queries) or hold output (`?2026`).
//! The saved cursor (`DECSC`) is not replayed: the emulator does not expose it.

use ghostty_vt::fmt::{Format, Formatter, FormatterOptions};
use ghostty_vt::render::{CursorVisualStyle, RenderState};
use ghostty_vt::screen::Screen;
use ghostty_vt::snapshot as binary;
use ghostty_vt::terminal::Mode;
use ghostty_vt::{Error, Terminal};

use super::Emu;

/// The modes replayed with `CSI ? n h` or `l`, with the state a fresh emulator starts in
/// (`true`: on). Only those that differ from it are written.
const DEC_MODES: [(Mode, u16, bool); 9] = [
    (Mode::DECCKM, 1, false),
    (Mode::WRAPAROUND, 7, true),
    (Mode::BRACKETED_PASTE, 2004, false),
    (Mode::NORMAL_MOUSE, 1000, false),
    (Mode::BUTTON_MOUSE, 1002, false),
    (Mode::ANY_MOUSE, 1003, false),
    (Mode::SGR_MOUSE, 1006, false),
    (Mode::FOCUS_EVENT, 1004, false),
    (Mode::ALT_SCROLL, 1007, true),
];

impl Emu {
    /// Bytes that, fed into a fresh `Emu` of the same size and scrollback limit, reproduce
    /// this terminal: the screens and history, cursor, modes and title. Never a query, so
    /// feeding it raises no reply. A program inside synchronized output (`?2026`) is
    /// snapshotted as if the output had ended; the caller waits for the end or the timeout
    /// first when it wants the finished frame.
    pub fn snapshot(&self) -> Vec<u8> {
        match self.try_snapshot() {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::warn!("cannot snapshot the emulator: {e}");
                Vec::new()
            }
        }
    }

    fn try_snapshot(&self) -> Result<Vec<u8>, Error> {
        let term = &self.term;
        let mut out = Vec::new();
        let alternate = term.active_screen()? == Screen::Alternate;
        if alternate {
            // The primary screen is not the active one: a copy of the terminal leaves the
            // alternate screen (which puts the primary cursor back) and the copy is read.
            let mut copy = binary::Decoder::from_slice(&binary::encode(term)?).decode()?;
            copy.set_mode(Mode::SYNC_OUTPUT, false)?;
            copy.vt_write(b"\x1b[?1049l");
            screen(&copy, &mut out)?;
            // The alternate screen starts blank, with the cursor where the primary one is:
            // its text is written from the top left corner.
            out.extend_from_slice(b"\x1b[?1049h\x1b[0m\x1b[H");
        }
        let start = out.len();
        screen(term, &mut out)?;
        if term.mode(Mode::ORIGIN)? {
            // `?6h` homes the cursor, so it comes after the screen, and the cursor is put
            // back relative to the top of the scroll region the screen set.
            let top = out.get(start..).map_or(1, region_top);
            let row =
                usize::from(term.cursor_y()?) + 2 - top.min(usize::from(term.cursor_y()?) + 1);
            out.extend_from_slice(
                format!("\x1b[?6h\x1b[{row};{}H", u32::from(term.cursor_x()?) + 1).as_bytes(),
            );
        }
        out.extend_from_slice(&self.settings()?);
        Ok(out)
    }

    /// Cursor shape and visibility, modes and title: what is set after the screens.
    fn settings(&self) -> Result<Vec<u8>, Error> {
        let term = &self.term;
        let mut out = Vec::new();
        let mut render = RenderState::new()?;
        let cursor = render.update(term)?.cursor()?;
        let blinking = cursor.blinking;
        // `DECSCUSR`: no shape was chosen (hollow block, the default) writes nothing.
        let shape = match cursor.visual_style {
            CursorVisualStyle::Block => Some(if blinking { 1 } else { 2 }),
            CursorVisualStyle::Underline => Some(if blinking { 3 } else { 4 }),
            CursorVisualStyle::Bar => Some(if blinking { 5 } else { 6 }),
            _ => None,
        };
        if let Some(n) = shape {
            out.extend_from_slice(format!("\x1b[{n} q").as_bytes());
        }
        for (mode, n, default) in DEC_MODES {
            let on = term.mode(mode)?;
            if on != default {
                out.extend_from_slice(
                    format!("\x1b[?{n}{}", if on { 'h' } else { 'l' }).as_bytes(),
                );
            }
        }
        if term.mode(Mode::INSERT)? {
            out.extend_from_slice(b"\x1b[4h");
        }
        if !term.is_cursor_visible()? {
            out.extend_from_slice(b"\x1b[?25l");
        }
        let title = term.title()?;
        if !title.is_empty() {
            // Control characters would end the sequence early.
            let clean: String = title.chars().filter(|c| !c.is_control()).collect();
            out.extend_from_slice(format!("\x1b]2;{clean}\x1b\\").as_bytes());
        }
        Ok(out)
    }
}

/// The active screen of `term` as VT: history, rows, style, scrolling region and cursor.
/// Soft-wrapped lines are written as one line, so the copy wraps them again.
fn screen(term: &Terminal, out: &mut Vec<u8>) -> Result<(), Error> {
    let options = FormatterOptions::new()
        .with_format(Format::Vt)
        .with_unwrap(true)
        .with_cursor(true)
        .with_style(true)
        .with_scrolling_region(true);
    let mut formatter = Formatter::new(term, options)?;
    out.extend_from_slice(&formatter.format_alloc()?);
    Ok(())
}

#[cfg(test)]
mod tests;

/// The first row (counting from 1) of the last `CSI top ; bottom r` in `vt`, 1 without one.
fn region_top(vt: &[u8]) -> usize {
    let mut top = 1;
    let mut rest = vt;
    while let Some(at) = rest.windows(2).position(|w| w == b"\x1b[") {
        rest = rest.get(at + 2..).unwrap_or_default();
        let params = rest
            .iter()
            .take_while(|b| b.is_ascii_digit() || **b == b';')
            .count();
        if rest.get(params) == Some(&b'r') {
            let first = rest.get(..params).unwrap_or_default();
            let first = first.split(|&b| b == b';').next().unwrap_or_default();
            top = std::str::from_utf8(first)
                .ok()
                .and_then(|n| n.parse().ok())
                .filter(|&n| n > 0)
                .unwrap_or(1);
        }
    }
    top
}
