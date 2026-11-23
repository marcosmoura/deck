//! Program and agent icons in the sidebar rows and the status bar.

use std::path::PathBuf;
use std::time::Duration;

use deck_core::agents::{AgentKind, AgentSource, AgentState, AgentStatus};
use deck_core::config::Config;
use deck_core::model::{NameSource, PanelId, SessionView};
use deck_core::protocol::ServerMsg;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Modifier;

use crate::app::App;
use crate::draw::{color, draw};
use crate::mirror::tests::{frame, mockup, panel, project};

fn agent(kind: AgentKind, status: AgentStatus) -> AgentState {
    AgentState {
        kind,
        status,
        source: AgentSource::Hooks,
        session_id: None,
        first_prompt: None,
        since: Duration::ZERO,
    }
}

/// The sidebar of the spec: two agents, an editor, a shell and a renamed panel running
/// `lazygit`. The first panel is active.
fn titled() -> SessionView {
    let mut view = mockup();
    let mut panels = vec![
        panel(3, "Fix login bug", 1),
        panel(4, "nvim", 1),
        panel(5, "Review the API changes", 1),
        panel(6, "~/Projects/terminal-multiplexer", 1),
        panel(7, "api", 1),
    ];
    panels[0].terminals[0].agent = Some(agent(AgentKind::Claude, AgentStatus::Working));
    panels[0].terminals[0].program = "node".into();
    panels[1].terminals[0].program = "nvim".into();
    panels[2].terminals[0].agent = Some(agent(AgentKind::Codex, AgentStatus::Waiting));
    panels[3].terminals[0].program = "zsh".into();
    panels[4].name_source = NameSource::Manual;
    panels[4].terminals[0].program = "lazygit".into();
    panels[4].terminals[0].title = Some("lazygit - repo".into());
    view.projects = vec![project(1, "terminal-multiplexer", false, panels)];
    view.active_project = Some(deck_core::model::ProjectId(1));
    view.active_panel = Some(PanelId(3));
    view
}

fn app_with(config: &str, view: SessionView) -> App {
    let config = Config::parse(config).unwrap().config;
    let mut app = App::new(&config, (100, 24), &[]).with_dirs(PathBuf::from("/work/proj"), None);
    app.on_server(ServerMsg::SessionView(view));
    if let Some(t) = app.mirror.focused_id() {
        let mut f = frame(true, (40, 10), vec![]);
        f.terminal = t;
        app.on_server(ServerMsg::Frame(f));
    }
    app
}

fn render(a: &App) -> (Buffer, Vec<String>) {
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
    (buf, rows)
}

/// The sidebar column of each row, with its separator.
fn sidebar(rows: &[String]) -> Vec<String> {
    rows.iter().map(|r| r.chars().take(29).collect()).collect()
}

fn pad(text: &str) -> String {
    format!("{text}{}│", " ".repeat(28 - text.chars().count()))
}

#[test]
fn the_sidebar_of_the_spec_at_28_columns() {
    let a = app_with("", titled());
    let (buf, rows) = render(&a);
    let side = sidebar(&rows);
    assert_eq!(side[0], pad(" main"));
    assert_eq!(side[1], pad(""));
    assert_eq!(side[2], pad(" ▾ terminal-multiplexer"));
    assert_eq!(side[3], pad("▌⠋ ✻ Fix login bug"));
    assert_eq!(side[4], pad("   ✎ nvim"));
    assert_eq!(side[5], pad(" ! ◎ Review the API changes"));
    assert_eq!(side[6], pad("   ❯ ~/Projects/terminal-m…"));
    assert_eq!(side[7], pad("   ± api"));
    // The icon has the colour of its row: the accent, bold, for the active panel.
    assert_eq!(buf[(3, 3)].fg, color(a.theme.accent));
    assert!(buf[(3, 3)].modifier.contains(Modifier::BOLD));
    assert_ne!(buf[(3, 4)].fg, color(a.theme.accent));
    // The status bar has the icon and the name of the active panel.
    assert!(
        rows[23].starts_with(" main  terminal-multiplexer › ⠋ ✻ Fix login bug  "),
        "{}",
        rows[23]
    );
    assert!(!rows[23].contains(" · "), "{}", rows[23]);
}

