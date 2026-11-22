//! Where each shell is: from OSC 7 when the shell reports it, else from the operating
//! system, asked every few seconds for the terminal's own child process.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use deck_core::model::SessionId;

use super::{Shared, State, terminal_model};
use crate::terminal::TermHandle;

impl Shared {
    /// A shell reported its directory with OSC 7. From now on the terminal is never asked
    /// the operating system.
    pub fn osc7_cwd(self: &Arc<Self>, h: &Arc<TermHandle>, path: PathBuf) {
        h.set_reports_cwd();
        let mut st = self.st();
        self.set_cwd(&mut st, h.session, h, path);
    }

    /// Stores a terminal's live directory. A change reaches the clients; it is saved when
    /// the saved directory (the live one, else the launch one) differs.
    fn set_cwd(self: &Arc<Self>, st: &mut State, sid: SessionId, h: &TermHandle, path: PathBuf) {
        let Some(sess) = st.session_mut(sid) else {
            return;
        };
        let Some(model) = terminal_model(&mut sess.model, h.id) else {
            return;
        };
        if model.cwd.as_ref() == Some(&path) {
            return;
        }
        let saved_changes = model.cwd.as_ref().unwrap_or(&model.launch.cwd) != &path;
        model.cwd = Some(path);
        if saved_changes {
            self.mark_dirty(st);
        }
        st.push_view(sid);
    }

    /// Every `every`, asks the operating system for the directory of each terminal that
    /// never reported one. One blocking call covers all of them, off the async workers.
    pub async fn track_cwds(self: Arc<Self>, every: Duration) {
        let mut ticks = tokio::time::interval(every);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticks.tick().await;
        loop {
            ticks.tick().await;
            let targets: Vec<(Arc<TermHandle>, u32)> = {
                let st = self.st();
                if st.shutting_down {
                    return;
                }
                st.sessions
                    .iter()
                    .flat_map(|s| s.terminals.values())
                    .filter(|h| !h.reports_cwd() && !h.has_exited())
                    .filter_map(|h| h.pid.map(|pid| (Arc::clone(h), pid)))
                    .collect()
            };
            if targets.is_empty() {
                continue;
            }
            let query = self.cwd_query.clone();
            let asked = tokio::task::spawn_blocking(move || {
                targets
                    .into_iter()
                    .map(|(h, pid)| (h, query.ask(pid)))
                    .collect::<Vec<_>>()
            })
            .await;
            let found = match asked {
                Ok(found) => found,
                Err(e) => {
                    tracing::warn!("the directory lookup failed: {e}");
                    continue;
                }
            };
            let mut st = self.st();
            if st.shutting_down {
                return;
            }
            for (h, cwd) in found {
                // OSC 7 may have arrived while the query ran: it wins.
                if let Some(cwd) = cwd
                    && !h.reports_cwd()
                {
                    self.set_cwd(&mut st, h.session, &h, cwd);
                }
            }
        }
    }
}
