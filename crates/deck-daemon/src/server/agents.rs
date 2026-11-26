//! The status of the coding agents in terminals: detection by the PTY's foreground process
//! on a 1 s tick, and the hook events that sharpen it (the rules are in `crate::agents`). The
//! same tick tells which terminals run a command (`busy`).

use std::sync::Arc;
use std::time::Duration;

use deck_core::agents::{
    AgentKind, AgentResume, AgentSource, AgentState, AgentStatus, match_argv, program_name,
};
use deck_core::icons::is_interactive;
use deck_core::model::{PanelId, SessionId, TerminalId};
use deck_core::protocol::{Response, ServerMsg};

use super::{Shared, State, terminal_model};
use crate::agents::{self, Event};
use crate::lock;
use crate::terminal::TermHandle;

/// What the saved state keeps of a terminal's agent.
fn resume_of(agent: Option<&AgentState>) -> Option<AgentResume> {
    agent.map(|a| AgentResume {
        kind: a.kind,
        session_id: a.session_id.clone(),
    })
}

/// A state without its activity line, to tell a change of the line alone.
fn without_activity(agent: Option<&AgentState>) -> Option<AgentState> {
    agent.map(|a| AgentState {
        activity: None,
        ..a.clone()
    })
}

impl Shared {
    /// Every `every`, asks the operating system for the foreground process of each terminal
    /// and updates its agent state and whether it runs a command. One blocking call covers
    /// all of them, off the async workers.
    pub async fn track_agents(self: Arc<Self>, every: Duration) {
        let mut ticks = tokio::time::interval(every);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticks.tick().await;
        loop {
            ticks.tick().await;
            let targets: Vec<Arc<TermHandle>> = {
                let st = self.st();
                if st.shutting_down {
                    return;
                }
                if st.upgrading {
                    continue;
                }
                st.sessions
                    .iter()
                    .flat_map(|s| s.terminals.values())
                    .filter(|h| !h.has_exited())
                    .cloned()
                    .collect()
            };
            if targets.is_empty() {
                continue;
            }
            let query = self.foreground_query.clone();
            let asked = tokio::task::spawn_blocking(move || {
                targets
                    .into_iter()
                    .map(|h| {
                        let found = query.ask(&h);
                        // Only a command's screen matters: a full-screen program is not busy.
                        let alternate = found.as_ref().is_some_and(|(own, _)| !own)
                            && h.with_emu(|e| e.alternate_screen()).unwrap_or(false);
                        (h, found, alternate)
                    })
                    .collect::<Vec<_>>()
            })
            .await;
            let found = match asked {
                Ok(found) => found,
                Err(e) => {
                    tracing::warn!("the foreground process lookup failed: {e}");
                    continue;
                }
            };
            let config = self.config();
            let mut st = self.st();
            // Once shutting down, the state that stays is the one resume needs.
            if st.shutting_down {
                return;
            }
            if st.upgrading {
                continue;
            }
            let now = self.clock.now();
            let mut changed: Vec<SessionId> = Vec::new();
            let mut moved_terminals: Vec<(SessionId, TerminalId)> = Vec::new();
            let mut resume_changed = false;
            let mut ended: Vec<Arc<TermHandle>> = Vec::new();
            for (h, found, alternate) in found {
                // It may have exited during the lookup, after its exit cleared its agent.
                if h.has_exited() {
                    continue;
                }
                let kind = found
                    .as_ref()
                    .and_then(|(_, a)| match_argv(a, &config.agents));
                let Some(sess) = st.session_mut(h.session) else {
                    continue;
                };
                let Some(model) = terminal_model(&mut sess.model, h.id) else {
                    continue;
                };
                // A process that could not be read leaves the program, the agent and
                // `busy` as they were.
                let Some((own, argv)) = found else {
                    continue;
                };
                let mut moved = false;
                if let Some(name) = program_name(&argv)
                    && name != model.program
                {
                    model.program = name;
                    moved = true;
                }
                let before = resume_of(model.agent.as_ref());
                moved |= agents::detect(
                    &mut model.agent,
                    &mut lock(&h.agent_rt),
                    kind,
                    h.last_output(),
                    now,
                );
                resume_changed |= before != resume_of(model.agent.as_ref());
                let running = !own && model.agent.is_none() && !alternate && !is_interactive(&argv);
                let busy_moved = {
                    let mut rt = lock(&h.agent_rt);
                    let moved = agents::detect_busy(&mut rt, running, now);
                    if moved && !rt.busy {
                        ended.push(Arc::clone(&h));
                    }
                    moved
                };
                if busy_moved && !changed.contains(&h.session) {
                    changed.push(h.session);
                }
                if moved {
                    moved_terminals.push((h.session, h.id));
                    if !changed.contains(&h.session) {
                        changed.push(h.session);
                    }
                }
            }
            // A program or an agent that moved can rename its panel.
            for (sid, tid) in moved_terminals {
                self.refresh_name_of(&mut st, sid, tid);
            }
            // A command that ended while nobody looked leaves its panel unseen.
            for h in ended {
                self.mark_unseen(&mut st, &h);
            }
            for sid in changed {
                self.agents_changed(&mut st, sid);
            }
            if resume_changed {
                self.mark_dirty(&mut st);
            }
        }
    }

