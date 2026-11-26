//! The terminal rows under a panel with two or more terminals: rows, drawing, keys, mouse,
//! the context menu and drags.

use std::time::Instant;

use deck_core::action::Action as Act;
use deck_core::agents::{AgentKind, AgentStatus};
use deck_core::config::Config;
use deck_core::model::{PanelId, ProjectId, SessionView, TerminalId};
use deck_core::protocol::{ClientMsg, ServerMsg};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event as ct;
use ratatui::style::Modifier;

use crate::app::{App, Out};
use crate::context::ContextTarget;
use crate::draw::{color, draw};
use crate::icons_tests::agent;
use crate::mirror::tests::{frame, mockup};
use crate::modal::{Modal, Target};
use crate::sidebar::{Hover, Reorder, Row, Sel, rows, rows_with};

const LEFT: ct::MouseButton = ct::MouseButton::Left;
const RIGHT: ct::MouseButton = ct::MouseButton::Right;

// The rows with `grid()` (no gap): 2 terminal-multiplexer, 3 editor (active, terminals 31 to
// 33), 4 zsh, 5 Write tests, 6 its activity line, 7 main.rs, 8 claude, 9 tests, 10 budget,
// 11 api.
const EDITOR: u16 = 3;
const T31: u16 = 4;
const T32: u16 = 5;
/// The activity line under the waiting agent's terminal row.
const A32: u16 = 6;
const T33: u16 = 7;
const CLAUDE: u16 = 8;
const BUDGET: u16 = 10;

/// `mockup()` with three terminals in panel 3 (the active one): a shell, an agent waiting
/// for the user (focused) and an editor.
fn grid() -> SessionView {
    let mut view = mockup();
    let panel = &mut view.projects[0].panels[0];
    let mut first = panel.terminals[0].clone();
    first.program = "zsh".into();
    let mut second = first.clone();
    second.id = TerminalId(32);
    second.program = "node".into();
    second.title = Some("✳ Write tests".into());
    second.agent = Some(agent(AgentKind::Claude, AgentStatus::Waiting));
    let mut third = first.clone();
    third.id = TerminalId(33);
    third.program = "nvim".into();
    third.title = Some("main.rs".into());
    panel.terminals = vec![first, second, third];
    panel.focused_terminal = Some(TerminalId(32));
    panel.ratios = deck_core::layout::Ratios::default_for(3);
    view
}

fn app_with(view: SessionView, width: u16) -> App {
    let config = Config {
        sidebar_width: width,
        ..Config::default()
    };
    let mut app = App::new(&config, (100, 24), &[]);
    app.on_server(ServerMsg::SessionView(view));
    let mut f = frame(true, (40, 10), vec![]);
    f.terminal = TerminalId(32);
    app.on_server(ServerMsg::Frame(f));
    app
}

fn app() -> App {
    app_with(grid(), 28)
}

fn mouse(a: &mut App, kind: ct::MouseEventKind, column: u16, row: u16) -> Vec<Out> {
    a.on_event(
        ct::Event::Mouse(ct::MouseEvent {
            kind,
            column,
            row,
            modifiers: ct::KeyModifiers::NONE,
        }),
        Instant::now(),
    )
}

fn down(a: &mut App, row: u16) -> Vec<Out> {
    mouse(a, ct::MouseEventKind::Down(LEFT), 8, row)
}

fn up(a: &mut App, row: u16) -> Vec<Out> {
    mouse(a, ct::MouseEventKind::Up(LEFT), 8, row)
}

fn click(a: &mut App, row: u16) -> Vec<Out> {
    down(a, row);
    up(a, row)
}

fn right(a: &mut App, row: u16) -> Vec<Out> {
    mouse(a, ct::MouseEventKind::Down(RIGHT), 8, row)
}

fn key(a: &mut App, c: ct::KeyCode) -> Vec<Out> {
    a.on_event(
        ct::Event::Key(ct::KeyEvent::new(c, ct::KeyModifiers::NONE)),
        Instant::now(),
    )
}

