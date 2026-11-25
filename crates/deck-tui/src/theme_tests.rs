//! The drawing with a custom theme: every role shows up, the border style reaches the
//! overlays, and RGB colours fall back to the palette without truecolor.

use std::path::PathBuf;
use std::time::Instant;

use deck_core::config::Config;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event as ct;
use ratatui::style::Color;

use crate::app::App;
use crate::draw::draw;
use crate::mirror::tests::{frame, mockup};
use crate::modal::Modal;

const THEME: &str = r##"
[theme]
accent = "#cba6f7"
muted = "bright-red"
warning = "#ffff00"
error = "magenta"
border = "cyan"
sidebar_fg = "white"
sidebar_bg = 236
status_fg = "green"
status_bg = 17
selection_fg = "black"
selection_bg = "yellow"
overlay_bg = 18
border_style = "plain"
"##;

fn app(theme: &str) -> App {
    let config = Config::parse(theme).unwrap().config;
    let mut app = App::new(&config, (100, 24), &[])
        .with_dirs(PathBuf::from("/work/proj"), Some(PathBuf::from("/home/me")));
    app.on_server(deck_core::protocol::ServerMsg::SessionView(mockup()));
    let t = app.mirror.focused_id().unwrap();
    let mut f = frame(true, (40, 10), vec![]);
    f.terminal = t;
    app.on_server(deck_core::protocol::ServerMsg::Frame(f));
    app
}

fn render(a: &App) -> Buffer {
    let mut t = Terminal::new(TestBackend::new(a.size.0, a.size.1)).unwrap();
    t.draw(|f| draw(f, a)).unwrap();
    t.backend().buffer().clone()
}

fn row(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
}

/// The cell where `text` starts, searching row by row (every symbol is one cell wide).
fn find(buf: &Buffer, text: &str) -> (u16, u16) {
    (0..buf.area.height)
        .find_map(|y| {
            let r = row(buf, y);
            r.find(text)
                .map(|i| (u16::try_from(r[..i].chars().count()).unwrap(), y))
        })
        .unwrap_or_else(|| panic!("{text:?} is not on the screen"))
}

fn send(a: &mut App, events: &[ct::Event]) {
    let now = Instant::now();
    for e in events {
        a.on_event(e.clone(), now);
    }
}

fn alt_space() -> ct::Event {
    ct::Event::Key(ct::KeyEvent::new(
        ct::KeyCode::Char(' '),
        ct::KeyModifiers::ALT,
    ))
}

#[test]
fn the_sidebar_follows_the_theme() {
    let buf = render(&app(THEME));
    // The session name is in the accent; the active row is a band of the selection colours.
    assert_eq!(buf[(4, 0)].fg, Color::Rgb(0xcb, 0xa6, 0xf7));
    assert_eq!(buf[(0, 3)].symbol(), " ");
    for x in [0, 3, 27] {
        assert_eq!(
            (buf[(x, 3)].fg, buf[(x, 3)].bg),
            (Color::Black, Color::Yellow)
        );
    }
    // The background and text colours, the separator and the collapsed glyph.
    assert_eq!(buf[(10, 5)].bg, Color::Indexed(236));
    assert_eq!(buf[(5, 5)].fg, Color::Gray);
    assert_eq!(buf[(28, 0)].fg, Color::Cyan);
    let (_, y) = find(&buf, "dotfiles");
    assert_eq!(buf[(25, y)].symbol(), "▸");
    assert_eq!(buf[(25, y)].fg, Color::LightRed);
}

