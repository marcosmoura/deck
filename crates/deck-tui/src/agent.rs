//! The glyphs of coding agents: the sidebar's status column and the terminal titles. Every
//! glyph and colour comes from the theme.

use deck_core::agents::{AgentKind, AgentStatus};
use deck_core::config::NotifyMode;
use deck_core::model::{PanelId, PanelView, ProjectView, SessionView, TerminalId, TerminalView};
use deck_core::theme::Theme;
use ratatui::style::{Modifier, Style};

use crate::draw::color;

/// How often the spinner moves a frame.
pub const SPIN_EVERY: std::time::Duration = std::time::Duration::from_millis(100);

/// Whether a status shows the spinner.
pub fn spins(status: AgentStatus) -> bool {
    matches!(status, AgentStatus::Working | AgentStatus::Active)
}

/// Whether any terminal of the session shows a spinner.
pub fn view_spins(view: &SessionView) -> bool {
    view.projects
        .iter()
        .flat_map(|p| &p.panels)
        .flat_map(|p| &p.terminals)
        .any(|t| t.agent.as_ref().is_some_and(|a| spins(a.status)))
}

/// A panel shows its most urgent terminal.
pub fn panel_status(panel: &PanelView) -> AgentStatus {
    AgentStatus::most_urgent(
        panel
            .terminals
            .iter()
            .filter_map(|t| t.agent.as_ref().map(|a| a.status)),
    )
}

/// The glyph of a status and its style (foreground and weight only); `spin` is the spinner's
/// frame counter. `None` for no agent.
pub fn glyph(status: AgentStatus, spin: usize, theme: &Theme) -> Option<(String, Style)> {
    let glyphs = &theme.glyphs;
    let frame = || {
        glyphs
            .spinner
            .get(spin % glyphs.spinner.len().max(1))
            .cloned()
            .unwrap_or_default()
    };
    let plain = |c| Style::new().fg(color(c));
    match status {
        AgentStatus::None => None,
        AgentStatus::Working => Some((frame(), plain(theme.accent))),
        AgentStatus::Active => Some((frame(), plain(theme.accent))),
        AgentStatus::Waiting => Some((
            glyphs.waiting.clone(),
            plain(theme.warning).add_modifier(Modifier::BOLD),
        )),
        AgentStatus::Idle => Some((glyphs.idle.clone(), plain(theme.ok))),
        AgentStatus::Quiet => Some((glyphs.quiet.clone(), plain(theme.muted))),
    }
}

/// What a terminal's title starts with when an agent runs in it: its glyph, style and the
/// agent's name.
pub fn title_mark(
    terminal: &TerminalView,
    spin: usize,
    theme: &Theme,
) -> Option<(String, Style, &'static str)> {
    let agent = terminal.agent.as_ref()?;
    let (text, style) = glyph(agent.status, spin, theme)?;
    Some((text, style, agent.kind.name()))
}

/// How many terminals of the session wait for the user.
pub fn waiting_terminals(view: &SessionView) -> usize {
    view.projects
        .iter()
        .flat_map(|p| &p.panels)
        .flat_map(|p| &p.terminals)
        .filter(|t| {
            t.agent
                .as_ref()
                .is_some_and(|a| a.status == AgentStatus::Waiting)
        })
        .count()
}

/// How many panels of a project wait for the user.
pub fn waiting_panels(project: &ProjectView) -> usize {
    project
        .panels
        .iter()
        .filter(|p| panel_status(p) == AgentStatus::Waiting)
        .count()
}

/// The terminal `prefix a` jumps to: the next waiting one in sidebar order after the focused
/// one, wrapping; else the next idle one in an unseen panel. Collapsed projects count.
pub fn next_attention(view: &SessionView) -> Option<(PanelId, TerminalId)> {
    let all: Vec<(&PanelView, &TerminalView)> = view
        .projects
        .iter()
        .flat_map(|p| &p.panels)
        .flat_map(|p| p.terminals.iter().map(move |t| (p, t)))
        .collect();
    let focused = view
        .projects
        .iter()
        .find(|p| Some(p.id) == view.active_project)
        .and_then(|p| p.panels.iter().find(|x| Some(x.id) == p.active_panel))
        .and_then(|p| p.focused_terminal);
    let at = focused.and_then(|f| all.iter().position(|(_, t)| t.id == f));
    // Starting just after the focused terminal; the focused one comes last.
    let start = at.map_or(0, |i| i + 1);
    let ordered = || all.iter().cycle().skip(start).take(all.len());
    let status = |t: &TerminalView| t.agent.as_ref().map(|a| a.status);
    ordered()
        .find(|(_, t)| status(t) == Some(AgentStatus::Waiting))
        .or_else(|| ordered().find(|(p, t)| p.unseen && status(t) == Some(AgentStatus::Idle)))
        .map(|(p, t)| (p.id, t.id))
}

/// The text a desktop notification carries: what the agent did, and where.
fn alert_body(kind: AgentKind, status: AgentStatus, project: &str, panel: &str) -> Option<String> {
    let what = match status {
        AgentStatus::Waiting => "needs you",
        AgentStatus::Idle => "finished",
        _ => return None,
    };
    let clean = |text: &str| text.chars().filter(|c| !c.is_control()).collect::<String>();
    Some(format!(
        "{} {what} · {} › {}",
        kind.name(),
        clean(project),
        clean(panel)
    ))
}

/// What to write to the outer terminal for an alert: OSC 9, a bell, or nothing.
pub fn alert_bytes(
    mode: NotifyMode,
    kind: AgentKind,
    status: AgentStatus,
    project: &str,
    panel: &str,
) -> Vec<u8> {
    match mode {
        NotifyMode::None => Vec::new(),
        NotifyMode::Bell => b"\x07".to_vec(),
        NotifyMode::Osc9 => alert_body(kind, status, project, panel)
            .map(|body| format!("\x1b]9;{body}\x07").into_bytes())
            .unwrap_or_default(),
    }
}
