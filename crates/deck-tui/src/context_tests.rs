//! The context menu: targets and items, placement, outcomes, keys and mouse, the right
//! button's rules and stale targets.

use std::path::PathBuf;
use std::time::Instant;

use deck_core::action::Action as Act;
use deck_core::config::Config;
use deck_core::model::{PanelId, ProjectId, SessionView, TerminalId};
use deck_core::protocol::{ClientMsg, Modes, ServerMsg};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event as ct;

use crate::app::{App, Out};
use crate::context::ContextTarget;
use crate::draw::draw;
use crate::mirror::tests::{frame, mockup};
use crate::modal::{Modal, PromptKind, Target};
use crate::sidebar::Sel;

const LEFT: ct::MouseButton = ct::MouseButton::Left;
const RIGHT: ct::MouseButton = ct::MouseButton::Right;
const NONE: ct::KeyModifiers = ct::KeyModifiers::NONE;

/// `mockup()` with two terminals (31 focused, 32) in the active panel.
fn two_terminals() -> SessionView {
    let mut view = mockup();
    let panel = &mut view.projects[0].panels[0];
    let mut second = panel.terminals[0].clone();
    second.id = TerminalId(32);
    panel.terminals.push(second);
    panel.ratios = deck_core::layout::Ratios::default_for(2);
    view
}

fn app_with(view: SessionView) -> App {
    let config = Config {
        sidebar_width: 28,
        ..Config::default()
    };
    let mut app = App::new(&config, (100, 24), &[])
        .with_dirs(PathBuf::from("/work"), Some(PathBuf::from("/home/me")));
    app.on_server(ServerMsg::SessionView(view));
    let mut f = frame(true, (40, 10), vec![]);
    f.terminal = TerminalId(31);
    app.on_server(ServerMsg::Frame(f));
    app
}

fn app() -> App {
    app_with(two_terminals())
}

fn event(
    a: &mut App,
    kind: ct::MouseEventKind,
    column: u16,
    row: u16,
    modifiers: ct::KeyModifiers,
) -> Vec<Out> {
    a.on_event(
        ct::Event::Mouse(ct::MouseEvent {
            kind,
            column,
            row,
            modifiers,
        }),
        Instant::now(),
    )
}

fn right(a: &mut App, column: u16, row: u16) -> Vec<Out> {
    event(a, ct::MouseEventKind::Down(RIGHT), column, row, NONE)
}

fn left(a: &mut App, column: u16, row: u16) -> Vec<Out> {
    event(a, ct::MouseEventKind::Down(LEFT), column, row, NONE)
}

fn key(a: &mut App, code: ct::KeyCode) -> Vec<Out> {
    a.on_event(
        ct::Event::Key(ct::KeyEvent::new(code, NONE)),
        Instant::now(),
    )
}

fn ch(a: &mut App, c: char) -> Vec<Out> {
    key(a, ct::KeyCode::Char(c))
}

fn act(a: Act) -> Out {
    Out::Send(ClientMsg::Action(a))
}

fn buffer(a: &App) -> Buffer {
    let mut t = Terminal::new(TestBackend::new(a.size.0, a.size.1)).unwrap();
    t.draw(|f| draw(f, a)).unwrap();
    t.backend().buffer().clone()
}

fn text(buf: &Buffer, y: u16, from: u16, to: u16) -> String {
    (from..to).map(|x| buf[(x, y)].symbol()).collect()
}

fn reporting(a: &mut App, terminal: u64) {
    let mut f = frame(true, (40, 10), vec![]);
    f.terminal = TerminalId(terminal);
    f.modes = Modes(Modes::MOUSE_CLICK | Modes::MOUSE_MOTION | Modes::SGR_MOUSE);
    a.on_server(ServerMsg::Frame(f));
}

/// A cell inside terminal 31 (focused) and one inside terminal 32.
const T30: (u16, u16) = (34, 5);
const T31: (u16, u16) = (70, 5);
/// The rows of the sidebar: the `budget` space and its `api` panel.
const BUDGET: u16 = 6;
const API: u16 = 7;

fn context(a: &App) -> &crate::context::ContextMenu {
    a.context.as_ref().expect("a context menu")
}

