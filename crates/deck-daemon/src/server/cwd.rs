//! Where each shell is: from OSC 7 while the shell reports it, else from the operating
//! system, asked every few seconds for the terminal's own child process.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use deck_core::model::SessionId;

use super::{Shared, State, terminal_model};
use crate::terminal::TermHandle;

impl Shared {
    /// A shell reported its directory with OSC 7. The report holds until the operating
    /// system sees the process change directory with no new report (see `track_cwds`).
    pub fn osc7_cwd(self: &Arc<Self>, h: &Arc<TermHandle>, path: PathBuf) {
        h.set_reports_cwd();
        let mut st = self.st();
        self.set_cwd(&mut st, h.session, h, path);
    }

    /// Stores a terminal's live directory. A change reaches the clients, no more often than
    /// title changes do; it is saved when the saved directory (the live one, else the launch
    /// one) differs.
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
        self.push_view_throttled(st, sid);
    }

    /// Every `every`, asks the operating system for the directory of each terminal's
    /// process. One blocking call covers all of them, off the async workers. The answer is
    /// the directory of a terminal that reports none. For one that does, it only tells when
    /// the reports went stale: the process moved and no report came with the move, as after
    /// an OSC 7 that some output carried and no shell sent.
    pub async fn track_cwds(self: Arc<Self>, every: Duration) {
        let mut ticks = tokio::time::interval(every);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticks.tick().await;
        loop {
            ticks.tick().await;
            // With each terminal, whether it reported since the last round.
            let targets: Vec<(Arc<TermHandle>, bool, u32)> = {
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
                    .filter_map(|h| h.pid.map(|pid| (Arc::clone(h), h.cwd_reported(true), pid)))
                    .collect()
            };
            if targets.is_empty() {
                continue;
            }
            let query = self.cwd_query.clone();
            let asked = tokio::task::spawn_blocking(move || {
                targets
                    .into_iter()
                    .map(|(h, reported, pid)| (h, reported, query.ask(pid)))
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
            if st.upgrading {
                continue;
            }
            for (h, reported, cwd) in found {
                let Some(cwd) = cwd else {
                    continue;
                };
                let moved = h.os_moved(&cwd);
                if h.reports_cwd() {
                    // A report that came with the move wins, also one that arrived while
                    // the query ran.
                    if !moved || reported || h.cwd_reported(false) {
                        continue;
                    }
                    h.clear_reports_cwd();
                }
                self.set_cwd(&mut st, h.session, &h, cwd);
            }
        }
    }
}
