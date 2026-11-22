//! The client's copy of what the daemon showed: the latest `SessionView` and, per terminal,
//! its lines, cursor and modes, built from `Frame`s.

use std::collections::HashMap;

use deck_core::model::{
    PanelId, PanelView, ProjectId, ProjectView, SessionView, TerminalId, TerminalView,
};
use deck_core::protocol::{Cursor, CursorShape, Frame, Line, Modes};

/// One terminal's screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermMirror {
    /// Columns and rows of the emulator.
    pub size: (u16, u16),
    pub lines: Vec<Line>,
    pub cursor: Cursor,
    pub modes: Modes,
    pub display_offset: u32,
    pub history: u32,
}

impl TermMirror {
    fn blank(size: (u16, u16)) -> Self {
        Self {
            size,
            lines: vec![Line::default(); usize::from(size.1)],
            cursor: Cursor {
                row: 0,
                col: 0,
                visible: false,
                shape: CursorShape::Block,
            },
            modes: Modes::default(),
            display_offset: 0,
            history: 0,
        }
    }

    /// A full frame replaces everything; a partial one patches its lines. A frame of
    /// another size than the mirror's starts from blank lines.
    fn apply(&mut self, frame: Frame) {
        if frame.full || frame.size != self.size {
            *self = Self::blank(frame.size);
        }
        self.cursor = frame.cursor;
        self.modes = frame.modes;
        self.display_offset = frame.display_offset;
        self.history = frame.history;
        for (row, line) in frame.lines {
            if let Some(slot) = self.lines.get_mut(usize::from(row)) {
                *slot = line;
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct Mirror {
    terminals: HashMap<TerminalId, TermMirror>,
    view: Option<SessionView>,
}

impl Mirror {
    pub fn apply_frame(&mut self, frame: Frame) {
        let id = frame.terminal;
        self.terminals
            .entry(id)
            .or_insert_with(|| TermMirror::blank(frame.size))
            .apply(frame);
    }

    /// Keeps the new view and only the screens of the terminals it still lists.
    pub fn set_view(&mut self, view: SessionView) {
        let alive = |id: &TerminalId| {
            view.projects
                .iter()
                .flat_map(|p| &p.panels)
                .flat_map(|p| &p.terminals)
                .any(|t| t.id == *id)
        };
        self.terminals.retain(|id, _| alive(id));
        self.view = Some(view);
    }

    pub fn view(&self) -> Option<&SessionView> {
        self.view.as_ref()
    }

    pub fn terminal(&self, id: TerminalId) -> Option<&TermMirror> {
        self.terminals.get(&id)
    }

    pub fn session_name(&self) -> Option<&str> {
        self.view.as_ref().map(|v| v.name.as_str())
    }

    /// The name of the active project.
    pub fn project_name(&self) -> Option<&str> {
        let view = self.view.as_ref()?;
        let id = view.active_project?;
        view.projects
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.name.as_str())
    }

    pub fn find_project(&self, id: ProjectId) -> Option<&ProjectView> {
        self.view.as_ref()?.projects.iter().find(|p| p.id == id)
    }

    pub fn find_panel(&self, id: PanelId) -> Option<(&ProjectView, &PanelView)> {
        self.view
            .as_ref()?
            .projects
            .iter()
            .find_map(|p| Some((p, p.panels.iter().find(|x| x.id == id)?)))
    }

    pub fn active_project(&self) -> Option<&ProjectView> {
        self.find_project(self.view.as_ref()?.active_project?)
    }

    /// The active project's active panel.
    pub fn active_panel(&self) -> Option<&PanelView> {
        let project = self.active_project()?;
        let id = project.active_panel?;
        project.panels.iter().find(|p| p.id == id)
    }

    /// The focused terminal of the active project's active panel.
    pub fn focused(&self) -> Option<&TerminalView> {
        let view = self.view.as_ref()?;
        let project = view
            .projects
            .iter()
            .find(|p| Some(p.id) == view.active_project)?;
        let panel = project
            .panels
            .iter()
            .find(|p| Some(p.id) == project.active_panel)?;
        let id = panel.focused_terminal?;
        panel.terminals.iter().find(|t| t.id == id)
    }

    pub fn focused_id(&self) -> Option<TerminalId> {
        self.focused().map(|t| t.id)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use deck_core::model::{
        NameSource, PanelId, PanelView, ProjectId, ProjectView, SessionId, TerminalView,
    };
    use deck_core::protocol::{Run, Style};

    /// The default config with the sidebar off, for tests of the panel area alone.
    pub fn no_sidebar() -> deck_core::config::Config {
        deck_core::config::Config {
            sidebar: false,
            ..deck_core::config::Config::default()
        }
    }

    pub fn line(text: &str) -> Line {
        Line {
            runs: vec![Run {
                col: 0,
                text: text.into(),
                style: Style::default(),
            }],
        }
    }

    pub fn frame(full: bool, size: (u16, u16), lines: Vec<(u16, Line)>) -> Frame {
        Frame {
            terminal: TerminalId(4),
            size,
            cursor: Cursor {
                row: 0,
                col: 1,
                visible: true,
                shape: CursorShape::Bar,
            },
            modes: Modes(Modes::BRACKETED_PASTE),
            display_offset: 0,
            history: 3,
            lines,
            full,
        }
    }

    pub fn panel(id: u64, name: &str, terminals: u64) -> PanelView {
        PanelView {
            id: PanelId(id),
            name: name.into(),
            name_source: NameSource::Default,
            focused_terminal: Some(TerminalId(id * 10 + 1)),
            ratios: deck_core::layout::Ratios::default_for(usize::try_from(terminals).unwrap_or(1)),
            unseen: false,
            terminals: (1..=terminals)
                .map(|n| TerminalView {
                    id: TerminalId(id * 10 + n),
                    program: "sh".into(),
                    title: None,
                    cwd: None,
                    exit_status: None,
                    pid: None,
                    keep: false,
                    agent: None,
                })
                .collect(),
        }
    }

    pub fn project(id: u64, name: &str, collapsed: bool, panels: Vec<PanelView>) -> ProjectView {
        ProjectView {
            id: ProjectId(id),
            name: name.into(),
            root: "/r".into(),
            collapsed,
            active_panel: panels.first().map(|p| p.id),
            panels,
        }
    }

    /// The sidebar mockup of the spec: panel 3 (`editor`, terminal 30) of project 1 is active.
    pub fn mockup() -> SessionView {
        SessionView {
            id: SessionId(1),
            name: "main".into(),
            active_project: Some(ProjectId(1)),
            active_panel: Some(PanelId(3)),
            layout_viewport: Some((71, 23)),
            attached_clients: 0,
            projects: vec![
                project(
                    1,
                    "terminal-multiplexer",
                    false,
                    vec![
                        panel(3, "editor", 1),
                        panel(4, "claude", 1),
                        panel(5, "tests", 1),
                    ],
                ),
                project(2, "budget", false, vec![panel(6, "api", 1)]),
                project(7, "empty", false, vec![]),
                project(
                    8,
                    "dotfiles",
                    true,
                    vec![panel(9, "vim", 1), panel(10, "git", 1)],
                ),
            ],
        }
    }

    pub fn view(title: Option<&str>) -> SessionView {
        SessionView {
            id: SessionId(1),
            name: "main".into(),
            active_project: Some(ProjectId(2)),
            active_panel: Some(PanelId(3)),
            layout_viewport: Some((80, 23)),
            attached_clients: 0,
            projects: vec![ProjectView {
                id: ProjectId(2),
                name: "proj".into(),
                root: "/r".into(),
                collapsed: false,
                active_panel: Some(PanelId(3)),
                panels: vec![PanelView {
                    id: PanelId(3),
                    name: "p".into(),
                    name_source: NameSource::Default,
                    focused_terminal: Some(TerminalId(4)),
                    ratios: deck_core::layout::Ratios::default_for(1),
                    unseen: false,
                    terminals: vec![TerminalView {
                        id: TerminalId(4),
                        program: "sh".into(),
                        title: title.map(str::to_string),
                        cwd: None,
                        exit_status: None,
                        pid: None,
                        keep: false,
                        agent: None,
                    }],
                }],
            }],
        }
    }

    #[test]
    fn a_full_frame_replaces_and_a_partial_one_patches() {
        let mut m = Mirror::default();
        m.apply_frame(frame(true, (10, 3), vec![(0, line("a")), (1, line("b"))]));
        let t = m.terminal(TerminalId(4)).unwrap();
        assert_eq!(t.lines.len(), 3);
        assert_eq!(t.lines[1], line("b"));
        assert_eq!(t.modes, Modes(Modes::BRACKETED_PASTE));
        assert_eq!(t.history, 3);

        m.apply_frame(frame(false, (10, 3), vec![(1, line("x"))]));
        let t = m.terminal(TerminalId(4)).unwrap();
        assert_eq!(t.lines[0], line("a"));
        assert_eq!(t.lines[1], line("x"));

        m.apply_frame(frame(true, (10, 3), vec![(2, line("z"))]));
        let t = m.terminal(TerminalId(4)).unwrap();
        assert_eq!(t.lines[0], Line::default());
        assert_eq!(t.lines[2], line("z"));
    }

    #[test]
    fn a_resized_partial_frame_starts_blank_and_rows_out_of_range_are_ignored() {
        let mut m = Mirror::default();
        m.apply_frame(frame(true, (10, 3), vec![(0, line("a"))]));
        m.apply_frame(frame(false, (8, 2), vec![(1, line("n")), (9, line("far"))]));
        let t = m.terminal(TerminalId(4)).unwrap();
        assert_eq!(t.size, (8, 2));
        assert_eq!(t.lines, vec![Line::default(), line("n")]);
    }

    #[test]
    fn a_view_drops_the_screens_of_terminals_that_are_gone() {
        let mut m = Mirror::default();
        m.apply_frame(frame(true, (10, 3), vec![]));
        m.set_view(view(None));
        assert!(m.terminal(TerminalId(4)).is_some());
        let mut v = view(None);
        v.projects[0].panels[0].terminals.clear();
        m.set_view(v);
        assert!(m.terminal(TerminalId(4)).is_none());
    }

    #[test]
    fn the_view_gives_names_and_the_focused_terminal() {
        let mut m = Mirror::default();
        assert_eq!(m.focused_id(), None);
        m.set_view(view(Some("vim")));
        assert_eq!(m.session_name(), Some("main"));
        assert_eq!(m.project_name(), Some("proj"));
        assert_eq!(m.focused_id(), Some(TerminalId(4)));
        assert_eq!(m.focused().unwrap().title.as_deref(), Some("vim"));
    }
}
