//! Several terminals in a panel: the grid drawn with borders and titles, focus styles, the
//! keys and the mouse, through the client's state machine on a `TestBackend`.

use std::time::Instant;

use deck_core::action::Action as Act;
use deck_core::config::Config;
use deck_core::layout::Direction;
use deck_core::model::{PanelId, ProjectId, SessionId, SessionView, TerminalId};
use deck_core::protocol::{ClientMsg, Modes, ScrollTo, ServerMsg};
use deck_core::theme::Theme;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event as ct;

use crate::app::{App, Out};
use crate::draw::{color, draw};
use crate::mirror::tests::{frame, line, no_sidebar, panel, project};
use crate::modal::Modal;

pub(crate) const SIZE: (u16, u16) = (100, 24);

/// A panel of `n` terminals (ids 31 to 3n) with terminal `focus` (1-based) focused, in a
/// 100×24 window without the sidebar.
pub(crate) fn app_with(n: u64, focus: u64) -> App {
    let mut p = panel(3, "tests", n);
    p.focused_terminal = Some(TerminalId(30 + focus));
    for (t, title) in p.terminals.iter_mut().zip(["nvim", "cargo watch", "sh"]) {
        t.title = Some(title.into());
    }
    let view = SessionView {
        id: SessionId(1),
        name: "main".into(),
        active_project: Some(ProjectId(1)),
        active_panel: Some(PanelId(3)),
        layout_viewport: Some((SIZE.0, SIZE.1 - 1)),
        attached_clients: 0,
        projects: vec![project(1, "terminal-multiplexer", false, vec![p])],
    };
    let mut a = App::new(&no_sidebar(), SIZE, &[]);
    a.on_server(ServerMsg::SessionView(view));
    let rects = deck_core::layout::panel_rects(
        deck_core::layout::Rect::new(0, 0, SIZE.0, SIZE.1 - 1),
        usize::try_from(n).unwrap(),
        &deck_core::layout::Ratios::default_for(usize::try_from(n).unwrap()),
    );
    for (i, rect) in rects.iter().enumerate() {
        let (cols, rows) = deck_core::layout::inner(*rect, usize::try_from(n).unwrap());
        let mut f = frame(
            true,
            (cols, rows),
            vec![(0, line(&format!("term {}", i + 1)))],
        );
        f.terminal = TerminalId(31 + u64::try_from(i).unwrap());
        // Only the focused terminal shows a cursor in the mirror's frame.
        f.cursor.visible = true;
        a.on_server(ServerMsg::Frame(f));
    }
    a
}

pub(crate) fn render(a: &App) -> (Terminal<TestBackend>, Buffer, Vec<String>) {
    let mut t = Terminal::new(TestBackend::new(a.size.0, a.size.1)).unwrap();
    t.draw(|f| draw(f, a)).unwrap();
    let buf = t.backend().buffer().clone();
    let rows = (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect();
    (t, buf, rows)
}

pub(crate) fn key(c: ct::KeyCode) -> ct::Event {
    ct::Event::Key(ct::KeyEvent::new(c, ct::KeyModifiers::NONE))
}

pub(crate) fn ch(c: char) -> ct::Event {
    key(ct::KeyCode::Char(c))
}

pub(crate) fn prefix() -> ct::Event {
    ct::Event::Key(ct::KeyEvent::new(
        ct::KeyCode::Char(' '),
        ct::KeyModifiers::ALT,
    ))
}

pub(crate) fn run(a: &mut App, events: &[ct::Event]) -> Vec<Out> {
    let now = Instant::now();
    events
        .iter()
        .flat_map(|e| a.on_event(e.clone(), now))
        .collect()
}

pub(crate) fn mouse(kind: ct::MouseEventKind, column: u16, row: u16) -> ct::Event {
    ct::Event::Mouse(ct::MouseEvent {
        kind,
        column,
        row,
        modifiers: ct::KeyModifiers::NONE,
    })
}

pub(crate) fn left(
    kind: fn(ct::MouseButton) -> ct::MouseEventKind,
    column: u16,
    row: u16,
) -> ct::Event {
    mouse(kind(ct::MouseButton::Left), column, row)
}

pub(crate) fn action(a: Act) -> Out {
    Out::Send(ClientMsg::Action(a))
}

#[test]
fn three_terminals_are_drawn_in_the_grid_with_borders_and_numbered_titles() {
    let a = app_with(3, 2);
    let (_, _, rows) = render(&a);
    assert!(rows[0].starts_with("╭─ 1 · nvim ─"), "{}", rows[0]);
    assert!(rows[0].contains("╮╭─ 2 · cargo watch ─"), "{}", rows[0]);
    // The big terminal takes the left half, the others share the right one.
    assert_eq!(rows[0].chars().nth(50), Some('╭'));
    assert_eq!(rows[0].chars().nth(49), Some('╮'));
    assert!(rows.iter().any(|r| r.contains("╭─ 3 · sh ─")));
    assert!(rows[1].starts_with("│term 1"), "{}", rows[1]);
    assert_eq!(rows[22].chars().next(), Some('╰'));
    // The status bar has the panel's icon and name, not a terminal's title.
    assert!(
        rows[23].contains("terminal-multiplexer › ❯  tests"),
        "{}",
        rows[23]
    );
    assert!(!rows[23].contains("cargo watch"), "{}", rows[23]);
}

#[test]
fn five_terminals_make_a_main_terminal_and_a_two_by_two_grid() {
    let a = app_with(5, 1);
    let (_, _, rows) = render(&a);
    for i in 1..=5 {
        let want = format!("╭─ {i} · ");
        assert!(rows.iter().any(|r| r.contains(&want)), "{want}");
    }
    let tops: Vec<_> = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| r.chars().nth(75) == Some('╭'))
        .map(|(y, _)| y)
        .collect();
    assert_eq!(tops.len(), 2, "two rows on the right: {tops:?}");
    assert!(rows[0].chars().nth(75) == Some('╭'));
    assert!(rows[0].chars().nth(50) == Some('╭'));
}

