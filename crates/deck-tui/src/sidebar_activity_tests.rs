//! The activity line under the rows of panels and terminals with an agent: rows, drawing,
//! keys, mouse, drags and scrolling.

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
use crate::sidebar::{Hover, Row, Sel, collapsed_rows, rows};

const LEFT: ct::MouseButton = ct::MouseButton::Left;

fn set(view: &mut SessionView, terminal: u64, status: AgentStatus, activity: Option<&str>) {
    let found = view
        .projects
        .iter_mut()
        .flat_map(|p| &mut p.panels)
        .flat_map(|p| &mut p.terminals)
        .find(|t| t.id == TerminalId(terminal))
        .unwrap();
    let mut state = agent(AgentKind::Claude, status);
    state.activity = activity.map(str::to_string);
    found.agent = Some(state);
}

/// `mockup()` with an agent in `editor` (3, active) and in `tests` (5, waiting). Rows, with no
/// gap: 2 terminal-multiplexer, 3 editor, 4 its line, 5 claude, 6 tests, 7 its line, 8 budget,
/// 9 api.
fn view() -> SessionView {
    let mut view = mockup();
    set(
        &mut view,
        31,
        AgentStatus::Working,
        Some("running cargo test"),
    );
    set(&mut view, 51, AgentStatus::Waiting, None);
    view
}

fn app_with(view: SessionView, width: u16, height: u16) -> App {
    let config = Config {
        sidebar_width: width,
        ..Config::default()
    };
    let mut app = App::new(&config, (100, height), &[]);
    app.on_server(ServerMsg::SessionView(view));
    let mut f = frame(true, (40, 10), vec![]);
    f.terminal = TerminalId(31);
    app.on_server(ServerMsg::Frame(f));
    app
}

fn app() -> App {
    app_with(view(), 28, 24)
}

fn buffer(a: &App) -> Buffer {
    let mut t = Terminal::new(TestBackend::new(a.size.0, a.size.1)).unwrap();
    t.draw(|f| draw(f, a)).unwrap();
    t.backend().buffer().clone()
}

