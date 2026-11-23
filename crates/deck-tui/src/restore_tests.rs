//! The Restore button and dialog: drawing, keys, mouse, project choice and late answers.

use std::path::PathBuf;
use std::time::Instant;

use deck_core::action::{Action as Act, ActionError, PanelLaunch};
use deck_core::agent_sessions::AgentSession;
use deck_core::agents::AgentKind;
use deck_core::config::Config;
use deck_core::model::{PanelId, ProjectId, SessionView};
use deck_core::protocol::{ClientMsg, Request, Response, ServerMsg};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event as ct;
use ratatui::style::Modifier;

use crate::app::{App, ERROR_FLASH_FOR, Out};
use crate::draw::{color, draw};
use crate::mirror::tests::{frame, mockup, panel, project};
use crate::restore::{Hit, Load};
use crate::sidebar::Hover;

const LEFT: ct::MouseButton = ct::MouseButton::Left;
/// The window the dialog tests use: the dialog is 90 columns wide at column 5 and 23 rows high
/// at row 3; its list starts on row 8, its columns at 7, 36 and 65.
const SIZE: (u16, u16) = (100, 30);
const NOW: u64 = 1_700_000_000;

fn two_projects() -> SessionView {
    let mut api = project(1, "api", false, vec![panel(3, "editor", 1)]);
    api.root = "/w/api".into();
    let mut web = project(2, "web", false, vec![panel(6, "site", 1)]);
    web.root = "/w/web".into();
    SessionView {
        projects: vec![api, web],
        active_project: Some(ProjectId(1)),
        active_panel: Some(PanelId(3)),
        ..mockup()
    }
}

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
    app_with(&config(), SIZE, two_projects())
}

fn session(kind: AgentKind, id: &str, title: &str, cwd: &str, age: u64) -> AgentSession {
    AgentSession {
        kind,
        id: id.into(),
        title: title.into(),
        cwd: cwd.into(),
        updated: NOW - age,
    }
}

fn claude() -> Vec<AgentSession> {
    vec![
        session(
            AgentKind::Claude,
            "c1",
            "Fix the login redirect",
            "/home/me/Projects/budget",
            2 * 3600,
        ),
        session(
            AgentKind::Claude,
            "c2",
            "Add rate limit",
            "/w/web/api",
            5 * 86_400,
        ),
    ]
}

fn opencode() -> Vec<AgentSession> {
    vec![session(
        AgentKind::OpenCode,
        "o1",
        "Rework login form",
        "/home/me/Projects/web",
        3 * 86_400,
    )]
}

fn codex() -> Vec<AgentSession> {
    vec![session(
        AgentKind::Codex,
        "x1",
        "Write the parser",
        "/elsewhere/deep/dir",
        60,
    )]
}

fn answer(a: &mut App, kind: AgentKind, sessions: Vec<AgentSession>, error: Option<&str>) {
    a.on_server(ServerMsg::Response(Response::AgentSessions {
        kind,
        sessions,
        error: error.map(str::to_string),
    }));
}

/// Opens the dialog by `prefix R` and answers all three columns.
fn opened() -> App {
    let mut a = app();
    open(&mut a);
    answer(&mut a, AgentKind::Claude, claude(), None);
    answer(&mut a, AgentKind::OpenCode, opencode(), None);
    answer(&mut a, AgentKind::Codex, codex(), None);
    a
}

