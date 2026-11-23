//! Applying the daemon's config while the client runs, and the notices in the floating bar.

use std::time::{Duration, Instant};

use deck_core::config::Config;
use deck_core::protocol::{ClientMsg, NoticeLevel, ServerMsg};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event as ct;
use ratatui::style::Color;

use crate::app::{App, Out};
use crate::draw::draw;
use crate::mirror::tests::{frame, mockup};

fn parse(text: &str) -> Config {
    Config::parse(text).unwrap().config
}

fn app() -> App {
    let mut app = App::new(&Config::default(), (100, 24), &[]);
    app.on_server(ServerMsg::SessionView(mockup()));
    let t = app.mirror.focused_id().unwrap();
    let mut f = frame(true, (40, 10), vec![]);
    f.terminal = t;
    app.on_server(ServerMsg::Frame(f));
    app
}

fn key(code: ct::KeyCode, mods: ct::KeyModifiers) -> ct::Event {
    ct::Event::Key(ct::KeyEvent::new(code, mods))
}

fn ch(c: char) -> ct::Event {
    key(ct::KeyCode::Char(c), ct::KeyModifiers::NONE)
}

fn prefix() -> ct::Event {
    key(ct::KeyCode::Char(' '), ct::KeyModifiers::ALT)
}

fn render(a: &App) -> Buffer {
    let mut t = Terminal::new(TestBackend::new(a.size.0, a.size.1)).unwrap();
    t.draw(|f| draw(f, a)).unwrap();
    t.backend().buffer().clone()
}

fn row(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
}

/// What a reload sends: the new size, without claiming the terminals.
fn viewport(cols: u16, rows: u16) -> Out {
    Out::Send(ClientMsg::ReloadViewport { cols, rows })
}

fn notice(level: NoticeLevel, text: &str) -> ServerMsg {
    ServerMsg::Notice {
        level,
        text: text.into(),
    }
}

#[test]
fn a_new_theme_applies_at_once() {
    let mut a = app();
    let before = render(&a);
    let accent = before[(1, 0)].fg;
    let outs = a.on_server(ServerMsg::Config(Box::new(parse(
        "[theme]\naccent = \"#ff0000\"",
    ))));
    assert!(outs.is_empty(), "a theme changes no size: {outs:?}");
    let after = render(&a);
    assert_ne!(after[(1, 0)].fg, accent);
    assert_eq!(after[(1, 0)].fg, Color::Rgb(255, 0, 0));
}

#[test]
fn new_bindings_and_prefix_apply_and_an_armed_prefix_is_kept() {
    let mut a = app();
    let now = Instant::now();
    a.on_event(prefix(), now);
    assert!(a.armed);
    a.on_server(ServerMsg::Config(Box::new(parse("[keys]\ndetach = \"q\""))));
    assert!(a.armed, "the prefix stays armed");
    // `d` no longer detaches; `q` does.
    assert!(a.on_event(ch('d'), now).is_empty());
    assert_eq!(a.flash.as_ref().unwrap().0, "no binding for d");
    a.on_event(prefix(), now);
    let outs = a.on_event(ch('q'), now);
    assert!(outs.contains(&Out::Send(ClientMsg::Detach)), "{outs:?}");

    // A new prefix: the old one is plain input again.
    let mut a = app();
    a.on_server(ServerMsg::Config(Box::new(parse("prefix = \"C-a\""))));
    assert!(
        a.on_event(prefix(), now)
            .iter()
            .all(|o| matches!(o, Out::Send(ClientMsg::Input { .. })))
    );
    assert!(!a.armed);
    a.on_event(key(ct::KeyCode::Char('a'), ct::KeyModifiers::CONTROL), now);
    assert!(a.armed);
}

#[test]
fn a_new_sidebar_width_resizes_and_sends_the_viewport() {
    let mut a = app();
    assert_eq!(a.viewport(), (71, 24));
    let outs = a.on_server(ServerMsg::Config(Box::new(parse("sidebar_width = 40"))));
    assert_eq!(outs, [viewport(59, 24)]);
    assert_eq!(a.sidebar_width, 40);
    let buf = render(&a);
    assert_eq!(buf[(40, 5)].symbol(), "│", "the separator moved");
}

