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

/// The bottom bar of the 24-row window is on row 23; its `+ New` chip is on columns 1..8.
const BAR: u16 = 23;
const NEW_CHIP: u16 = 3;

/// The menu of `budget` (project 2), opened by selecting it in sidebar focus and clicking
/// `+ New`.
fn open_by_click() -> App {
    let mut a = app();
    prefix(&mut a);
    press(&mut a, ct::KeyCode::Tab);
    for _ in 0..3 {
        press(&mut a, ct::KeyCode::Down);
    }
    assert_eq!(a.selection, Some(Sel::Project(ProjectId(2))));
    assert!(click(&mut a, NEW_CHIP, BAR).is_empty());
    assert_eq!(a.menu.as_ref().map(|m| m.project), Some(ProjectId(2)));
    a
}

#[test]
fn project_rows_have_no_button_and_keep_their_counts_under_the_pointer() {
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
    assert_eq!(text(&buffer(&a), 2, 0, 29), " ▸ terminal-multiplexer   1 │");
    moved(&mut a, 10, 2);
    assert_eq!(a.hover, Some(Hover::Row(Sel::Project(ProjectId(1)))));
    assert_eq!(text(&buffer(&a), 2, 0, 29), " ▸ terminal-multiplexer   1 │");
    // Selecting it in sidebar focus does not add a button either.
    prefix(&mut a);
    press(&mut a, ct::KeyCode::Tab);
    press(&mut a, ct::KeyCode::Up);
    press(&mut a, ct::KeyCode::Up);
    assert_eq!(a.selection, Some(Sel::Project(ProjectId(1))));
    assert!(!text(&buffer(&a), 2, 0, 29).contains("[ New"));
}

#[test]
fn the_new_chip_has_the_hover_background_under_the_pointer_and_is_muted_otherwise() {
    let mut a = app();
    let buf = buffer(&a);
    assert_eq!(text(&buf, BAR, 0, 29), "  + New             Resume  │");
    assert_eq!(buf[(NEW_CHIP, BAR)].fg, color(a.theme.muted));
    moved(&mut a, NEW_CHIP, BAR);
    assert_eq!(a.hover, Some(Hover::New));
    let buf = buffer(&a);
    let cell = &buf[(NEW_CHIP, BAR)];
    assert_eq!(cell.bg, color(a.theme.hover));
    assert_eq!(cell.fg, color(a.theme.sidebar_fg));
    // Moving along the chip does not redraw; leaving it does.
    assert!(a.take_dirty());
    moved(&mut a, NEW_CHIP + 1, BAR);
    assert!(!a.take_dirty());
    moved(&mut a, 20, 5);
    assert!(a.take_dirty());
}

#[test]
fn the_new_chip_opens_the_menu_for_the_selected_project_else_the_active_one() {
    // No sidebar focus: the active project (1).
    let mut a = app();
    assert!(click(&mut a, NEW_CHIP, BAR).is_empty());
    let menu = a.menu.as_ref().unwrap();
    assert_eq!((menu.project, menu.selected), (ProjectId(1), 0));
    assert!(a.overlay_open());
    // Sidebar focus on a panel of another project: that panel's project.
    let mut a = app();
    prefix(&mut a);
    press(&mut a, ct::KeyCode::Tab);
    for _ in 0..4 {
        press(&mut a, ct::KeyCode::Down);
    }
    assert_eq!(a.selection, Some(Sel::Panel(ProjectId(2), PanelId(6))));
    click(&mut a, NEW_CHIP, BAR);
    assert_eq!(a.menu.as_ref().unwrap().project, ProjectId(2));
    // A click that starts on the chip and ends elsewhere does nothing.
    let mut a = app();
    down(&mut a, NEW_CHIP, BAR);
    mouse(&mut a, ct::MouseEventKind::Up(LEFT), 20, 5);
    assert!(a.menu.is_none());
}

#[test]
fn without_a_project_the_new_chip_flashes_no_project() {
    let empty = SessionView {
        projects: vec![],
        active_project: None,
        active_panel: None,
        ..mockup()
    };
    let mut a = app_with(&config(), (100, 24), empty);
    assert!(click(&mut a, NEW_CHIP, BAR).is_empty());
    assert!(a.menu.is_none());
    assert_eq!(a.flash.as_ref().unwrap().0, "no space");
}

#[test]
fn the_menu_sits_above_the_bottom_bar_with_its_icons_and_no_gutter() {
    let a = open_by_click();
    let buf = buffer(&a);
    let rows: Vec<String> = (16..24).map(|y| text(&buf, y, 0, 22)).collect();
    assert_eq!(
        rows,
        [
            "╭─ New in budget ────╮",
            "│ ❯  Terminal      1 │",
            "│ ✻  Claude        2 │",
            "│ ◇  OpenCode      3 │",
            "│ ◎  Codex         4 │",
            "│ ⏎ choose  esc close│",
            "╰────────────────────╯",
            "  + New             Re",
        ]
    );
}