#[test]
fn a_terminal_count_and_a_long_name_leave_room_for_the_icon() {
    let mut view = titled();
    view.projects[0].panels[1].name = "a-very-long-panel-name-indeed".into();
    let extra = view.projects[0].panels[1].terminals[0].clone();
    view.projects[0].panels[1].terminals.push(extra);
    let a = app_with("", view);
    let (_, rows) = render(&a);
    assert_eq!(sidebar(&rows)[4], pad("   ✎ a-very-long-panel-n… 2 "));
}

#[test]
fn icons_none_draws_the_plain_rows() {
    let a = app_with(
        "[theme]\nicons = \"none\"\n[theme.icon_glyphs]\nclaude = \"C\"",
        titled(),
    );
    let (_, rows) = render(&a);
    let side = sidebar(&rows);
    assert_eq!(side[3], pad("▌⠋ Fix login bug"));
    assert_eq!(side[4], pad("   nvim"));
    assert_eq!(side[5], pad(" ! Review the API changes"));
    assert_eq!(side[7], pad("   api"));
    assert!(
        rows[23].starts_with(" main  terminal-multiplexer › ⠋ Fix login bug  "),
        "{}",
        rows[23]
    );
}

#[test]
fn the_nerd_preset_and_overrides_pick_the_glyphs() {
    let a = app_with("[theme]\nicons = \"nerd\"", titled());
    let (_, rows) = render(&a);
    let side = sidebar(&rows);
    assert_eq!(side[3], pad("▌⠋ \u{f069} Fix login bug"));
    assert_eq!(side[4], pad("   \u{f040} nvim"));
    assert_eq!(side[5], pad(" ! \u{f1ce} Review the API changes"));
    assert_eq!(side[7], pad("   \u{f126} api"));

    let a = app_with("[theme.icon_glyphs]\nclaude = \"C\"\ngit = \"G\"", titled());
    let (_, rows) = render(&a);
    let side = sidebar(&rows);
    assert_eq!(side[3], pad("▌⠋ C Fix login bug"));
    assert_eq!(side[7], pad("   G api"));
    assert_eq!(side[4], pad("   ✎ nvim"));
}

#[test]
fn a_manually_named_panel_keeps_its_name_with_the_live_icon_and_shows_the_title() {
    let mut view = titled();
    view.active_panel = Some(PanelId(7));
    view.projects[0].active_panel = Some(PanelId(7));
    let mut a = app_with("", view);
    let (_, rows) = render(&a);
    assert_eq!(sidebar(&rows)[7], pad("▌  ± api"));
    assert!(
        rows[23].starts_with(" main  terminal-multiplexer › ± api · lazygit - repo"),
        "{}",
        rows[23]
    );

    // The program ends: the name stays, the icon follows what is left.
    let mut view = a.mirror.view().unwrap().clone();
    view.projects[0].panels[4].terminals[0].program = "zsh".into();
    view.projects[0].panels[4].terminals[0].title = Some("api".into());
    a.on_server(ServerMsg::SessionView(view));
    let (_, rows) = render(&a);
    assert_eq!(sidebar(&rows)[7], pad("▌  ❯ api"));
    // A title that equals the name adds nothing.
    assert!(
        rows[23].starts_with(" main  terminal-multiplexer › ❯ api  "),
        "{}",
        rows[23]
    );
}

#[test]
fn a_panel_without_terminals_has_no_icon() {
    let mut view = titled();
    view.projects[0].panels[1].terminals.clear();
    view.projects[0].panels[1].focused_terminal = None;
    let a = app_with("", view);
    let (_, rows) = render(&a);
    assert_eq!(sidebar(&rows)[4], pad("   nvim"));
}