fn confirm_question(a: &App) -> Option<(&str, &Target)> {
    match &a.modal {
        Some(Modal::Confirm(c)) => Some((c.question.as_str(), &c.target)),
        _ => None,
    }
}

#[test]
fn a_right_click_on_a_terminal_opens_its_menu_at_the_pointer() {
    let mut a = app();
    assert!(right(&mut a, T30.0, T30.1).is_empty());
    let menu = context(&a);
    assert_eq!(menu.target, ContextTarget::Terminal(TerminalId(31)));
    assert_eq!(
        menu.items(),
        ["New terminal", "Rename panel", "Close terminal"]
    );
    assert_eq!(menu.selected, 0);
    let rect = a.context_rect().unwrap();
    assert_eq!((rect.x, rect.y, rect.width, rect.height), (34, 5, 20, 5));
    let buf = buffer(&a);
    assert_eq!(text(&buf, 5, 34, 54), "╭──────────────────╮");
    assert_eq!(text(&buf, 6, 34, 54), "│ New terminal   1 │");
    assert_eq!(text(&buf, 8, 34, 54), "│ Close terminal 3 │");
    assert_eq!(text(&buf, 9, 34, 54), "╰──────────────────╯");
}

#[test]
fn the_row_menus_have_their_items_and_a_minimum_width() {
    let mut a = app();
    right(&mut a, 14, API);
    assert_eq!(context(&a).target, ContextTarget::Panel(PanelId(6)));
    assert_eq!(context(&a).items(), ["Rename", "Close"]);
    a.context = None;
    right(&mut a, 14, BUDGET);
    assert_eq!(context(&a).target, ContextTarget::Project(ProjectId(2)));
    assert_eq!(context(&a).items(), ["Rename", "New panel", "Close"]);
    let rect = a.context_rect().unwrap();
    assert_eq!((rect.x, rect.y, rect.width, rect.height), (14, 6, 18, 5));
    let buf = buffer(&a);
    assert_eq!(text(&buf, 7, 14, 32), "│ Rename       1 │");
    assert_eq!(text(&buf, 8, 14, 32), "│ New panel    2 │");
}

#[test]
fn the_menu_moves_left_and_up_to_stay_in_the_window() {
    let mut a = app();
    right(&mut a, 90, 21);
    let rect = a.context_rect().unwrap();
    assert_eq!(rect.x + rect.width, 100, "right edge on the last column");
    assert_eq!(rect.y + rect.height, 24, "bottom edge on the last row");
}

#[test]
fn nothing_else_opens_a_menu() {
    let mut a = app();
    // The header, its chip, the blank row, a space with no panels' `no panels`, the bottom
    // bar, the separator, a terminal's border and title.
    for (column, row) in [
        (2, 0),
        (20, 0),
        (2, 1),
        (6, 9),
        (3, 23),
        (28, 5),
        (29, 5),
        (70, 0),
        (35 + 29, 12),
        (60, 23),
    ] {
        right(&mut a, column, row);
        assert!(a.context.is_none(), "({column}, {row})");
        event(&mut a, ct::MouseEventKind::Up(RIGHT), column, row, NONE);
    }
}

#[test]
fn a_right_click_on_a_border_of_an_unfocused_terminal_only_focuses_it() {
    let mut a = app();
    let outs = right(&mut a, 70, 0);
    assert_eq!(
        outs,
        vec![act(Act::FocusTerminal {
            terminal: TerminalId(32)
        })]
    );
    assert!(a.context.is_none());
}

