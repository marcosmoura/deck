//! Dragging the sidebar's separator: the state machine, the viewport it sends, and the look.

use std::time::{Duration, Instant};

use deck_core::config::Config;
use deck_core::protocol::{ClientMsg, ServerMsg};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event as ct;

use crate::app::{App, Out};
use crate::draw::{color, draw};
use crate::mirror::tests::{frame, mockup};
use crate::modal::Modal;
use crate::resize::DRAG_EVERY;

fn app_sized(size: (u16, u16)) -> App {
    let mut app = App::new(&Config::default(), size, &[]);
    app.on_server(ServerMsg::SessionView(mockup()));
    let t = app.mirror.focused_id().unwrap();
    let mut f = frame(true, (40, 10), vec![]);
    f.terminal = t;
    app.on_server(ServerMsg::Frame(f));
    app
}

fn app() -> App {
    app_sized((100, 24))
}

fn mouse(kind: ct::MouseEventKind, column: u16, row: u16) -> ct::Event {
    ct::Event::Mouse(ct::MouseEvent {
        kind,
        column,
        row,
        modifiers: ct::KeyModifiers::NONE,
    })
}

const LEFT: ct::MouseButton = ct::MouseButton::Left;

fn press(a: &mut App, column: u16, now: Instant) -> Vec<Out> {
    a.on_event(mouse(ct::MouseEventKind::Down(LEFT), column, 5), now)
}

fn drag(a: &mut App, column: u16, now: Instant) -> Vec<Out> {
    a.on_event(mouse(ct::MouseEventKind::Drag(LEFT), column, 5), now)
}

fn release(a: &mut App, column: u16, now: Instant) -> Vec<Out> {
    a.on_event(mouse(ct::MouseEventKind::Up(LEFT), column, 5), now)
}

fn viewport(cols: u16) -> Out {
    Out::Send(ClientMsg::Viewport { cols, rows: 23 })
}

fn later(t0: Instant, ms: u64) -> Instant {
    t0 + Duration::from_millis(ms)
}

fn render(a: &App) -> Buffer {
    let mut t = Terminal::new(TestBackend::new(a.size.0, a.size.1)).unwrap();
    t.draw(|f| draw(f, a)).unwrap();
    t.backend().buffer().clone()
}

fn row(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
}

fn parse(text: &str) -> Config {
    Config::parse(text).unwrap().config
}

#[test]
fn dragging_the_separator_follows_the_pointer_and_sends_the_viewport() {
    let mut a = app();
    let t0 = Instant::now();
    assert!(press(&mut a, 28, t0).is_empty());
    assert!(a.sidebar_drag.is_some());
    assert_eq!(drag(&mut a, 40, t0), [viewport(59)]);
    assert_eq!(a.sidebar_width, 40);
    assert_eq!(release(&mut a, 40, later(t0, 5)), []);
    assert!(a.sidebar_drag.is_none());
    // Dragging back.
    press(&mut a, 40, later(t0, 100));
    assert_eq!(drag(&mut a, 28, later(t0, 100)), [viewport(71)]);
    release(&mut a, 28, later(t0, 100));
    assert_eq!(a.sidebar_width, 28);
}

#[test]
fn the_pointer_never_goes_past_the_limits() {
    let mut a = app();
    let t0 = Instant::now();
    press(&mut a, 28, t0);
    assert_eq!(drag(&mut a, 90, t0), [viewport(39)]);
    assert_eq!(a.sidebar_width, 60);
    assert_eq!(drag(&mut a, 3, later(t0, 100)), [viewport(79)]);
    assert_eq!(a.sidebar_width, 20);
    // The same width again sends nothing.
    assert_eq!(drag(&mut a, 0, later(t0, 200)), []);
}

#[test]
fn the_panel_area_keeps_its_twenty_columns() {
    let mut a = app_sized((60, 24));
    let t0 = Instant::now();
    press(&mut a, 28, t0);
    assert_eq!(drag(&mut a, 55, t0), [viewport(20)]);
    assert_eq!(a.sidebar_width, 39);
}

#[test]
fn motion_is_throttled_and_the_last_position_is_sent_on_release() {
    let mut a = app();
    let t0 = Instant::now();
    press(&mut a, 28, t0);
    assert_eq!(drag(&mut a, 30, t0), [viewport(69)]);
    assert_eq!(drag(&mut a, 35, later(t0, 10)), []);
    assert_eq!(a.sidebar_width, 30);
    assert_eq!(drag(&mut a, 36, t0 + DRAG_EVERY), [viewport(63)]);
    assert_eq!(
        drag(&mut a, 38, t0 + DRAG_EVERY + Duration::from_millis(1)),
        []
    );
    assert_eq!(
        release(&mut a, 38, t0 + DRAG_EVERY + Duration::from_millis(2)),
        [viewport(61)]
    );
    assert_eq!(a.sidebar_width, 38);
}