fn open(a: &mut App) -> Vec<Out> {
    prefix(a);
    let outs = ch(a, 'R');
    if let Some(r) = a.restore.as_mut() {
        r.now = NOW;
    }
    outs
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

fn typed(a: &mut App, text: &str) {
    for c in text.chars() {
        ch(a, c);
    }
}

fn ctrl(a: &mut App, c: char) -> Vec<Out> {
    a.on_event(
        key(ct::KeyCode::Char(c), ct::KeyModifiers::CONTROL),
        Instant::now(),
    )
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

fn click(a: &mut App, column: u16, row: u16) -> Vec<Out> {
    let mut outs = mouse(a, ct::MouseEventKind::Down(LEFT), column, row);
    outs.extend(mouse(a, ct::MouseEventKind::Up(LEFT), column, row));
    outs
}

fn buffer(a: &App) -> Buffer {
    let mut t = Terminal::new(TestBackend::new(a.size.0, a.size.1)).unwrap();
    t.draw(|f| draw(f, a)).unwrap();
    t.backend().buffer().clone()
}

fn screen(a: &App) -> Vec<String> {
    let buf = buffer(a);
    (0..buf.area.height)
        .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect())
        .collect()
}

/// The dialog's text, trimmed, one string.
fn dialog(a: &App) -> String {
    screen(a).join("\n")
}

fn resume(kind: AgentKind, id: &str, cwd: &str, project: u64) -> Out {
    Out::Send(ClientMsg::Action(Act::NewPanelWith {
        project: ProjectId(project),
        launch: PanelLaunch::Resume {
            kind,
            session_id: id.into(),
            cwd: cwd.into(),
        },
    }))
}

fn selected(a: &App) -> Option<String> {
    a.restore.as_ref()?.selected_session().map(|s| s.id.clone())
}

#[test]
fn opening_asks_each_agent_at_once_and_routes_answers_by_kind_never_by_the_queue() {
    let mut a = app();
    prefix(&mut a);
    let outs = ch(&mut a, 'R');
    let ask = |kind| {
        Out::Send(ClientMsg::Request(Request::ListAgentSessions {
            kind,
            limit: 50,
        }))
    };
    assert_eq!(
        outs,
        [
            ask(AgentKind::Claude),
            ask(AgentKind::OpenCode),
            ask(AgentKind::Codex)
        ]
    );
    assert!(a.requests.is_empty(), "the picker's queue is not used");
    // Answers in any order land in their own columns.
    answer(&mut a, AgentKind::Codex, codex(), None);
    let restore = a.restore.as_ref().unwrap();
    assert_eq!(restore.cols[0].load, Load::Loading);
    assert_eq!(restore.cols[1].load, Load::Loading);
    assert!(matches!(restore.cols[2].load, Load::Ready(_)));
    assert!(a.requests.is_empty());
}

#[test]
fn the_dialog_draws_three_columns_with_title_directory_and_age() {
    let a = opened();
    let text = dialog(&a);
    for want in [
        "Restore agent session",
        "Claude Code (2)",
        "OpenCode (1)",
        "Codex (1)",
        "Fix the login redirect",
        "…Projects/budget · 2h ago",
        "Add rate limit",
        "/w/web/api · 5d ago",
        "Rework login form",
        "~/Projects/web · 3d ago",
        "Write the parser",
        "…ewhere/deep/dir · 1m ago",
        "> █",
        "⏎ resume  ←→ column  ↑↓ row  type to filter  esc close",
    ] {
        assert!(text.contains(want), "missing {want:?} in\n{text}");
    }
    // The first non-empty column's first row starts selected.
    assert_eq!(selected(&a).as_deref(), Some("c1"));
    let buf = buffer(&a);
    let cell = &buf[(7, 8)];
    let themed = cell.bg == color(a.theme.selection_bg) && cell.fg == color(a.theme.selection_fg);
    assert!(
        cell.modifier.contains(Modifier::REVERSED) || themed,
        "{cell:?}"
    );
    // The active column's header is in the accent colour and bold.
    assert_eq!(buf[(7, 6)].fg, color(a.theme.accent));
    assert!(buf[(10, 6)].modifier.contains(Modifier::BOLD));
    assert_eq!(buf[(10, 6)].fg, color(a.theme.accent));
    assert_ne!(buf[(36, 6)].fg, color(a.theme.accent));
    assert_ne!(buf[(39, 6)].fg, color(a.theme.accent));
}

#[test]
fn a_column_shows_loading_its_error_or_no_sessions() {
    let mut a = app();
    open(&mut a);
    answer(&mut a, AgentKind::Claude, claude(), None);
    answer(&mut a, AgentKind::OpenCode, vec![], Some("opencode: boom"));
    let text = dialog(&a);
    assert!(text.contains("opencode: boom"), "{text}");
    assert!(text.contains("loading…"), "{text}");
    let buf = buffer(&a);
    assert_eq!(buf[(37, 8)].fg, color(a.theme.warning));
    assert_eq!(buf[(66, 8)].fg, color(a.theme.muted));
    answer(&mut a, AgentKind::Codex, vec![], None);
    let text = dialog(&a);
    assert!(!text.contains("loading…"));
    assert!(
        text.contains("Codex (0)") && text.contains("no sessions"),
        "{text}"
    );
    assert_eq!(buffer(&a)[(66, 8)].fg, color(a.theme.muted));
}

#[test]
fn a_slow_column_never_blocks_the_others() {
    let mut a = app();
    open(&mut a);
    answer(&mut a, AgentKind::Claude, claude(), None);
    answer(&mut a, AgentKind::Codex, codex(), None);
    assert!(dialog(&a).contains("loading…"));
    // Keys work while it loads, and its late answer joins in.
    press(&mut a, ct::KeyCode::Down);
    assert_eq!(selected(&a).as_deref(), Some("c2"));
    answer(&mut a, AgentKind::OpenCode, opencode(), None);
    assert!(dialog(&a).contains("Rework login form"));
    assert_eq!(selected(&a).as_deref(), Some("c2"));
}

#[test]
fn typing_filters_all_columns_and_escape_clears_before_it_closes() {
    let mut a = opened();
    typed(&mut a, "login");
    let text = dialog(&a);
    assert!(text.contains("> login█"), "{text}");
    assert!(text.contains("Claude Code (1)") && text.contains("OpenCode (1)"));
    assert!(text.contains("Codex (0)") && text.contains("no sessions"));
    assert!(!text.contains("Add rate limit"));
    // The filter matches the directory too, every word separately.
    ctrl(&mut a, 'u');
    typed(&mut a, "budget fix");
    assert_eq!(a.restore.as_ref().unwrap().visible(0).len(), 1);
    press(&mut a, ct::KeyCode::Backspace);
    assert_eq!(a.restore.as_ref().unwrap().filter.text(), "budget fi");
    press(&mut a, ct::KeyCode::Esc);
    let restore = a.restore.as_ref().unwrap();
    assert_eq!(restore.filter.text(), "");
    assert_eq!(restore.visible(0).len(), 2);
    press(&mut a, ct::KeyCode::Esc);
    assert!(a.restore.is_none());
}

#[test]
fn the_selection_moves_to_the_nearest_row_when_the_filter_changes() {
    let mut a = opened();
    press(&mut a, ct::KeyCode::Down);
    assert_eq!(selected(&a).as_deref(), Some("c2"));
    // Only the first session matches: the selection falls back onto it.
    typed(&mut a, "login");
    assert_eq!(selected(&a).as_deref(), Some("c1"));
    // The active column empties: the selection goes to the nearest column with rows.
    ctrl(&mut a, 'u');
    typed(&mut a, "parser");
    assert_eq!(selected(&a).as_deref(), Some("x1"));
    assert_eq!(a.restore.as_ref().unwrap().active, 2);
}

#[test]
fn arrows_tab_and_ctrl_keys_move_and_empty_columns_are_skipped() {
    let mut a = app();
    open(&mut a);
    answer(&mut a, AgentKind::Claude, claude(), None);
    answer(&mut a, AgentKind::OpenCode, vec![], None);
    answer(&mut a, AgentKind::Codex, codex(), None);
    // Right skips the empty OpenCode column.
    press(&mut a, ct::KeyCode::Right);
    assert_eq!(selected(&a).as_deref(), Some("x1"));
    press(&mut a, ct::KeyCode::Right);
    assert_eq!(selected(&a).as_deref(), Some("x1"));
    press(&mut a, ct::KeyCode::Left);
    assert_eq!(selected(&a).as_deref(), Some("c1"));
    // Tab visits every column, empty ones too, and wraps; Shift+Tab goes back.
    press(&mut a, ct::KeyCode::Tab);
    assert_eq!(a.restore.as_ref().unwrap().active, 1);
    assert_eq!(selected(&a), None);
    press(&mut a, ct::KeyCode::Tab);
    assert_eq!(selected(&a).as_deref(), Some("x1"));
    press(&mut a, ct::KeyCode::Tab);
    assert_eq!(selected(&a).as_deref(), Some("c1"));
    a.on_event(
        key(ct::KeyCode::Tab, ct::KeyModifiers::SHIFT),
        Instant::now(),
    );
    assert_eq!(selected(&a).as_deref(), Some("x1"));
    // Rows: Down and Ctrl+n, Up and Ctrl+p, clamped.
    press(&mut a, ct::KeyCode::Left);
    ctrl(&mut a, 'n');
    ctrl(&mut a, 'n');
    assert_eq!(selected(&a).as_deref(), Some("c2"));
    ctrl(&mut a, 'p');
    assert_eq!(selected(&a).as_deref(), Some("c1"));
    press(&mut a, ct::KeyCode::End);
    assert_eq!(selected(&a).as_deref(), Some("c2"));
    press(&mut a, ct::KeyCode::Home);
    assert_eq!(selected(&a).as_deref(), Some("c1"));
    press(&mut a, ct::KeyCode::PageDown);
    assert_eq!(selected(&a).as_deref(), Some("c2"));
    press(&mut a, ct::KeyCode::PageUp);
    assert_eq!(selected(&a).as_deref(), Some("c1"));
}

#[test]
fn with_every_column_empty_the_arrows_still_move_between_columns() {
    let mut a = app();
    open(&mut a);
    press(&mut a, ct::KeyCode::Right);
    assert_eq!(a.restore.as_ref().unwrap().active, 1);
    press(&mut a, ct::KeyCode::Right);
    press(&mut a, ct::KeyCode::Right);
    assert_eq!(a.restore.as_ref().unwrap().active, 2);
}

#[test]
fn the_prefix_is_ordinary_text_and_keys_never_reach_the_terminal() {
    let mut a = opened();
    assert!(
        a.on_event(
            key(ct::KeyCode::Char(' '), ct::KeyModifiers::ALT),
            Instant::now()
        )
        .is_empty()
    );
    assert!(!a.armed);
    assert!(ch(&mut a, 'x').is_empty());
    assert!(ch(&mut a, 'R').is_empty());
    assert_eq!(a.restore.as_ref().unwrap().filter.text(), "xR");
    a.on_event(ct::Event::Paste("a\nb".into()), Instant::now());
    assert_eq!(a.restore.as_ref().unwrap().filter.text(), "xRab");
}

#[test]
fn enter_on_the_codex_column_resumes_that_session_in_the_active_project() {
    let mut a = opened();
    press(&mut a, ct::KeyCode::Right);
    press(&mut a, ct::KeyCode::Right);
    press(&mut a, ct::KeyCode::Down);
    let outs = press(&mut a, ct::KeyCode::Enter);
    // The session is outside every project: the active one (1) gets it.
    assert_eq!(
        outs,
        [resume(AgentKind::Codex, "x1", "/elsewhere/deep/dir", 1)]
    );
    assert!(a.restore.is_none());
    assert!(a.requests.is_empty());
}

#[test]
fn a_session_inside_another_projects_root_opens_there() {
    let mut a = opened();
    press(&mut a, ct::KeyCode::Down);
    assert_eq!(selected(&a).as_deref(), Some("c2"));
    let outs = press(&mut a, ct::KeyCode::Enter);
    assert_eq!(outs, [resume(AgentKind::Claude, "c2", "/w/web/api", 2)]);
}

#[test]
fn a_folded_project_unfolds_for_the_new_panel() {
    let mut view = two_projects();
    view.projects[1].collapsed = true;
    let mut a = app_with(&config(), SIZE, view);
    open(&mut a);
    answer(&mut a, AgentKind::Claude, claude(), None);
    press(&mut a, ct::KeyCode::Down);
    let outs = press(&mut a, ct::KeyCode::Enter);
    assert_eq!(outs.len(), 2);
    assert_eq!(
        outs[1],
        Out::Send(ClientMsg::Action(Act::SetProjectCollapsed {
            project: ProjectId(2),
            collapsed: false
        }))
    );
}

#[test]
fn enter_with_nothing_selected_does_nothing_and_a_refusal_is_an_error_flash() {
    let mut a = app();
    open(&mut a);
    assert!(press(&mut a, ct::KeyCode::Enter).is_empty());
    assert!(a.restore.is_some());
    answer(&mut a, AgentKind::Claude, claude(), None);
    press(&mut a, ct::KeyCode::Enter);
    a.on_server(ServerMsg::ActionResult(Err(ActionError::ProgramNotFound {
        program: "claude".into(),
        kind: AgentKind::Claude,
        key: "resume".into(),
    })));
    assert!(a.flash_error);
    let (text, until) = a.flash.clone().unwrap();
    assert!(text.contains("not found"), "{text}");
    assert!(until <= Instant::now() + ERROR_FLASH_FOR);
}

#[test]
fn a_successful_launch_ends_the_sidebar_focus() {
    let mut a = opened();
    a.sidebar_focus = true;
    press(&mut a, ct::KeyCode::Enter);
    assert!(a.sidebar_focus);
    a.on_server(ServerMsg::ActionResult(Ok(())));
    assert!(!a.sidebar_focus);
}

#[test]
fn closing_drops_late_answers_and_changes_nothing() {
    let mut a = app();
    open(&mut a);
    answer(&mut a, AgentKind::Claude, claude(), None);
    press(&mut a, ct::KeyCode::Esc);
    assert!(a.restore.is_none());
    let before = a.flash.clone();
    answer(&mut a, AgentKind::OpenCode, opencode(), None);
    answer(&mut a, AgentKind::Codex, vec![], Some("late"));
    assert!(a.restore.is_none());
    assert_eq!(a.flash, before);
    assert!(a.requests.is_empty());
    // Reopening starts from scratch: nothing of the old answers is there.
    open(&mut a);
    assert_eq!(a.restore.as_ref().unwrap().cols[0].load, Load::Loading);
}

#[test]
fn a_hundred_sessions_scroll_with_the_selection_and_end_reaches_the_last() {
    let many: Vec<_> = (0..50)
        .map(|i| {
            session(
                AgentKind::Claude,
                &format!("id{i}"),
                &format!("Session number {i}"),
                "/w/api",
                60 * (i + 1),
            )
        })
        .collect();
    let mut a = app();
    open(&mut a);
    answer(&mut a, AgentKind::Claude, many, None);
    let first_page = dialog(&a);
    assert!(first_page.contains("Session number 0") && !first_page.contains("Session number 9\n"));
    for _ in 0..20 {
        press(&mut a, ct::KeyCode::Down);
    }
    let text = dialog(&a);
    assert!(text.contains("Session number 20"), "{text}");
    assert!(!text.contains("Session number 0 "), "scrolled past the top");
    press(&mut a, ct::KeyCode::End);
    assert_eq!(selected(&a).as_deref(), Some("id49"));
    assert!(dialog(&a).contains("Session number 49"));
    press(&mut a, ct::KeyCode::PageUp);
    assert_eq!(selected(&a).as_deref(), Some("id42"));
    assert!(dialog(&a).contains("Session number 42"));
    press(&mut a, ct::KeyCode::Home);
    assert!(dialog(&a).contains("Session number 0"));
}

#[test]
fn a_click_selects_a_second_click_resumes_and_a_click_outside_closes() {
    let mut a = opened();
    // The second Claude row: list rows 8-9 are the first session, 10-11 the second.
    assert!(click(&mut a, 10, 10).is_empty());
    assert_eq!(selected(&a).as_deref(), Some("c2"));
    // A row of another column.
    click(&mut a, 40, 8);
    assert_eq!(selected(&a).as_deref(), Some("o1"));
    click(&mut a, 70, 9);
    assert_eq!(selected(&a).as_deref(), Some("x1"));
    let outs = click(&mut a, 70, 8);
    assert_eq!(
        outs,
        [resume(AgentKind::Codex, "x1", "/elsewhere/deep/dir", 1)]
    );
    assert!(a.restore.is_none());

    let mut a = opened();
    assert!(click(&mut a, 1, 1).is_empty());
    assert!(a.restore.is_none());
    // The release of a click that closed the dialog goes nowhere.
    assert!(!a.swallow_release);
}

#[test]
fn a_click_inside_the_dialog_but_off_a_row_keeps_it_open_and_a_header_moves_the_column() {
    let mut a = opened();
    click(&mut a, 20, 20);
    click(&mut a, 20, 5);
    assert!(a.restore.is_some());
    click(&mut a, 40, 6);
    assert_eq!(selected(&a).as_deref(), Some("o1"));
    assert_eq!(a.restore.as_ref().unwrap().active, 1);
}

#[test]
fn hover_marks_a_row_with_muted_reversed_and_the_wheel_scrolls_the_column_under_it() {
    let mut a = opened();
    moved(&mut a, 40, 8);
    assert_eq!(a.restore.as_ref().unwrap().hover, Some((1, 0)));
    let buf = buffer(&a);
    assert_eq!(buf[(37, 8)].fg, color(a.theme.muted));
    assert!(buf[(37, 8)].modifier.contains(Modifier::REVERSED));
    moved(&mut a, 40, 20);
    assert_eq!(a.restore.as_ref().unwrap().hover, None);

    let many: Vec<_> = (0..30)
        .map(|i| {
            session(
                AgentKind::OpenCode,
                &format!("id{i}"),
                &format!("S{i}"),
                "/w/api",
                60,
            )
        })
        .collect();
    answer(&mut a, AgentKind::OpenCode, many, None);
    let offsets = |a: &App| {
        let r = a.restore.as_ref().unwrap();
        r.cols.clone().map(|c| c.offset)
    };
    mouse(&mut a, ct::MouseEventKind::ScrollDown, 40, 10);
    assert_eq!(offsets(&a), [0, 2, 0]);
    mouse(&mut a, ct::MouseEventKind::ScrollDown, 10, 10);
    assert_eq!(offsets(&a), [0, 2, 0], "the other column is short");
    mouse(&mut a, ct::MouseEventKind::ScrollUp, 40, 10);
    assert_eq!(offsets(&a), [0, 0, 0]);
    // The selection did not move with the wheel.
    assert_eq!(selected(&a).as_deref(), Some("c1"));
}

#[test]
fn a_narrow_window_shows_one_column_and_tab_switches() {
    let mut a = app_with(&config(), (50, 24), two_projects());
    open(&mut a);
    answer(&mut a, AgentKind::Claude, claude(), None);
    answer(&mut a, AgentKind::OpenCode, opencode(), None);
    answer(&mut a, AgentKind::Codex, codex(), None);
    let text = dialog(&a);
    assert!(text.contains("‹ ✻  Claude Code (2) ›"), "{text}");
    assert!(text.contains("Fix the login redirect"));
    assert!(!text.contains("Rework login form"));
    assert!(text.contains("⏎ resume  tab column  esc close"));
    press(&mut a, ct::KeyCode::Tab);
    let text = dialog(&a);
    assert!(text.contains("‹ ◇  OpenCode (1) ›") && text.contains("Rework login form"));
    assert!(!text.contains("Fix the login redirect"));
    // The status bar shows the dialog's hints.
    assert!(
        screen(&a)
            .last()
            .unwrap()
            .contains("⏎ resume  tab column  esc close")
    );
    // A click on the selected row resumes it.
    let rows = screen(&a);
    let y = rows
        .iter()
        .position(|l| l.contains("Rework login"))
        .unwrap();
    let outs = click(&mut a, 8, u16::try_from(y).unwrap());
    assert_eq!(
        outs,
        [resume(
            AgentKind::OpenCode,
            "o1",
            "/home/me/Projects/web",
            1
        )]
    );
}

#[test]
fn the_status_bar_shows_the_dialogs_hints_and_the_dialog_is_gone_with_esc() {
    let mut a = opened();
    assert!(
        screen(&a)
            .last()
            .unwrap()
            .contains("⏎ resume  ←→ column  ↑↓ row  type to filter  esc close")
    );
    press(&mut a, ct::KeyCode::Esc);
    assert!(!dialog(&a).contains("Restore agent session"));
}

fn chip_row(a: &App) -> u16 {
    a.size.1 - 2
}

fn row_text(a: &App, y: u16, from: u16, to: u16) -> String {
    let buf = buffer(a);
    (from..to).map(|x| buf[(x, y)].symbol()).collect()
}

#[test]
fn the_button_is_drawn_at_the_right_of_the_bar_and_hovers_like_the_new_chip() {
    let mut a = app();
    let y = chip_row(&a);
    assert_eq!(row_text(&a, y, 0, 29), "  + New            Restore  │");
    moved(&mut a, 20, y);
    assert_eq!(a.hover, Some(Hover::Restore));
    let buf = buffer(&a);
    let cell = &buf[(20, y)];
    let themed = cell.bg == color(a.theme.selection_bg) && cell.fg == color(a.theme.selection_fg);
    assert!(
        cell.modifier.contains(Modifier::REVERSED) || themed,
        "{cell:?}"
    );
    // The new project chip is idle meanwhile, and the margin and the gap stay plain.
    assert_eq!(buf[(3, y)].fg, color(a.theme.muted));
    assert!(!buf[(27, y)].modifier.contains(Modifier::REVERSED));
    moved(&mut a, 3, y);
    assert_eq!(a.hover, Some(Hover::New));
    moved(&mut a, 10, y);
    assert_eq!(a.hover, None);
    moved(&mut a, 17, y);
    assert_eq!(a.hover, None);
    moved(&mut a, 18, y);
    assert_eq!(a.hover, Some(Hover::Restore));
    moved(&mut a, 27, y);
    assert_eq!(a.hover, None);
}

#[test]
fn at_a_sidebar_of_twenty_columns_the_button_is_a_single_glyph() {
    let config = Config {
        sidebar_width: 20,
        ..Config::default()
    };
    let mut a = app_with(&config, SIZE, two_projects());
    let y = chip_row(&a);
    assert_eq!(row_text(&a, y, 0, 21), "  +              ↻  │");
    moved(&mut a, 16, y);
    assert_eq!(a.hover, Some(Hover::Restore));
    moved(&mut a, 15, y);
    assert_eq!(a.hover, None);
    let outs = click(&mut a, 17, y);
    assert!(a.restore.is_some());
    assert_eq!(outs.len(), 3);
}

#[test]
fn clicking_the_button_opens_the_dialog_and_a_press_released_elsewhere_does_not() {
    let mut a = app();
    let y = chip_row(&a);
    let outs = click(&mut a, 20, y);
    assert_eq!(outs.len(), 3);
    assert!(a.restore.is_some());
    assert_eq!(a.hover, None);

    let mut a = app();
    mouse(&mut a, ct::MouseEventKind::Down(LEFT), 20, y);
    mouse(&mut a, ct::MouseEventKind::Up(LEFT), 30, y);
    assert!(a.restore.is_none());
}

#[test]
fn r_in_sidebar_focus_and_the_binding_open_the_dialog() {
    let mut a = app();
    a.sidebar_focus = true;
    let outs = ch(&mut a, 'R');
    assert_eq!(outs.len(), 3);
    assert!(a.restore.is_some());
    // The dialog has the keyboard, not the sidebar.
    press(&mut a, ct::KeyCode::Esc);
    assert!(a.restore.is_none() && a.sidebar_focus);
}

#[test]
fn without_a_project_the_button_and_the_binding_flash_and_open_nothing() {
    let view = SessionView {
        projects: vec![],
        active_project: None,
        active_panel: None,
        ..mockup()
    };
    let mut a = app_with(&config(), SIZE, view);
    prefix(&mut a);
    assert!(ch(&mut a, 'R').is_empty());
    assert!(a.restore.is_none());
    assert_eq!(a.flash.as_ref().unwrap().0, "no space to restore into");
    a.flash = None;
    let y = chip_row(&a);
    assert!(click(&mut a, 20, y).is_empty());
    assert!(a.restore.is_none());
    assert_eq!(a.flash.as_ref().unwrap().0, "no space to restore into");
}

#[test]
fn a_configured_key_moves_the_binding_and_the_help_lists_it() {
    let config = Config::parse("[keys]\nrestore = \"Z\"").unwrap().config;
    let mut a = app_with(&config, SIZE, two_projects());
    prefix(&mut a);
    ch(&mut a, 'R');
    assert!(a.restore.is_none());
    prefix(&mut a);
    ch(&mut a, 'Z');
    assert!(a.restore.is_some());
    press(&mut a, ct::KeyCode::Esc);
    prefix(&mut a);
    ch(&mut a, '?');
    let text = dialog(&a);
    assert!(
        text.lines()
            .any(|l| l.contains("Restore an agent session") && l.contains('Z')),
        "{text}"
    );
}

#[test]
fn the_dialog_closes_over_the_hover_and_a_new_session_view_forgets_it() {
    let mut a = app();
    let y = chip_row(&a);
    moved(&mut a, 20, y);
    assert_eq!(a.hover, Some(Hover::Restore));
    open(&mut a);
    assert_eq!(a.hover, None);
    a.reset_session();
    assert!(a.restore.is_none());
}

/// Every cell outside the dialog's rectangle is the same as in a frame without the dialog.
fn nothing_outside(a: &mut App) {
    let g = a.restore_geometry().unwrap();
    // The status bar (the last row) shows the dialog's hints; the dialog stays above it.
    assert!(g.rect.y + g.rect.height < a.size.1);
    let with = buffer(a);
    let restore = a.restore.take();
    let without = buffer(a);
    a.restore = restore;
    for y in 0..with.area.height - 1 {
        for x in 0..with.area.width {
            let inside = (g.rect.x..g.rect.x + g.rect.width).contains(&x)
                && (g.rect.y..g.rect.y + g.rect.height).contains(&y);
            if !inside {
                assert_eq!(with[(x, y)], without[(x, y)], "cell {x},{y} changed");
            }
        }
    }
}

#[test]
fn a_short_window_draws_without_panicking_or_leaving_the_dialog() {
    for size in [
        (60, 5),
        (60, 6),
        (60, 7),
        (60, 9),
        (20, 5),
        (20, 6),
        (20, 8),
    ] {
        let mut a = app_with(&config(), size, two_projects());
        open(&mut a);
        // Claude loading, OpenCode empty, Codex failed.
        answer(&mut a, AgentKind::OpenCode, vec![], None);
        answer(
            &mut a,
            AgentKind::Codex,
            vec![],
            Some("could not run codex"),
        );
        for active in 0..3 {
            a.restore.as_mut().unwrap().active = active;
            nothing_outside(&mut a);
        }
        // Loaded rows too.
        answer(&mut a, AgentKind::Claude, claude(), None);
        for active in 0..3 {
            a.restore.as_mut().unwrap().active = active;
            nothing_outside(&mut a);
        }
        // The pointer anywhere does not panic either.
        for y in 0..size.1 {
            for x in 0..size.0 {
                moved(&mut a, x, y);
            }
        }
    }
}

#[test]
fn tab_reaches_a_failed_column_in_the_narrow_mode() {
    let mut a = app_with(&config(), (50, 20), two_projects());
    open(&mut a);
    answer(&mut a, AgentKind::Claude, claude(), None);
    answer(
        &mut a,
        AgentKind::OpenCode,
        vec![],
        Some("could not run opencode"),
    );
    answer(&mut a, AgentKind::Codex, codex(), None);
    press(&mut a, ct::KeyCode::Tab);
    let text = dialog(&a);
    assert!(text.contains("OpenCode"), "{text}");
    assert!(text.contains("could not run opencode"), "{text}");
    press(&mut a, ct::KeyCode::Tab);
    assert!(dialog(&a).contains("Codex"));
}

#[test]
fn the_blank_last_line_of_an_odd_list_is_not_a_row() {
    let height = (20..60)
        .find(|&h| {
            let mut b = app_with(&config(), (100, h), two_projects());
            open(&mut b);
            b.restore_geometry().is_some_and(|g| g.list_rows % 2 == 1)
        })
        .unwrap();
    let mut b = app_with(&config(), (100, height), two_projects());
    open(&mut b);
    answer(&mut b, AgentKind::Claude, claude(), None);
    let g = b.restore_geometry().unwrap();
    let (x, _) = g.column(0);
    let last = g.list_y + g.list_rows - 1;
    assert_eq!(b.restore_hit(x + 2, last), Hit::List(0));
}

#[test]
fn an_answer_does_not_snap_a_wheel_scrolled_column_back() {
    let mut a = app();
    open(&mut a);
    let many: Vec<AgentSession> = (0..30)
        .map(|i| {
            session(
                AgentKind::OpenCode,
                &format!("o{i}"),
                "Title",
                "/work/proj",
                i,
            )
        })
        .collect();
    answer(&mut a, AgentKind::OpenCode, many, None);
    let g = a.restore_geometry().unwrap();
    let (x, _) = g.column(1);
    mouse(&mut a, ct::MouseEventKind::ScrollDown, x + 2, g.list_y);
    let offset = a.restore.as_ref().unwrap().cols[1].offset;
    assert!(offset > 0);
    answer(&mut a, AgentKind::Claude, claude(), None);
    assert_eq!(a.restore.as_ref().unwrap().cols[1].offset, offset);
}

#[test]
fn with_images_the_headings_draw_logos_sent_before_the_frame() {
    let mut a = opened();
    a.truecolor = true;
    a.images = Some(crate::graphics::Images::new(
        Some((10, 20, 30)),
        Some((1, 1, 1)),
        [None; 16],
    ));
    let whole = |buf: &Buffer, x: u16| {
        buf[(x, 6)].symbol() == "\u{10EEEE}\u{0305}\u{0305}"
            && buf[(x + 1, 6)].symbol() == "\u{10EEEE}\u{0305}\u{030D}"
    };
    // Before the images are sent the headings show text icons.
    assert!(!whole(&buffer(&a), 7));
    let mut sink = Vec::new();
    a.send_images(&mut sink).unwrap();
    let buf = buffer(&a);
    // Each column's heading starts with its logo: the active column in the accent colour.
    let starts: Vec<u16> = (3..95).filter(|&x| whole(&buf, x)).collect();
    assert_eq!(starts.len(), 3, "{starts:?}");
    assert_eq!(starts[0], 7);
    let sent = &a.images.as_ref().unwrap().order;
    for x in starts {
        let ratatui::style::Color::Rgb(r, g, b) = buf[(x, 6)].fg else {
            panic!("no id colour at {x}")
        };
        let id = u32::from_be_bytes([0, r, g, b]);
        assert!(sent.iter().any(|(i, _)| *i == id), "{id:x}");
    }
    // The name follows the icon and a space.
    assert_eq!(row_text(&a, 6, 10, 21), "Claude Code");
}
