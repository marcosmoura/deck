//! Selecting text with the mouse through the client's state machine: who gets the mouse,
//! the gestures, the copy on release, the clearing rules and the look.

use std::time::{Duration, Instant};

use deck_core::model::TerminalId;
use deck_core::protocol::{ClientMsg, Modes, ServerMsg};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event as ct;
use ratatui::style::Modifier;

use crate::app::{App, Out};
use crate::draw::draw;
use crate::grid_tests::app_with;
use crate::mirror::tests::{frame, line, no_sidebar, view};
use crate::select::Unit;

const LEFT: ct::MouseButton = ct::MouseButton::Left;
const SGR_DRAG: u16 = Modes::MOUSE_DRAG | Modes::SGR_MOUSE;

/// One terminal (id 4, 78×21) at the window's top left, showing `lines` with `modes`.
fn app(modes: u16, lines: &[&str]) -> App {
    let mut a = App::new(&no_sidebar(), (80, 24), &[]);
    a.on_server(ServerMsg::SessionView(view(None)));
    show(&mut a, modes, lines);
    a
}

fn show(a: &mut App, modes: u16, lines: &[&str]) {
    let rows = lines
        .iter()
        .enumerate()
        .map(|(i, l)| (u16::try_from(i).unwrap(), line(l)))
        .collect();
    let mut f = frame(true, (78, 21), rows);
    f.modes = Modes(modes);
    a.on_server(ServerMsg::Frame(f));
}

fn ev(
    a: &mut App,
    kind: ct::MouseEventKind,
    cell: (u16, u16),
    alt: bool,
    now: Instant,
) -> Vec<Out> {
    let modifiers = if alt {
        ct::KeyModifiers::ALT
    } else {
        ct::KeyModifiers::NONE
    };
    a.on_event(
        ct::Event::Mouse(ct::MouseEvent {
            kind,
            column: cell.0,
            row: cell.1,
            modifiers,
        }),
        now,
    )
}

/// Press at `from`, drag to `to`, release there; the outputs of each step.
fn select(a: &mut App, from: (u16, u16), to: (u16, u16), alt: bool) -> Vec<Out> {
    let now = Instant::now();
    let mut outs = ev(a, ct::MouseEventKind::Down(LEFT), from, alt, now);
    outs.extend(ev(a, ct::MouseEventKind::Drag(LEFT), to, alt, now));
    outs.extend(ev(a, ct::MouseEventKind::Up(LEFT), to, alt, now));
    outs
}

fn copied(text: &str) -> Out {
    let b64 = deck_core::base64::encode(text.as_bytes());
    Out::Clipboard(format!("\x1b]52;c;{b64}\x07").into_bytes())
}

#[test]
fn a_drag_selects_and_the_release_copies() {
    let mut a = app(0, &["hello world", "second line"]);
    let outs = select(&mut a, (6, 0), (5, 1), false);
    assert_eq!(outs, [copied("world\nsecond")]);
    // The selection stays drawn after the copy.
    let sel = a.text_sel.as_ref().unwrap();
    assert_eq!(
        (sel.anchor, sel.head, sel.dragging),
        ((6, 0), (5, 1), false)
    );
}

#[test]
fn a_click_without_a_drag_selects_nothing() {
    let mut a = app(0, &["hello"]);
    assert!(select(&mut a, (2, 0), (2, 0), false).is_empty());
    assert_eq!(a.text_sel, None);
}

#[test]
fn a_program_with_mouse_reporting_keeps_the_mouse_without_option() {
    let mut a = app(SGR_DRAG, &["hello"]);
    let outs = select(&mut a, (0, 0), (3, 0), false);
    assert!(
        outs.iter()
            .all(|o| matches!(o, Out::Send(ClientMsg::Input { .. }))),
        "{outs:?}"
    );
    assert_eq!(outs.len(), 3);
    assert_eq!(a.text_sel, None);
}

#[test]
fn option_takes_the_mouse_back_from_the_program() {
    let mut a = app(SGR_DRAG, &["hello"]);
    assert_eq!(select(&mut a, (0, 0), (3, 0), true), [copied("hell")]);
    // An Option click alone sends the program nothing either, not even its release.
    assert!(select(&mut a, (1, 0), (1, 0), true).is_empty());
}

#[test]
fn scroll_mode_presses_select() {
    let mut a = app(0, &["hello"]);
    a.scroll_mode = Some(TerminalId(4));
    assert_eq!(select(&mut a, (0, 0), (1, 0), false), [copied("he")]);
}

