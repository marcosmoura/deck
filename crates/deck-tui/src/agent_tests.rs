//! Agent glyphs: the sidebar's status column, the terminal titles and the spinner's timing.

use std::path::PathBuf;
use std::time::Duration;

use deck_core::agents::{AgentKind, AgentSource, AgentState, AgentStatus};
use deck_core::config::Config;
use deck_core::model::{SessionView, TerminalId};
use deck_core::protocol::ServerMsg;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Modifier;

use crate::agent::{self, SPIN_EVERY};
use crate::app::App;
use crate::draw::{color, draw};
use crate::mirror::tests::{frame, mockup, panel};

fn state(kind: AgentKind, status: AgentStatus) -> AgentState {
    AgentState {
        kind,
        status,
        source: AgentSource::Hooks,
        session_id: None,
        first_prompt: None,
        since: Duration::ZERO,
    }
}

fn set(view: &mut SessionView, terminal: u64, status: AgentStatus) {
    let found = view
        .projects
        .iter_mut()
        .flat_map(|p| &mut p.panels)
        .flat_map(|p| &mut p.terminals)
        .find(|t| t.id == TerminalId(terminal))
        .unwrap();
    found.agent = Some(state(AgentKind::Claude, status));
}

fn app_with(view: SessionView) -> App {
    let mut app =
        App::new(&Config::default(), (100, 24), &[]).with_dirs(PathBuf::from("/work/proj"), None);
    app.on_server(ServerMsg::SessionView(view));
    if let Some(t) = app.mirror.focused_id() {
        let mut f = frame(true, (40, 10), vec![]);
        f.terminal = t;
        app.on_server(ServerMsg::Frame(f));
    }
    app
}

fn render(a: &App) -> Buffer {
    let mut t = Terminal::new(TestBackend::new(a.size.0, a.size.1)).unwrap();
    t.draw(|f| draw(f, a)).unwrap();
    t.backend().buffer().clone()
}

fn row(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
}

/// The mockup with an agent in each of its first four panels.
fn mixed() -> SessionView {
    let mut view = mockup();
    set(&mut view, 31, AgentStatus::Working);
    set(&mut view, 41, AgentStatus::Waiting);
    set(&mut view, 51, AgentStatus::Idle);
    set(&mut view, 61, AgentStatus::Quiet);
    view
}

#[test]
fn the_sidebar_status_column_shows_each_panels_glyph() {
    let a = app_with(mixed());
    let buf = render(&a);
    let side = |y| row(&buf, y).chars().take(14).collect::<String>();
    assert_eq!(side(3), "▌⠋ ✻  editor  ");
    assert_eq!(side(4), " ! ✻  claude  ");
    assert_eq!(side(5), " ● ✻  tests   ");
    assert_eq!(side(7), " ○ ✻  api     ");
    // Roles: waiting is a bold warning, idle is ok, quiet is muted, working is the accent.
    let theme = &a.theme;
    assert_eq!(buf[(1, 3)].fg, color(theme.accent));
    assert_eq!(buf[(1, 4)].fg, color(theme.warning));
    assert!(buf[(1, 4)].modifier.contains(Modifier::BOLD));
    assert_eq!(buf[(1, 5)].fg, color(theme.ok));
    assert_eq!(buf[(1, 7)].fg, color(theme.muted));
}

#[test]
fn a_panel_shows_its_most_urgent_terminal() {
    use AgentStatus::{Active, Idle, Quiet, Waiting, Working};
    let mut p = panel(3, "mixed", 4);
    assert_eq!(agent::panel_status(&p), AgentStatus::None);
    for (t, status) in p.terminals.iter_mut().zip([Quiet, Idle, Quiet, Idle]) {
        t.agent = Some(state(AgentKind::Codex, status));
    }
    assert_eq!(agent::panel_status(&p), Idle);
    p.terminals[2].agent = Some(state(AgentKind::Codex, Active));
    assert_eq!(agent::panel_status(&p), Active);
    p.terminals[1].agent = Some(state(AgentKind::Codex, Working));
    assert_eq!(agent::panel_status(&p), Working);
    p.terminals[0].agent = Some(state(AgentKind::Codex, Waiting));
    assert_eq!(agent::panel_status(&p), Waiting);
}

#[test]
fn a_grid_title_carries_the_glyph_and_the_kind() {
    let mut view = mockup();
    view.projects[0].panels[0] = panel(3, "editor", 2);
    set(&mut view, 31, AgentStatus::Waiting);
    let a = app_with(view);
    let buf = render(&a);
    let top = row(&buf, 0);
    assert!(top.contains("╭─ ! claude · sh "), "{top}");
    // The other terminal keeps its number.
    assert!(top.contains(" 2 · sh "), "{top}");
    let at = top.chars().position(|c| c == '!').unwrap();
    assert!(
        buf[(u16::try_from(at).unwrap(), 0)]
            .modifier
            .contains(Modifier::BOLD)
    );
}

#[test]
fn a_cut_grid_title_keeps_its_closing_border() {
    let mut view = mockup();
    view.projects[0].panels[0] = panel(3, "editor", 2);
    for t in &mut view.projects[0].panels[0].terminals {
        t.title = Some("a very long title that cannot fit in a narrow grid cell".into());
    }
    set(&mut view, 31, AgentStatus::Waiting);
    let a = app_with(view);
    let buf = render(&a);
    let top = row(&buf, 0);
    assert!(top.contains("… ─╮"), "{top}");
}