#[test]
fn the_items_do_what_their_bindings_do() {
    // New terminal.
    let mut a = app();
    right(&mut a, T30.0, T30.1);
    let outs = ch(&mut a, '1');
    assert_eq!(outs, vec![act(Act::NewTerminal { panel: None })]);
    assert!(a.context.is_none());
    // Rename panel.
    let mut a = app();
    right(&mut a, T30.0, T30.1);
    ch(&mut a, '2');
    let Some(Modal::Prompt(p)) = &a.modal else {
        panic!("{:?}", a.modal)
    };
    assert_eq!(p.kind, PromptKind::RenamePanel(PanelId(3)));
    // Close terminal asks with the terminal's place in the panel.
    let mut a = app();
    right(&mut a, T30.0, T30.1);
    ch(&mut a, '3');
    assert_eq!(
        confirm_question(&a),
        Some(("Close terminal 1?", &Target::Terminal(TerminalId(31), 1)))
    );
    // A panel row.
    let mut a = app();
    right(&mut a, 14, API);
    ch(&mut a, '1');
    assert!(
        matches!(&a.modal, Some(Modal::Prompt(p)) if p.kind == PromptKind::RenamePanel(PanelId(6)))
    );
    let mut a = app();
    right(&mut a, 14, API);
    ch(&mut a, '2');
    assert!(matches!(
        confirm_question(&a),
        Some((_, Target::Panel(PanelId(6), _)))
    ));
    // A space row.
    let mut a = app();
    right(&mut a, 14, BUDGET);
    ch(&mut a, '1');
    assert!(
        matches!(&a.modal, Some(Modal::Prompt(p)) if p.kind == PromptKind::RenameProject(ProjectId(2)))
    );
    let mut a = app();
    right(&mut a, 14, BUDGET);
    ch(&mut a, '3');
    assert!(matches!(
        confirm_question(&a),
        Some((_, Target::Project(ProjectId(2), _)))
    ));
}

#[test]
fn new_panel_opens_the_new_menu_at_the_click() {
    let mut a = app();
    right(&mut a, 14, BUDGET);
    assert!(ch(&mut a, '2').is_empty());
    assert!(a.context.is_none());
    let menu = a.menu.as_ref().expect("the New menu");
    assert_eq!(menu.project, ProjectId(2));
    assert_eq!(menu.anchor, Some((14, BUDGET)));
    let rect = a.menu_rect().unwrap();
    assert_eq!((rect.x, rect.y), (14, BUDGET));
    let buf = buffer(&a);
    assert!(text(&buf, BUDGET, 14, 40).contains("New in budget"));
    // Near the bottom right it moves to stay in the window.
    let mut a = app();
    a.context = None;
    a.open_new_menu_at(ProjectId(2), Some((98, 22)));
    let rect = a.menu_rect().unwrap();
    assert_eq!(rect.x + rect.width, 100);
    assert_eq!(rect.y + rect.height, 24);
    // From the key it sits where it did.
    let mut a = app();
    a.open_new_menu(ProjectId(2));
    assert_eq!(a.menu_rect().unwrap().x, 0);
}

#[test]
fn keys_move_choose_and_close() {
    let mut a = app();
    right(&mut a, 14, BUDGET);
    ch(&mut a, 'j');
    assert_eq!(context(&a).selected, 1);
    key(&mut a, ct::KeyCode::Down);
    key(&mut a, ct::KeyCode::Down);
    assert_eq!(context(&a).selected, 2, "stops at the last item");
    key(&mut a, ct::KeyCode::Up);
    ch(&mut a, 'k');
    assert_eq!(context(&a).selected, 0);
    a.on_event(
        ct::Event::Key(ct::KeyEvent::new(
            ct::KeyCode::Char('n'),
            ct::KeyModifiers::CONTROL,
        )),
        Instant::now(),
    );
    assert_eq!(context(&a).selected, 1);
    a.on_event(
        ct::Event::Key(ct::KeyEvent::new(
            ct::KeyCode::Char('p'),
            ct::KeyModifiers::CONTROL,
        )),
        Instant::now(),
    );
    assert_eq!(context(&a).selected, 0);
    // A digit past the items does nothing.
    ch(&mut a, '4');
    assert!(a.context.is_some());
    key(&mut a, ct::KeyCode::Down);
    key(&mut a, ct::KeyCode::Down);
    key(&mut a, ct::KeyCode::Enter);
    assert!(matches!(a.modal, Some(Modal::Confirm(_))));
    for close in [ct::KeyCode::Esc, ct::KeyCode::Char('q')] {
        let mut a = app();
        right(&mut a, 14, BUDGET);
        key(&mut a, close);
        assert!(a.context.is_none());
        assert!(a.modal.is_none());
    }
}

