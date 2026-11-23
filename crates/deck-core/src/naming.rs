//! Automatic panel names: what a panel is called after what runs in it.

use unicode_width::UnicodeWidthChar;

use crate::agents::AgentState;
use crate::model::{NameSource, Panel, PanelId, PanelView, Session, Terminal, TerminalView};

/// The longest name, in display cells.
pub const NAME_MAX_CELLS: usize = 32;

/// The names agents give themselves when they have nothing better to say.
const PRODUCT_NAMES: [&str; 4] = ["claude", "claude code", "codex", "opencode"];

/// What the name of a panel is chosen from: its source terminal.
#[derive(Debug, Clone, Copy)]
pub struct NameInput<'a> {
    pub agent: Option<&'a AgentState>,
    /// The title the program set (OSC 0/2).
    pub title: Option<&'a str>,
    /// The foreground program's name.
    pub program: &'a str,
}

impl<'a> NameInput<'a> {
    pub fn of(terminal: &'a Terminal) -> Self {
        Self {
            agent: terminal.agent.as_ref(),
            title: terminal.title.as_deref(),
            program: &terminal.program,
        }
    }
}

/// The terminal a panel is named after: the first one (in grid order) that has an agent,
/// otherwise the focused one.
pub fn source_terminal(panel: &Panel) -> Option<&Terminal> {
    panel
        .terminals
        .iter()
        .find(|t| t.agent.is_some())
        .or_else(|| {
            panel
                .terminals
                .iter()
                .find(|t| Some(t.id) == panel.focused_terminal)
        })
}

/// [`source_terminal`] for the view of a panel.
pub fn source_terminal_view(panel: &PanelView) -> Option<&TerminalView> {
    panel
        .terminals
        .iter()
        .find(|t| t.agent.is_some())
        .or_else(|| {
            panel
                .terminals
                .iter()
                .find(|t| Some(t.id) == panel.focused_terminal)
        })
}

/// A title without its leading spinner and status glyphs and its surrounding space: what is
/// left when only an animation changed is the same.
pub fn title_core(title: &str) -> &str {
    title
        .trim_start_matches(|c: char| !c.is_alphanumeric())
        .trim()
}

/// Recomputes the name of a panel whose name follows what runs in it (`Default` or
/// `Auto`); true when the name changed.
pub fn refresh_panel(session: &mut Session, id: PanelId) -> bool {
    let Some(panel) = session
        .projects
        .iter_mut()
        .flat_map(|p| p.panels.iter_mut())
        .find(|p| p.id == id)
    else {
        return false;
    };
    if panel.name_source == NameSource::Manual {
        return false;
    }
    let name = source_terminal(panel).and_then(|t| auto_name(&NameInput::of(t)));
    match name {
        Some(name) if name != panel.name => {
            panel.name = name;
            panel.name_source = NameSource::Auto;
            true
        }
        _ => false,
    }
}

/// The name for a panel, or `None` when it should keep the one it has.
///
/// An agent is named by its title with leading spinner and status glyphs stripped, unless
/// that is only a product name, then by the first line of its first prompt. Any other
/// program is named by its title, else by its own name.
pub fn auto_name(input: &NameInput) -> Option<String> {
    match input.agent {
        Some(agent) => agent_name(agent, input),
        // An agent's title seen before the agent is recognised ("✳ Claude Code") is named
        // without its glyphs, so the name the agent then keeps is clean.
        None => input
            .title
            .map(|t| {
                // Only glyphs go, not `~` or `/`: a shell in `~/codex` keeps its path.
                let core = t
                    .trim_start_matches(|c: char| {
                        !c.is_alphanumeric() && (c.is_whitespace() || !c.is_ascii())
                    })
                    .trim();
                if PRODUCT_NAMES.contains(&core.to_lowercase().as_str()) {
                    core
                } else {
                    t
                }
            })
            .and_then(finish)
            .or_else(|| finish(input.program)),
    }
}

fn agent_name(agent: &AgentState, input: &NameInput) -> Option<String> {
    let from_title = input.title.and_then(|t| {
        let clean: String = t.chars().filter(|c| !c.is_control()).collect();
        let stripped = clean.trim_start_matches(|c: char| !c.is_alphanumeric());
        let lower = stripped.trim().to_lowercase();
        let bare = lower.is_empty()
            || lower == input.program.to_lowercase()
            || PRODUCT_NAMES.contains(&lower.as_str());
        if bare { None } else { finish(stripped) }
    });
    from_title.or_else(|| {
        agent
            .first_prompt
            .as_deref()
            .and_then(|p| p.lines().find(|l| !l.trim().is_empty()))
            .and_then(finish)
    })
}