fn line(buf: &Buffer, y: u16, to: u16) -> String {
    (0..to).map(|x| buf[(x, y)].symbol()).collect()
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

fn click(a: &mut App, row: u16) -> Vec<Out> {
    mouse(a, ct::MouseEventKind::Down(LEFT), 8, row);
    mouse(a, ct::MouseEventKind::Up(LEFT), 8, row)
}

fn key(a: &mut App, c: ct::KeyCode) -> Vec<Out> {
    a.on_event(
        ct::Event::Key(ct::KeyEvent::new(c, ct::KeyModifiers::NONE)),
        Instant::now(),
    )
}

fn panel(id: u64) -> Sel {
    Sel::Panel(ProjectId(1), PanelId(id))
}

fn kinds(rows: &[Row]) -> Vec<String> {
    rows.iter()
        .map(|r| match r {
            Row::Project(_) => "P".to_string(),
            Row::Panel(_, p) => format!("p{}", p.0),
            Row::Terminal(_, _, t) => format!("t{}", t.0),
            Row::Activity(Sel::Panel(_, p)) => format!("a{}", p.0),
            Row::Activity(Sel::Terminal(_, _, t)) => format!("a{}", t.0),
            Row::Activity(Sel::Project(_)) => "a?".to_string(),
            Row::NoPanels => "n".into(),
            Row::Gap => "_".into(),
            Row::Rule => "-".into(),
        })
        .collect()
}

#[test]
fn a_single_terminal_panel_with_an_agent_gets_a_line() {
    let r = kinds(&rows(&view(), 0));
    assert_eq!(
        r[..7],
        ["P", "p3", "a3", "p4", "p5", "a5", "P"],
        "none under claude, which has no agent"
    );
}

#[test]
fn a_grid_panel_gets_a_line_under_each_agent_terminal_and_none_under_its_own_row() {
    let mut view = mockup();
    let panel = &mut view.projects[0].panels[0];
    let mut second = panel.terminals[0].clone();
    second.id = TerminalId(32);
    let mut third = panel.terminals[0].clone();
    third.id = TerminalId(33);
    panel.terminals.extend([second, third]);
    set(&mut view, 32, AgentStatus::Idle, None);
    set(&mut view, 33, AgentStatus::Quiet, None);
    let r = kinds(&rows(&view, 0));
    assert_eq!(r[..8], ["P", "p3", "t31", "t32", "a32", "t33", "a33", "p4"]);
}

#[test]
fn the_collapsed_sidebar_and_a_collapsed_space_have_no_lines() {
    let any = |rows: &[Row]| rows.iter().any(|r| matches!(r, Row::Activity(_)));
    assert!(!any(&collapsed_rows(&view())));
    let mut view = view();
    view.projects[0].collapsed = true;
    assert!(!any(&rows(&view, 0)));
    let mut a = app();
    a.set_sidebar_collapsed(true);
    assert!(!any(&a.sidebar_rows()));
}

#[test]
fn a_line_is_not_selectable() {
    let activity = Row::Activity(panel(3));
    assert_eq!(activity.sel(), None);
    assert_eq!(activity.owner(), Some(panel(3)));
}

#[test]
fn the_line_shows_the_activity_else_the_status_word() {
    let a = app();
    let buf = buffer(&a);
    assert_eq!(line(&buf, 4, 29), "    running cargo test      │");
    assert_eq!(line(&buf, 7, 29), "    needs you               │");
    // The rest of the rows is as before: one line, with the line after the row of its owner.
    assert_eq!(line(&buf, 5, 29), " ❯  claude                  │");
    assert_eq!(line(&buf, 6, 29), " ✻  tests                !  │");
}

#[test]
fn the_status_words_are_the_fallback() {
    for (status, word) in [
        (AgentStatus::Working, "working"),
        (AgentStatus::Waiting, "needs you"),
        (AgentStatus::Idle, "ready"),
        (AgentStatus::Active, "printing"),
        (AgentStatus::Quiet, "quiet"),
    ] {
        let mut view = mockup();
        set(&mut view, 41, status, None);
        let buf = buffer(&app_with(view, 28, 24));
        assert_eq!(line(&buf, 5, 29)[4..], format!("{word:<24}│"));
    }
}

#[test]
fn a_long_line_is_cut_with_an_ellipsis_before_the_margin() {
    let mut view = mockup();
    set(
        &mut view,
        41,
        AgentStatus::Working,
        Some("dispatching worker: implement sub-spec 02"),
    );
    let buf = buffer(&app_with(view, 28, 24));
    // Starts at column 4, ends before the one-cell margin: 23 cells.
    assert_eq!(line(&buf, 5, 29), "    dispatching worker: im… │");
}

#[test]
fn wide_characters_count_two_cells_and_control_characters_are_dropped() {
    let mut view = mockup();
    set(
        &mut view,
        41,
        AgentStatus::Working,
        Some("日本語日本語日本語日本語日本語日本語"),
    );
    set(&mut view, 51, AgentStatus::Working, Some("a\u{1b}[31mb\tc"));
    let a = app_with(view, 28, 24);
    let buf = buffer(&a);
    // 23 cells: eleven wide characters and the ellipsis (the buffer pads each wide one).
    let text = line(&buf, 5, 28).replace(' ', "");
    assert_eq!(text, "日本語日本語日本語日本…");
    assert_eq!(line(&buf, 7, 29), "    a[31mbc                 │");
}

#[test]
fn a_sidebar_too_narrow_for_the_line_shows_nothing_on_it() {
    let mut view = mockup();
    set(&mut view, 41, AgentStatus::Working, Some("hello"));
    let a = app_with(view, 28, 24);
    assert_eq!(a.sidebar_rows().len(), 10, "the row stays");
    let drawn = |columns: u16| {
        let area = ratatui::layout::Rect::new(0, 0, columns, 24);
        let mut buf = Buffer::empty(ratatui::layout::Rect::new(0, 0, columns + 1, 24));
        crate::sidebar::draw(&mut buf, area, &a, &a.theme);
        line(&buf, 5, columns)
    };
    // The line starts at column 4 and keeps the margin: 5 columns leave no cell for it.
    assert_eq!(drawn(5).trim(), "");
    assert_eq!(drawn(6), "    … ");
    assert_eq!(drawn(7), "    h… ");
}

#[test]
fn the_line_is_muted_and_a_waiting_agents_is_a_warning() {
    // The active panel has the band; look at the others.
    let mut view = view();
    view.active_panel = Some(PanelId(4));
    view.projects[0].active_panel = Some(PanelId(4));
    let a = app_with(view, 28, 24);
    let buf = buffer(&a);
    assert_eq!(buf[(4, 4)].fg, color(a.theme.muted));
    assert_eq!(buf[(4, 7)].fg, color(a.theme.warning));
    assert!(!buf[(4, 4)].modifier.contains(Modifier::BOLD));
}

#[test]
fn on_the_band_the_line_has_the_band_colours_and_is_not_bold() {
    let a = app();
    let buf = buffer(&a);
    // The active panel's row and its line share the band.
    assert!(buf[(4, 3)].modifier.contains(Modifier::BOLD));
    assert!(buf[(4, 3)].modifier.contains(Modifier::REVERSED));
    assert!(buf[(4, 4)].modifier.contains(Modifier::REVERSED));
    assert!(!buf[(4, 4)].modifier.contains(Modifier::BOLD));
    assert!((0..28).all(|x| buf[(x, 4)].modifier.contains(Modifier::REVERSED)));
    // The selected row in sidebar focus too, and the active one then has none.
    let mut a = app();
    a.enter_sidebar();
    a.selection = Some(panel(5));
    let buf = buffer(&a);
    assert!((0..28).all(|x| buf[(x, 7)].modifier.contains(Modifier::REVERSED)));
    assert!(!buf[(4, 4)].modifier.contains(Modifier::REVERSED));
}

#[test]
fn hover_covers_the_row_and_its_line() {
    let mut a = app();
    mouse(&mut a, ct::MouseEventKind::Moved, 8, 7);
    assert_eq!(a.hover, Some(Hover::Row(panel(5))));
    let buf = buffer(&a);
    let hover = color(a.theme.hover);
    assert!((0..28).all(|x| buf[(x, 6)].bg == hover));
    assert!((0..28).all(|x| buf[(x, 7)].bg == hover));
    assert!((0..28).all(|x| buf[(x, 8)].bg != hover));
}

#[test]
fn the_tree_glyph_continues_while_another_terminal_follows() {
    let mut view = mockup();
    let panel = &mut view.projects[0].panels[0];
    let mut second = panel.terminals[0].clone();
    second.id = TerminalId(32);
    panel.terminals.push(second);
    set(&mut view, 31, AgentStatus::Working, Some("first"));
    set(&mut view, 32, AgentStatus::Idle, Some("last"));
    let buf = buffer(&app_with(view, 28, 24));
    // Rows: 3 editor, 4 first terminal, 5 its line, 6 last terminal, 7 its line.
    assert_eq!(line(&buf, 5, 29), "  │    first                │");
    assert_eq!(line(&buf, 7, 29), "       last                 │");
    assert_eq!(buf[(2, 6)].symbol(), "└");
}

fn focus_pair(panel: u64, terminal: u64) -> Vec<Out> {
    vec![
        Out::Send(ClientMsg::Action(Act::FocusPanel {
            panel: PanelId(panel),
        })),
        Out::Send(ClientMsg::Action(Act::FocusTerminal {
            terminal: TerminalId(terminal),
        })),
    ]
}

#[test]
fn the_keyboard_skips_the_line() {
    let mut a = app();
    a.enter_sidebar();
    assert_eq!(a.selection, Some(panel(3)));
    key(&mut a, ct::KeyCode::Down);
    assert_eq!(a.selection, Some(panel(4)));
    key(&mut a, ct::KeyCode::Down);
    assert_eq!(a.selection, Some(panel(5)));
    key(&mut a, ct::KeyCode::Down);
    assert_eq!(a.selection, Some(Sel::Project(ProjectId(2))));
    key(&mut a, ct::KeyCode::Up);
    key(&mut a, ct::KeyCode::Up);
    assert_eq!(a.selection, Some(panel(4)));
}

#[test]
fn hover_and_click_on_the_line_act_on_its_owner() {
    let mut a = app();
    mouse(&mut a, ct::MouseEventKind::Moved, 8, 4);
    assert_eq!(a.hover, Some(Hover::Row(panel(3))));
    assert_eq!(a.hover_at(30, 4), None, "outside the sidebar");
    let outs = click(&mut a, 7);
    assert_eq!(
        outs,
        vec![Out::Send(ClientMsg::Action(Act::FocusPanel {
            panel: PanelId(5)
        }))]
    );
    assert_eq!(a.selection, Some(panel(5)));
}

#[test]
fn a_click_on_a_terminals_line_focuses_that_terminal() {
    let mut view = mockup();
    let panel = &mut view.projects[0].panels[0];
    let mut second = panel.terminals[0].clone();
    second.id = TerminalId(32);
    panel.terminals.push(second);
    set(&mut view, 32, AgentStatus::Idle, None);
    let mut a = app_with(view, 28, 24);
    // Rows: 3 editor, 4 first terminal, 5 second terminal, 6 its line.
    assert_eq!(click(&mut a, 6), focus_pair(3, 32));
}

#[test]
fn a_right_click_on_the_line_opens_the_menu_of_its_owner() {
    let mut a = app();
    mouse(
        &mut a,
        ct::MouseEventKind::Down(ct::MouseButton::Right),
        8,
        7,
    );
    let menu = a.context.as_ref().expect("a menu");
    assert_eq!(menu.row(), Some(panel(5)));
    assert!(matches!(menu.target, ContextTarget::Panel(PanelId(5))));
}

#[test]
fn a_drag_that_starts_on_the_line_drags_its_owner_and_dims_both_rows() {
    let mut a = app();
    // The line of `tests` dragged over `editor`.
    mouse(&mut a, ct::MouseEventKind::Down(LEFT), 8, 7);
    mouse(&mut a, ct::MouseEventKind::Drag(LEFT), 8, 3);
    assert_eq!(a.row_drag.map(|d| d.item), Some(panel(5)));
    assert_eq!(a.row_drag.map(|d| d.to), Some(0));
    let buf = buffer(&a);
    assert_eq!(buf[(4, 7)].fg, color(a.theme.muted));
    assert_eq!(buf[(4, 6)].fg, color(a.theme.muted));
    // The target is `editor`: its line is shifted two columns with it, without a second caret.
    assert_eq!(a.drag_target_sel(), Some(panel(3)));
    assert_eq!(line(&buf, 4, 29), "      running cargo test    │");
}

#[test]
fn a_pointer_over_a_line_while_dragging_targets_its_owner() {
    let mut a = app();
    mouse(&mut a, ct::MouseEventKind::Down(LEFT), 8, 5);
    mouse(&mut a, ct::MouseEventKind::Drag(LEFT), 8, 4);
    assert_eq!(
        a.row_drag.map(|d| d.to),
        Some(0),
        "`claude` over the line of `editor`"
    );
}

#[test]
fn the_active_panel_scrolls_in_with_its_line() {
    // 9 rows: the header, the bar and its spacer leave 5 for the list.
    let mut view = view();
    view.active_panel = Some(PanelId(5));
    view.projects[0].active_panel = Some(PanelId(5));
    let mut a = app_with(view, 28, 9);
    a.sidebar_offset = 0;
    a.ensure_visible();
    // Rows: 0 space, 1 editor, 2 its line, 3 claude, 4 tests, 5 its line: the line of the
    // last visible row needs the list to scroll by one.
    assert_eq!(a.sidebar_offset, 1);
    let buf = buffer(&a);
    assert_eq!(line(&buf, 5, 29), " ✻  tests                !  │");
    assert_eq!(line(&buf, 6, 29), "    needs you               │");
}

#[test]
fn an_empty_or_control_only_activity_falls_back_to_the_status_word() {
    for activity in ["", "\u{1b}\t"] {
        let mut view = mockup();
        set(&mut view, 41, AgentStatus::Idle, Some(activity));
        let buf = buffer(&app_with(view, 28, 24));
        assert_eq!(line(&buf, 5, 29), "    ready                   │");
    }
}