    /// A hook event for a terminal: sets its state with `source: Hooks`.
    pub fn agent_event(
        self: &Arc<Self>,
        terminal: TerminalId,
        event: Event,
        raw: Option<String>,
    ) -> Response {
        // Parsed before the locks are taken.
        let doc = raw
            .as_deref()
            .filter(|_| event.kind == AgentKind::OpenCode)
            .map(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());
        let mut st = self.st();
        if st.shutting_down || st.upgrading {
            return Response::Ok;
        }
        let Some(handle) = st
            .sessions
            .iter()
            .find_map(|s| s.terminals.get(&terminal).cloned())
        else {
            return Response::Error(format!("no terminal {terminal}"));
        };
        // An event that arrives after the terminal exited changes nothing (hooks race exits).
        if handle.has_exited() {
            return Response::Ok;
        }
        let now = self.clock.now();
        let sid = handle.session;
        let tid = handle.id;
        let mut before = None;
        let moved = self.with_agent(&mut st, &handle, |agent, rt| {
            before = agent.clone();
            match &doc {
                Some(Some(doc)) => agents::apply_opencode(agent, rt, doc, now),
                Some(None) => false,
                None => agents::apply_event(agent, rt, &event, now),
            }
        });
        if moved {
            let after = self.agent_of(&mut st, &handle);
            // Only the line changed: a tool call every few milliseconds must not flood the
            // clients with views, so it goes through the title throttle.
            if before.is_some()
                && without_activity(before.as_ref()) == without_activity(after.as_ref())
            {
                self.push_view_throttled(&mut st, sid);
                self.bump();
                return Response::Ok;
            }
            self.raise_attention(&mut st, &handle, before.as_ref(), after.as_ref());
            if resume_of(before.as_ref()) != resume_of(after.as_ref()) {
                self.mark_dirty(&mut st);
            }
            self.refresh_name_of(&mut st, sid, tid);
            self.agents_changed(&mut st, sid);
        }
        Response::Ok
    }

    /// The user typed into a terminal: an agent that waited for them works again.
    pub(super) fn agent_answered(self: &Arc<Self>, st: &mut State, handle: &TermHandle) {
        let now = self.clock.now();
        let moved = self.with_agent(st, handle, |agent, rt| agents::answered(agent, rt, now));
        if moved {
            self.refresh_name_of(st, handle.session, handle.id);
            self.agents_changed(st, handle.session);
        }
    }

    /// A terminal's agent state.
    fn agent_of(&self, st: &mut State, handle: &TermHandle) -> Option<AgentState> {
        let sess = st.session_mut(handle.session)?;
        terminal_model(&mut sess.model, handle.id)?.agent.clone()
    }

    /// A hook-sourced agent went from `working` to `waiting` or `idle`: unless a focused
    /// client shows its panel, the panel is marked unseen and the client that was active last
    /// is alerted.
    fn raise_attention(
        &self,
        st: &mut State,
        handle: &TermHandle,
        before: Option<&AgentState>,
        after: Option<&AgentState>,
    ) {
        let (Some(before), Some(after)) = (before, after) else {
            return;
        };
        let hooked = |a: &AgentState| a.source == AgentSource::Hooks;
        if !hooked(before)
            || !hooked(after)
            || before.status != AgentStatus::Working
            || !matches!(after.status, AgentStatus::Waiting | AgentStatus::Idle)
        {
            return;
        }
        let Some((project, panel_id, name)) = self.mark_unseen(st, handle) else {
            return;
        };
        if let Some(client) = st.active_client(handle.session) {
            client.queue.push_msg(ServerMsg::AgentAlert {
                kind: after.kind,
                status: after.status,
                project,
                panel: name,
                panel_id,
            });
        }
    }

    /// Marks the panel of a terminal unseen, unless a focused client shows it. Returns its
    /// project's name, its id and its name when it marked it.
    fn mark_unseen(
        &self,
        st: &mut State,
        handle: &TermHandle,
    ) -> Option<(String, PanelId, String)> {
        let focused = st.session_focused(handle.session);
        let sess = st.session_mut(handle.session)?;
        let (project, panel, name): (String, PanelId, String) =
            sess.model.projects.iter().find_map(|project| {
                let panel = project
                    .panels
                    .iter()
                    .find(|p| p.terminals.iter().any(|t| t.id == handle.id))?;
                Some((project.name.clone(), panel.id, panel.name.clone()))
            })?;
        if focused && sess.model.active_panel() == Some(panel) {
            return None;
        }
        sess.unseen.insert(panel);
        Some((project, panel, name))
    }

    /// Runs `f` on a terminal's agent state and what the daemon remembers beside it.
    fn with_agent(
        &self,
        st: &mut State,
        handle: &TermHandle,
        f: impl FnOnce(&mut Option<AgentState>, &mut agents::AgentRt) -> bool,
    ) -> bool {
        let Some(sess) = st.session_mut(handle.session) else {
            return false;
        };
        let Some(model) = terminal_model(&mut sess.model, handle.id) else {
            return false;
        };
        f(&mut model.agent, &mut lock(&handle.agent_rt))
    }

    /// Tells the session's clients about a changed agent state.
    fn agents_changed(self: &Arc<Self>, st: &mut State, sid: SessionId) {
        st.push_view(sid);
        self.bump();
    }
}
