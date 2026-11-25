//! The collapsed sidebar: the header chips, the entries, the keys, the mouse and the config.

use std::path::PathBuf;
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
use crate::draw::draw;
use crate::icons_tests::agent;
use crate::mirror::tests::{frame, mockup, panel, project};
use crate::modal::Modal;
use crate::sidebar::{Hover, Row, Sel, collapsed_rows, rows};

const LEFT: ct::MouseButton = ct::MouseButton::Left;

// The collapsed rows of `view()`, from line 2: 31 (1), 41 (2), 51 (3), a rule, 61 (4), a rule,
// 91 (5), 101 (6). Project 7 has no terminals; project 8 is folded.
const T31: u16 = 2;
const T41: u16 = 3;
const T51: u16 = 4;
const T61: u16 = 6;
const T91: u16 = 8;

/// `mockup()` with an agent working in 41, an exited terminal 51 and an unseen panel 6.
fn view() -> SessionView {
    let mut view = mockup();
    let t41 = &mut view.projects[0].panels[1].terminals[0];
    t41.agent = Some(agent(AgentKind::Claude, AgentStatus::Waiting));
    view.projects[0].panels[2].terminals[0].exit_status = Some(0);
    view.projects[1].panels[0].unseen = true;
    view
}

fn app_at(config: &Config, size: (u16, u16), view: SessionView) -> App {
    let mut app = App::new(config, size, &[])
        .with_dirs(PathBuf::from("/work/proj"), Some(PathBuf::from("/home/me")));
    app.on_server(ServerMsg::SessionView(view));
    let mut f = frame(true, (40, 10), vec![]);
    f.terminal = TerminalId(31);
    app.on_server(ServerMsg::Frame(f));
    app
}

fn collapsed_config() -> Config {
    Config {
        sidebar_collapsed: true,
        ..Config::default()
    }
}

fn app() -> App {
    app_at(&collapsed_config(), (100, 24), view())
}