#[test]
fn the_prefix_key_is_ordinary_and_the_wheel_does_nothing() {
    let mut a = app();
    right(&mut a, 14, BUDGET);
    let outs = a.on_event(
        ct::Event::Key(ct::KeyEvent::new(
            ct::KeyCode::Char(' '),
            ct::KeyModifiers::ALT,
        )),
        Instant::now(),
    );
    assert!(outs.is_empty());
    assert!(!a.armed && a.context.is_some());
    event(&mut a, ct::MouseEventKind::ScrollDown, 14, BUDGET, NONE);
    assert!(a.context.is_some());
    assert_eq!(context(&a).selected, 0);
}

#[test]
fn hover_moves_the_highlight_and_a_click_chooses() {
    let mut a = app();
    right(&mut a, 14, BUDGET);
    event(&mut a, ct::MouseEventKind::Moved, 16, BUDGET + 3, NONE);
    assert_eq!(context(&a).selected, 2);
    event(&mut a, ct::MouseEventKind::Moved, 16, BUDGET + 2, NONE);
    assert_eq!(context(&a).selected, 1);
    let outs = left(&mut a, 16, BUDGET + 1);
    assert!(outs.is_empty());
    assert!(matches!(&a.modal, Some(Modal::Prompt(_))));
    assert!(a.swallow_release);
}

#[test]
fn a_press_elsewhere_closes_the_menu_and_goes_no_further() {
    for button in [LEFT, RIGHT] {
        let mut a = app();
        reporting(&mut a, 31);
        right(&mut a, 14, BUDGET);
        let outs = event(&mut a, ct::MouseEventKind::Down(button), T30.0, T30.1, NONE);
        assert!(outs.is_empty(), "{button:?}");
        assert!(a.context.is_none());
        assert!(a.menu.is_none());
        assert!(a.text_sel.is_none());
    }
    // A press on the menu's border or an item with the right button keeps it.
    let mut a = app();
    right(&mut a, 14, BUDGET);
    right(&mut a, 16, BUDGET + 1);
    assert!(a.context.is_some());
}

#[test]
fn the_release_of_the_opening_press_is_swallowed() {
    let mut a = app();
    right(&mut a, T30.0, T30.1);
    assert!(a.swallow_release);
    event(&mut a, ct::MouseEventKind::Up(RIGHT), T30.0, T30.1, NONE);
    assert!(!a.swallow_release);
}

#[test]
fn a_program_with_mouse_reporting_gets_the_right_button_unless_alt_is_held() {
    let mut a = app();
    reporting(&mut a, 31);
    let outs = right(&mut a, T30.0, T30.1);
    assert!(matches!(
        outs.as_slice(),
        [Out::Send(ClientMsg::Input { .. })]
    ));
    assert!(a.context.is_none());
    let outs = event(
        &mut a,
        ct::MouseEventKind::Down(RIGHT),
        T30.0,
        T30.1,
        ct::KeyModifiers::ALT,
    );
    assert!(outs.is_empty());
    assert!(a.context.is_some());
}

#[test]
fn scroll_mode_always_opens_the_menu() {
    let mut a = app();
    reporting(&mut a, 31);
    a.scroll_mode = Some(TerminalId(31));
    assert!(right(&mut a, T30.0, T30.1).is_empty());
    assert!(a.context.is_some());
}

#[test]
fn an_unfocused_terminal_is_focused_and_gets_the_menu_only_when_deck_has_the_button() {
    let focus = || {
        vec![act(Act::FocusTerminal {
            terminal: TerminalId(32),
        })]
    };
    // No mouse reporting there: focus and menu.
    let mut a = app();
    assert_eq!(right(&mut a, T31.0, T31.1), focus());
    assert_eq!(context(&a).target, ContextTarget::Terminal(TerminalId(32)));
    assert!(a.swallow_release);
    // Reporting: focus only.
    let mut a = app();
    reporting(&mut a, 32);
    assert_eq!(right(&mut a, T31.0, T31.1), focus());
    assert!(a.context.is_none());
    assert!(a.swallow_release);
    // Reporting with Alt held: focus and menu.
    event(&mut a, ct::MouseEventKind::Up(RIGHT), T31.0, T31.1, NONE);
    let outs = event(
        &mut a,
        ct::MouseEventKind::Down(RIGHT),
        T31.0,
        T31.1,
        ct::KeyModifiers::ALT,
    );
    assert!(a.context.is_some(), "{outs:?}");
}