#[test]
fn a_sidebar_the_user_hid_stays_hidden_unless_the_setting_changes() {
    let mut a = app();
    let now = Instant::now();
    a.on_event(prefix(), now);
    a.on_event(ch('b'), now);
    assert!(a.sidebar_hidden);
    // The same `sidebar`, a new width: still hidden.
    let outs = a.on_server(ServerMsg::Config(Box::new(parse("sidebar_width = 40"))));
    assert!(a.sidebar_hidden);
    assert!(outs.is_empty(), "hidden before and after: {outs:?}");
    // The same config again changes nothing.
    assert!(
        a.on_server(ServerMsg::Config(Box::new(parse("sidebar_width = 40"))))
            .is_empty()
    );
    // `sidebar` itself changes: it follows.
    a.on_server(ServerMsg::Config(Box::new(parse(
        "sidebar = false\nsidebar_width = 40",
    ))));
    assert!(a.sidebar_hidden);
    let outs = a.on_server(ServerMsg::Config(Box::new(parse("sidebar_width = 40"))));
    assert!(!a.sidebar_hidden);
    assert_eq!(outs, [viewport(59, 24)]);
}

#[test]
fn the_welcome_config_is_applied_and_an_open_prompt_is_kept() {
    let mut a = app();
    let now = Instant::now();
    a.on_event(prefix(), now);
    a.on_event(ch('$'), now);
    assert!(a.modal.is_some());
    a.on_server(ServerMsg::Welcome {
        session: deck_core::model::SessionId(1),
        config: Box::new(parse("sidebar_width = 30\n[theme]\naccent = \"red\"")),
    });
    assert_eq!(a.sidebar_width, 30);
    assert!(a.modal.is_some());
}

#[test]
fn a_notice_flashes_for_its_level_and_colour() {
    let mut a = app();
    let now = Instant::now();
    for (level, secs, theme_fg) in [
        (NoticeLevel::Info, 2, Color::DarkGray),
        (NoticeLevel::Warning, 4, Color::Yellow),
        (NoticeLevel::Error, 6, Color::Red),
    ] {
        a.on_server(notice(level, "hello notice"));
        let (text, until) = a.flash.clone().unwrap();
        assert_eq!(text, "hello notice");
        let shown = until.checked_duration_since(now).unwrap();
        assert!(
            shown >= Duration::from_secs(secs) && shown < Duration::from_secs(secs + 1),
            "{level:?}: {shown:?}"
        );
        let buf = render(&a);
        let line = row(&buf, 23);
        let at = line.find("hello notice").unwrap();
        let x = u16::try_from(line[..at].chars().count()).unwrap();
        assert_eq!(buf[(x, 23)].fg, theme_fg, "{level:?}");
        a.tick(until);
        assert!(a.flash.is_none());
    }
}

#[test]
fn notices_queue_one_after_the_other() {
    let mut a = app();
    a.on_server(notice(NoticeLevel::Info, "first"));
    a.on_server(notice(NoticeLevel::Warning, "second"));
    a.on_server(notice(NoticeLevel::Error, "third"));
    assert_eq!(a.flash.as_ref().unwrap().0, "first");
    let until = a.flash_deadline().unwrap();
    a.tick(until - Duration::from_millis(1));
    assert_eq!(a.flash.as_ref().unwrap().0, "first");
    a.tick(until);
    assert_eq!(a.flash.as_ref().unwrap().0, "second");
    assert_eq!(a.flash_level, Some(NoticeLevel::Warning));
    let until = a.flash_deadline().unwrap();
    assert!(until >= Instant::now() + Duration::from_secs(3));
    a.tick(until);
    assert_eq!(a.flash.as_ref().unwrap().0, "third");
    assert!(a.flash_error);
    a.tick(a.flash_deadline().unwrap());
    assert!(a.flash.is_none());
}

#[test]
fn keys_typed_before_the_config_arrives_run_with_it() {
    let mut a = app();
    a.hold_until_configured();
    let now = Instant::now();
    assert!(a.on_event(prefix(), now).is_empty());
    assert!(a.on_event(ch('q'), now).is_empty());
    assert!(!a.is_dirty(), "nothing is drawn yet");
    let outs = a.on_server(ServerMsg::Welcome {
        session: deck_core::model::SessionId(1),
        config: Box::new(parse("[keys]\ndetach = \"q\"")),
    });
    assert!(outs.contains(&Out::Send(ClientMsg::Detach)), "{outs:?}");
    assert!(a.is_dirty());
}
