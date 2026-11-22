//! Saving the model to `state.json` and rebuilding it at start.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use deck_core::agents::{
    AgentSource, AgentState, AgentStatus, RESUME_EXEC_TAIL, match_argv, resume_command,
    valid_session_id,
};
use deck_core::model::{LaunchSpec, PanelId, ProjectId, SessionId};
use deck_core::protocol::NoticeLevel;
use deck_core::state::{StateFile, TerminalState};

use super::{
    DEFAULT_VIEWPORT, Env, Place, SessionRt, Shared, State, TITLE_VIEW_INTERVAL, Throttle,
    panel_of_terminal, program_name, shell_program, single_size, terminal,
};
use crate::lock;
use crate::persist;
use crate::terminal::SpawnRequest;

/// A change schedules a save this long after the first one; changes in between join it.
const SAVE_DELAY: Duration = Duration::from_millis(500);

/// A restored terminal that is still to be started.
#[derive(Debug, Clone)]
pub struct Launch {
    session: SessionId,
    project: ProjectId,
    panel: PanelId,
    root: PathBuf,
    saved: TerminalState,
}

/// The first of `candidates` that is a directory.
pub(super) fn first_dir(candidates: impl IntoIterator<Item = Option<PathBuf>>) -> Option<PathBuf> {
    candidates
        .into_iter()
        .flatten()
        .find(|p| fs::metadata(p).is_ok_and(|m| m.is_dir()))
}

fn snapshot(st: &State) -> StateFile {
    let mut attached: Vec<(u64, SessionId)> = st
        .sessions
        .iter()
        .filter(|s| s.last_attach > 0)
        .map(|s| (s.last_attach, s.model.id))
        .collect();
    attached.sort();
    StateFile::new(
        st.next_id + 1,
        attached.into_iter().map(|(_, id)| id).collect(),
        st.sessions.iter().map(|s| &s.model),
    )
}

impl Shared {
    /// Marks the state as changed: the first mark schedules a save 500 ms later, and marks
    /// before it joins that save. Nothing is scheduled once the daemon is shutting down:
    /// the shutdown saves.
    pub(super) fn mark_dirty(self: &Arc<Self>, st: &mut State) {
        if st.shutting_down || st.save_pending {
            return;
        }
        st.save_pending = true;
        let shared = Arc::clone(self);
        self.handle.spawn(async move {
            tokio::time::sleep(SAVE_DELAY).await;
            let writer = Arc::clone(&shared);
            // Writing waits for the disk: not on an async worker.
            let _ = tokio::task::spawn_blocking(move || writer.save(false)).await;
        });
    }

    /// Writes the state. A scheduled save (`last == false`) gives up when the daemon is
    /// shutting down; the shutdown save (`last == true`) always writes.
    fn save(&self, last: bool) {
        let _writing = lock(&self.save_lock);
        let file = {
            let mut st = self.st();
            if st.shutting_down && !last {
                return;
            }
            st.save_pending = false;
            snapshot(&st)
        };
        if let Err(e) = persist::write(&self.paths.state_file(), &file) {
            tracing::error!("cannot save {}: {e}", self.paths.state_file().display());
        }
    }

    /// The save of a shutdown, before the terminals are hung up.
    pub(super) async fn save_final(self: &Arc<Self>) {
        let shared = Arc::clone(self);
        if let Err(e) = tokio::task::spawn_blocking(move || shared.save(true)).await {
            tracing::error!("the shutdown save did not finish: {e}");
        }
    }