#[test]
fn the_drag_is_never_forwarded_to_a_program() {
    let mut a = app();
    let t0 = Instant::now();
    let mut outs = press(&mut a, 28, t0);
    for (i, column) in [29, 45, 80].into_iter().enumerate() {
        outs.extend(drag(&mut a, column, later(t0, 40 * (i as u64 + 1))));
    }
    outs.extend(release(&mut a, 80, later(t0, 500)));
    assert!(
        outs.iter()
            .all(|o| matches!(o, Out::Send(ClientMsg::Viewport { .. }))),
        "{outs:?}"
    );
}

#[test]
fn a_lost_release_a_focus_loss_an_overlay_and_a_resize_end_the_drag() {
    let t0 = Instant::now();
    let started = || {
        let mut a = app();
        press(&mut a, 28, t0);
        assert!(a.sidebar_drag.is_some());
        a
    };
    let mut a = started();
    a.on_event(mouse(ct::MouseEventKind::Moved, 50, 5), t0);
    assert!(a.sidebar_drag.is_none());

    let mut a = started();
    a.on_event(ct::Event::FocusLost, t0);
    assert!(a.sidebar_drag.is_none());

    let mut a = started();
    a.modal = Some(Modal::Help);
    assert_eq!(drag(&mut a, 40, t0), []);
    assert!(a.sidebar_drag.is_none());
    assert_eq!(a.sidebar_width, 28);

    let mut a = started();
    a.on_event(ct::Event::Resize(90, 24), t0);
    assert!(a.sidebar_drag.is_none());
}

#[test]
fn an_overlay_cannot_be_dragged_under() {
    let mut a = app();
    a.modal = Some(Modal::Help);
    let t0 = Instant::now();
    press(&mut a, 28, t0);
    assert!(a.sidebar_drag.is_none());
}

#[test]
fn a_press_on_the_separator_with_the_sidebar_hidden_does_nothing() {
    let mut a = app();
    let t0 = Instant::now();
    a.on_event(
        ct::Event::Key(ct::KeyEvent::new(
            ct::KeyCode::Char(' '),
            ct::KeyModifiers::ALT,
        )),
        t0,
    );
    a.on_event(
        ct::Event::Key(ct::KeyEvent::from(ct::KeyCode::Char('b'))),
        t0,
    );
    assert!(a.sidebar_hidden);
    assert!(
        press(&mut a, 28, t0)
            .iter()
            .all(|o| !matches!(o, Out::Send(ClientMsg::Viewport { .. })))
    );
    assert!(a.sidebar_drag.is_none());
}

#[test]
fn a_press_on_a_sidebar_row_still_focuses_it() {
    let mut a = app();
    let outs = press(&mut a, 5, Instant::now());
    assert!(a.sidebar_drag.is_none());
    assert!(!outs.is_empty(), "the row is acted on");
}

#[test]
fn the_dragged_width_survives_the_toggle_and_a_session_switch() {
    let mut a = app();
    let t0 = Instant::now();
    press(&mut a, 28, t0);
    drag(&mut a, 40, t0);
    release(&mut a, 40, t0);
    let b = |code| ct::Event::Key(ct::KeyEvent::from(ct::KeyCode::Char(code)));
    let prefix = ct::Event::Key(ct::KeyEvent::new(
        ct::KeyCode::Char(' '),
        ct::KeyModifiers::ALT,
    ));
    a.on_event(prefix.clone(), t0);
    assert_eq!(a.on_event(b('b'), t0), [viewport(100)]);
    a.on_event(prefix, t0);
    assert_eq!(a.on_event(b('b'), t0), [viewport(59)]);
    assert_eq!(a.sidebar_width, 40);
    a.reset_session();
    assert_eq!(a.sidebar_width, 40);
}

#[test]
fn a_config_reload_replaces_the_width_only_when_it_changes_it() {
    let mut a = app();
    let t0 = Instant::now();
    press(&mut a, 28, t0);
    drag(&mut a, 40, t0);
    release(&mut a, 40, t0);
    let outs = a.on_server(ServerMsg::Config(Box::new(parse(
        "[theme]\naccent = \"red\"",
    ))));
    assert!(outs.is_empty(), "{outs:?}");
    assert_eq!(a.sidebar_width, 40);
    let outs = a.on_server(ServerMsg::Config(Box::new(parse("sidebar_width = 35"))));
    assert_eq!(a.sidebar_width, 35);
    assert_eq!(
        outs,
        [Out::Send(ClientMsg::ReloadViewport { cols: 64, rows: 23 })]
    );
}

#[test]
fn a_resize_shrinks_a_wide_sidebar_and_never_grows_it() {
    let mut a = app_sized((81, 24));
    a.sidebar_width = 60;
    assert!(a.sidebar_visible());
    let t0 = Instant::now();
    assert_eq!(a.on_event(ct::Event::Resize(80, 24), t0), [viewport(20)]);
    assert!(a.sidebar_visible());
    assert_eq!(a.sidebar_width, 59);
    a.on_event(ct::Event::Resize(120, 24), t0);
    assert_eq!(a.sidebar_width, 59);
}

