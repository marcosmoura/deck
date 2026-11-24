//! The outer terminal: raw mode, the alternate screen and the input modes, and the guard
//! that puts everything back on exit, error, panic and the signals.

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use ratatui::crossterm::cursor::{SetCursorStyle, Show};
use ratatui::crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, EnableMouseCapture,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

use crate::colors::{self, Probe, ReadInput};

/// Set while the terminal is in deck's mode, so that restoring happens once.
static ACTIVE: AtomicBool = AtomicBool::new(false);

/// The kitty keyboard flags deck has pushed on the outer terminal; 0 when none are.
static KEYBOARD: AtomicU8 = AtomicU8::new(0);

/// The bytes that take the outer terminal's keyboard flags from `from` to `to` (0 for none
/// pushed): a pop, then a push.
pub fn keyboard_change(from: u8, to: u8) -> Vec<u8> {
    let mut bytes = Vec::new();
    if from == to {
        return bytes;
    }
    if from != 0 {
        bytes.extend_from_slice(b"\x1b[<u");
    }
    if to != 0 {
        bytes.extend_from_slice(format!("\x1b[>{to}u").as_bytes());
    }
    bytes
}

/// Makes `flags` the kitty keyboard flags deck has pushed on the outer terminal (0: none).
pub fn set_keyboard(flags: u8) {
    let before = KEYBOARD.swap(flags, Ordering::SeqCst);
    let bytes = keyboard_change(before, flags);
    if !bytes.is_empty() {
        let mut out = io::stdout();
        let _ = out.write_all(&bytes);
        let _ = out.flush();
    }
}

/// Puts the outer terminal back: the modes off, the cursor shown and in its default shape,
/// the main screen, cooked mode. Safe to call twice.
pub fn restore() {
    if !ACTIVE.swap(false, Ordering::SeqCst) {
        return;
    }
    let mut out = io::stdout();
    // The flags were pushed on the alternate screen, which has its own stack.
    if KEYBOARD.swap(0, Ordering::SeqCst) != 0 {
        let _ = out.write_all(b"\x1b[<u");
    }
    let _ = execute!(
        out,
        DisableMouseCapture,
        DisableBracketedPaste,
        DisableFocusChange,
        SetCursorStyle::DefaultUserShape,
        LeaveAlternateScreen,
        Show
    );
    let _ = out.flush();
    let _ = disable_raw_mode();
}

/// Restores the terminal before the panic message is printed.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        previous(info);
    }));
}

/// Holds the terminal in deck's mode; dropping it restores the terminal.
#[derive(Debug)]
pub struct Guard(());

impl Guard {
    /// Raw mode, the colour query, the alternate screen and the input modes. The query
    /// comes before the alternate screen, as the spec says.
    pub fn enter(read: ReadInput<'_>) -> io::Result<(Self, Probe)> {
        install_panic_hook();
        ACTIVE.store(true, Ordering::SeqCst);
        let guard = Self(());
        enable_raw_mode()?;
        let mut out = io::stdout();
        out.write_all(colors::QUERY)?;
        out.flush()?;
        let probe = colors::probe(read, colors::ANSWER_TIMEOUT);
        execute!(
            out,
            EnterAlternateScreen,
            EnableMouseCapture,
            EnableBracketedPaste,
            EnableFocusChange
        )?;
        Ok((guard, probe))
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        restore();
    }
}

#[cfg(test)]
mod tests {
    use super::keyboard_change;

    #[test]
    fn the_keyboard_flags_are_popped_before_new_ones_are_pushed() {
        assert_eq!(keyboard_change(0, 0), b"");
        assert_eq!(keyboard_change(3, 3), b"");
        assert_eq!(keyboard_change(0, 1), b"\x1b[>1u");
        assert_eq!(keyboard_change(1, 0), b"\x1b[<u");
        assert_eq!(keyboard_change(1, 7), b"\x1b[<u\x1b[>7u");
    }
}