    /// Rebuilds the sessions of a saved state; their terminals are started by
    /// [`Shared::launch_restored`] with the returned list.
    pub fn restore(self: &Arc<Self>, file: StateFile) -> Vec<Launch> {
        let session_env: Env = std::env::vars_os().collect();
        let shell = program_name(&shell_program(&self.config(), &session_env));
        let display = |t: &TerminalState| {
            t.program
                .as_deref()
                .map_or_else(|| shell.clone(), program_name)
        };

        let mut st = self.st();
        st.next_id = file.next_id.saturating_sub(1);
        let mut ticks: HashMap<SessionId, u64> = HashMap::new();
        for id in &file.last_attached {
            let tick = st.touch();
            ticks.insert(*id, tick);
        }
        let mut launches = Vec::new();
        for saved in &file.sessions {
            for project in &saved.projects {
                for panel in &project.panels {
                    launches.extend(panel.terminals.iter().map(|t| Launch {
                        session: saved.id,
                        project: project.id,
                        panel: panel.id,
                        root: project.root.clone(),
                        saved: t.clone(),
                    }));
                }
            }
            let mut model = saved.to_model(&display);
            // A terminal that resumes its agent has it from the start: a save before the
            // first detection tick must keep the kind and the session id.
            let now = self.clock.now();
            for terminal in model
                .projects
                .iter_mut()
                .flat_map(|p| &mut p.panels)
                .flat_map(|p| &mut p.terminals)
            {
                let resumed = saved
                    .projects
                    .iter()
                    .flat_map(|p| &p.panels)
                    .flat_map(|p| &p.terminals)
                    .find(|t| t.id == terminal.id)
                    .filter(|t| self.resume_plan(t, &session_env).is_some())
                    .and_then(|t| t.agent.as_ref());
                if let Some(resume) = resumed {
                    terminal.agent = Some(AgentState {
                        kind: resume.kind,
                        status: AgentStatus::Quiet,
                        source: AgentSource::Process,
                        session_id: resume.session_id.clone(),
                        first_prompt: None,
                        since: now,
                    });
                }
            }
            st.sessions.push(SessionRt {
                model,
                env: session_env.clone(),
                terminals: HashMap::new(),
                viewport: None,
                title_throttle: Throttle::new(TITLE_VIEW_INTERVAL),
                last_attach: ticks.get(&saved.id).copied().unwrap_or(0),
                startup: None,
                unseen: HashSet::new(),
            });
        }
        launches
    }

    /// Starts the restored terminals one after the other; clients can already attach.
    pub async fn launch_restored(self: Arc<Self>, launches: Vec<Launch>) {
        for launch in launches {
            if self.st().shutting_down {
                return;
            }
            self.launch_one(launch).await;
        }
    }

    /// The program and arguments a restored terminal is started with: the resume command
    /// when it resumes its agent (see [`Shared::resume_plan`]), else what was saved. The
    /// terminal's launch spec in the model stays what it was.
    fn restored_command(&self, saved: &TerminalState, env: &Env) -> (Option<String>, Vec<String>) {
        self.resume_plan(saved, env)
            .unwrap_or_else(|| (saved.program.clone(), saved.args.clone()))
    }

    /// How a terminal that had an agent resumes it (`resume_agents`), `None` when it does
    /// not: a shell terminal runs the resume command and then replaces `sh` with the shell,
    /// an explicit agent command is replaced by the resume command. Only a validated
    /// template and session id reach the command line.
    fn resume_plan(
        &self,
        saved: &TerminalState,
        env: &Env,
    ) -> Option<(Option<String>, Vec<String>)> {
        let config = self.config();
        let resume = saved.agent.as_ref().filter(|_| config.resume_agents)?;
        if let Some(id) = resume.session_id.as_deref()
            && !valid_session_id(id)
        {
            tracing::warn!("the saved session id of terminal {} is not used", saved.id);
        }
        let Some(command) = resume_command(&config.agents, resume) else {
            tracing::warn!("no valid resume command for terminal {}", saved.id);
            return None;
        };
        match &saved.program {
            None => {
                let script = format!("{}{RESUME_EXEC_TAIL}", command.join(" "));
                let mut args = vec!["-c".to_string(), script, shell_program(&config, env)];
                args.extend(config.shell_args.iter().cloned());
                Some((Some("/bin/sh".to_string()), args))
            }
            Some(program) => {
                let argv: Vec<String> = std::iter::once(program.clone())
                    .chain(saved.args.iter().cloned())
                    .collect();
                if match_argv(&argv, &config.agents) != Some(resume.kind) {
                    return None;
                }
                let mut words = command.into_iter();
                Some((words.next(), words.collect()))
            }
        }
    }

