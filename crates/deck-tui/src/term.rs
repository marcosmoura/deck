//! The outer terminal: raw mode, the alternate screen and the input modes, and the guard
//! that puts everything back on exit, error, panic and the signals.

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};

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

/// Puts the outer terminal back: the modes off, the cursor shown and in its default shape,
/// the main screen, cooked mode. Safe to call twice.
pub fn restore() {
    if !ACTIVE.swap(false, Ordering::SeqCst) {
        return;
    }
    let mut out = io::stdout();
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
