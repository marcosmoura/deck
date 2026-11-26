//! The status icons of coding agents: the sidebar's status column and the terminal titles.
//! Every colour comes from the theme.

use deck_core::agents::{AgentKind, AgentStatus};
use deck_core::config::NotifyMode;
use deck_core::icons::Icon;
use deck_core::model::{PanelId, PanelView, ProjectView, SessionView, TerminalId, TerminalView};
use deck_core::theme::Theme;
use ratatui::style::{Modifier, Style};

use crate::draw::color;

/// How many frames a second the spinner moves, in every tier.
pub const SPIN_FPS: usize = 8;
/// How often the spinner moves a frame.
#[cfg(test)]
pub const SPIN_EVERY: std::time::Duration =
    std::time::Duration::from_millis(1000 / SPIN_FPS as u64);

/// Whether a status shows the spinner.
pub fn spins(status: AgentStatus) -> bool {
    matches!(status, AgentStatus::Working | AgentStatus::Active)
}

/// Whether any terminal of the session shows a spinner: a working agent or a command.
pub fn view_spins(view: &SessionView) -> bool {
    view.projects
        .iter()
        .flat_map(|p| &p.panels)
        .flat_map(|p| &p.terminals)
        .any(|t| t.busy || t.agent.as_ref().is_some_and(|a| spins(a.status)))
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

/// The icon of a status and its style (foreground and weight only). `None` for no agent.
pub fn status_icon(status: AgentStatus, theme: &Theme) -> Option<(Icon, Style)> {
    let plain = |c| Style::new().fg(color(c));
    match status {
        AgentStatus::None => None,
        AgentStatus::Working | AgentStatus::Active => Some((Icon::Spinner, plain(theme.accent))),
        AgentStatus::Waiting => Some((
            Icon::Waiting,
            plain(theme.warning).add_modifier(Modifier::BOLD),
        )),
        AgentStatus::Idle => Some((Icon::Idle, plain(theme.ok))),
        AgentStatus::Quiet => Some((Icon::Quiet, plain(theme.muted))),
    }
}

/// The mark of a running command: the spinner, muted so a working agent stands out.
fn busy_icon(theme: &Theme) -> (Icon, Style) {
    (Icon::Spinner, Style::new().fg(color(theme.muted)))
}

/// What a terminal's status slot shows: its agent's status, else the muted spinner while a
/// command runs. `None` for neither.
pub fn terminal_icon(terminal: &TerminalView, theme: &Theme) -> Option<(Icon, Style)> {
    match &terminal.agent {
        Some(agent) => status_icon(agent.status, theme),
        None if terminal.busy => Some(busy_icon(theme)),
        None => None,
    }
}

/// What a panel's status slot shows: its most urgent agent, except that a running command
/// comes before an idle or quiet one.
pub fn panel_icon(panel: &PanelView, theme: &Theme) -> Option<(Icon, Style)> {
    let status = panel_status(panel);
    if status.urgency() < AgentStatus::Active.urgency() && panel.terminals.iter().any(|t| t.busy) {
        return Some(busy_icon(theme));
    }
    status_icon(status, theme)
}

/// What a terminal's title starts with when an agent runs in it: its status icon, style and
/// the agent's name.
pub fn title_mark(terminal: &TerminalView, theme: &Theme) -> Option<(Icon, Style, &'static str)> {
    let agent = terminal.agent.as_ref()?;
    let (icon, style) = status_icon(agent.status, theme)?;
    Some((icon, style, agent.kind.name()))
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

/// The terminal of one panel that needs the user: a waiting one, else an idle one when the
/// panel is unseen. `None` when the panel needs nothing (already seen, or nobody waits).
pub fn panel_attention(panel: &PanelView) -> Option<TerminalId> {
    let status = |t: &TerminalView| t.agent.as_ref().map(|a| a.status);
    panel
        .terminals
        .iter()
        .find(|t| status(t) == Some(AgentStatus::Waiting))
        .or_else(|| {
            panel
                .terminals
                .iter()
                .find(|t| panel.unseen && status(t) == Some(AgentStatus::Idle))
        })
        .map(|t| t.id)
}

/// The text a desktop notification carries: what the agent did, and where.
pub(crate) fn alert_body(
    kind: AgentKind,
    status: AgentStatus,
    project: &str,
    panel: &str,
) -> Option<String> {
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
        // `System` writes it only when deck cannot show its own notification.
        NotifyMode::Terminal | NotifyMode::System => alert_body(kind, status, project, panel)
            .map(|body| format!("\x1b]9;{body}\x07").into_bytes())
            .unwrap_or_default(),
    }
}