/// Trims, drops control characters and cuts to [`NAME_MAX_CELLS`]; `None` when empty.
fn finish(text: &str) -> Option<String> {
    let clean: String = text.chars().filter(|c| !c.is_control()).collect();
    let clean = clean.trim();
    if clean.is_empty() {
        return None;
    }
    let width = |c: char| c.width().unwrap_or(0);
    if clean.chars().map(width).sum::<usize>() <= NAME_MAX_CELLS {
        return Some(clean.to_string());
    }
    let mut out = String::new();
    let mut used = 0;
    for c in clean.chars() {
        if used + width(c) > NAME_MAX_CELLS - 1 {
            break;
        }
        used += width(c);
        out.push(c);
    }
    out.push('…');
    Some(out)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::agents::{AgentKind, AgentSource, AgentStatus};
    use crate::layout::Ratios;
    use crate::model::{LaunchSpec, PanelId, Project, ProjectId, SessionId, TerminalId};

    fn agent(prompt: Option<&str>) -> AgentState {
        AgentState {
            kind: AgentKind::Claude,
            status: AgentStatus::Idle,
            source: AgentSource::Hooks,
            session_id: None,
            first_prompt: prompt.map(Into::into),
            since: Duration::ZERO,
        }
    }

    fn name(agent: Option<&AgentState>, title: Option<&str>, program: &str) -> Option<String> {
        auto_name(&NameInput {
            agent,
            title,
            program,
        })
    }

    #[test]
    fn an_agent_is_named_by_its_title_without_the_spinner() {
        let a = agent(None);
        assert_eq!(
            name(Some(&a), Some("✳ Fix login bug"), "claude").as_deref(),
            Some("Fix login bug")
        );
        assert_eq!(
            name(Some(&a), Some("⠹ Fix login bug"), "claude").as_deref(),
            Some("Fix login bug")
        );
    }

    #[test]
    fn a_product_name_falls_back_to_the_first_prompt() {
        let a = agent(Some("Review the API"));
        for title in ["claude", "Claude Code", "⠹ codex", "✳ OpenCode", "", "✳ "] {
            assert_eq!(
                name(Some(&a), Some(title), "claude").as_deref(),
                Some("Review the API"),
                "{title:?}"
            );
        }
        assert_eq!(
            name(Some(&a), None, "claude").as_deref(),
            Some("Review the API")
        );
        let none = agent(None);
        for title in ["claude", "Claude Code", "⠹ codex"] {
            assert_eq!(name(Some(&none), Some(title), "claude"), None, "{title:?}");
        }
        assert_eq!(name(Some(&none), None, "claude"), None);
    }

    #[test]
    fn a_bare_product_name_is_not_a_name_for_any_program() {
        let a = agent(Some("Fix it\nthen more"));
        assert_eq!(
            name(Some(&a), Some("claude"), "zsh").as_deref(),
            Some("Fix it")
        );
        assert_eq!(name(Some(&agent(None)), Some("claude"), "zsh"), None);
    }

    #[test]
    fn the_program_of_an_agent_is_not_a_name() {
        let a = agent(None);
        assert_eq!(name(Some(&a), Some("⠂ mytool"), "mytool"), None);
    }

    #[test]
    fn other_programs_keep_their_title_or_use_their_name() {
        assert_eq!(name(None, Some(" ~/src "), "zsh").as_deref(), Some("~/src"));
        assert_eq!(
            name(None, Some("[No Name] - VIM"), "vim").as_deref(),
            Some("[No Name] - VIM")
        );
        assert_eq!(name(None, None, "zsh").as_deref(), Some("zsh"));
        assert_eq!(name(None, Some("  "), "zsh").as_deref(), Some("zsh"));
    }

    #[test]
    fn a_product_title_before_the_agent_is_known_drops_its_glyphs() {
        assert_eq!(
            name(None, Some("✳ Claude Code"), "claude").as_deref(),
            Some("Claude Code")
        );
        assert_eq!(
            name(None, Some("⠹ codex"), "codex").as_deref(),
            Some("codex")
        );
        // Other titles keep their leading characters, and a path stays a path.
        assert_eq!(
            name(None, Some("~/codex"), "zsh").as_deref(),
            Some("~/codex")
        );
        assert_eq!(
            name(None, Some("/srv/claude"), "zsh").as_deref(),
            Some("/srv/claude")
        );
        assert_eq!(
            name(None, Some("✳ Fix login bug"), "zsh").as_deref(),
            Some("✳ Fix login bug")
        );
    }

    #[test]
    fn names_are_cut_to_32_cells() {
        let long = "a".repeat(40);
        let cut = name(None, Some(&long), "zsh").unwrap();
        assert_eq!(cut, format!("{}…", "a".repeat(31)));
        let exact = "b".repeat(32);
        assert_eq!(name(None, Some(&exact), "zsh").unwrap(), exact);
        // Wide characters take two cells: 15 of them and the ellipsis make 31 cells.
        let wide = "漢".repeat(20);
        let cut = name(None, Some(&wide), "zsh").unwrap();
        assert_eq!(cut, format!("{}…", "漢".repeat(15)));
    }

    #[test]
    fn control_characters_are_removed() {
        assert_eq!(
            name(None, Some("a\x07b\x1b[0mc\n"), "zsh").as_deref(),
            Some("ab[0mc")
        );
    }

    fn terminal(id: u64, agent: Option<AgentState>, title: &str) -> Terminal {
        Terminal {
            id: TerminalId(id),
            launch: LaunchSpec {
                program: None,
                args: Vec::new(),
                cwd: "/".into(),
                env: Vec::new(),
            },
            program: "zsh".into(),
            title: Some(title.into()),
            cwd: None,
            exit_status: None,
            keep: false,
            agent,
        }
    }

    fn panel(terminals: Vec<Terminal>, focused: u64) -> Panel {
        Panel {
            id: PanelId(1),
            name: "Panel 1".into(),
            name_source: NameSource::Default,
            focused_terminal: Some(TerminalId(focused)),
            ratios: Ratios::default_for(terminals.len()),
            terminals,
        }
    }

    #[test]
    fn refreshing_renames_a_panel_unless_it_is_manual() {
        let mut manual = panel(vec![terminal(1, None, "x")], 1);
        manual.id = PanelId(2);
        manual.name = "api".into();
        manual.name_source = NameSource::Manual;
        let auto = panel(vec![terminal(3, Some(agent(None)), "✳ Fix it")], 3);
        let mut session = Session {
            id: SessionId(1),
            name: "s".into(),
            projects: vec![Project {
                id: ProjectId(1),
                name: "p".into(),
                root: "/".into(),
                panels: vec![auto, manual],
                active_panel: None,
                collapsed: false,
            }],
            active_project: None,
        };
        assert!(refresh_panel(&mut session, PanelId(1)));
        assert!(!refresh_panel(&mut session, PanelId(2)));
        assert!(!refresh_panel(&mut session, PanelId(9)));
        let panels = &session.projects[0].panels;
        assert_eq!(panels[0].name, "Fix it");
        assert_eq!(panels[0].name_source, NameSource::Auto);
        assert_eq!(panels[1].name, "api");
        assert!(!refresh_panel(&mut session, PanelId(1)));
        assert_eq!(title_core("⠹ Fix it "), title_core("⠸ Fix it"));
    }

    #[test]
    fn the_first_agent_terminal_is_the_source() {
        let p = panel(
            vec![
                terminal(1, None, "shell"),
                terminal(2, Some(agent(None)), "a"),
                terminal(3, Some(agent(None)), "b"),
            ],
            1,
        );
        assert_eq!(source_terminal(&p).map(|t| t.id), Some(TerminalId(2)));
    }

    #[test]
    fn without_an_agent_the_focused_terminal_is_the_source() {
        let p = panel(vec![terminal(1, None, "x"), terminal(2, None, "y")], 2);
        assert_eq!(source_terminal(&p).map(|t| t.id), Some(TerminalId(2)));
        let empty = panel(Vec::new(), 1);
        assert!(source_terminal(&empty).is_none());
    }

    #[test]
    fn the_view_of_a_panel_has_the_same_source() {
        let agents = panel(
            vec![terminal(1, None, "s"), terminal(2, Some(agent(None)), "a")],
            1,
        );
        let plain = panel(vec![terminal(1, None, "x"), terminal(2, None, "y")], 2);
        for p in [agents, plain, panel(Vec::new(), 1)] {
            assert_eq!(
                source_terminal_view(&p.view()).map(|t| t.id),
                source_terminal(&p).map(|t| t.id)
            );
        }
    }
}