#[test]
fn double_and_triple_clicks_select_a_word_and_a_line() {
    let mut a = app(0, &["cat src/main.rs | less"]);
    let t0 = Instant::now();
    let click = |a: &mut App, at: Instant| {
        let mut outs = ev(a, ct::MouseEventKind::Down(LEFT), (6, 0), false, at);
        outs.extend(ev(a, ct::MouseEventKind::Up(LEFT), (6, 0), false, at));
        outs
    };
    assert!(click(&mut a, t0).is_empty());
    let t1 = t0 + Duration::from_millis(200);
    assert_eq!(click(&mut a, t1), [copied("src/main.rs")]);
    assert_eq!(a.text_sel.as_ref().unwrap().unit, Unit::Word);
    let t2 = t1 + Duration::from_millis(200);
    assert_eq!(click(&mut a, t2), [copied("cat src/main.rs | less")]);
    // Too late for a fourth: a plain click again.
    assert!(click(&mut a, t2 + Duration::from_secs(1)).is_empty());
}

#[test]
fn a_drag_after_a_double_click_extends_by_words() {
    let mut a = app(0, &["one two three"]);
    let t0 = Instant::now();
    ev(&mut a, ct::MouseEventKind::Down(LEFT), (5, 0), false, t0);
    ev(&mut a, ct::MouseEventKind::Up(LEFT), (5, 0), false, t0);
    ev(&mut a, ct::MouseEventKind::Down(LEFT), (5, 0), false, t0);
    ev(&mut a, ct::MouseEventKind::Drag(LEFT), (9, 0), false, t0);
    let outs = ev(&mut a, ct::MouseEventKind::Up(LEFT), (9, 0), false, t0);
    assert_eq!(outs, [copied("two three")]);
}

#[test]
fn a_drag_past_the_screen_is_clamped_to_it() {
    let mut a = app(0, &["abc"]);
    select(&mut a, (1, 0), (79, 23), false);
    assert_eq!(a.text_sel.as_ref().unwrap().head, (77, 20));
}

#[test]
fn a_drag_into_another_terminal_stays_in_its_own() {
    // Two side by side: terminal 31 on the left is focused.
    let mut a = app_with(2, 1);
    let outs = select(&mut a, (2, 1), (90, 1), false);
    // Column 0 is the border: window column 2 is the screen's column 1.
    assert_eq!(outs, [copied("erm 1")]);
    let sel = a.text_sel.as_ref().unwrap();
    assert_eq!(sel.terminal, TerminalId(31));
}

#[test]
fn a_press_or_a_change_under_the_selection_clears_it() {
    let mut a = app(0, &["hello", "world"]);
    select(&mut a, (0, 0), (2, 0), false);
    // A change on another row keeps it.
    let mut f = frame(false, (78, 21), vec![(5, line("x"))]);
    f.modes = Modes(0);
    a.on_server(ServerMsg::Frame(f));
    assert!(a.text_sel.is_some());
    // A change on its row clears it.
    a.on_server(ServerMsg::Frame(frame(
        false,
        (78, 21),
        vec![(0, line("hi"))],
    )));
    assert_eq!(a.text_sel, None);
    // A press anywhere clears it too.
    select(&mut a, (0, 1), (2, 1), false);
    ev(
        &mut a,
        ct::MouseEventKind::Down(ct::MouseButton::Right),
        (9, 9),
        false,
        Instant::now(),
    );
    assert_eq!(a.text_sel, None);
}

#[test]
fn a_selection_cleared_mid_drag_sends_the_program_nothing() {
    let mut a = app(SGR_DRAG, &["hello"]);
    let now = Instant::now();
    ev(&mut a, ct::MouseEventKind::Down(LEFT), (0, 0), true, now);
    a.on_server(ServerMsg::Frame(frame(
        false,
        (78, 21),
        vec![(0, line("bye"))],
    )));
    assert_eq!(a.text_sel, None);
    let mut outs = ev(&mut a, ct::MouseEventKind::Drag(LEFT), (3, 0), true, now);
    outs.extend(ev(&mut a, ct::MouseEventKind::Up(LEFT), (3, 0), true, now));
    assert!(outs.is_empty(), "{outs:?}");
}

#[test]
fn the_selection_is_drawn_reversed() {
    let mut a = app(0, &["hello world"]);
    select(&mut a, (0, 0), (4, 0), false);
    let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
    t.draw(|f| draw(f, &a)).unwrap();
    let buf = t.backend().buffer();
    // The terminal's screen starts at the window's top left here.
    for x in 0..5 {
        assert!(
            buf[(x, 0)].modifier.contains(Modifier::REVERSED),
            "cell {x}"
        );
    }
    assert!(!buf[(5, 0)].modifier.contains(Modifier::REVERSED));
}
