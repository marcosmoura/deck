//! Smoke test: `ghostty-vt` builds, links and runs.

use ghostty_vt::Terminal;
use ghostty_vt::style::{PaletteIndex, StyleColor};
use ghostty_vt::terminal::{Options, Point, PointCoordinate};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn text_at(terminal: &Terminal, x: u16) -> Result<String, Box<dyn std::error::Error>> {
    let grid_ref = terminal.grid_ref(Point::Active(PointCoordinate { x, y: 0 }))?;
    let mut buf = ['\0'; 8];
    let len = grid_ref.graphemes(&mut buf)?;
    Ok(buf.iter().take(len).collect())
}

#[test]
fn ghostty_vt_parses_text_and_colour() -> TestResult {
    let mut terminal = Terminal::new(Options {
        cols: 20,
        rows: 5,
        max_scrollback: 100,
    })?;
    terminal.vt_write(b"hi\x1b[31mX");

    assert_eq!(text_at(&terminal, 0)?, "h");
    assert_eq!(text_at(&terminal, 1)?, "i");
    assert_eq!(text_at(&terminal, 2)?, "X");

    let style = terminal
        .grid_ref(Point::Active(PointCoordinate { x: 2, y: 0 }))?
        .style()?;
    assert_eq!(style.fg_color, StyleColor::Palette(PaletteIndex::RED));
    Ok(())
}
