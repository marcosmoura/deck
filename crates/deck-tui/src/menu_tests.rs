//! The New button and menu: hover, drawing and anchoring, keys, mouse routing, actions.

use std::path::PathBuf;
use std::time::Instant;

use deck_core::action::{Action as Act, ActionError, PanelLaunch};
use deck_core::agents::AgentKind;
use deck_core::config::Config;
use deck_core::model::{PanelId, ProjectId, SessionView};
use deck_core::protocol::{ClientMsg, ServerMsg};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event as ct;
use ratatui::style::Modifier;

use crate::app::{App, ERROR_FLASH_FOR, Out};
use crate::draw::{color, draw};
use crate::mirror::tests::{frame, mockup, no_sidebar, panel, project};
use crate::sidebar::{Hover, SIDEBAR_HINTS, Sel};

const LEFT: ct::MouseButton = ct::MouseButton::Left;

fn app_with(config: &Config, size: (u16, u16), view: SessionView) -> App {
    let mut app = App::new(config, size, &[])
        .with_dirs(PathBuf::from("/work/proj"), Some(PathBuf::from("/home/me")));
    app.on_server(ServerMsg::SessionView(view));
    if let Some(terminal) = app.mirror.focused_id() {
        let mut f = frame(true, (40, 10), vec![]);
        f.terminal = terminal;
        app.on_server(ServerMsg::Frame(f));
    }
    app
}

fn config() -> Config {
    Config {
        sidebar_width: 28,
        ..Config::default()
    }
}

fn app() -> App {
    app_with(&config(), (100, 24), mockup())
}

fn key(code: ct::KeyCode, mods: ct::KeyModifiers) -> ct::Event {
    ct::Event::Key(ct::KeyEvent::new(code, mods))
}

fn press(a: &mut App, code: ct::KeyCode) -> Vec<Out> {
    a.on_event(key(code, ct::KeyModifiers::NONE), Instant::now())
}

fn ch(a: &mut App, c: char) -> Vec<Out> {
    let mods = if c.is_uppercase() {
        ct::KeyModifiers::SHIFT
    } else {
        ct::KeyModifiers::NONE
    };
    a.on_event(key(ct::KeyCode::Char(c), mods), Instant::now())
}