    /// The process of a restored terminal, in `dir`, with the daemon's start environment
    /// plus the additions of its launch spec.
    fn restored_request(&self, launch: &Launch, dir: &std::path::Path) -> SpawnRequest {
        let env: Env = std::env::vars_os().collect();
        let (program, args) = self.restored_command(&launch.saved, &env);
        let spec = LaunchSpec {
            program,
            args,
            cwd: dir.to_path_buf(),
            env: launch
                .saved
                .env
                .iter()
                .map(|(k, v)| (OsString::from(k), OsString::from(v)))
                .collect(),
        };
        self.spawn_request(
            &env,
            Some(&spec),
            dir,
            single_size(DEFAULT_VIEWPORT),
            Place {
                session: launch.session,
                panel: launch.panel,
                terminal: launch.saved.id,
            },
        )
    }

    async fn launch_one(self: &Arc<Self>, launch: Launch) {
        let tid = launch.saved.id;
        // The first directory that exists: portable-pty would silently use the home
        // directory for a missing one, so the daemon looks itself.
        let candidates = [
            launch.saved.cwd.clone(),
            Some(launch.root.clone()),
            std::env::var_os("HOME").map(PathBuf::from),
        ];
        let template = self.restored_request(&launch, &launch.root);
        let opened = tokio::task::spawn_blocking(move || {
            let dir = first_dir(candidates)
                .ok_or_else(|| "no working directory exists any more".to_string())?;
            let req = SpawnRequest {
                cwd: dir,
                ..template
            };
            terminal::open(&req).map(|parts| (parts, req))
        })
        .await
        .unwrap_or_else(|e| Err(format!("could not start the terminal: {e}")));

        let mut st = self.st();
        let sid = launch.session;
        let present = st
            .session_mut(sid)
            .and_then(|s| super::terminal_model(&mut s.model, tid))
            .is_some();
        if st.shutting_down || !present {
            // Closed or killed while it started, or the daemon is stopping.
            if let Ok((parts, _)) = opened {
                parts.discard();
            }
            return;
        }
        match opened {
            Ok((parts, req)) => {
                let handle = terminal::start(self, tid, sid, &req, parts);
                st.restored.insert(tid);
                let Some(sess) = st.session_mut(sid) else {
                    return;
                };
                sess.terminals.insert(tid, handle);
                if let Some(t) = super::terminal_model(&mut sess.model, tid) {
                    t.launch.cwd = req.cwd;
                }
                let before = sess.model.active_panel();
                self.settle(&mut st, sid, before);
                for frame in st.full_frames_of(sid, &[tid]) {
                    st.push_frame(sid, &frame);
                }
            }
            Err(reason) => {
                let Some(sess) = st.session_mut(sid) else {
                    return;
                };
                let before = sess.model.active_panel();
                let mut names = (String::new(), String::new());
                if let Some(project) = sess.model.projects.iter().find(|p| p.id == launch.project) {
                    names.0.clone_from(&project.name);
                }
                let mut emptied = None;
                if let Some(panel) = panel_of_terminal(&mut sess.model, tid) {
                    names.1.clone_from(&panel.name);
                    panel.remove_terminal(tid);
                    if panel.terminals.is_empty() {
                        emptied = Some(panel.id);
                    }
                }
                if let Some(panel) = emptied {
                    sess.model.remove_panel(panel);
                }
                self.settle(&mut st, sid, before);
                drop(st);
                self.notify(
                    NoticeLevel::Warning,
                    format!(
                        "could not restore a terminal in {}/{}: {reason}",
                        names.0, names.1
                    ),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_directory_that_exists_wins() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file");
        fs::write(&file, b"x").unwrap();
        let gone = dir.path().join("gone");
        let there = dir.path().to_path_buf();
        let found = first_dir([Some(gone.clone()), Some(file), None, Some(there.clone())]);
        assert_eq!(found, Some(there));
        assert_eq!(first_dir([Some(gone), None]), None);
    }
}