#[test]
fn the_viewport_is_checked_after_the_first_view_of_an_attach() {
    let mut a = App::new(&Config::default(), (100, 24), &[]);
    // The user hid the sidebar before the switch.
    a.sidebar_hidden = true;
    a.reset_session();
    let outs = a.on_server(ServerMsg::SessionView(mockup()));
    assert!(outs.contains(&viewport(100)), "{outs:?}");
    // Only the first view of an attach.
    let outs = a.on_server(ServerMsg::SessionView(mockup()));
    assert!(!outs.contains(&viewport(100)), "{outs:?}");

    // A dragged width is sent after a switch too.
    let mut a = app();
    a.sidebar_width = 40;
    a.reset_session();
    let outs = a.on_server(ServerMsg::SessionView(mockup()));
    assert!(outs.contains(&viewport(59)), "{outs:?}");

    // Nothing differs: nothing is sent.
    let mut a = app();
    a.reset_session();
    let outs = a.on_server(ServerMsg::SessionView(mockup()));
    assert!(
        !outs
            .iter()
            .any(|o| matches!(o, Out::Send(ClientMsg::Viewport { .. }))),
        "{outs:?}"
    );
}

#[test]
fn the_separator_is_accent_while_dragged_and_the_badge_shows_the_width() {
    let mut a = app();
    let theme = a.theme.clone();
    let normal = render(&a);
    assert_eq!(normal[(28, 5)].symbol(), "│");
    assert_eq!(normal[(28, 5)].fg, color(theme.border));

    let t0 = Instant::now();
    press(&mut a, 28, t0);
    drag(&mut a, 40, t0);
    let dragged = render(&a);
    assert_eq!(dragged[(40, 5)].symbol(), "┃");
    assert_eq!(dragged[(40, 5)].fg, color(theme.accent));
    assert!(
        row(&dragged, 23).starts_with(" SIDEBAR 40 "),
        "{}",
        row(&dragged, 23)
    );

    release(&mut a, 40, t0);
    let after = render(&a);
    assert_eq!(after[(40, 5)].symbol(), "│");
    assert_eq!(after[(40, 5)].fg, color(theme.border));
    assert!(!row(&after, 23).contains("SIDEBAR"));
}

#[test]
fn a_wide_sidebar_is_clamped_to_the_window_at_attach_and_on_a_config_change() {
    let config = parse("sidebar_width = 60");
    let mut a = App::new(&config, (70, 24), &[]);
    assert_eq!(a.sidebar_width, 49);
    assert!(a.sidebar_visible());
    assert_eq!(a.viewport(), (20, 23));
    // The daemon derives the same area, so the first view sends nothing.
    let outs = a.on_server(ServerMsg::SessionView(mockup()));
    assert!(
        !outs
            .iter()
            .any(|o| matches!(o, Out::Send(ClientMsg::Viewport { .. }))),
        "{outs:?}"
    );
    // Shrinking by a column keeps it shown; it does not appear from nowhere.
    a.on_event(ct::Event::Resize(69, 24), Instant::now());
    assert!(a.sidebar_visible());
    assert_eq!(a.sidebar_width, 48);

    let mut a = app_sized((70, 24));
    a.on_server(ServerMsg::Config(Box::new(parse("sidebar_width = 60"))));
    assert_eq!(a.sidebar_width, 49);
    assert!(a.sidebar_visible());
}

#[test]
fn a_key_ends_the_drag_and_the_rest_of_the_gesture_reaches_no_program() {
    let mut a = app();
    // The focused terminal asked for the mouse.
    let t = a.mirror.focused_id().unwrap();
    let mut f = frame(true, (40, 10), vec![]);
    f.terminal = t;
    f.modes = deck_core::protocol::Modes(
        deck_core::protocol::Modes::MOUSE_CLICK
            | deck_core::protocol::Modes::MOUSE_DRAG
            | deck_core::protocol::Modes::SGR_MOUSE,
    );
    a.on_server(ServerMsg::Frame(f));
    let t0 = Instant::now();
    press(&mut a, 28, t0);
    a.on_event(
        ct::Event::Key(ct::KeyEvent::from(ct::KeyCode::Char('z'))),
        t0,
    );
    assert!(a.sidebar_drag.is_none());
    let outs = a.on_event(mouse(ct::MouseEventKind::Drag(LEFT), 50, 5), t0);
    assert!(outs.is_empty(), "{outs:?}");
    assert_eq!(release(&mut a, 50, t0), []);
    // After the release a click reaches the program again.
    let outs = press(&mut a, 50, later(t0, 100));
    assert!(
        outs.iter()
            .any(|o| matches!(o, Out::Send(ClientMsg::Input { .. }))),
        "{outs:?}"
    );
}