fn expanded() -> App {
    app_at(&Config::default(), (100, 24), view())
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

fn click_at(a: &mut App, column: u16, row: u16) -> Vec<Out> {
    mouse(a, ct::MouseEventKind::Down(LEFT), column, row);
    mouse(a, ct::MouseEventKind::Up(LEFT), column, row)
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

fn prefix(a: &mut App, c: char) -> Vec<Out> {
    let mods = if c.is_uppercase() {
        ct::KeyModifiers::SHIFT
    } else {
        ct::KeyModifiers::NONE
    };
    a.on_event(
        ct::Event::Key(ct::KeyEvent::new(
            ct::KeyCode::Char(' '),
            ct::KeyModifiers::ALT,
        )),
        Instant::now(),
    );
    a.on_event(
        ct::Event::Key(ct::KeyEvent::new(ct::KeyCode::Char(c), mods)),
        Instant::now(),
    )
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

fn viewport(cols: u16, rows: u16) -> Out {
    Out::Send(ClientMsg::Viewport {
        cols,
        rows,
        cell: None,
    })
}

fn bound(action: &str, key: &str) -> Config {
    Config::parse(&format!("[keys]\n{action} = \"{key}\""))
        .unwrap()
        .config
}

fn term(p: u64, panel: u64, t: u64) -> Sel {
    Sel::Terminal(ProjectId(p), PanelId(panel), TerminalId(t))
}

fn focus_sidebar(a: &mut App) {
    a.enter_sidebar();
    assert!(a.sidebar_focus);
}

#[test]
fn collapsed_rows_list_every_terminal_with_rules_between_spaces() {
    let v = view();
    let kinds: Vec<String> = collapsed_rows(&v)
        .iter()
        .map(|r| match r {
            Row::Terminal(_, _, t) => format!("t{}", t.0),
            Row::Rule => "-".into(),
            other => format!("{other:?}"),
        })
        .collect();
    // The folded space's terminals show; the empty space adds neither entries nor a rule.
    assert_eq!(kinds, ["t31", "t41", "t51", "-", "t61", "-", "t91", "t101"]);
    assert_eq!(Row::Rule.sel(), None);
    // A grid panel: its terminals in grid order, a single-terminal panel one entry.
    let mut grid = mockup();
    let mut extra = grid.projects[0].panels[0].terminals[0].clone();
    extra.id = TerminalId(32);
    grid.projects[0].panels[0].terminals.push(extra);
    assert_eq!(
        collapsed_rows(&grid)
            .iter()
            .take(4)
            .filter_map(|r| r.sel())
            .count(),
        4
    );
    assert!(collapsed_rows(&view_without_terminals()).is_empty());
}

fn view_without_terminals() -> SessionView {
    let mut v = mockup();
    for p in &mut v.projects {
        for panel in &mut p.panels {
            panel.terminals.clear();
        }
    }
    v
}

#[test]
fn the_expanded_header_has_the_chip_and_the_name_from_column_four() {
    let long = "terminal-multiplexer-long-name-that-keeps-going-past-the-widest-sidebar-here";
    let header = |width: u16| {
        let config = Config {
            sidebar_width: width,
            ..Config::default()
        };
        let mut v = mockup();
        v.name = long.into();
        line(&buffer(&app_at(&config, (100, 24), v)), 0, width)
    };
    // The name gets columns 4 to 13 of a 20-column sidebar, which shows only the chip's icon.
    assert_eq!(header(20), " «  terminal-…  +   ");
    assert_eq!(header(28), " «  termina…  +  New space  ");
    // At every width the name is cut before the chip, with a column between.
    for width in 20..=60 {
        let text = header(width);
        assert!(text.starts_with(" «  ter"), "{width}: {text}");
        let end = text.chars().position(|c| c == '…').unwrap();
        let plus = text.chars().position(|c| c == '+').unwrap();
        assert!(end + 2 < plus, "{width}: {text}");
    }
    // A name that fits is whole, from column 4.
    let a = app_at(&Config::default(), (100, 24), mockup());
    assert!(line(&buffer(&a), 0, 28).starts_with(" «  main "));
}

#[test]
fn the_collapsed_sidebar_draws_the_header_chip_entries_and_the_bottom_bar() {
    let a = app();
    assert_eq!(a.viewport(), (91, 24));
    let buf = buffer(&a);
    let rows: Vec<String> = (0..24).map(|y| line(&buf, y, 9)).collect();
    assert_eq!(rows[0], "   »    │");
    assert_eq!(rows[1], "        │");
    assert!(rows[T31 as usize].starts_with(" ❯  1   │"), "{}", rows[2]);
    assert!(rows[T41 as usize].starts_with(" !  2   │"), "{}", rows[3]);
    // 51 exited: the program icon, muted.
    assert!(rows[T51 as usize].starts_with(" ❯  3   │"));
    assert_eq!(rows[5], " ────── │");
    assert!(rows[T61 as usize].starts_with(" ❯  4   │"));
    assert_eq!(rows[7], " ────── │");
    assert!(rows[T91 as usize].starts_with(" ❯  5   │"));
    assert!(rows[9].starts_with(" ❯  6   │"));
    assert_eq!(rows[23], "   +    │");
    assert_eq!(rows[22], "        │");
    // The panel area took the columns.
    assert_eq!(buf[(8, 5)].symbol(), "│");
}

#[test]
fn an_entry_has_the_band_when_focused_and_marks_follow_state() {
    let a = app();
    let buf = buffer(&a);
    // The focused terminal of the active panel has the band, across the sidebar.
    let reversed = |x, y| buf[(x, y)].modifier.contains(Modifier::REVERSED);
    assert!((0..8).all(|x| reversed(x, T31)));
    assert!(!reversed(0, T41));
    // The waiting agent's mark is in the warning colour; the exited terminal is muted.
    let theme = &a.theme;
    assert_eq!(buf[(1, T41)].fg, crate::draw::color(theme.warning));
    assert_eq!(buf[(1, T51)].fg, crate::draw::color(theme.muted));
    assert_eq!(buf[(4, T51)].fg, crate::draw::color(theme.muted));
    // The unseen panel's entry is bold.
    assert!(buf[(4, T61)].modifier.contains(Modifier::BOLD));
    assert!(!buf[(4, T41)].modifier.contains(Modifier::BOLD));
}

#[test]
fn three_digit_numbers_fit_before_the_margin() {
    let mut v = mockup();
    v.projects = vec![project(1, "big", false, vec![panel(1, "p", 1)])];
    v.projects[0].panels[0].terminals = (1..=123)
        .map(|n| {
            let mut t = mockup().projects[0].panels[0].terminals[0].clone();
            t.id = TerminalId(n);
            t
        })
        .collect();
    v.projects[0].panels[0].focused_terminal = Some(TerminalId(123));
    v.active_project = Some(ProjectId(1));
    v.active_panel = Some(PanelId(1));
    let a = app_at(&collapsed_config(), (100, 130), v);
    let buf = buffer(&a);
    assert_eq!(line(&buf, 2 + 122, 9), " ❯  123 │");
}

#[test]
fn the_number_follows_the_scroll() {
    let mut v = view();
    v.projects[0].panels = (3..=40).map(|id| panel(id, "p", 1)).collect();
    let mut a = app_at(&collapsed_config(), (100, 14), v);
    // 14 rows: 12 under the header, 10 with the bar; the focused terminal is the first.
    assert_eq!(a.sidebar_offset, 0);
    mouse(&mut a, ct::MouseEventKind::ScrollDown, 3, 5);
    assert_eq!(a.sidebar_offset, 3);
    let buf = buffer(&a);
    assert_eq!(line(&buf, 2, 9), " ❯  4   │");
    mouse(&mut a, ct::MouseEventKind::ScrollUp, 3, 5);
    assert_eq!(a.sidebar_offset, 0);
}

#[test]
fn the_list_scrolls_to_keep_the_focused_terminal_in_view() {
    let mut v = view();
    v.projects[0].panels = (3..=40).map(|id| panel(id, "p", 1)).collect();
    let mut a = app_at(&collapsed_config(), (100, 14), v.clone());
    v.active_panel = Some(PanelId(40));
    v.projects[0].active_panel = Some(PanelId(40));
    a.on_server(ServerMsg::SessionView(v));
    assert!(a.sidebar_offset > 0);
    let buf = buffer(&a);
    assert!(
        (2..12).any(|y| line(&buf, y, 9).contains(" 38 ")),
        "{}",
        a.sidebar_offset
    );
}

#[test]
fn the_collapse_chip_collapses_and_the_expand_chip_expands() {
    let mut a = expanded();
    assert_eq!(a.viewport(), (71, 24));
    assert_eq!(a.hover_at(1, 0), Some(Hover::CollapseSidebar));
    assert_eq!(a.hover_at(3, 0), Some(Hover::CollapseSidebar));
    assert_eq!(a.hover_at(4, 0), None);
    // Press and release on the chip; neither opens the New space prompt.
    assert_eq!(click_at(&mut a, 1, 0), [viewport(91, 24)]);
    assert!(a.sidebar_collapsed);
    assert!(a.modal.is_none());
    assert_eq!(a.sidebar_width, 28, "the full width is kept");
    // Centred in the 8-column strip: columns 2 to 5.
    assert_eq!(a.hover_at(1, 0), None);
    assert_eq!(a.hover_at(2, 0), Some(Hover::ExpandSidebar));
    assert_eq!(a.hover_at(5, 0), Some(Hover::ExpandSidebar));
    assert_eq!(a.hover_at(6, 0), None);
    assert_eq!(click_at(&mut a, 3, 0), [viewport(71, 24)]);
    assert!(!a.sidebar_collapsed);
    assert!(a.modal.is_none());
    // A press on one chip released on another does nothing.
    mouse(&mut a, ct::MouseEventKind::Down(LEFT), 1, 0);
    assert!(mouse(&mut a, ct::MouseEventKind::Up(LEFT), 10, 5).is_empty());
    assert!(!a.sidebar_collapsed);
}

#[test]
fn the_chips_are_ignored_under_an_overlay() {
    let mut a = expanded();
    a.modal = Some(Modal::Help);
    assert!(click_at(&mut a, 1, 0).is_empty());
    assert!(!a.sidebar_collapsed);
    let mut a = app();
    a.modal = Some(Modal::Help);
    assert!(click_at(&mut a, 1, 0).is_empty());
    assert!(a.sidebar_collapsed);
}

#[test]
fn the_action_flips_the_state_and_shows_a_hidden_sidebar() {
    let mut a = app_at(&bound("collapse_sidebar", "B"), (100, 24), view());
    assert!(!a.sidebar_collapsed);
    assert_eq!(prefix(&mut a, 'B'), [viewport(91, 24)]);
    assert!(a.sidebar_collapsed);
    // Hide it, then the action shows it (still collapsed would be flipped: it expands).
    prefix(&mut a, 'b');
    assert!(a.sidebar_hidden);
    assert_eq!(prefix(&mut a, 'B'), [viewport(71, 24)]);
    assert!(!a.sidebar_hidden);
    assert!(!a.sidebar_collapsed);
}

#[test]
fn a_window_under_29_columns_hides_the_collapsed_sidebar_and_it_returns() {
    let mut a = app();
    let outs = a.on_event(ct::Event::Resize(28, 24), Instant::now());
    assert_eq!(outs, [viewport(28, 24)]);
    assert!(!a.sidebar_visible());
    let outs = a.on_event(ct::Event::Resize(29, 24), Instant::now());
    assert_eq!(outs, [viewport(20, 24)]);
    assert!(a.sidebar_visible());
}

#[test]
fn expanding_in_a_narrow_window_is_refused() {
    let config = Config {
        sidebar_collapsed: true,
        ..bound("collapse_sidebar", "B")
    };
    let mut a = app_at(&config, (35, 24), view());
    assert_eq!(a.sidebar_cols(), 9);
    assert_eq!(click_at(&mut a, 3, 0), Vec::<Out>::new());
    assert!(a.sidebar_collapsed);
    assert_eq!(a.flash.as_ref().unwrap().0, "window too narrow to expand");
    // The key is refused the same way.
    a.flash = None;
    assert!(prefix(&mut a, 'B').is_empty());
    assert!(a.sidebar_collapsed);
    assert_eq!(a.flash.as_ref().unwrap().0, "window too narrow to expand");
    // Collapsing the full sidebar in a window that fits it works.
    let mut a = app_at(&bound("collapse_sidebar", "B"), (45, 24), view());
    assert_eq!(prefix(&mut a, 'B'), [viewport(36, 24)]);
}

#[test]
fn the_action_in_a_tiny_window_flashes_that_the_sidebar_stays_hidden() {
    let mut a = app_at(&bound("collapse_sidebar", "B"), (28, 24), view());
    prefix(&mut a, 'B');
    assert_eq!(
        a.flash.as_ref().unwrap().0,
        "sidebar hidden (window too narrow)"
    );
}

#[test]
fn a_click_on_an_entry_focuses_that_terminal() {
    let mut a = app();
    assert_eq!(a.hover_at(2, T41), Some(Hover::Row(term(1, 4, 41))));
    assert_eq!(a.hover_at(2, 5), None, "a rule");
    assert_eq!(a.hover_at(2, 22), None);
    assert_eq!(a.hover_at(2, 23), Some(Hover::New));
    assert_eq!(a.hover_at(5, 23), Some(Hover::New));
    assert_eq!(a.hover_at(1, 23), None);
    assert_eq!(a.hover_at(6, 23), None);
    assert_eq!(a.hover_at(8, T41), None, "the separator");
    assert_eq!(click_at(&mut a, 2, T41), focus_pair(4, 41));
    // A click on a rule does nothing.
    assert!(click_at(&mut a, 2, 5).is_empty());
}

#[test]
fn a_folded_spaces_entry_is_clickable_and_a_press_never_drags() {
    let mut a = app();
    assert_eq!(click_at(&mut a, 2, T91), focus_pair(9, 91));
    // Pressing an entry and moving to another ends the press.
    mouse(&mut a, ct::MouseEventKind::Down(LEFT), 2, T31);
    mouse(&mut a, ct::MouseEventKind::Drag(LEFT), 2, T41);
    assert!(a.row_drag.is_none());
    assert!(mouse(&mut a, ct::MouseEventKind::Up(LEFT), 2, T41).is_empty());
}

#[test]
fn a_right_click_on_an_entry_opens_the_terminal_menu() {
    let mut a = app();
    mouse(
        &mut a,
        ct::MouseEventKind::Down(ct::MouseButton::Right),
        2,
        T61,
    );
    let menu = a.context.as_ref().unwrap();
    assert_eq!(menu.target, ContextTarget::Terminal(TerminalId(61)));
    assert_eq!(menu.row(), Some(term(2, 6, 61)));
}

#[test]
fn a_press_on_the_separator_does_nothing_while_collapsed() {
    let mut a = app();
    assert!(mouse(&mut a, ct::MouseEventKind::Down(LEFT), 7, 10).is_empty());
    assert!(a.sidebar_drag.is_none());
    assert!(mouse(&mut a, ct::MouseEventKind::Drag(LEFT), 12, 10).is_empty());
    assert!(mouse(&mut a, ct::MouseEventKind::Up(LEFT), 12, 10).is_empty());
    assert_eq!(a.sidebar_width, 28);
    assert!(a.sidebar_collapsed);
}

#[test]
fn the_plus_chip_opens_the_new_menu_and_there_is_no_resume_chip() {
    let mut a = app();
    click_at(&mut a, 3, 23);
    assert!(a.menu.is_some());
    let mut a = app();
    assert_eq!(a.hover_at(20, 23), None);
    assert!(click_at(&mut a, 7, 23).is_empty());
    assert!(a.restore.is_none());
}

#[test]
fn sidebar_focus_moves_over_entries_and_keys_act_on_them() {
    let mut a = app();
    focus_sidebar(&mut a);
    assert_eq!(a.selection, Some(term(1, 3, 31)));
    ch(&mut a, 'j');
    assert_eq!(a.selection, Some(term(1, 4, 41)));
    key(&mut a, ct::KeyCode::Down);
    key(&mut a, ct::KeyCode::Down);
    assert_eq!(a.selection, Some(term(2, 6, 61)), "the rule is skipped");
    key(&mut a, ct::KeyCode::Up);
    assert_eq!(a.selection, Some(term(1, 5, 51)));
    // Left and right do nothing.
    assert!(key(&mut a, ct::KeyCode::Left).is_empty());
    assert!(ch(&mut a, 'l').is_empty());
    assert_eq!(a.selection, Some(term(1, 5, 51)));
    // The band is on the selection, the focused terminal is accent and bold.
    let buf = buffer(&a);
    let reversed = |x, y| buf[(x, y)].modifier.contains(Modifier::REVERSED);
    assert!((0..8).all(|x| reversed(x, T51)));
    assert!(!reversed(0, T41) && !reversed(0, T31));
    assert_eq!(buf[(4, T31)].fg, crate::draw::color(a.theme.accent));
    assert!(buf[(4, T31)].modifier.contains(Modifier::BOLD));
    // Enter focuses; x asks to close.
    // The exited terminal closes at once.
    assert_eq!(
        ch(&mut a, 'x'),
        [send(Act::CloseTerminal {
            terminal: TerminalId(51)
        })]
    );
    key(&mut a, ct::KeyCode::Down);
    ch(&mut a, 'x');
    assert!(matches!(a.modal, Some(Modal::Confirm(_))));
    key(&mut a, ct::KeyCode::Esc);
    key(&mut a, ct::KeyCode::Up);
    assert_eq!(key(&mut a, ct::KeyCode::Enter), focus_pair(5, 51));
    assert!(!a.sidebar_focus);
}

#[test]
fn the_selection_maps_across_a_collapse() {
    for (from, to) in [
        (term(1, 4, 41), term(1, 4, 41)),
        (Sel::Panel(ProjectId(1), PanelId(4)), term(1, 4, 41)),
        // The space's active panel's focused terminal.
        (Sel::Project(ProjectId(1)), term(1, 3, 31)),
        // Folded space: its first entry.
        (Sel::Project(ProjectId(8)), term(8, 9, 91)),
    ] {
        let mut a = expanded();
        focus_sidebar(&mut a);
        a.selection = Some(from);
        a.set_sidebar_collapsed(true);
        assert_eq!(a.selection, Some(to), "{from:?}");
    }
    // A space with no terminals goes to the focused terminal of the active panel.
    let mut a = expanded();
    focus_sidebar(&mut a);
    a.selection = Some(Sel::Project(ProjectId(7)));
    a.set_sidebar_collapsed(true);
    assert_eq!(a.selection, Some(term(1, 3, 31)));
}

#[test]
fn the_selection_maps_across_an_expand() {
    let mut v = view();
    let mut extra = v.projects[0].panels[0].terminals[0].clone();
    extra.id = TerminalId(32);
    v.projects[0].panels[0].terminals.push(extra);
    for (from, to) in [
        (term(1, 3, 32), term(1, 3, 32)),
        (term(1, 4, 41), Sel::Panel(ProjectId(1), PanelId(4))),
        (term(8, 9, 91), Sel::Project(ProjectId(8))),
    ] {
        let mut a = app_at(&collapsed_config(), (100, 24), v.clone());
        focus_sidebar(&mut a);
        a.selection = Some(from);
        a.set_sidebar_collapsed(false);
        assert_eq!(a.selection, Some(to), "{from:?}");
    }
}

#[test]
fn a_toggle_in_sidebar_focus_keeps_the_focus() {
    let mut a = app_at(&bound("collapse_sidebar", "B"), (100, 24), view());
    focus_sidebar(&mut a);
    ch(&mut a, 'j');
    // A sidebar focus swallows keys, so use the chip.
    click_at(&mut a, 1, 0);
    assert!(a.sidebar_collapsed);
    assert!(a.sidebar_focus);
    assert_eq!(a.selection, Some(term(1, 4, 41)));
}

#[test]
fn the_selection_falls_back_when_its_terminal_closes() {
    let mut a = app();
    focus_sidebar(&mut a);
    a.selection = Some(term(1, 4, 41));
    let mut v = view();
    v.projects[0].panels.remove(1);
    a.on_server(ServerMsg::SessionView(v));
    // The first entry of its space.
    assert_eq!(a.selection, Some(term(1, 3, 31)));
    // A selection that is a panel row (not drawn) falls back too.
    a.selection = Some(Sel::Panel(ProjectId(1), PanelId(5)));
    a.normalize_selection();
    assert_eq!(a.selection, Some(term(1, 3, 31)));
    // A space that is gone falls back to the focused terminal of the active panel.
    a.selection = Some(term(99, 98, 97));
    a.normalize_selection();
    assert_eq!(a.selection, Some(term(1, 3, 31)));
}

#[test]
fn a_click_on_a_single_terminal_panels_entry_survives_the_next_snapshot() {
    let mut a = app();
    click_at(&mut a, 2, T41);
    assert_eq!(a.selection, Some(term(1, 4, 41)));
    a.on_server(ServerMsg::SessionView(view()));
    assert_eq!(a.selection, Some(term(1, 4, 41)));
}

#[test]
fn the_config_starts_collapsed_and_a_reload_applies_only_a_change() {
    let mut a = app_at(&Config::default(), (100, 24), view());
    assert!(!a.sidebar_collapsed);
    // The user collapses; a reload that does not change the key keeps the state.
    click_at(&mut a, 1, 0);
    assert!(a.sidebar_collapsed);
    let outs = a.on_server(ServerMsg::Config(Box::new(
        Config::parse("sidebar_width = 30").unwrap().config,
    )));
    assert!(a.sidebar_collapsed);
    assert!(
        outs.is_empty(),
        "the collapsed strip has the same size: {outs:?}"
    );
    // A reload that changes it applies it, and says the new size without claiming.
    let outs = a.on_server(ServerMsg::Config(Box::new(
        Config::parse("sidebar_width = 30\nsidebar_collapsed = true")
            .unwrap()
            .config,
    )));
    assert!(a.sidebar_collapsed && outs.is_empty());
    let outs = a.on_server(ServerMsg::Config(Box::new(
        Config::parse("sidebar_width = 30\nsidebar_collapsed = false")
            .unwrap()
            .config,
    )));
    assert!(!a.sidebar_collapsed);
    assert_eq!(
        outs,
        [Out::Send(ClientMsg::ReloadViewport {
            cols: 69,
            rows: 24,
            cell: None
        })]
    );
    // And the other way: collapsing from the config.
    let outs = a.on_server(ServerMsg::Config(Box::new(
        Config::parse("sidebar_width = 30\nsidebar_collapsed = true")
            .unwrap()
            .config,
    )));
    assert!(a.sidebar_collapsed);
    assert_eq!(
        outs,
        [Out::Send(ClientMsg::ReloadViewport {
            cols: 91,
            rows: 24,
            cell: None
        })]
    );
}

#[test]
fn a_client_attached_collapsed_sends_no_corrective_viewport() {
    let mut a = App::new(&collapsed_config(), (100, 24), &[]);
    // The daemon sized the terminals for the collapsed strip.
    let outs = a.on_server(ServerMsg::SessionView(view()));
    assert!(
        !outs
            .iter()
            .any(|o| matches!(o, Out::Send(ClientMsg::Viewport { .. }))),
        "{outs:?}"
    );
    // A client with the sidebar expanded in a config that says collapsed would differ.
    assert_eq!(a.viewport(), (91, 24));
}

#[test]
fn the_rows_helper_is_unchanged_for_the_expanded_sidebar() {
    assert!(!rows(&view(), 0).contains(&Row::Rule));
}

#[test]
fn the_sidebar_hints_leave_out_fold_while_collapsed() {
    let bar = |a: &App| line(&buffer(a), 23, 100);
    let mut a = expanded();
    focus_sidebar(&mut a);
    assert!(bar(&a).contains("fold"), "{}", bar(&a));
    let mut a = app();
    focus_sidebar(&mut a);
    assert!(bar(&a).contains("move"), "{}", bar(&a));
    assert!(!bar(&a).contains("fold"), "{}", bar(&a));
}

#[test]
fn a_reload_cannot_expand_where_only_the_strip_fits() {
    let off = Config::parse("sidebar_collapsed = false").unwrap().config;
    let on = Config::parse("sidebar_collapsed = true").unwrap().config;
    let mut a = app_at(&on, (35, 24), view());
    let outs = a.on_server(ServerMsg::Config(Box::new(off.clone())));
    assert!(a.sidebar_collapsed);
    assert!(outs.is_empty(), "{outs:?}");
    assert_eq!(a.flash.as_ref().unwrap().0, "window too narrow to expand");
    assert_eq!(a.sidebar_cols(), 9);
    // In a window that fits the full sidebar the reload expands it.
    let mut a = app_at(&on, (100, 24), view());
    a.on_server(ServerMsg::Config(Box::new(off)));
    assert!(!a.sidebar_collapsed);
}
