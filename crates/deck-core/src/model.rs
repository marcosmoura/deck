//! The model shared by the daemon and the clients: session, project, panel, terminal.

use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        pub struct $name(pub u64);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

id_type!(
    /// Identifies a session; ids come from one daemon-wide counter and are never reused.
    SessionId
);
id_type!(
    /// Identifies a project.
    ProjectId
);
id_type!(
    /// Identifies a panel.
    PanelId
);
id_type!(
    /// Identifies a terminal.
    TerminalId
);

/// Where a panel's name came from; a `Manual` name is never overwritten automatically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NameSource {
    Default,
    Manual,
    Auto,
}

/// What to start in a terminal. `program: None` means the configured shell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchSpec {
    pub program: Option<String>,
    pub args: Vec<String>,
    #[serde(with = "crate::wire::path")]
    pub cwd: PathBuf,
    #[serde(with = "crate::wire::env")]
    pub env: Vec<(OsString, OsString)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub name: String,
    pub projects: Vec<Project>,
    pub active_project: Option<ProjectId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    #[serde(with = "crate::wire::path")]
    pub root: PathBuf,
    pub panels: Vec<Panel>,
    pub active_panel: Option<PanelId>,
    pub collapsed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Panel {
    pub id: PanelId,
    pub name: String,
    pub name_source: NameSource,
    pub terminals: Vec<Terminal>,
    pub focused_terminal: Option<TerminalId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Terminal {
    pub id: TerminalId,
    pub launch: LaunchSpec,
    /// Name of the program that runs (the shell's basename for `program: None`).
    pub program: String,
    /// Title set by the program (OSC 0/2).
    pub title: Option<String>,
    /// Live working directory, when known.
    #[serde(with = "crate::wire::opt_path")]
    pub cwd: Option<PathBuf>,
    /// Exit status of a kept terminal whose process ended.
    pub exit_status: Option<i32>,
}

/// A serializable snapshot of one session for clients. It carries no launch specs or
/// environments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionView {
    pub id: SessionId,
    pub name: String,
    pub projects: Vec<ProjectView>,
    pub active_project: Option<ProjectId>,
    /// The active project's active panel.
    pub active_panel: Option<PanelId>,
    /// The panel area the daemon sized the active panel's terminals for.
    pub layout_viewport: Option<(u16, u16)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectView {
    pub id: ProjectId,
    pub name: String,
    #[serde(with = "crate::wire::path")]
    pub root: PathBuf,
    pub collapsed: bool,
    pub panels: Vec<PanelView>,
    pub active_panel: Option<PanelId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanelView {
    pub id: PanelId,
    pub name: String,
    pub name_source: NameSource,
    pub terminals: Vec<TerminalView>,
    pub focused_terminal: Option<TerminalId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalView {
    pub id: TerminalId,
    pub program: String,
    pub title: Option<String>,
    #[serde(with = "crate::wire::opt_path")]
    pub cwd: Option<PathBuf>,
    pub exit_status: Option<i32>,
}

impl Terminal {
    pub fn view(&self) -> TerminalView {
        TerminalView {
            id: self.id,
            program: self.program.clone(),
            title: self.title.clone(),
            cwd: self.cwd.clone(),
            exit_status: self.exit_status,
        }
    }
}

impl Panel {
    pub fn view(&self) -> PanelView {
        PanelView {
            id: self.id,
            name: self.name.clone(),
            name_source: self.name_source,
            terminals: self.terminals.iter().map(Terminal::view).collect(),
            focused_terminal: self.focused_terminal,
        }
    }
}

impl Project {
    pub fn view(&self) -> ProjectView {
        ProjectView {
            id: self.id,
            name: self.name.clone(),
            root: self.root.clone(),
            collapsed: self.collapsed,
            panels: self.panels.iter().map(Panel::view).collect(),
            active_panel: self.active_panel,
        }
    }
}

impl Session {
    pub fn view(&self, layout_viewport: Option<(u16, u16)>) -> SessionView {
        SessionView {
            id: self.id,
            name: self.name.clone(),
            projects: self.projects.iter().map(Project::view).collect(),
            active_project: self.active_project,
            active_panel: self.active_panel(),
            layout_viewport,
        }
    }

    /// The active project's active panel.
    pub fn active_panel(&self) -> Option<PanelId> {
        self.projects
            .iter()
            .find(|p| Some(p.id) == self.active_project)
            .and_then(|p| p.active_panel)
    }

    /// Panels in sidebar order: projects in order, their panels in order, skipping the
    /// panels of collapsed projects.
    pub fn sidebar_panels(&self) -> Vec<PanelId> {
        self.projects
            .iter()
            .filter(|p| !p.collapsed)
            .flat_map(|p| p.panels.iter().map(|panel| panel.id))
            .collect()
    }

    /// Makes `panel` and its project active; false when no such panel.
    pub fn focus_panel(&mut self, panel: PanelId) -> bool {
        let Some(project) = self
            .projects
            .iter_mut()
            .find(|p| p.panels.iter().any(|x| x.id == panel))
        else {
            return false;
        };
        project.active_panel = Some(panel);
        self.active_project = Some(project.id);
        true
    }

    /// Makes `project` active, keeping its last active panel; false when no such project.
    pub fn focus_project(&mut self, project: ProjectId) -> bool {
        if self.projects.iter().any(|p| p.id == project) {
            self.active_project = Some(project);
            true
        } else {
            false
        }
    }

    /// Moves the active panel by `delta` places in [`Session::sidebar_panels`] order,
    /// wrapping around. With no active panel in that order, a forward move lands on the
    /// first panel and a backward one on the last. Returns the new active panel.
    pub fn cycle_panel(&mut self, delta: i32) -> Option<PanelId> {
        let order = self.sidebar_panels();
        if order.is_empty() {
            return None;
        }
        let len = i64::try_from(order.len()).unwrap_or(i64::MAX);
        let current = self
            .active_panel()
            .and_then(|a| order.iter().position(|p| *p == a));
        let target = match current {
            Some(i) => (i64::try_from(i).unwrap_or(0) + i64::from(delta)).rem_euclid(len),
            None if delta < 0 => len - 1,
            None => 0,
        };
        let panel = order[usize::try_from(target).unwrap_or(0)];
        self.focus_panel(panel);
        Some(panel)
    }

    /// Removes a panel. When it was its project's active panel, focus goes to the panel
    /// after it, else the one before, else none.
    pub fn remove_panel(&mut self, panel: PanelId) -> Option<Panel> {
        let project = self
            .projects
            .iter_mut()
            .find(|p| p.panels.iter().any(|x| x.id == panel))?;
        let at = project.panels.iter().position(|x| x.id == panel)?;
        let removed = project.panels.remove(at);
        if project.active_panel == Some(panel) {
            project.active_panel = project
                .panels
                .get(at)
                .or_else(|| at.checked_sub(1).and_then(|i| project.panels.get(i)))
                .map(|p| p.id);
        }
        Some(removed)
    }

    /// Removes a project. When it was active, focus goes to the project after it, else the
    /// one before, else none.
    pub fn remove_project(&mut self, project: ProjectId) -> Option<Project> {
        let at = self.projects.iter().position(|p| p.id == project)?;
        let removed = self.projects.remove(at);
        if self.active_project == Some(project) {
            self.active_project = self
                .projects
                .get(at)
                .or_else(|| at.checked_sub(1).and_then(|i| self.projects.get(i)))
                .map(|p| p.id);
        }
        Some(removed)
    }
}

impl Project {
    /// The lowest number, from 1, that no panel of the project is named.
    pub fn next_panel_name(&self) -> String {
        (1u64..)
            .map(|n| n.to_string())
            .find(|n| self.panels.iter().all(|p| p.name != *n))
            .expect("an unbounded range has a free number")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_display_and_order() {
        assert_eq!(TerminalId(7).to_string(), "7");
        assert!(PanelId(1) < PanelId(2));
        assert_eq!(SessionId(3), SessionId(3));
    }

    #[test]
    fn view_drops_launch_spec() {
        let session = Session {
            id: SessionId(1),
            name: "main".into(),
            active_project: Some(ProjectId(2)),
            projects: vec![Project {
                id: ProjectId(2),
                name: "p".into(),
                root: "/r".into(),
                collapsed: false,
                active_panel: Some(PanelId(3)),
                panels: vec![Panel {
                    id: PanelId(3),
                    name: "editor".into(),
                    name_source: NameSource::Manual,
                    focused_terminal: Some(TerminalId(4)),
                    terminals: vec![Terminal {
                        id: TerminalId(4),
                        launch: LaunchSpec {
                            program: None,
                            args: vec![],
                            cwd: "/r".into(),
                            env: vec![("A".into(), "b".into())],
                        },
                        program: "sh".into(),
                        title: Some("vim".into()),
                        cwd: None,
                        exit_status: None,
                    }],
                }],
            }],
        };
        let view = session.view(Some((80, 24)));
        assert_eq!(view.layout_viewport, Some((80, 24)));
        let t = &view.projects[0].panels[0].terminals[0];
        assert_eq!(t.id, TerminalId(4));
        assert_eq!(t.title.as_deref(), Some("vim"));
        assert_eq!(view.projects[0].active_panel, Some(PanelId(3)));
    }

    fn panel(id: u64, name: &str) -> Panel {
        Panel {
            id: PanelId(id),
            name: name.into(),
            name_source: NameSource::Default,
            terminals: vec![],
            focused_terminal: None,
        }
    }

    fn project(id: u64, panels: &[u64]) -> Project {
        Project {
            id: ProjectId(id),
            name: format!("p{id}"),
            root: "/r".into(),
            collapsed: false,
            active_panel: panels.first().map(|p| PanelId(*p)),
            panels: panels.iter().map(|p| panel(*p, &p.to_string())).collect(),
        }
    }

    fn session(projects: Vec<Project>) -> Session {
        Session {
            id: SessionId(1),
            name: "main".into(),
            active_project: projects.first().map(|p| p.id),
            projects,
        }
    }

    #[test]
    fn removing_the_active_panel_focuses_next_then_previous_then_none() {
        let mut s = session(vec![project(1, &[10, 11, 12])]);
        s.focus_panel(PanelId(11));
        s.remove_panel(PanelId(11));
        assert_eq!(s.active_panel(), Some(PanelId(12)), "next");
        s.remove_panel(PanelId(12));
        assert_eq!(s.active_panel(), Some(PanelId(10)), "previous");
        s.remove_panel(PanelId(10));
        assert_eq!(s.active_panel(), None, "none");
        assert_eq!(s.active_project, Some(ProjectId(1)), "the project stays");
    }

    #[test]
    fn removing_an_inactive_panel_keeps_focus() {
        let mut s = session(vec![project(1, &[10, 11])]);
        s.remove_panel(PanelId(11));
        assert_eq!(s.active_panel(), Some(PanelId(10)));
    }

    #[test]
    fn removing_the_active_project_focuses_next_then_previous_then_none() {
        let mut s = session(vec![
            project(1, &[10]),
            project(2, &[20]),
            project(3, &[30]),
        ]);
        s.focus_project(ProjectId(2));
        s.remove_project(ProjectId(2));
        assert_eq!(s.active_project, Some(ProjectId(3)));
        s.remove_project(ProjectId(3));
        assert_eq!(s.active_project, Some(ProjectId(1)));
        s.remove_project(ProjectId(1));
        assert_eq!(s.active_project, None);
        assert_eq!(s.active_panel(), None);
    }

    #[test]
    fn focus_project_keeps_its_last_active_panel() {
        let mut s = session(vec![project(1, &[10, 11]), project(2, &[20])]);
        s.focus_panel(PanelId(11));
        s.focus_project(ProjectId(2));
        assert_eq!(s.active_panel(), Some(PanelId(20)));
        s.focus_project(ProjectId(1));
        assert_eq!(s.active_panel(), Some(PanelId(11)));
        assert!(!s.focus_project(ProjectId(9)));
    }

    #[test]
    fn cycling_follows_the_sidebar_and_skips_collapsed_projects() {
        let mut s = session(vec![
            project(1, &[10, 11]),
            project(2, &[20]),
            project(3, &[30]),
        ]);
        s.projects[1].collapsed = true;
        assert_eq!(s.sidebar_panels(), [PanelId(10), PanelId(11), PanelId(30)]);
        let walk: Vec<_> = (0..4).map(|_| s.cycle_panel(1).unwrap()).collect();
        assert_eq!(walk, [PanelId(11), PanelId(30), PanelId(10), PanelId(11)]);
        assert_eq!(s.cycle_panel(-1), Some(PanelId(10)));
        assert_eq!(s.cycle_panel(-1), Some(PanelId(30)), "wraps");
        // From a panel hidden by a collapsed project.
        s.focus_panel(PanelId(20));
        assert_eq!(s.cycle_panel(1), Some(PanelId(10)));
        s.focus_panel(PanelId(20));
        assert_eq!(s.cycle_panel(-1), Some(PanelId(30)));
    }

    #[test]
    fn cycling_with_no_panels_does_nothing() {
        let mut s = session(vec![project(1, &[])]);
        assert_eq!(s.cycle_panel(1), None);
    }

    #[test]
    fn panel_names_reuse_the_lowest_free_number() {
        let mut p = project(1, &[10, 11, 12]);
        assert_eq!(p.next_panel_name(), "1");
        p.panels = ["1", "2", "3"].iter().map(|n| panel(0, n)).collect();
        p.panels.remove(1);
        assert_eq!(p.next_panel_name(), "2");
        p.panels[0].name = "custom".into();
        assert_eq!(p.next_panel_name(), "1");
    }
}
