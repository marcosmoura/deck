//! The control requests that edit or list the tree: thin wrappers that find their targets
//! and call the same functions as the attached client's actions.

use std::sync::Arc;

use deck_core::action::{Action, ActionError, NameKind, validate_name};
use deck_core::model::{LaunchSpec, PanelId, ProjectId, Session, SessionId, SessionView};
use deck_core::protocol::{Request, Response, Target};

use super::{PanelTarget, Shared, panel_of_terminal};

/// What an id names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Terminal,
    Panel,
    Project,
}

fn locate(session: &Session, id: u64) -> Option<Kind> {
    let projects = &session.projects;
    if projects.iter().any(|p| p.id.0 == id) {
        return Some(Kind::Project);
    }
    let mut panels = projects.iter().flat_map(|p| &p.panels);
    if panels.clone().any(|p| p.id.0 == id) {
        return Some(Kind::Panel);
    }
    panels
        .any(|p| p.terminals.iter().any(|t| t.id.0 == id))
        .then_some(Kind::Terminal)
}

impl Shared {
    /// Answers the control requests that read or edit the tree.
    pub async fn control(self: &Arc<Self>, request: Request) -> Response {
        let result = match request {
            Request::Tree { session } => return self.tree(session.as_deref()),
            Request::NewProject {
                session,
                name,
                root,
                focus,
            } => self
                .control_new_project(session, name, root, focus)
                .await
                .map(|id| Response::Created { id }),
            Request::NewPanel {
                session,
                project,
                name,
                launch,
                focus,
            } => self
                .control_new_panel(session, project, name, launch, focus)
                .await
                .map(|id| Response::Created { id }),
            Request::NewTerminal {
                panel,
                launch,
                keep,
                focus,
            } => self
                .control_new_terminal(panel, launch, keep, focus)
                .await
                .map(|id| Response::Created { id }),
            Request::Focus { id } => self.control_focus(id).map(|()| Response::Ok),
            Request::Close { id } => self.control_close(id).map(|()| Response::Ok),
            Request::RenamePanel { panel, name } => self
                .control_rename_panel(panel, name)
                .map(|()| Response::Ok),
            Request::AutoNamePanel { panel } => {
                self.control_auto_name_panel(panel).map(|()| Response::Ok)
            }
            _ => Err("not a tree request".to_string()),
        };
        result.unwrap_or_else(Response::Error)
    }

    /// One session's tree (by name), or every session's, by name.
    fn tree(&self, session: Option<&str>) -> Response {
        let st = self.st();
        let mut views: Vec<SessionView> = st
            .sessions
            .iter()
            .filter(|s| session.is_none_or(|name| s.model.name == name))
            .map(|s| st.view_of(s))
            .collect();
        if let Some(name) = session
            && views.is_empty()
        {
            return Response::Error(format!("no session named \"{name}\""));
        }
        views.sort_by(|a, b| a.name.cmp(&b.name));
        Response::Tree(views)
    }

    fn session_exists(&self, session: SessionId) -> Result<(), String> {
        self.st()
            .session(session)
            .map(|_| ())
            .ok_or_else(|| format!("no session {session}"))
    }

    async fn control_new_project(
        self: &Arc<Self>,
        session: SessionId,
        name: String,
        root: std::path::PathBuf,
        focus: bool,
    ) -> Result<u64, String> {
        self.session_exists(session)?;
        let name = validate_name(&name, NameKind::Project).map_err(|e| e.to_string())?;
        super::check_dir(&root).map_err(|e| e.to_string())?;
        let made = self
            .add_panel(session, PanelTarget::NewProject { name, root }, None, focus)
            .await
            .map_err(|e| e.to_string())?;
        Ok(made.project.0)
    }