#[test]
fn the_floating_bar_follows_the_theme() {
    let mut a = app(THEME);
    // No bar until something is going on.
    assert!(!row(&render(&a), 23).contains("PREFIX"));
    send(&mut a, &[alt_space()]);
    let buf = render(&a);
    let (x, y) = find(&buf, "PREFIX");
    assert_eq!(y, 23);
    assert_eq!(buf[(x, y)].fg, Color::Rgb(255, 255, 0));
    // The padding cell is on the status background, the labels are muted, the keys are in
    // the status foreground.
    assert_eq!(buf[(x - 2, y)].bg, Color::Indexed(17));
    let (x, y) = find(&buf, "cancel");
    assert_eq!(
        (buf[(x, y)].fg, buf[(x, y)].bg),
        (Color::LightRed, Color::Indexed(17))
    );
    let (x, y) = find(&buf, "esc");
    assert_eq!(buf[(x, y)].fg, Color::Green);
    a.flash = Some(("oops".into(), Instant::now()));
    a.flash_error = true;
    let buf = render(&a);
    let (x, y) = find(&buf, "oops");
    assert_eq!(buf[(x, y)].fg, Color::Magenta);
}

#[test]
fn the_selection_uses_the_selection_roles() {
    let mut a = app(THEME);
    send(
        &mut a,
        &[
            alt_space(),
            ct::Event::Key(ct::KeyEvent::new(ct::KeyCode::Tab, ct::KeyModifiers::NONE)),
        ],
    );
    assert!(a.sidebar_focus);
    let buf = render(&a);
    let selected = buf
        .content
        .iter()
        .filter(|c| c.bg == Color::Yellow && c.fg == Color::Black)
        .count();
    assert!(selected >= 10, "{selected}");
    // The active panel's row keeps the selection colour, bold, when it is selected.
    let (x, y) = (0, 3);
    assert_eq!(buf[(x, y)].bg, Color::Yellow);
    assert_eq!(buf[(x + 4, y)].fg, Color::Black);
    assert!(
        buf[(x + 4, y)]
            .modifier
            .contains(ratatui::style::Modifier::BOLD)
    );
    // Both roles at default: reversed.
    let mut a = app("");
    send(
        &mut a,
        &[
            alt_space(),
            ct::Event::Key(ct::KeyEvent::new(ct::KeyCode::Tab, ct::KeyModifiers::NONE)),
        ],
    );
    let buf = render(&a);
    assert!(
        buf.content
            .iter()
            .any(|c| c.modifier.contains(ratatui::style::Modifier::REVERSED))
    );
}

#[test]
fn the_help_overlay_follows_the_theme() {
    for (style, corner) in [
        ("plain", '┌'),
        ("rounded", '╭'),
        ("thick", '┏'),
        ("double", '╔'),
    ] {
        let mut a = app(&THEME.replace("\"plain\"", &format!("\"{style}\"")));
        a.modal = Some(Modal::Help);
        let buf = render(&a);
        let (x, y) = find(&buf, &corner.to_string());
        assert_eq!(buf[(x, y)].fg, Color::Rgb(0xcb, 0xa6, 0xf7), "{style}");
        assert_eq!(buf[(x, y)].bg, Color::Indexed(18), "{style}");
        assert_eq!(buf[(x + 3, y + 3)].bg, Color::Indexed(18), "{style}");
    }
}

#[test]
fn the_picker_follows_the_border_style() {
    let mut a = app("[theme]\nborder_style = \"double\"\noverlay_bg = 18");
    a.open_picker();
    let buf = render(&a);
    let (x, y) = find(&buf, "╔");
    assert_eq!(buf[(x, y)].bg, Color::Indexed(18));
    assert!(row(&buf, y).contains('╗'));
}

#[test]
fn without_truecolor_rgb_becomes_the_nearest_palette_colour() {
    let mut a = app(THEME);
    a.truecolor = false;
    let buf = render(&a);
    assert!(matches!(buf[(4, 0)].fg, Color::Indexed(n) if n >= 16));
    send(&mut a, &[alt_space()]);
    let buf = render(&a);
    let (x, y) = find(&buf, "PREFIX");
    assert!(matches!(buf[(x, y)].fg, Color::Indexed(n) if n >= 16));
    assert!(buf.content.iter().all(|c| !matches!(c.fg, Color::Rgb(..))));
    // Indexed theme colours pass through unchanged.
    assert_eq!(buf[(10, 5)].bg, Color::Indexed(236));
    assert_eq!(buf[(28, 0)].fg, Color::Cyan);
}