#[test]
fn a_single_terminal_has_no_border() {
    let a = app_with(1, 1);
    let (_, _, rows) = render(&a);
    assert!(rows[0].starts_with("term 1"), "{}", rows[0]);
    assert!(!rows.iter().any(|r| r.contains('╭') || r.contains('│')));
}

#[test]
fn the_focused_terminal_has_the_accent_border_and_title() {
    let a = app_with(3, 2);
    let theme = Theme::default();
    let (_, buf, _) = render(&a);
    // Terminal 1 (unfocused): its corner and title; terminal 2 (focused).
    assert_eq!(buf[(0, 0)].fg, color(theme.border));
    assert_eq!(buf[(4, 0)].fg, color(theme.title), "the title of the 1st");
    assert_eq!(buf[(50, 0)].fg, color(theme.border_focused));
    assert_eq!(buf[(54, 0)].fg, color(theme.accent), "the title of the 2nd");
    assert_ne!(color(theme.border), color(theme.border_focused));
}

#[test]
fn the_cursor_is_shown_at_the_focused_terminals_cursor_only() {
    let a = app_with(3, 3);
    let mut t = Terminal::new(TestBackend::new(SIZE.0, SIZE.1)).unwrap();
    t.draw(|f| draw(f, &a)).unwrap();
    // Terminal 3: its rect starts at (50, 11), its screen one cell in; the cursor is at
    // column 1, row 0 of that screen.
    t.backend_mut().assert_cursor_position((52, 12));
}

#[test]
fn the_keys_after_the_prefix_add_close_and_move_focus() {
    let mut a = app_with(3, 3);
    assert_eq!(
        run(&mut a, &[prefix(), ch('t')]),
        [action(Act::NewTerminal { panel: None })]
    );
    for (keys, dir) in [
        (vec![prefix(), ch('h')], Direction::Left),
        (vec![prefix(), key(ct::KeyCode::Left)], Direction::Left),
        (vec![prefix(), ch('l')], Direction::Right),
        (vec![prefix(), key(ct::KeyCode::Right)], Direction::Right),
        (vec![prefix(), ch('k')], Direction::Up),
        (vec![prefix(), key(ct::KeyCode::Up)], Direction::Up),
        (vec![prefix(), ch('j')], Direction::Down),
        (vec![prefix(), key(ct::KeyCode::Down)], Direction::Down),
    ] {
        assert_eq!(run(&mut a, &keys), [action(Act::FocusDirection { dir })]);
    }
}

#[test]
fn closing_a_terminal_asks_first_and_names_its_index() {
    let mut a = app_with(3, 2);
    assert!(run(&mut a, &[prefix(), ch('x')]).is_empty());
    let Some(Modal::Confirm(c)) = &a.modal else {
        panic!("a confirmation");
    };
    assert_eq!(c.question, "Close terminal 2? (y/n)");
    assert!(run(&mut a, &[ch('n')]).is_empty());
    assert!(a.modal.is_none());
    run(&mut a, &[prefix(), ch('x')]);
    assert_eq!(
        run(&mut a, &[ch('y')]),
        [action(Act::CloseTerminal {
            terminal: TerminalId(32)
        })]
    );
}