fn ch(a: &mut App, c: char) -> Vec<Out> {
    key(a, ct::KeyCode::Char(c))
}

fn send(act: Act) -> Out {
    Out::Send(ClientMsg::Action(act))
}

fn focus_pair(panel: u64, terminal: u64) -> Vec<Out> {
    vec![
        send(Act::FocusPanel {
            panel: PanelId(panel),
        }),
        send(Act::FocusTerminal {
            terminal: TerminalId(terminal),
        }),
    ]
}

fn buffer(a: &App) -> Buffer {
    let mut t = Terminal::new(TestBackend::new(a.size.0, a.size.1)).unwrap();
    t.draw(|f| draw(f, a)).unwrap();
    t.backend().buffer().clone()
}

fn line(buf: &Buffer, y: u16, to: u16) -> String {
    (0..to).map(|x| buf[(x, y)].symbol()).collect()
}

fn kinds(rows: &[Row]) -> String {
    rows.iter()
        .map(|r| match r {
            Row::Project(_) => "P".to_string(),
            Row::Panel(_, p) => format!("p{}", p.0),
            Row::Terminal(_, _, t) => format!("t{}", t.0),
            Row::Activity(_) => "a".into(),
            Row::NoPanels => "n".into(),
            Row::Gap => "_".into(),
            Row::Rule => "-".into(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn sel_terminal(terminal: u64) -> Sel {
    let panel = if terminal > 40 { terminal / 10 } else { 3 };
    Sel::Terminal(ProjectId(1), PanelId(panel), TerminalId(terminal))
}

#[test]
fn only_panels_with_two_terminals_or_more_have_terminal_rows() {
    let single = rows(&mockup(), 0);
    assert!(!kinds(&single).contains('t'));
    let mut two = mockup();
    let mut extra = two.projects[0].panels[0].terminals[0].clone();
    extra.id = TerminalId(32);
    two.projects[0].panels[0].terminals.push(extra);
    assert_eq!(
        kinds(&rows(&two, 0)).split(' ').take(5).collect::<Vec<_>>(),
        ["P", "p3", "t31", "t32", "p4"]
    );
    assert_eq!(
        kinds(&rows(&grid(), 0))
            .split(' ')
            .take(8)
            .collect::<Vec<_>>(),
        ["P", "p3", "t31", "t32", "a", "t33", "p4", "p5"]
    );
}

#[test]
fn a_collapsed_space_hides_its_terminal_rows() {
    let mut view = grid();
    view.projects[0].collapsed = true;
    assert_eq!(kinds(&rows(&view, 0)).split(' ').next(), Some("P"));
    assert!(!kinds(&rows(&view, 0)).contains('t'));
}

#[test]
fn a_panel_reorder_moves_the_terminal_rows_along() {
    let reorder = Reorder {
        item: Sel::Panel(ProjectId(1), PanelId(3)),
        to: 2,
    };
    let moved = kinds(&rows_with(&grid(), 0, Some(reorder)));
    assert_eq!(
        moved.split(' ').take(8).collect::<Vec<_>>(),
        ["P", "p4", "p5", "p3", "t31", "t32", "a", "t33"]
    );
}

#[test]
fn adding_and_closing_terminals_changes_the_rows_at_once() {
    let mut a = app();
    assert_eq!(a.sidebar_rows().len(), 13);
    let mut view = grid();
    view.projects[0].panels[0].terminals.pop();
    a.on_server(ServerMsg::SessionView(view));
    assert_eq!(a.sidebar_rows().len(), 12);
    let mut view = grid();
    view.projects[0].panels[0].terminals.truncate(1);
    a.on_server(ServerMsg::SessionView(view));
    assert!(!kinds(&a.sidebar_rows()).contains('t'));
}

#[test]
fn terminal_rows_are_drawn_under_their_panel() {
    let a = app();
    let buf = buffer(&a);
    // The panel row has no count; its status is the most urgent of its terminals.
    assert_eq!(line(&buf, EDITOR, 29), " ✻  editor               !  │");
    assert_eq!(line(&buf, T31, 29), "  ├ ❯  zsh                  │");
    // The title without its status glyph, and the agent's own status.
    assert_eq!(line(&buf, T32, 29), "  ├ ✻  Write tests       !  │");
    // The agent's line sits under its terminal's label; the tree continues to the last row.
    assert_eq!(line(&buf, A32, 29), "  │    needs you            │");
    assert_eq!(line(&buf, T33, 29), "  └ ✎  main.rs              │");
    // A single-terminal panel is one row, unchanged.
    assert_eq!(line(&buf, CLAUDE, 29), " ❯  claude                  │");
}

#[test]
fn the_focused_terminal_is_accent_and_bold_and_the_band_stays_on_the_panel() {
    let a = app();
    let buf = buffer(&a);
    let focused = &buf[(7, T32)];
    assert_eq!(focused.fg, color(a.theme.accent));
    assert!(focused.modifier.contains(Modifier::BOLD));
    assert!(!focused.modifier.contains(Modifier::REVERSED));
    let other = &buf[(7, T31)];
    assert_ne!(other.fg, color(a.theme.accent));
    assert!(!other.modifier.contains(Modifier::BOLD));
    // The glyph is muted, the band is on the panel's row.
    assert_eq!(buf[(2, T31)].fg, color(a.theme.muted));
    assert!(buf[(5, EDITOR)].modifier.contains(Modifier::REVERSED));
}

#[test]
fn the_band_goes_to_the_selected_terminal_row() {
    let mut a = app();
    a.enter_sidebar();
    key(&mut a, ct::KeyCode::Down);
    assert_eq!(a.selection, Some(sel_terminal(31)));
    let buf = buffer(&a);
    assert!(buf[(7, T31)].modifier.contains(Modifier::REVERSED));
    assert!(!buf[(5, EDITOR)].modifier.contains(Modifier::REVERSED));
    // The active panel's row is accent and bold without a band.
    assert_eq!(buf[(5, EDITOR)].fg, color(a.theme.accent));
    assert!(buf[(5, EDITOR)].modifier.contains(Modifier::BOLD));
}

#[test]
fn a_narrow_sidebar_cuts_the_label_first_and_then_the_note() {
    let mut view = grid();
    let panel = &mut view.projects[0].panels[0];
    panel.terminals[0].exit_status = Some(1);
    panel.terminals[0].keep = true;
    panel.terminals[1].title = Some("Review the work".into());
    let a = app_with(view, 20);
    let buf = buffer(&a);
    assert_eq!(line(&buf, T31, 21), "  ├ ❯  … · exited 1 │");
    assert_eq!(line(&buf, T32, 21), "  ├ ✻  Review t… !  │");
}

#[test]
fn the_label_falls_back_to_the_program() {
    let mut view = grid();
    view.projects[0].panels[0].terminals[1].title = Some("✳ ".into());
    view.projects[0].panels[0].terminals[2].title = None;
    let a = app_with(view, 28);
    let buf = buffer(&a);
    assert_eq!(line(&buf, T32, 29), "  ├ ✻  node              !  │");
    assert_eq!(line(&buf, T33, 29), "  └ ✎  nvim                 │");
}

#[test]
fn the_selection_moves_across_terminal_rows() {
    let mut a = app();
    a.enter_sidebar();
    assert_eq!(a.selection, Some(Sel::Panel(ProjectId(1), PanelId(3))));
    for terminal in [31, 32, 33] {
        key(&mut a, ct::KeyCode::Down);
        assert_eq!(a.selection, Some(sel_terminal(terminal)));
    }
    key(&mut a, ct::KeyCode::Down);
    assert_eq!(a.selection, Some(Sel::Panel(ProjectId(1), PanelId(4))));
    key(&mut a, ct::KeyCode::Up);
    assert_eq!(a.selection, Some(sel_terminal(33)));
}

#[test]
fn enter_focuses_the_terminal_and_leaves_the_sidebar() {
    let mut a = app();
    a.enter_sidebar();
    key(&mut a, ct::KeyCode::Down);
    let outs = key(&mut a, ct::KeyCode::Enter);
    assert_eq!(outs, focus_pair(3, 31));
    assert!(!a.sidebar_focus);
}

#[test]
fn x_asks_before_closing_a_terminal_and_a_kept_one_that_exited_closes_at_once() {
    let mut a = app();
    a.enter_sidebar();
    key(&mut a, ct::KeyCode::Down);
    key(&mut a, ct::KeyCode::Down);
    assert!(ch(&mut a, 'x').is_empty());
    let Some(Modal::Confirm(c)) = &a.modal else {
        panic!("a confirmation")
    };
    assert_eq!(c.question, "Close terminal 2?");
    assert_eq!(c.target, Target::Terminal(TerminalId(32), 2));
    let mut view = grid();
    view.projects[0].panels[0].terminals[0].exit_status = Some(0);
    view.projects[0].panels[0].terminals[0].keep = true;
    let mut a = app_with(view, 28);
    a.enter_sidebar();
    key(&mut a, ct::KeyCode::Down);
    assert_eq!(
        ch(&mut a, 'x'),
        vec![send(Act::CloseTerminal {
            terminal: TerminalId(31)
        })]
    );
}

#[test]
fn r_renames_the_panel_and_the_space_keys_act_on_its_space() {
    let mut a = app();
    a.enter_sidebar();
    key(&mut a, ct::KeyCode::Down);
    ch(&mut a, 'r');
    assert!(matches!(a.modal, Some(Modal::Prompt(_))));
    let mut a = app();
    a.enter_sidebar();
    key(&mut a, ct::KeyCode::Down);
    assert_eq!(
        ch(&mut a, 'c'),
        vec![send(Act::NewPanel {
            project: ProjectId(1),
            name: None
        })]
    );
    assert_eq!(a.new_target(), Some(ProjectId(1)));
    assert_eq!(
        key(&mut a, ct::KeyCode::Left),
        vec![send(Act::SetProjectCollapsed {
            project: ProjectId(1),
            collapsed: true
        })]
    );
    // Folded, the selection is the space's row.
    assert_eq!(a.selection, Some(Sel::Project(ProjectId(1))));
}

#[test]
fn a_selection_whose_terminal_is_gone_falls_back_to_its_panel() {
    let mut a = app();
    a.enter_sidebar();
    a.selection = Some(sel_terminal(33));
    let mut view = grid();
    view.projects[0].panels[0].terminals.pop();
    a.on_server(ServerMsg::SessionView(view));
    a.normalize_selection();
    assert_eq!(a.selection, Some(Sel::Panel(ProjectId(1), PanelId(3))));
}

#[test]
fn a_click_focuses_the_panel_and_the_terminal() {
    let mut a = app();
    assert_eq!(click(&mut a, T31), focus_pair(3, 31));
    // A terminal of a panel that is not active.
    let mut view = grid();
    view.active_panel = Some(PanelId(4));
    view.projects[0].active_panel = Some(PanelId(4));
    let mut a = app_with(view, 28);
    assert_eq!(click(&mut a, T33), focus_pair(3, 33));
}

#[test]
fn the_pointer_hovers_terminal_rows() {
    let mut a = app();
    mouse(&mut a, ct::MouseEventKind::Moved, 8, T32);
    assert_eq!(a.hover, Some(Hover::Row(sel_terminal(32))));
    let buf = buffer(&a);
    assert_eq!(buf[(20, T32)].bg, color(a.theme.hover));
}

#[test]
fn a_right_click_opens_the_terminal_menu_of_that_terminal() {
    let mut view = grid();
    view.active_panel = Some(PanelId(4));
    view.projects[0].active_panel = Some(PanelId(4));
    let mut a = app_with(view, 28);
    right(&mut a, T33);
    let menu = a.context.as_ref().unwrap();
    assert_eq!(menu.target, ContextTarget::Terminal(TerminalId(33)));
    assert_eq!(menu.row(), Some(sel_terminal(33)));
    // New terminal adds to that panel and makes it active.
    assert_eq!(
        ch(&mut a, '1'),
        vec![
            send(Act::FocusPanel { panel: PanelId(3) }),
            send(Act::NewTerminal {
                panel: Some(PanelId(3))
            })
        ]
    );
}

#[test]
fn a_menu_opened_on_a_grid_terminal_has_no_sidebar_row() {
    let mut a = app();
    a.open_terminal_menu(TerminalId(31), (70, 5));
    assert_eq!(a.context.as_ref().unwrap().row(), None);
}

#[test]
fn a_press_on_a_terminal_row_moved_to_another_row_sends_nothing() {
    let mut a = app();
    down(&mut a, T31);
    assert!(mouse(&mut a, ct::MouseEventKind::Drag(LEFT), 8, T32).is_empty());
    assert!(a.row_press.is_none() && a.row_drag.is_none());
    assert!(up(&mut a, T32).is_empty());
    // The release was swallowed, and the next click works.
    assert_eq!(click(&mut a, T33), focus_pair(3, 33));
}

#[test]
fn a_panel_drag_over_a_terminal_row_targets_its_panel() {
    let mut a = app();
    // `claude` dragged over the second terminal of `editor`: the target is `editor`.
    down(&mut a, CLAUDE);
    mouse(&mut a, ct::MouseEventKind::Drag(LEFT), 8, T32);
    assert_eq!(a.row_drag.map(|d| d.to), Some(0));
    assert_eq!(
        a.drag_target_sel(),
        Some(Sel::Panel(ProjectId(1), PanelId(3)))
    );
    // A space drag over a terminal row is over its space.
    let mut a = app();
    down(&mut a, BUDGET);
    mouse(&mut a, ct::MouseEventKind::Drag(LEFT), 8, T32);
    assert_eq!(a.row_drag.map(|d| d.to), Some(0));
}

#[test]
fn the_terminal_rows_of_a_dragged_panel_are_dimmed() {
    let mut a = app();
    down(&mut a, EDITOR);
    mouse(&mut a, ct::MouseEventKind::Drag(LEFT), 8, CLAUDE);
    let buf = buffer(&a);
    for y in [EDITOR, T31, T32, T33] {
        assert_eq!(buf[(9, y)].fg, color(a.theme.muted), "row {y}");
    }
}

#[test]
fn a_press_on_a_terminal_that_goes_away_ends_the_gesture() {
    let mut a = app();
    down(&mut a, T33);
    let mut view = grid();
    view.projects[0].panels[0].terminals.pop();
    a.on_server(ServerMsg::SessionView(view));
    assert!(a.row_press.is_none());
}

#[test]
fn a_selection_on_a_terminal_row_below_the_list_scrolls_it_into_view() {
    let config = Config {
        sidebar_width: 28,
        ..Config::default()
    };
    // 8 rows: the header, the bar and its spacer leave 4 for the list.
    let mut a = App::new(&config, (100, 8), &[]);
    a.on_server(ServerMsg::SessionView(grid()));
    a.enter_sidebar();
    for _ in 0..3 {
        key(&mut a, ct::KeyCode::Down);
    }
    assert_eq!(a.selection, Some(sel_terminal(33)));
    assert_eq!(a.sidebar_offset, 2);
    // The last terminal row is the list's last line, row 5 of the sidebar.
    assert_eq!(a.hover_at(8, 5), Some(Hover::Row(sel_terminal(33))));
}