#[test]
fn the_menu_closes_the_given_terminal_even_if_focus_moves() {
    let mut a = app();
    right(&mut a, T31.0, T31.1);
    // Focus moves back to terminal 30 before an item is chosen.
    let mut view = two_terminals();
    view.projects[0].panels[0].focused_terminal = Some(TerminalId(31));
    a.on_server(ServerMsg::SessionView(view));
    ch(&mut a, '3');
    assert_eq!(
        confirm_question(&a),
        Some(("Close terminal 2?", &Target::Terminal(TerminalId(32), 2)))
    );
}

#[test]
fn closing_a_kept_terminal_that_exited_needs_no_question() {
    let mut view = two_terminals();
    view.projects[0].panels[0].terminals[1].exit_status = Some(0);
    let mut a = app_with(view);
    right(&mut a, T31.0, T31.1);
    let outs = ch(&mut a, '3');
    assert_eq!(
        outs,
        vec![act(Act::CloseTerminal {
            terminal: TerminalId(32)
        })]
    );
    assert!(a.modal.is_none());
}

#[test]
fn the_key_binding_still_closes_the_focused_terminal() {
    let mut a = app();
    a.on_event(
        ct::Event::Key(ct::KeyEvent::new(
            ct::KeyCode::Char(' '),
            ct::KeyModifiers::ALT,
        )),
        Instant::now(),
    );
    ch(&mut a, 'x');
    assert_eq!(
        confirm_question(&a),
        Some(("Close terminal 1?", &Target::Terminal(TerminalId(31), 1)))
    );
}

#[test]
fn a_target_closed_meanwhile_closes_the_menu() {
    let flash = |a: &App| a.flash.as_ref().map(|(t, _)| t.clone());
    // A space.
    let mut a = app();
    right(&mut a, 14, BUDGET);
    let mut view = two_terminals();
    view.projects.remove(1);
    a.on_server(ServerMsg::SessionView(view));
    assert!(a.context.is_none());
    assert_eq!(flash(&a).as_deref(), Some("space \"budget\" was closed"));
    // A panel.
    let mut a = app();
    right(&mut a, 14, API);
    let mut view = two_terminals();
    view.projects[1].panels.clear();
    a.on_server(ServerMsg::SessionView(view));
    assert!(a.context.is_none());
    assert_eq!(flash(&a).as_deref(), Some("panel \"api\" was closed"));
    // A terminal: silently.
    let mut a = app();
    right(&mut a, T31.0, T31.1);
    a.on_server(ServerMsg::SessionView(mockup()));
    assert!(a.context.is_none());
    assert!(a.flash.is_none());
}

#[test]
fn the_row_keeps_its_hover_background_and_the_bar_is_not_drawn() {
    let mut a = app();
    right(&mut a, 14, API);
    assert_eq!(a.hover, None);
    assert!(a.overlay_open());
    assert_eq!(
        context(&a).row(&a),
        Some(Sel::Panel(ProjectId(2), PanelId(6)))
    );
    let buf = buffer(&a);
    let hover = crate::draw::color(a.theme.hover);
    // The row's cell left of the menu has the hover background.
    assert_eq!(buf[(1, API)].bg, hover);
    assert!(a.bar().is_none());
}

#[test]
fn the_menu_is_drawn_under_a_dialog() {
    let mut a = app();
    right(&mut a, 14, BUDGET);
    ch(&mut a, '3');
    assert!(a.context.is_none(), "a dialog replaces the menu");
}

#[test]
fn the_floating_bar_opens_no_menu() {
    let mut a = app();
    a.on_server(ServerMsg::Notice {
        level: deck_core::protocol::NoticeLevel::Info,
        text: "a message that covers cells".into(),
    });
    let rect = a.bar().unwrap().rect;
    right(&mut a, rect.x + 3, rect.y);
    assert!(a.context.is_none());
}
