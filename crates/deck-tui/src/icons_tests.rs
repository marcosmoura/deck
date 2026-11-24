//! Program and agent icons in the sidebar rows.

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
use crate::draw::draw;
use crate::mirror::tests::{frame, mockup, panel, project};
use crate::tier::Tier;

pub(crate) fn agent(kind: AgentKind, status: AgentStatus) -> AgentState {
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
pub(crate) fn titled() -> SessionView {
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

pub(crate) fn app_with(config: &str, view: SessionView) -> App {
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

pub(crate) fn render(a: &App) -> (Buffer, Vec<String>) {
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

/// A panel row with its status slot on the right, before the margin.
fn st(text: &str, status: &str) -> String {
    format!("{text:<25}{status}")
}

fn pad(text: &str) -> String {
    format!("{text}{}│", " ".repeat(28 - text.chars().count()))
}

#[test]
fn the_sidebar_of_the_spec_at_28_columns() {
    let a = app_with("", titled());
    let (buf, rows) = render(&a);
    let side = sidebar(&rows);
    assert_eq!(side[0], pad(" main         +  New space  "));
    assert_eq!(side[1], pad(""));
    assert_eq!(side[2], pad(" terminal-multiplexer    ▾"));
    assert_eq!(side[3], pad(&st(" ✻  Fix login bug", "⠋")));
    assert_eq!(side[4], pad(" ✎  nvim"));
    assert_eq!(side[5], pad(&st(" ◎  Review the API chan…", "!")));
    assert_eq!(side[6], pad(" ❯  ~/Projects/terminal-mu…"));
    assert_eq!(side[7], pad(" ±  api"));
    // The icon has the style of its row: the selection band, bold, for the active panel.
    assert!(
        buf[(1, 3)]
            .modifier
            .contains(Modifier::BOLD | Modifier::REVERSED)
    );
    assert!(!buf[(1, 4)].modifier.contains(Modifier::REVERSED));
}

#[test]
fn a_grid_panel_has_no_count_and_a_long_name_leaves_room_for_the_icon() {
    let mut view = titled();
    view.projects[0].panels[1].name = "a-very-long-panel-name-indeed".into();
    let extra = view.projects[0].panels[1].terminals[0].clone();
    view.projects[0].panels[1].terminals.push(extra);
    let a = app_with("", view);
    let (_, rows) = render(&a);
    assert_eq!(sidebar(&rows)[4], pad(" ✎  a-very-long-panel-name…"));
}

#[test]
fn the_nerd_tier_draws_the_nerd_glyphs_in_the_same_cells() {
    let mut a = app_with("", titled());
    a.tier = Tier::Nerd;
    let (_, rows) = render(&a);
    let side = sidebar(&rows);
    assert_eq!(side[2], pad(" terminal-multiplexer    \u{eab4}"));
    assert_eq!(side[3], pad(&st(" \u{ec82}  Fix login bug", "\u{ee06}")));
    assert_eq!(side[4], pad(" \u{f040}  nvim"));
    assert_eq!(
        side[5],
        pad(&st(" \u{ec81}  Review the API chan…", "\u{f06a}"))
    );
    assert_eq!(side[7], pad(" \u{f126}  api"));
}

#[test]
fn the_image_tier_without_a_sent_image_draws_the_nerd_glyphs() {
    let mut a = app_with("", titled());
    a.tier = Tier::Image;
    let (_, rows) = render(&a);
    let side = sidebar(&rows);
    assert_eq!(side[3], pad(&st(" \u{ec82}  Fix login bug", "\u{ee06}")));
    assert_eq!(side[4], pad(" \u{f040}  nvim"));
}

#[test]
fn the_spinner_wraps_at_each_tiers_frame_count() {
    for (tier, frames) in [(Tier::Unicode, 10), (Tier::Nerd, 6)] {
        let mut a = app_with("", titled());
        a.tier = tier;
        a.spin = 0;
        let (buf, _) = render(&a);
        let first = buf[(25, 3)].symbol().to_string();
        a.spin = 1;
        assert_ne!(render(&a).0[(25, 3)].symbol(), first, "{tier:?}");
        a.spin = frames;
        assert_eq!(render(&a).0[(25, 3)].symbol(), first, "{tier:?}");
    }
}

#[test]
fn the_removed_icon_keys_still_load_and_change_nothing() {
    let a = app_with(
        "[theme]\nicons = \"none\"\n[theme.icon_glyphs]\nclaude = \"C\"",
        titled(),
    );
    let b = app_with("", titled());
    assert_eq!(render(&a).0, render(&b).0);
}

#[test]
fn a_manually_named_panel_keeps_its_name_with_the_live_icon_and_shows_the_title() {
    let mut view = titled();
    view.active_panel = Some(PanelId(7));
    view.projects[0].active_panel = Some(PanelId(7));
    let mut a = app_with("", view);
    let (_, rows) = render(&a);
    assert_eq!(sidebar(&rows)[7], pad(" ±  api"));

    // The program ends: the name stays, the icon follows what is left.
    let mut view = a.mirror.view().unwrap().clone();
    view.projects[0].panels[4].terminals[0].program = "zsh".into();
    view.projects[0].panels[4].terminals[0].title = Some("api".into());
    a.on_server(ServerMsg::SessionView(view));
    let (_, rows) = render(&a);
    assert_eq!(sidebar(&rows)[7], pad(" ❯  api"));
}

#[test]
fn a_panel_without_terminals_has_no_icon() {
    let mut view = titled();
    view.projects[0].panels[1].terminals.clear();
    view.projects[0].panels[1].focused_terminal = None;
    let a = app_with("", view);
    let (_, rows) = render(&a);
    assert_eq!(sidebar(&rows)[4], pad("    nvim"));
}

#[test]
fn agent_icons_take_their_icon_colour() {
    use ratatui::style::Color;
    let claude = Color::Rgb(0xd9, 0x77, 0x57);
    for tier in [Tier::Unicode, Tier::Nerd, Tier::Image] {
        let mut a = app_with("", titled());
        a.tier = tier;
        let (buf, _) = render(&a);
        // The active Claude row is reversed: the colour goes in the background, which it shows
        // as text. The name keeps the row's style.
        assert!(buf[(1, 3)].modifier.contains(Modifier::REVERSED));
        assert_eq!(buf[(1, 3)].bg, claude, "{tier:?}");
        assert_ne!(buf[(4, 3)].bg, claude);
        // Codex takes the row's colour, so does an editor.
        assert_eq!(buf[(1, 5)].fg, buf[(4, 5)].fg);
        assert_eq!(buf[(1, 4)].fg, buf[(4, 4)].fg);
        // Outside the band the colour is the text's.
        let mut view = titled();
        view.active_panel = Some(PanelId(4));
        let mut a = app_with("", view);
        a.tier = tier;
        let (buf, _) = render(&a);
        assert_eq!(buf[(1, 3)].fg, claude, "{tier:?}");
        // Codex on the active row is in the row's colour, not a brand colour.
        let mut view = titled();
        view.active_panel = Some(PanelId(5));
        let mut a = app_with("", view);
        a.tier = tier;
        let (buf, _) = render(&a);
        assert_eq!(buf[(1, 5)].fg, buf[(4, 5)].fg);
        assert_eq!(buf[(1, 5)].bg, buf[(4, 5)].bg);
    }
}

#[test]
fn the_status_marks_keep_their_roles() {
    let a = app_with("", titled());
    let (buf, _) = render(&a);
    // Spinner: accent. Waiting: warning, bold. Both are the row's colours on a reversed
    // band only for the active row.
    assert_eq!(buf[(25, 3)].symbol(), "⠋");
    let (buf, _) = render(&app_with("", {
        let mut v = titled();
        v.active_panel = Some(PanelId(4));
        v
    }));
    use crate::draw::color;
    assert_eq!(buf[(25, 3)].fg, color(a.theme.accent));
    assert_eq!(buf[(25, 5)].fg, color(a.theme.warning));
    assert!(buf[(25, 5)].modifier.contains(Modifier::BOLD));
    // The caret of a space is muted.
    assert_eq!(buf[(25, 2)].symbol(), "▾");
    assert_eq!(buf[(25, 2)].fg, color(a.theme.muted));
    assert_eq!(buf[(26, 2)].symbol(), " ");
}

#[test]
fn the_name_is_cut_before_the_unseen_slot_and_the_status_slot() {
    let mut view = titled();
    // Panel 4 has a working agent; it is also unseen, with a long name and two terminals.
    let panel = &mut view.projects[0].panels[0];
    panel.name = "a-very-long-panel-name-indeed".into();
    panel.unseen = true;
    let extra = panel.terminals[0].clone();
    panel.terminals.push(extra);
    let a = app_with("", view);
    let (_, rows) = render(&a);
    // Name from column 4, then the unseen slot and the status slot, one space each.
    assert_eq!(sidebar(&rows)[3], pad(&st(" ✻  a-very-long-pane… •", "⠋")));
}