#[test]
fn a_click_in_an_unfocused_terminal_focuses_it_and_is_not_forwarded() {
    let mut a = app_with(3, 1);
    // Terminal 1 runs a program that asked for the mouse.
    for id in [31, 32] {
        let mut f = frame(false, (48, if id == 31 { 21 } else { 9 }), vec![]);
        f.terminal = TerminalId(id);
        f.modes = Modes(Modes::MOUSE_CLICK | Modes::SGR_MOUSE);
        a.on_server(ServerMsg::Frame(f));
    }
    let down = ct::MouseEventKind::Down;
    let up = ct::MouseEventKind::Up;
    // A cell inside terminal 2 (rect 50..100 x 0..11).
    assert_eq!(
        run(&mut a, &[left(down, 60, 4)]),
        [action(Act::FocusTerminal {
            terminal: TerminalId(32)
        })]
    );
    // The release of that click goes nowhere either.
    assert!(run(&mut a, &[left(up, 60, 4)]).is_empty());
    // Once focus moved (the daemon's view), a click is forwarded to the program.
    let mut p = panel(3, "tests", 3);
    p.focused_terminal = Some(TerminalId(32));
    let mut view = a.mirror.view().unwrap().clone();
    view.projects[0].panels[0] = p;
    a.on_server(ServerMsg::SessionView(view));
    let out = run(&mut a, &[left(down, 60, 4)]);
    assert_eq!(out.len(), 1);
    // Terminal 2's screen starts at (51, 1): the cell is (9, 3), 1-based (10, 4).
    assert_eq!(
        out[0],
        Out::Send(ClientMsg::Input {
            terminal: TerminalId(32),
            bytes: b"\x1b[<0;10;4M".to_vec()
        })
    );
    // A click on a border is not a click on the program.
    assert!(run(&mut a, &[left(down, 50, 4)]).is_empty());
}

#[test]
fn the_wheel_scrolls_the_terminal_under_the_pointer_without_moving_focus() {
    let mut a = app_with(3, 1);
    let mut f = frame(false, (48, 9), vec![]);
    f.terminal = TerminalId(33);
    f.history = 50;
    a.on_server(ServerMsg::Frame(f));
    let out = run(&mut a, &[mouse(ct::MouseEventKind::ScrollUp, 60, 15)]);
    assert_eq!(
        out,
        [Out::Send(ClientMsg::Scroll {
            terminal: TerminalId(33),
            to: ScrollTo::Lines(3)
        })]
    );
    assert!(a.scroll_mode.is_none());
    assert_eq!(a.mirror.focused_id(), Some(TerminalId(31)));
}

#[test]
fn the_default_config_binds_the_new_actions() {
    let c = Config::default();
    for name in [
        "new_terminal",
        "close_terminal",
        "focus_left",
        "focus_right",
        "focus_up",
        "focus_down",
    ] {
        assert!(!c.keys[name].is_empty(), "{name}");
    }
}

/// Marks terminal `id` of the app's only panel as an exited kept terminal.
fn exit_terminal(a: &mut App, id: u64, code: i32) {
    let mut view = a.mirror.view().unwrap().clone();
    for t in &mut view.projects[0].panels[0].terminals {
        if t.id == TerminalId(id) {
            t.keep = true;
            t.exit_status = Some(code);
        }
    }
    a.on_server(ServerMsg::SessionView(view));
}

#[test]
fn an_exited_terminal_shows_its_code_on_a_muted_border() {
    let mut a = app_with(3, 1);
    exit_terminal(&mut a, 32, 3);
    let (_, buf, rows) = render(&a);
    let title = rows.iter().find(|r| r.contains("exited 3")).unwrap();
    assert!(title.contains("2 · cargo watch · exited 3"), "{title}");
    let y = rows.iter().position(|r| r.contains("exited 3")).unwrap();
    let x = title.chars().count() - title.chars().rev().position(|c| c == '╭').unwrap() - 1;
    // The border cell next to the title is muted, not the unfocused-border color.
    let theme = Theme::default();
    let (x, y) = (u16::try_from(x).unwrap(), u16::try_from(y).unwrap());
    assert_eq!(buf[(x, y)].fg, color(theme.muted));
    // A running neighbor keeps its own color.
    assert!(rows.iter().any(|r| r.contains("1 · nvim")));
}

#[test]
fn a_single_exited_terminal_shows_its_code_in_the_status_bar() {
    let mut a = app_with(1, 1);
    exit_terminal(&mut a, 31, 143);
    let (_, _, rows) = render(&a);
    assert!(rows[0].contains("exited 143") || rows[23].contains("exited 143"));
}

#[test]
fn closing_an_exited_terminal_needs_no_confirmation() {
    let mut a = app_with(3, 2);
    exit_terminal(&mut a, 32, 0);
    let out = run(&mut a, &[prefix(), ch('x')]);
    assert!(a.modal.is_none());
    assert_eq!(
        out,
        [action(Act::CloseTerminal {
            terminal: TerminalId(32)
        })]
    );
}
