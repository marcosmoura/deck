//! Actions an attached client asks of its session, their errors, and the pure validation
//! and default names the daemon and the client share.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::agents::AgentKind;
use crate::layout::{Direction, Ratios};
use crate::model::{PanelId, ProjectId, TerminalId};

/// Longest name, in characters.
pub const MAX_NAME_LEN: usize = 64;

/// What a new panel starts, resolved by the daemon from the config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PanelLaunch {
    /// The agent's `[agents.<kind>] command`, run directly in the project root.
    Agent(AgentKind),
}

/// What an action changes, on the attached session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    NewProject {
        name: String,
        #[serde(with = "crate::wire::path")]
        root: PathBuf,
    },
    RenameProject {
        project: ProjectId,
        name: String,
    },
    CloseProject {
        project: ProjectId,
    },
    SetProjectCollapsed {
        project: ProjectId,
        collapsed: bool,
    },
    FocusProject {
        project: ProjectId,
    },
    NewPanel {
        project: ProjectId,
        name: Option<String>,
    },
    /// A new panel that runs a program chosen by the daemon, in the project root.
    NewPanelWith {
        project: ProjectId,
        launch: PanelLaunch,
    },
    RenamePanel {
        panel: PanelId,
        name: String,
    },
    /// Unlocks a manually named panel: its name follows what runs in it again.
    AutoNamePanel {
        panel: PanelId,
    },
    ClosePanel {
        panel: PanelId,
    },
    FocusPanel {
        panel: PanelId,
    },
    CyclePanel {
        delta: i32,
    },
    RenameSession {
        name: String,
    },
    KillSession,
    /// A new shell in `panel`, or in the active panel.
    NewTerminal {
        panel: Option<PanelId>,
    },
    CloseTerminal {
        terminal: TerminalId,
    },
    FocusTerminal {
        terminal: TerminalId,
    },
    /// Moves focus in the active panel to the terminal in that direction.
    FocusDirection {
        dir: Direction,
    },
    /// Stores `ratios` for `panel`, which the client computed for `n` terminals.
    SetRatios {
        panel: PanelId,
        n: usize,
        ratios: Ratios,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum ActionError {
    #[error("name must not be empty")]
    EmptyName,
    #[error("name must be at most {MAX_NAME_LEN} characters")]
    NameTooLong,
    #[error("name must not contain control characters")]
    NameHasControl,
    #[error("name must not contain /")]
    NameHasSlash,
    #[error("a project named \"{0}\" already exists")]
    ProjectExists(String),
    #[error("a session named \"{0}\" already exists")]
    SessionExists(String),
    #[error("\"{0}\" is not an absolute path")]
    NotAbsolute(String),
    #[error("\"{0}\" is not a directory")]
    NotADirectory(String),
    #[error("no project {0}")]
    NoSuchProject(ProjectId),
    #[error("no panel {0}")]
    NoSuchPanel(PanelId),
    #[error("no terminal {0}")]
    NoSuchTerminal(TerminalId),
    #[error("not enough room for another terminal")]
    NoRoom,
    #[error("layout changed, try again")]
    LayoutChanged,
    #[error("invalid layout ratios")]
    InvalidRatios,
    #[error("no panel to act on")]
    NoActivePanel,
    #[error("no session named \"{0}\"")]
    NoSuchSession(String),
    #[error("not attached to a session")]
    NotAttached,
    #[error("the daemon is shutting down")]
    ShuttingDown,
    #[error("{0}")]
    Spawn(String),
    /// `key` is the `[agents.<kind>]` setting that supplied `program`.
    #[error("\"{program}\" not found in PATH (set [agents.{}] {key})", .kind.name())]
    ProgramNotFound {
        program: String,
        kind: AgentKind,
        key: String,
    },
    #[error("{0}")]
    LaunchFailed(String),
}

/// What a name names; session and project names may not contain `/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameKind {
    Session,
    Project,
    Panel,
}

/// The name as it is stored: trimmed, 1 to [`MAX_NAME_LEN`] characters, no control
/// characters, and no `/` unless it names a panel.
pub fn validate_name(raw: &str, kind: NameKind) -> Result<String, ActionError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(ActionError::EmptyName);
    }
    if name.chars().count() > MAX_NAME_LEN {
        return Err(ActionError::NameTooLong);
    }
    if name.chars().any(char::is_control) {
        return Err(ActionError::NameHasControl);
    }
    if kind != NameKind::Panel && name.contains('/') {
        return Err(ActionError::NameHasSlash);
    }
    Ok(name.to_string())
}

