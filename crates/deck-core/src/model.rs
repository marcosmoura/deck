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
    pub title: Option<String>,
    #[serde(with = "crate::wire::opt_path")]
    pub cwd: Option<PathBuf>,
    pub exit_status: Option<i32>,
}

impl Terminal {
    pub fn view(&self) -> TerminalView {
        TerminalView {
            id: self.id,
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
            layout_viewport,
        }
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
}