#[test]
fn the_highlighted_row_uses_the_selection_colours_and_the_hint_is_muted() {
    let a = open_by_click();
    let buf = buffer(&a);
    let on = &buf[(10, 17)];
    let reversed = on.modifier.contains(Modifier::REVERSED);
    let themed = on.bg == color(a.theme.selection_bg) && on.fg == color(a.theme.selection_fg);
    assert!(reversed || themed);
    // The highlight covers the icon.
    assert_eq!(buf[(2, 17)].modifier, on.modifier);
    assert_eq!(buf[(10, 21)].fg, color(a.theme.muted));
    assert_eq!(buf[(10, 16)].fg, color(a.theme.border));
    assert_eq!(buf[(0, 18)].fg, color(a.theme.border));
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
fn the_menu_sits_right_above_the_bottom_bar_whatever_the_window_height() {
    let mut a = app_with(&config(), (100, 16), mockup());
    a.open_new_menu(ProjectId(1));
    let rect = a.menu_rect().unwrap();
    // 16 rows: the sidebar's bar is row 15, the menu's last row is 14.
    assert_eq!((rect.x, rect.y, rect.y + rect.height - 1), (0, 8, 14));
    let buf = buffer(&a);
    assert!(text(&buf, 14, 0, 5).starts_with('╰'));
    assert!(text(&buf, 8, 0, 5).starts_with('╭'));
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
            assert!(rect.y + rect.height <= size.1, "{size:?} {rect:?}");
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
    assert_eq!((rect.x, rect.y, rect.width), (0, 16, 22));
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
    assert!(bar.contains("SIDEBAR"), "{bar}");
    assert!(SIDEBAR_HINTS.contains(&("n", "menu")));
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
    moved(&mut a, 12, 20);
    assert_eq!(a.menu.as_ref().unwrap().selected, 3);
    assert!(a.take_dirty());
    moved(&mut a, 13, 20);
    assert!(!a.take_dirty());
    // The border and the hint row are not items.
    moved(&mut a, 12, 16);
    moved(&mut a, 12, 22);
    assert_eq!(a.menu.as_ref().unwrap().selected, 3);
    assert_eq!(
        down(&mut a, 12, 19),
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
    assert_eq!(a.selection, Some(Sel::Project(ProjectId(2))));
    let mut a = open_by_click();
    assert!(click(&mut a, 60, 12).is_empty());
    assert!(a.menu.is_none());
    // A click on its border or hint row keeps it.
    let mut a = open_by_click();
    assert!(click(&mut a, 12, 22).is_empty());
    assert!(click(&mut a, 6, 21).is_empty());
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
    assert_eq!(a.flash.as_ref().unwrap().0, "space \"budget\" was closed");
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

/// An app in Ghostty: images on, 24-bit colour, with the New menu of `budget` open.
fn menu_with_images() -> App {
    let mut a = open_by_click();
    a.truecolor = true;
    a.images = Some(crate::graphics::Images::new(
        Some((10, 20, 30)),
        Some((1, 1, 1)),
        [None; 16],
    ));
    a
}

/// The id of the image drawn in the two cells at `x`, `y`, when it is a whole logo.
fn logo_at(buf: &Buffer, x: u16, y: u16) -> Option<u32> {
    let (first, second) = (&buf[(x, y)], &buf[(x + 1, y)]);
    if first.symbol() != "\u{10EEEE}\u{0305}\u{0305}"
        || second.symbol() != "\u{10EEEE}\u{0305}\u{030D}"
        || !first.modifier.is_empty()
    {
        return None;
    }
    match first.fg {
        ratatui::style::Color::Rgb(r, g, b) => Some(u32::from_be_bytes([0, r, g, b])),
        _ => None,
    }
}

#[test]
fn with_images_the_menu_items_draw_logos_sent_before_the_frame() {
    let mut a = menu_with_images();
    // Not sent yet: text icons.
    assert_eq!(text(&buffer(&a), 18, 2, 4), "✻ ");
    let mut sink = Vec::new();
    a.send_images(&mut sink).unwrap();
    let buf = buffer(&a);
    // Terminal keeps its glyph; the three agents have logos at columns 2 and 3.
    assert_eq!(text(&buf, 17, 2, 4), "❯ ");
    let ids: Vec<u32> = (18..21)
        .map(|y| logo_at(&buf, 2, y).unwrap_or_else(|| panic!("no logo on row {y}")))
        .collect();
    assert!(ids[0] != ids[1] && ids[1] != ids[2] && ids[0] != ids[2]);
    let images = a.images.as_ref().unwrap();
    for id in &ids {
        assert!(images.order.iter().any(|(i, _)| i == id), "{id:x}");
    }
    // Labels and digits sit where they did.
    assert_eq!(text(&buf, 18, 5, 11), "Claude");
    assert_eq!(text(&buf, 18, 19, 20), "2");
}

#[test]
fn the_highlighted_logo_is_sent_in_the_selection_colour() {
    let mut a = menu_with_images();
    press(&mut a, ct::KeyCode::Down);
    let mut sink = Vec::new();
    a.send_images(&mut sink).unwrap();
    let buf = buffer(&a);
    let id = logo_at(&buf, 2, 18).expect("logo on the highlighted row");
    let (_, pair) = *a
        .images
        .as_ref()
        .unwrap()
        .order
        .iter()
        .find(|(i, _)| *i == id)
        .unwrap();
    // The selection is reversed over default colours here: the logo shows the outer background.
    let shown = a
        .images
        .as_ref()
        .unwrap()
        .cell_for(crate::menu::item_style(&a.theme, true))
        .0;
    assert_eq!(pair.rgb, shown);
    // An unselected logo is sent in another colour.
    let other = logo_at(&buf, 2, 19).unwrap();
    let (_, other_pair) = *a
        .images
        .as_ref()
        .unwrap()
        .order
        .iter()
        .find(|(i, _)| *i == other)
        .unwrap();
    assert_ne!(other_pair.rgb, pair.rgb);
}

#[test]
fn icons_none_menu_has_no_icon_column() {
    let config = Config::parse("[theme]\nicons = \"none\"").unwrap().config;
    let mut a = app_with(&config, (100, 24), mockup());
    a.open_new_menu(ProjectId(2));
    let buf = buffer(&a);
    assert_eq!(text(&buf, 17, 0, 22), "│ Terminal         1 │");
    assert_eq!(text(&buf, 18, 0, 22), "│ Claude           2 │");
}