    async fn control_new_panel(
        self: &Arc<Self>,
        session: SessionId,
        project: Target,
        name: Option<String>,
        launch: Option<LaunchSpec>,
        focus: bool,
    ) -> Result<u64, String> {
        let project = {
            let st = self.st();
            let sess = st
                .session(session)
                .ok_or_else(|| format!("no session {session}"))?;
            let found = match &project {
                Target::Id(id) => sess.model.projects.iter().find(|p| p.id.0 == *id),
                Target::Name(n) => sess.model.projects.iter().find(|p| p.name == *n),
            };
            match (found, &project) {
                (Some(p), _) => p.id,
                (None, Target::Id(id)) => {
                    return Err(ActionError::NoSuchProject(ProjectId(*id)).to_string());
                }
                (None, Target::Name(n)) => {
                    return Err(format!(
                        "no space named \"{n}\" in session \"{}\"",
                        sess.model.name
                    ));
                }
            }
        };
        let name = name
            .map(|n| validate_name(&n, NameKind::Panel))
            .transpose()
            .map_err(|e| e.to_string())?;
        let made = self
            .add_panel(
                session,
                PanelTarget::Project { project, name },
                launch,
                focus,
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(made.panel.0)
    }

    /// The session that holds `panel`.
    fn session_of_panel(&self, panel: PanelId) -> Result<SessionId, String> {
        self.st()
            .sessions
            .iter()
            .find(|s| {
                s.model
                    .projects
                    .iter()
                    .any(|p| p.panels.iter().any(|x| x.id == panel))
            })
            .map(|s| s.model.id)
            .ok_or_else(|| ActionError::NoSuchPanel(panel).to_string())
    }

    async fn control_new_terminal(
        self: &Arc<Self>,
        panel: PanelId,
        launch: Option<LaunchSpec>,
        keep: bool,
        focus: bool,
    ) -> Result<u64, String> {
        let sid = self.session_of_panel(panel)?;
        let tid = self
            .add_terminal(sid, Some(panel), launch, focus, keep)
            .await
            .map_err(|e| e.to_string())?;
        if focus {
            self.control_focus(tid.0)?;
        }
        Ok(tid.0)
    }

    /// Focuses a terminal (and its panel and project) or a panel.
    fn control_focus(self: &Arc<Self>, id: u64) -> Result<(), String> {
        let mut st = self.st();
        let found = st
            .sessions
            .iter()
            .find_map(|s| locate(&s.model, id).map(|k| (s.model.id, k)));
        let Some((sid, kind @ (Kind::Terminal | Kind::Panel))) = found else {
            return Err(format!("{id} is not a terminal or panel"));
        };
        let Some(sess) = st.session_mut(sid) else {
            return Err(format!("no session {sid}"));
        };
        let before = sess.model.active_panel();
        let mut renamed = None;
        let panel = match kind {
            Kind::Terminal => {
                let terminal = deck_core::model::TerminalId(id);
                panel_of_terminal(&mut sess.model, terminal).map(|p| {
                    p.focused_terminal = Some(terminal);
                    renamed = Some(p.id);
                    p.id
                })
            }
            _ => Some(PanelId(id)),
        };
        if let Some(panel) = panel {
            sess.model.focus_panel(panel);
        }
        self.settle_named(&mut st, sid, before, renamed);
        Ok(())
    }

    /// Closes a terminal, a panel or a project, without asking.
    fn control_close(self: &Arc<Self>, id: u64) -> Result<(), String> {
        let found = self
            .st()
            .sessions
            .iter()
            .find_map(|s| locate(&s.model, id).map(|k| (s.model.id, k)));
        let Some((sid, kind)) = found else {
            return Err(format!("{id} is not a terminal, panel or space"));
        };
        let action = match kind {
            Kind::Terminal => Action::CloseTerminal {
                terminal: deck_core::model::TerminalId(id),
            },
            Kind::Panel => Action::ClosePanel { panel: PanelId(id) },
            Kind::Project => Action::CloseProject {
                project: ProjectId(id),
            },
        };
        self.edit(sid, action).map_err(|e| e.to_string())
    }

    fn control_rename_panel(self: &Arc<Self>, panel: PanelId, name: String) -> Result<(), String> {
        let sid = self.session_of_panel(panel)?;
        self.edit(sid, Action::RenamePanel { panel, name })
            .map_err(|e| e.to_string())
    }

    fn control_auto_name_panel(self: &Arc<Self>, panel: PanelId) -> Result<(), String> {
        let sid = self.session_of_panel(panel)?;
        self.edit(sid, Action::AutoNamePanel { panel })
            .map_err(|e| e.to_string())
    }
}
