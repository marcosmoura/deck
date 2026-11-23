//! `[theme]`, end to end: the real `deck` in a PTY, read back cell by cell.

mod support;

use alacritty_terminal::vte::ansi::{Color, Rgb};
use support::{Harness, TIMEOUT, wait_until};

const SIZE: (u16, u16) = (100, 24);
const PREFIX: &[u8] = b"\x1b ";
const ACCENT: Color = Color::Spec(Rgb {
    r: 0xcb,
    g: 0xa6,
    b: 0xf7,
});
/// The Screen config of the spec.
const THEMED: &str = r##"{ accent = "#cba6f7", border_style = "plain", sidebar_bg = 236 }"##;

/// `deck` with the sidebar and the given `theme` table, once its first prompt is up.
fn themed(theme: &str, env: &[(&str, Option<&str>)]) -> Harness {
    let mut builder = Harness::builder(SIZE)
        .config_unset("sidebar")
        .config("theme", theme);
    for (key, value) in env {
        builder = match value {
            Some(v) => builder.env(key, v),
            None => builder.env_remove(key),
        };
    }
    let h = builder.deck();
    h.wait_for_text("│$", TIMEOUT);
    h
}

fn prefixed(h: &mut Harness, key: &str) {
    h.send_bytes(PREFIX);
    h.send_keys(key);
}

fn rows(h: &Harness) -> Vec<String> {
    h.screen_text().lines().map(str::to_string).collect()
}

/// The first cell showing `ch`, as (row, col) in the sidebar-or-overlay text rows.
#[expect(clippy::panic, reason = "a missing glyph fails the integration test")]
fn find_char(h: &Harness, ch: char) -> (usize, usize) {
    rows(h)
        .iter()
        .enumerate()
        .find_map(|(y, r)| r.chars().position(|c| c == ch).map(|x| (y, x)))
        .unwrap_or_else(|| panic!("{ch:?} is not on the screen:\n{}", h.screen_text()))
}

fn is_palette(color: Color) -> bool {
    matches!(color, Color::Indexed(n) if n >= 16)
}

#[test]
fn the_screen_config_colours_the_sidebar_and_the_overlays() {
    let mut h = themed(THEMED, &[]);
    h.wait_for_text("▌  ❯  sh", TIMEOUT);
    // The active panel row and the session name.
    assert_eq!(h.cell(3, 0).ch, '▌');
    assert_eq!(h.cell(3, 0).fg, ACCENT);
    assert_eq!(h.cell(3, 3).fg, ACCENT);
    assert_eq!(h.cell(23, 1).fg, ACCENT, "the session name");
    assert_eq!(h.cell(0, 3).fg, ACCENT, "the sidebar header");
    // The sidebar's background, on empty cells of its rows.
    for row in [1, 8, 15] {
        assert_eq!(h.cell(row, 20).bg, Color::Indexed(236), "row {row}");
    }
    // The terminal area is not painted by the theme.
    assert_ne!(h.cell(8, 60).bg, Color::Indexed(236));

    prefixed(&mut h, "?");
    // At 24 rows the last row is scrolled out of view: End brings it.
    h.send_bytes(b"\x1b[F");
    h.wait_for_text("This help", TIMEOUT);
    let (row, col) = find_char(&h, '┌');
    assert_eq!(h.cell(row, col).fg, ACCENT);
    assert!(rows(&h)[row].contains("┌ Help"), "{}", rows(&h)[row]);
    assert!(!h.screen_text().contains('╭'));
    h.capture("themed");
}

#[test]
fn without_truecolor_the_theme_and_the_programs_use_the_palette() {
    let mut h = themed(THEMED, &[("COLORTERM", None)]);
    h.wait_for_text("▌  ❯  sh", TIMEOUT);
    assert!(is_palette(h.cell(3, 0).fg), "{:?}", h.cell(3, 0).fg);
    assert!(is_palette(h.cell(23, 1).fg));
    assert_eq!(h.cell(1, 20).bg, Color::Indexed(236));

    h.send_keys("printf '\\033[38;2;203;166;247mQ\\033[0m\\n'\r");
    wait_until("the coloured Q", TIMEOUT, || {
        (0..24).any(|y| (29..100).any(|x| h.cell(y, x).ch == 'Q' && is_palette(h.cell(y, x).fg)))
    });
    assert!(
        !(0..24).any(|y| (0..100).any(|x| h.cell(y, x).fg == ACCENT)),
        "no truecolor reaches the outer terminal"
    );
    h.capture("no-truecolor");
}

#[test]
fn with_truecolor_programs_keep_their_rgb_colours() {
    let mut h = themed(THEMED, &[("COLORTERM", Some("truecolor"))]);
    h.send_keys("printf '\\033[38;2;203;166;247mQ\\033[0m\\n'\r");
    wait_until("the coloured Q", TIMEOUT, || {
        (0..24).any(|y| (29..100).any(|x| h.cell(y, x).ch == 'Q' && h.cell(y, x).fg == ACCENT))
    });
}

#[test]
fn a_double_border_style_reaches_the_help_overlay_and_the_session_picker() {
    let mut h = themed(r#"{ border_style = "double" }"#, &[]);
    prefixed(&mut h, "?");
    // At 24 rows the last row is scrolled out of view: End brings it.
    h.send_bytes(b"\x1b[F");
    h.wait_for_text("This help", TIMEOUT);
    // The last text row can be on screen before the bottom border.
    wait_until("the help box's bottom border", TIMEOUT, || {
        h.screen_text().contains('╝')
    });
    let screen = h.screen_text();
    assert!(screen.contains("╔ Help ═"), "{screen}");
    assert!(screen.contains('╗') && screen.contains('╚') && screen.contains('╝'));
    assert!(!screen.contains('╭'));
    h.capture("double-help");
    h.send_keys("\x1b");
    wait_until("the help to close", TIMEOUT, || {
        !h.screen_text().contains("This help")
    });

    prefixed(&mut h, "s");
    h.wait_for_text("Sessions", TIMEOUT);
    wait_until("the picker's bottom border", TIMEOUT, || {
        h.screen_text().contains('╝')
    });
    let screen = h.screen_text();
    assert!(screen.contains('╔') && screen.contains('╗'), "{screen}");
    assert!(
        screen.contains("╔─ Sessions") || screen.contains("╔═ Sessions"),
        "{screen}"
    );
    assert!(!screen.contains('╭'));
    h.capture("double-picker");
}

#[test]
fn glyphs_follow_the_theme() {
    let mut h = themed(
        r#"{ glyphs = { expanded = "v", collapsed = ">", active = "|" } }"#,
        &[],
    );
    h.wait_for_text("|  ❯  sh", TIMEOUT);
    assert!(rows(&h)[2].starts_with(" v dk"), "{}", rows(&h)[2]);
    h.capture("glyphs");
    prefixed(&mut h, "\t");
    h.wait_for_text("SIDEBAR", TIMEOUT);
    h.send_keys("h");
    wait_until("the collapsed marker", TIMEOUT, || {
        rows(&h).iter().any(|r| r.starts_with(" > dk"))
    });
}