/// A new project's default name: the directory's basename, `~` for the home directory and
/// `root` for `/`.
pub fn default_project_name(dir: &Path, home: Option<&Path>) -> String {
    if home.is_some_and(|h| h == dir) {
        return "~".into();
    }
    let base = dir
        .file_name()
        .map_or_else(|| "root".to_string(), |n| n.to_string_lossy().into_owned());
    let clean: String = base
        .chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .take(MAX_NAME_LEN)
        .collect();
    let clean = clean.trim();
    if clean.is_empty() {
        "root".into()
    } else {
        clean.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{ClientMsg, ServerMsg, decode, encode};

    #[test]
    fn names_are_trimmed_and_checked() {
        let ok = validate_name;
        assert_eq!(ok("  api ", NameKind::Project).unwrap(), "api");
        assert_eq!(ok("", NameKind::Project), Err(ActionError::EmptyName));
        assert_eq!(ok("   ", NameKind::Session), Err(ActionError::EmptyName));
        assert_eq!(
            ok(&"x".repeat(65), NameKind::Panel),
            Err(ActionError::NameTooLong)
        );
        assert!(ok(&"x".repeat(64), NameKind::Panel).is_ok());
        assert_eq!(
            ok("a\nb", NameKind::Panel),
            Err(ActionError::NameHasControl)
        );
        assert_eq!(ok("a/b", NameKind::Session), Err(ActionError::NameHasSlash));
        assert_eq!(ok("a/b", NameKind::Project), Err(ActionError::NameHasSlash));
        assert_eq!(ok("a/b", NameKind::Panel).unwrap(), "a/b");
    }

    #[test]
    fn error_messages_are_readable() {
        assert_eq!(
            ActionError::ProjectExists("api".into()).to_string(),
            "a project named \"api\" already exists"
        );
        assert_eq!(
            ActionError::NotADirectory("/nope".into()).to_string(),
            "\"/nope\" is not a directory"
        );
        assert_eq!(ActionError::EmptyName.to_string(), "name must not be empty");
        assert_eq!(
            ActionError::ProgramNotFound {
                program: "opencode".into(),
                kind: AgentKind::OpenCode,
                key: "command".into(),
            }
            .to_string(),
            "\"opencode\" not found in PATH (set [agents.opencode] command)"
        );
        assert_eq!(
            ActionError::LaunchFailed("Permission denied".into()).to_string(),
            "Permission denied"
        );
    }

    #[test]
    fn default_names_follow_the_directory() {
        let home = Path::new("/home/me");
        assert_eq!(
            default_project_name(Path::new("/srv/api"), Some(home)),
            "api"
        );
        assert_eq!(default_project_name(home, Some(home)), "~");
        assert_eq!(default_project_name(Path::new("/"), Some(home)), "root");
        assert_eq!(default_project_name(Path::new("/"), None), "root");
        assert_eq!(default_project_name(Path::new("/a/b/"), None), "b");
        assert_eq!(
            default_project_name(Path::new(&format!("/{}", "x".repeat(100))), None).len(),
            MAX_NAME_LEN
        );
    }

    #[test]
    fn every_action_round_trips() {
        let actions = [
            Action::NewProject {
                name: "p".into(),
                root: "/r".into(),
            },
            Action::RenameProject {
                project: ProjectId(1),
                name: "q".into(),
            },
            Action::CloseProject {
                project: ProjectId(1),
            },
            Action::SetProjectCollapsed {
                project: ProjectId(1),
                collapsed: true,
            },
            Action::FocusProject {
                project: ProjectId(1),
            },
            Action::NewPanel {
                project: ProjectId(1),
                name: Some("n".into()),
            },
            Action::NewPanel {
                project: ProjectId(1),
                name: None,
            },
            Action::NewPanelWith {
                project: ProjectId(1),
                launch: PanelLaunch::Agent(AgentKind::OpenCode),
            },
            Action::RenamePanel {
                panel: PanelId(2),
                name: "x".into(),
            },
            Action::AutoNamePanel { panel: PanelId(2) },
            Action::ClosePanel { panel: PanelId(2) },
            Action::FocusPanel { panel: PanelId(2) },
            Action::CyclePanel { delta: -1 },
            Action::RenameSession { name: "s".into() },
            Action::KillSession,
            Action::NewTerminal { panel: None },
            Action::NewTerminal {
                panel: Some(PanelId(2)),
            },
            Action::CloseTerminal {
                terminal: TerminalId(5),
            },
            Action::FocusTerminal {
                terminal: TerminalId(5),
            },
            Action::FocusDirection {
                dir: Direction::Left,
            },
            Action::SetRatios {
                panel: PanelId(2),
                n: 3,
                ratios: Ratios::default_for(3),
            },
        ];
        for a in actions {
            let msg = ClientMsg::Action(a);
            assert_eq!(decode::<ClientMsg>(&encode(&msg).unwrap()).unwrap(), msg);
        }
        for r in [
            Ok(()),
            Err(ActionError::EmptyName),
            Err(ActionError::NoSuchPanel(PanelId(4))),
            Err(ActionError::Spawn("boom".into())),
            Err(ActionError::NoRoom),
            Err(ActionError::NoSuchTerminal(TerminalId(3))),
            Err(ActionError::LayoutChanged),
            Err(ActionError::InvalidRatios),
            Err(ActionError::ProgramNotFound {
                program: "opencode".into(),
                kind: AgentKind::OpenCode,
                key: "command".into(),
            }),
            Err(ActionError::LaunchFailed("denied".into())),
        ] {
            let msg = ServerMsg::ActionResult(r);
            assert_eq!(decode::<ServerMsg>(&encode(&msg).unwrap()).unwrap(), msg);
        }
    }
}