#[test]
fn a_single_terminal_shows_its_icon_and_name_in_the_status_bar() {
    let mut view = mockup();
    set(&mut view, 31, AgentStatus::Idle);
    let a = app_with(view);
    let buf = render(&a);
    let bar = row(&buf, 23);
    assert!(bar.contains("terminal-multiplexer › ● ✻  editor"), "{bar}");
    assert!(!bar.contains(" · "), "{bar}");
    let at = u16::try_from(bar.chars().position(|c| c == '●').unwrap()).unwrap();
    assert_eq!(buf[(at, 23)].fg, color(a.theme.ok));
}

#[test]
fn the_spinner_advances_every_100_ms_and_only_while_something_spins() {
    let mut a = app_with(mockup());
    a.take_dirty();
    assert_eq!(a.wake_deadline(), None, "no timer while nothing spins");

    a.on_server(ServerMsg::SessionView({
        let mut v = mockup();
        set(&mut v, 31, AgentStatus::Working);
        v
    }));
    a.take_dirty();
    let at = a.wake_deadline().expect("a timer for the spinner");
    a.tick(at - Duration::from_millis(1));
    assert!(!a.take_dirty());
    assert_eq!(row(&render(&a), 3).chars().nth(1), Some('⠋'));

    a.tick(at);
    assert!(a.take_dirty());
    assert_eq!(row(&render(&a), 3).chars().nth(1), Some('⠙'));
    assert_eq!(a.wake_deadline(), Some(at + SPIN_EVERY));

    // The agent finished: the last tick stops the timer.
    a.on_server(ServerMsg::SessionView({
        let mut v = mockup();
        set(&mut v, 31, AgentStatus::Idle);
        v
    }));
    a.tick(at + SPIN_EVERY);
    assert_eq!(a.wake_deadline(), None);
    assert!(a.take_dirty());
    a.tick(at + SPIN_EVERY * 2);
    assert!(!a.take_dirty());
}

#[test]
fn a_process_detected_agent_spins_in_the_accent_colour() {
    let mut view = mockup();
    set(&mut view, 31, AgentStatus::Active);
    let a = app_with(view);
    let buf = render(&a);
    assert_eq!(buf[(1, 3)].symbol(), "⠋");
    assert_eq!(buf[(1, 3)].fg, color(a.theme.accent));
    assert!(a.wake_deadline().is_some());
}

#[test]
fn an_unseen_panel_is_bold_with_the_glyph_after_its_name() {
    let mut view = mockup();
    set(&mut view, 41, AgentStatus::Waiting);
    view.projects[0].panels[1].unseen = true;
    let a = app_with(view);
    let buf = render(&a);
    let side = |y| row(&buf, y).chars().take(14).collect::<String>();
    assert_eq!(side(4), " ! ✻  claude •");
    assert_eq!(side(5), "   ❯  tests   ");
    for x in 6..12 {
        assert!(buf[(x, 4)].modifier.contains(Modifier::BOLD), "{x}");
    }
    assert!(!buf[(6, 5)].modifier.contains(Modifier::BOLD));
    assert_eq!(buf[(13, 4)].fg, color(a.theme.accent));
}

#[test]
fn a_collapsed_project_counts_its_waiting_panels() {
    let mut view = mockup();
    // `dotfiles` is collapsed with panels 9 and 10.
    set(&mut view, 91, AgentStatus::Waiting);
    set(&mut view, 101, AgentStatus::Working);
    let a = app_with(view);
    let buf = render(&a);
    let line = row(&buf, 10).chars().take(28).collect::<String>();
    assert!(line.starts_with(" ▸ dotfiles !1"), "{line}");
    assert!(line.ends_with("2 "), "{line}");
    let at = u16::try_from(line.chars().position(|c| c == '!').unwrap()).unwrap();
    assert_eq!(buf[(at, 10)].fg, color(a.theme.warning));
    // An expanded project shows the panels' own glyphs instead.
    let mut view = mockup();
    set(&mut view, 41, AgentStatus::Waiting);
    let buf = render(&app_with(view));
    assert!(!row(&buf, 2).contains("!1"));
}

#[test]
fn the_status_bar_counts_waiting_terminals_and_hints_the_jump() {
    let a = app_with(mockup());
    let bar = row(&render(&a), 23);
    assert!(
        !bar.contains("waiting") && !bar.contains(" a next"),
        "{bar}"
    );

    let mut view = mockup();
    set(&mut view, 41, AgentStatus::Waiting);
    set(&mut view, 61, AgentStatus::Waiting);
    set(&mut view, 91, AgentStatus::Idle);
    let a = app_with(view);
    let buf = render(&a);
    let bar = row(&buf, 23);
    assert!(bar.contains("2 waiting  "), "{bar}");
    assert!(bar.contains(" a next  "), "{bar}");
    let at = u16::try_from(bar.chars().position(|c| c == '2').unwrap()).unwrap();
    assert_eq!(buf[(at, 23)].fg, color(a.theme.warning));
}
