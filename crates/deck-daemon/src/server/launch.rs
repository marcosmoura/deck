//! `Action::NewPanelWith`: a new panel that runs the program the config names for an agent,
//! or that resumes one of its sessions.

use std::ffi::{OsStr, OsString};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;

use deck_core::action::{ActionError, PanelLaunch};
use deck_core::agents::{AgentKind, check_template, render_template, valid_session_id};
use deck_core::model::{LaunchSpec, ProjectId, SessionId};
use deck_core::protocol::NoticeLevel;

use super::{ClientId, PanelTarget, Shared};
use crate::env::{self, Env};

/// The `[agents.<kind>]` setting that supplies the program of a `NewPanelWith`.
const COMMAND_KEY: &str = "command";
/// The `[agents.<kind>]` setting that supplies the program of a resume.
const RESUME_KEY: &str = "resume";

/// Whether `path` is a file some user can execute (symlinks are followed).
fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Whether `program` names an executable file: as a path when it has a `/`, else in a
/// directory of `path` (an empty entry is skipped; a relative one is taken from `cwd`, the
/// terminal's directory, as the PTY layer does). Blocking: it reads the file system.
///
/// Any execute bit counts: telling which one applies to the daemon's user needs its
/// effective uid and gids, which std does not give without `unsafe`.
fn program_exists(program: &str, path: Option<&OsStr>, cwd: &Path) -> bool {
    if program.contains('/') {
        return is_executable(Path::new(program));
    }
    path.is_some_and(|path| {
        std::env::split_paths(path)
            .filter(|dir| !dir.as_os_str().is_empty())
            .any(|dir| is_executable(&cwd.join(dir).join(program)))
    })
}

impl Shared {
    /// Starts a panel in `project` that runs the command of `launch`, directly, in the
    /// project's root (or, for a resume, the session's directory). Nothing is created when the
    /// program is missing or fails to start.
    pub(super) async fn new_panel_with(
        self: &Arc<Self>,
        cid: ClientId,
        sid: SessionId,
        project: ProjectId,
        launch: PanelLaunch,
    ) -> Result<(), ActionError> {
        let (kind, key, template, session_id, want_cwd) = match launch {
            PanelLaunch::Agent(kind) => (
                kind,
                COMMAND_KEY,
                self.config().agents.get(kind).command.clone(),
                None,
                None,
            ),
            PanelLaunch::Resume {
                kind,
                session_id,
                cwd,
            } => {
                if !valid_session_id(&session_id) {
                    return Err(ActionError::InvalidSessionId);
                }
                (
                    kind,
                    RESUME_KEY,
                    self.config().agents.get(kind).resume.clone(),
                    Some(session_id),
                    Some(cwd),
                )
            }
        };
        let argv = if check_template(&template, session_id.is_some()).is_ok() {
            render_template(&template, session_id.as_deref())
        } else {
            Vec::new()
        };
        let mut words = argv.into_iter();
        let Some(program) = words.next() else {
            return Err(ActionError::LaunchFailed(format!(
                "[agents.{}] {key} is not a valid command",
                kind.name()
            )));
        };
        let (root, session_env) = {
            let st = self.st();
            let sess = st.session(sid).ok_or(ActionError::NotAttached)?;
            let root = sess
                .model
                .projects
                .iter()
                .find(|p| p.id == project)
                .map(|p| p.root.clone())
                .ok_or(ActionError::NoSuchProject(project))?;
            (root, sess.env.clone())
        };
        // The session's directory when it still exists, else the project root.
        let (cwd, missing) = match want_cwd {
            Some(want) => {
                let probe = want.clone();
                let exists =
                    tokio::task::spawn_blocking(move || probe.is_absolute() && probe.is_dir())
                        .await
                        .unwrap_or(false);
                if exists {
                    (want, None)
                } else {
                    (root.clone(), Some(want))
                }
            }
            None => (root.clone(), None),
        };
        lookup(&program, &session_env, &cwd, kind, key).await?;
        let spec = LaunchSpec {
            program: Some(program),
            args: words.collect(),
            cwd,
            env: Vec::new(),
        };
        self.add_panel(
            sid,
            PanelTarget::Project {
                project,
                name: None,
            },
            Some(spec),
            true,
        )
        .await
        .map(|_| ())
        .map_err(|e| match e {
            ActionError::Spawn(text) => ActionError::LaunchFailed(text),
            other => other,
        })?;
        if let Some(want) = missing {
            // Only to the client that asked, and not logged: a directory is private.
            self.notify_client(
                cid,
                NoticeLevel::Warning,
                format!(
                    "{} no longer exists; started in {}",
                    want.display(),
                    root.display()
                ),
            );
        }
        Ok(())
    }
}

/// Looks `program` up in the `PATH` of the session's environment, on the blocking pool.
async fn lookup(
    program: &str,
    session_env: &Env,
    cwd: &Path,
    kind: AgentKind,
    key: &str,
) -> Result<(), ActionError> {
    let path: Option<OsString> = env::lookup(session_env, "PATH").map(OsStr::to_os_string);
    let name = program.to_string();
    let cwd = cwd.to_path_buf();
    let found = tokio::task::spawn_blocking(move || program_exists(&name, path.as_deref(), &cwd))
        .await
        .map_err(|e| ActionError::LaunchFailed(format!("could not look up {program}: {e}")))?;
    if found {
        Ok(())
    } else {
        Err(ActionError::ProgramNotFound {
            program: program.to_string(),
            kind,
            key: key.to_string(),
        })
    }
}