fn prefix(a: &mut App) {
    a.on_event(
        key(ct::KeyCode::Char(' '), ct::KeyModifiers::ALT),
        Instant::now(),
    );
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

fn moved(a: &mut App, column: u16, row: u16) -> Vec<Out> {
    mouse(a, ct::MouseEventKind::Moved, column, row)
}

fn down(a: &mut App, column: u16, row: u16) -> Vec<Out> {
    mouse(a, ct::MouseEventKind::Down(LEFT), column, row)
}

fn click(a: &mut App, column: u16, row: u16) -> Vec<Out> {
    let mut outs = down(a, column, row);
    outs.extend(mouse(a, ct::MouseEventKind::Up(LEFT), column, row));
    outs
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

/// `budget` (project 2) is on window row 6 of the mockup; the button is at columns 18..27.
const BUDGET: u16 = 6;
const BUTTON: u16 = 20;

fn open_by_click() -> App {
    let mut a = app();
    moved(&mut a, 10, BUDGET);
    assert!(down(&mut a, BUTTON, BUDGET).is_empty());
    a
}

#[test]
fn the_button_shows_on_the_hovered_project_row_and_follows_the_pointer() {
    let mut a = app();
    assert_eq!(
        text(&buffer(&a), BUDGET, 0, 29),
        " ▾ budget                   │"
    );
    moved(&mut a, 10, BUDGET);
    assert_eq!(a.hover, Some(Hover::ProjectNew(ProjectId(2))));
    assert_eq!(
        text(&buffer(&a), BUDGET, 0, 29),
        " ▾ budget         [ New ▾ ] │"
    );
    // Moving to another project row moves the button.
    moved(&mut a, 3, 2);
    let buf = buffer(&a);
    assert_eq!(text(&buf, BUDGET, 0, 29), " ▾ budget                   │");
    assert!(text(&buf, 2, 0, 29).contains("[ New ▾ ]"));
    // A panel row has none; neither has the panel area.
    moved(&mut a, 3, 3);
    assert!(!text(&buffer(&a), 3, 0, 29).contains("[ New"));
    moved(&mut a, 3, 2);
    moved(&mut a, 50, 5);
    assert_eq!(a.hover, None);
    assert!(!text(&buffer(&a), 2, 0, 29).contains("[ New"));
}

#[test]
fn the_button_is_accent_coloured_and_selection_coloured_under_the_pointer() {
    let mut a = app();
    moved(&mut a, 10, BUDGET);
    let buf = buffer(&a);
    assert_eq!(buf[(BUTTON, BUDGET)].fg, color(a.theme.accent));
    assert_eq!(buf[(BUTTON, BUDGET)].bg, color(a.theme.sidebar_bg));
    moved(&mut a, BUTTON, BUDGET);
    let buf = buffer(&a);
    let cell = &buf[(BUTTON, BUDGET)];
    let reversed = cell.modifier.contains(Modifier::REVERSED);
    let themed = cell.bg == color(a.theme.selection_bg) && cell.fg == color(a.theme.selection_fg);
    assert!(reversed || themed, "{cell:?}");
}

#[test]
fn the_button_wins_the_right_edge_and_takes_its_room_off_the_name() {
    let view = SessionView {
        projects: vec![
            project(1, "terminal-multiplexer", true, vec![panel(3, "a", 1)]),
            project(2, "budget", false, vec![panel(6, "api", 1)]),
        ],
        active_project: Some(ProjectId(2)),
        active_panel: Some(PanelId(6)),
        ..mockup()
    };
    let mut a = app_with(&config(), (100, 24), view);
    // The collapsed project shows its count while idle.
    assert_eq!(text(&buffer(&a), 2, 0, 29), " ▸ terminal-multiplexer   1 │");
    moved(&mut a, 10, 2);
    assert_eq!(text(&buffer(&a), 2, 0, 29), " ▸ terminal-mu…   [ New ▾ ] │");
    // The count reappears when the hover ends.
    moved(&mut a, 60, 2);
    assert_eq!(text(&buffer(&a), 2, 0, 29), " ▸ terminal-multiplexer   1 │");
}

#[test]
fn the_button_shows_on_the_selected_project_in_sidebar_focus() {
    let mut a = app();
    prefix(&mut a);
    press(&mut a, ct::KeyCode::Tab);
    assert_eq!(a.selection, Some(Sel::Panel(ProjectId(1), PanelId(3))));
    assert!(!text(&buffer(&a), 2, 0, 29).contains("[ New"));
    press(&mut a, ct::KeyCode::Up);
    press(&mut a, ct::KeyCode::Up);
    assert_eq!(a.selection, Some(Sel::Project(ProjectId(1))));
    assert!(text(&buffer(&a), 2, 0, 29).contains("[ New ▾ ]"));
}

#[test]
fn hovering_causes_a_redraw_only_when_the_hovered_row_changes() {
    let mut a = app();
    moved(&mut a, 10, BUDGET);
    assert!(a.take_dirty());
    moved(&mut a, 11, BUDGET);
    assert!(!a.take_dirty());
    // Getting on the button changes its colours; moving along it does not.
    moved(&mut a, BUTTON, BUDGET);
    assert!(a.take_dirty());
    moved(&mut a, BUTTON + 1, BUDGET);
    assert!(!a.take_dirty());
    moved(&mut a, 11, 2);
    assert!(a.take_dirty());
}

#[test]
fn a_click_on_the_button_opens_the_menu_and_a_click_on_the_name_folds() {
    let mut a = open_by_click();
    let menu = a.menu.as_ref().unwrap();
    assert_eq!((menu.project, menu.selected), (ProjectId(2), 0));
    assert!(a.overlay_open());
    // The click on the name of another project still folds it.
    a.menu = None;
    moved(&mut a, 10, 2);
    assert_eq!(
        click(&mut a, 6, 2),
        [act(Act::SetProjectCollapsed {
            project: ProjectId(1),
            collapsed: true
        })]
    );
    assert!(a.menu.is_none());
    // Without the button on show, its place is the row's.
    let mut a = app();
    assert_eq!(click(&mut a, BUTTON, BUDGET).len(), 1);
    assert!(a.menu.is_none());
}

#[test]
fn the_menu_is_drawn_below_the_row_with_its_right_edge_on_the_sidebars_last_column() {
    let a = open_by_click();
    let buf = buffer(&a);
    let rows: Vec<String> = (BUDGET..BUDGET + 9).map(|y| text(&buf, y, 0, 29)).collect();
    assert_eq!(
        rows,
        [
            " ▾ budget         [ New ▾ ] │",
            "   ❯ a╭─ New in budget ────╮│",
            " ▾ emp│ ▌ Terminal       1 ││",
            "   no │   Claude         2 ││",
            " ▸ dot│   OpenCode       3 ││",
            "      │   Codex          4 ││",
            "      │ ⏎ choose  esc close││",
            "      ╰────────────────────╯│",
            "                            │",
        ]
    );
}

#[test]
fn the_highlighted_row_uses_the_selection_colours_and_the_hint_is_muted() {
    let a = open_by_click();
    let buf = buffer(&a);
    let on = &buf[(10, 8)];
    let reversed = on.modifier.contains(Modifier::REVERSED);
    let themed = on.bg == color(a.theme.selection_bg) && on.fg == color(a.theme.selection_fg);
    assert!(reversed || themed);
    assert_eq!(buf[(10, 12)].fg, color(a.theme.muted));
    assert_eq!(buf[(7, 7)].fg, color(a.theme.border));
    assert_eq!(buf[(10, 7)].fg, color(a.theme.border));
}

#[test]
fn the_menu_is_wide_enough_for_a_long_project_name_and_never_wider_than_the_window() {
    let view = SessionView {
        projects: vec![project(
            1,
            "terminal-multiplexer",
            false,
            vec![panel(3, "a", 1)],
        )],
        active_project: Some(ProjectId(1)),
        active_panel: Some(PanelId(3)),
        ..mockup()
    };
    let mut a = app_with(&config(), (100, 24), view.clone());
    prefix(&mut a);
    ch(&mut a, 'N');
    let rect = a.menu_rect().unwrap();
    assert_eq!(rect.width, 20 + 13);
    assert!(
        text(&buffer(&a), rect.y, rect.x, rect.x + rect.width)
            .contains("New in terminal-multiplexer")
    );
    let mut a = app_with(&config(), (24, 24), view);
    prefix(&mut a);
    ch(&mut a, 'N');
    let rect = a.menu_rect().unwrap();
    assert!(rect.x + rect.width <= 24, "{rect:?}");
}

#[test]
fn the_menu_flips_above_the_row_near_the_bottom_of_the_window() {
    let mut a = app_with(&config(), (100, 16), mockup());
    // `dotfiles` is on row 10; below it there are 4 rows left, the menu needs 7.
    moved(&mut a, 10, 10);
    down(&mut a, BUTTON, 10);
    let rect = a.menu_rect().unwrap();
    // The bottom border is on the row above the project row.
    assert_eq!((rect.y, rect.y + rect.height - 1), (3, 9));
    let buf = buffer(&a);
    assert!(text(&buf, 9, rect.x, rect.x + rect.width).starts_with('╰'));
    assert!(text(&buf, 3, rect.x, rect.x + rect.width).starts_with('╭'));
}

#[test]
fn the_menu_stays_inside_a_small_window_and_over_a_narrow_sidebar() {
    for size in [(40, 12), (40, 9), (20, 5)] {
        let config = Config {
            sidebar_width: 20,
            ..Config::default()
        };
        let mut a = app_with(&config, size, mockup());
        for project in [1, 2, 7, 8] {
            a.menu = None;
            a.open_new_menu(ProjectId(project));
            let rect = a.menu_rect().unwrap();
            assert!(rect.x + rect.width <= size.0, "{size:?} {rect:?}");
            assert!(rect.y + rect.height < size.1, "{size:?} {rect:?}");
            buffer(&a);
        }
    }
    // At the minimum sidebar width the menu extends over the panel area.
    let config = Config {
        sidebar_width: 20,
        ..Config::default()
    };
    let mut a = app_with(&config, (100, 24), mockup());
    a.open_new_menu(ProjectId(2));
    let rect = a.menu_rect().unwrap();
    assert_eq!((rect.x, rect.y, rect.width), (0, 7, 22));
}

#[test]
fn with_the_sidebar_hidden_the_menu_is_at_the_top_right_of_the_window() {
    let mut a = app_with(&no_sidebar(), (100, 24), mockup());
    prefix(&mut a);
    ch(&mut a, 'N');
    let rect = a.menu_rect().unwrap();
    assert_eq!((rect.x, rect.y, rect.width, rect.height), (67, 0, 33, 7));
    assert!(text(&buffer(&a), 0, 67, 100).contains("New in terminal-multiplexer"));
}

#[test]
fn prefix_n_opens_the_menu_for_the_active_project_and_nothing_without_one() {
    let mut a = app();
    prefix(&mut a);
    assert!(ch(&mut a, 'N').is_empty());
    assert_eq!(a.menu.as_ref().unwrap().project, ProjectId(1));
    let empty = SessionView {
        projects: vec![],
        active_project: None,
        active_panel: None,
        ..mockup()
    };
    let mut a = app_with(&config(), (100, 24), empty);
    prefix(&mut a);
    ch(&mut a, 'N');
    assert!(a.menu.is_none());
}

#[test]
fn n_in_sidebar_focus_opens_the_menu_for_the_selected_project_or_the_project_of_the_panel() {
    let mut a = app();
    prefix(&mut a);
    press(&mut a, ct::KeyCode::Tab);
    for _ in 0..3 {
        press(&mut a, ct::KeyCode::Down);
    }
    assert_eq!(a.selection, Some(Sel::Project(ProjectId(2))));
    ch(&mut a, 'n');
    assert_eq!(a.menu.as_ref().unwrap().project, ProjectId(2));
    press(&mut a, ct::KeyCode::Esc);
    assert!(a.menu.is_none());
    // On a panel row, the panel's project.
    press(&mut a, ct::KeyCode::Down);
    assert_eq!(a.selection, Some(Sel::Panel(ProjectId(2), PanelId(6))));
    ch(&mut a, 'n');
    assert_eq!(a.menu.as_ref().unwrap().project, ProjectId(2));
}

#[test]
fn the_sidebar_hints_list_the_menu() {
    let mut a = app();
    prefix(&mut a);
    press(&mut a, ct::KeyCode::Tab);
    let buf = buffer(&a);
    let bar = text(&buf, 23, 0, 100);
    assert!(
        bar.trim_end()
            .ends_with("↑↓ move  ⏎ open  ←→ fold  c new  n menu  r rename  x close  esc back"),
        "{bar}"
    );
    assert!(SIDEBAR_HINTS.contains("n menu"));
}

#[test]
fn the_keys_move_and_choose() {
    let mut a = open_by_click();
    let sel = |a: &App| a.menu.as_ref().unwrap().selected;
    ch(&mut a, 'j');
    press(&mut a, ct::KeyCode::Down);
    assert_eq!(sel(&a), 2);
    a.on_event(
        key(ct::KeyCode::Char('p'), ct::KeyModifiers::CONTROL),
        Instant::now(),
    );
    assert_eq!(sel(&a), 1);
    a.on_event(
        key(ct::KeyCode::Char('n'), ct::KeyModifiers::CONTROL),
        Instant::now(),
    );
    ch(&mut a, 'k');
    press(&mut a, ct::KeyCode::Up);
    press(&mut a, ct::KeyCode::Up);
    assert_eq!(sel(&a), 0);
    for _ in 0..6 {
        press(&mut a, ct::KeyCode::Down);
    }
    assert_eq!(sel(&a), 3);
    // Other letters, the prefix and the sidebar's keys do nothing while it is open.
    assert!(ch(&mut a, 'c').is_empty());
    prefix(&mut a);
    assert!(ch(&mut a, 'x').is_empty());
    assert!(!a.armed && a.modal.is_none() && a.menu.is_some());
    assert_eq!(
        press(&mut a, ct::KeyCode::Enter),
        [act(Act::NewPanelWith {
            project: ProjectId(2),
            launch: PanelLaunch::Agent(AgentKind::Codex)
        })]
    );
    assert!(a.menu.is_none());
}

#[test]
fn the_digits_choose_the_items_and_esc_and_q_close() {
    let launch = |kind| {
        act(Act::NewPanelWith {
            project: ProjectId(2),
            launch: PanelLaunch::Agent(kind),
        })
    };
    let mut a = open_by_click();
    assert_eq!(
        ch(&mut a, '1'),
        [act(Act::NewPanel {
            project: ProjectId(2),
            name: None
        })]
    );
    let mut a = open_by_click();
    assert_eq!(ch(&mut a, '2'), [launch(AgentKind::Claude)]);
    let mut a = open_by_click();
    assert_eq!(ch(&mut a, '3'), [launch(AgentKind::OpenCode)]);
    let mut a = open_by_click();
    assert_eq!(ch(&mut a, '4'), [launch(AgentKind::Codex)]);
    assert!(a.menu.is_none());
    let mut a = open_by_click();
    assert!(ch(&mut a, '5').is_empty() && a.menu.is_some());
    press(&mut a, ct::KeyCode::Esc);
    assert!(a.menu.is_none());
    let mut a = open_by_click();
    ch(&mut a, 'q');
    assert!(a.menu.is_none());
}

#[test]
fn an_accepted_launch_leaves_the_sidebar_focus_and_a_refused_one_does_not() {
    for (result, left) in [
        (Ok(()), true),
        (
            Err(ActionError::ProgramNotFound {
                program: "claude".into(),
                kind: AgentKind::Claude,
                key: "command".into(),
            }),
            false,
        ),
    ] {
        let mut a = app();
        prefix(&mut a);
        press(&mut a, ct::KeyCode::Tab);
        ch(&mut a, 'n');
        ch(&mut a, '2');
        assert!(a.sidebar_focus, "not before the answer");
        a.on_server(ServerMsg::ActionResult(result));
        assert_eq!(!a.sidebar_focus, left);
    }
}

#[test]
fn choosing_on_a_folded_project_also_unfolds_it() {
    let mut a = app();
    // `dotfiles` (project 8) is folded in the mockup.
    a.open_new_menu(ProjectId(8));
    assert_eq!(
        ch(&mut a, '1'),
        [
            act(Act::NewPanel {
                project: ProjectId(8),
                name: None
            }),
            act(Act::SetProjectCollapsed {
                project: ProjectId(8),
                collapsed: false
            })
        ]
    );
    // An open project only gets the launch.
    a.open_new_menu(ProjectId(2));
    assert_eq!(ch(&mut a, '2').len(), 1);
}

#[test]
fn hover_moves_the_highlight_and_a_click_on_an_item_chooses_it() {
    let mut a = open_by_click();
    moved(&mut a, 12, 11);
    assert_eq!(a.menu.as_ref().unwrap().selected, 3);
    assert!(a.take_dirty());
    moved(&mut a, 13, 11);
    assert!(!a.take_dirty());
    // The border and the hint row are not items.
    moved(&mut a, 12, 7);
    moved(&mut a, 12, 12);
    assert_eq!(a.menu.as_ref().unwrap().selected, 3);
    assert_eq!(
        down(&mut a, 12, 10),
        [act(Act::NewPanelWith {
            project: ProjectId(2),
            launch: PanelLaunch::Agent(AgentKind::OpenCode)
        })]
    );
    assert!(a.menu.is_none());
    // The release of that click reaches nobody.
    assert!(mouse(&mut a, ct::MouseEventKind::Up(LEFT), 12, 10).is_empty());
}

#[test]
fn a_click_outside_is_not_forwarded_to_a_terminal_that_reports_the_mouse() {
    let mut a = app();
    let mut f = frame(false, (40, 10), vec![]);
    f.terminal = a.mirror.focused_id().unwrap();
    f.modes = deck_core::protocol::Modes(
        deck_core::protocol::Modes::MOUSE_CLICK | deck_core::protocol::Modes::SGR_MOUSE,
    );
    a.on_server(ServerMsg::Frame(f));
    // Without the menu the click reaches the program.
    assert!(!click(&mut a, 40, 5).is_empty());
    a.open_new_menu(ProjectId(2));
    assert!(down(&mut a, 40, 5).is_empty());
    assert!(a.menu.is_none());
    assert!(mouse(&mut a, ct::MouseEventKind::Up(LEFT), 40, 5).is_empty());
}

#[test]
fn a_click_outside_closes_the_menu_and_is_not_passed_on() {
    let mut a = open_by_click();
    // On the row `empty`, which a click would fold, and on the panel area.
    assert!(click(&mut a, 3, 8).is_empty());
    assert!(a.menu.is_none());
    assert_eq!(a.selection, None);
    let mut a = open_by_click();
    assert!(click(&mut a, 60, 12).is_empty());
    assert!(a.menu.is_none());
    // A click on its border or hint row keeps it.
    let mut a = open_by_click();
    assert!(click(&mut a, 12, 12).is_empty());
    assert!(click(&mut a, 6, 7).is_empty());
    assert!(a.menu.is_some());
}

#[test]
fn the_wheel_does_nothing_in_the_menu() {
    let mut a = open_by_click();
    for kind in [ct::MouseEventKind::ScrollDown, ct::MouseEventKind::ScrollUp] {
        assert!(mouse(&mut a, kind, 12, 9).is_empty());
        assert!(mouse(&mut a, kind, 3, 4).is_empty());
    }
    assert_eq!(a.sidebar_offset, 0);
    assert_eq!(a.menu.as_ref().unwrap().selected, 0);
}

#[test]
fn an_error_flashes_for_four_seconds_and_the_menu_is_already_closed() {
    let mut a = open_by_click();
    ch(&mut a, '3');
    let now = Instant::now();
    a.on_server(ServerMsg::ActionResult(Err(ActionError::ProgramNotFound {
        program: "opencode".into(),
        kind: AgentKind::OpenCode,
        key: "command".into(),
    })));
    let (text, until) = a.flash.clone().unwrap();
    assert_eq!(
        text,
        "\"opencode\" not found in PATH (set [agents.opencode] command)"
    );
    assert!(a.flash_error);
    assert!(until >= now + ERROR_FLASH_FOR - std::time::Duration::from_millis(50));
    assert!(a.menu.is_none());
}

#[test]
fn a_project_removed_meanwhile_closes_the_menu_with_a_flash() {
    let mut a = open_by_click();
    let mut view = mockup();
    view.projects.retain(|p| p.id != ProjectId(2));
    a.on_server(ServerMsg::SessionView(view));
    assert!(a.menu.is_none());
    assert_eq!(a.flash.as_ref().unwrap().0, "project \"budget\" was closed");
    assert!(!a.flash_error);
}

#[test]
fn a_new_menu_key_override_moves_the_binding() {
    let config = Config::parse("[keys]\nnew_menu = \"M\"").unwrap().config;
    let mut a = app_with(&config, (100, 24), mockup());
    prefix(&mut a);
    ch(&mut a, 'N');
    assert!(a.menu.is_none());
    prefix(&mut a);
    ch(&mut a, 'M');
    assert!(a.menu.is_some());
    let rows = a.bindings.help_rows();
    let row = rows.iter().find(|r| r.text.starts_with("New panel menu"));
    assert!(row.unwrap().keys.ends_with(" M"), "{row:?}");
}

#[test]
fn the_menu_is_part_of_the_projects_help_group() {
    let a = app();
    let row = a
        .bindings
        .help_rows()
        .into_iter()
        .find(|r| r.keys.ends_with(" N"))
        .unwrap();
    assert_eq!(row.group, deck_core::config::HelpGroup::Projects);
}
