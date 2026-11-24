//! The live upgrade as the daemon's state sees it: freezing it, taking it out into a
//! handoff file, giving it back after a failed exec, and rebuilding it in the new image.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::MutexGuard;
use std::time::Duration;

use deck_core::model::{Session, SessionId, Terminal, TerminalId};
use deck_core::protocol::{ByeReason, NoticeLevel, ServerMsg};

use super::restore::Launch;
use super::{
    SessionRt, Shared, State, TITLE_VIEW_INTERVAL, Throttle, WRITER_FLUSH, panel_of_terminal,
};
use crate::handoff::file::{ExitRec, HandoffFile, NoticeRec, SessionRec, TermRec};
use crate::handoff::{HANDOFF_VERSION, Inherited};
use crate::terminal::{self, Parts, Seed, TermHandle};

/// How long the handoff waits for processes that are being started.
const SPAWN_WAIT: Duration = Duration::from_secs(2);
/// How long each terminal's queued input has to reach its PTY.
const INPUT_WAIT: Duration = Duration::from_millis(500);

/// What a terminal's threads handed over when they paused.
#[derive(Debug, Default)]
struct Paused {
    held: Vec<u8>,
    reaped: Option<i32>,
}

/// The daemon between the freeze and the exec: every terminal's reader and wait thread is
/// paused, and nothing changes the model.
#[derive(Debug)]
pub(crate) struct Frozen {
    handles: Vec<Arc<TermHandle>>,
    paused: HashMap<TerminalId, Paused>,
}

/// What the new image has rebuilt.
#[derive(Debug)]
pub(crate) struct Adopted {
    /// The terminals that run, to nudge.
    pub live: Vec<Arc<TermHandle>>,
    /// Terminals that did not come over.
    pub dropped: usize,
    /// Terminals the old daemon had not started yet.
    pub launches: Vec<Launch>,
}

impl Frozen {
    pub(crate) fn handles(&self) -> &[Arc<TermHandle>] {
        &self.handles
    }
}

impl Shared {
    /// The lock, once no handoff holds the list of terminals: a process started while one
    /// runs is added when it is over (when it fails, never).
    pub(super) async fn st_after_open(&self) -> MutexGuard<'_, State> {
        loop {
            let frozen = self.st().frozen;
            if !frozen {
                let st = self.st();
                if !st.frozen {
                    return st;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    pub(crate) fn st_is_upgrading(&self) -> bool {
        self.st().upgrading
    }

    pub(crate) fn st_is_shutting_down(&self) -> bool {
        self.st().shutting_down
    }

    /// How many terminals the sessions hold.
    pub fn terminal_count(&self) -> usize {
        self.st().sessions.iter().map(|s| s.terminals.len()).sum()
    }

    /// The first half of a handoff: refuses what changes the model, lets processes that are
    /// being started finish, says goodbye to the clients and pauses every terminal's threads.
    /// On an error the caller gives the state back with [`Shared::thaw`].
    pub(crate) async fn freeze(
        self: &Arc<Self>,
        exe: &std::path::Path,
    ) -> Result<Frozen, (String, Vec<Arc<TermHandle>>)> {
        self.st().upgrading = true;
        let deadline = tokio::time::Instant::now() + SPAWN_WAIT;
        loop {
            let busy = self.opening.load(std::sync::atomic::Ordering::Acquire) > 0 || {
                let st = self.st();
                st.creating > 0 || !st.hanging.is_empty()
            };
            if !busy || tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let handles: Vec<Arc<TermHandle>> = {
            let mut st = self.st();
            st.frozen = true;
            // The clients hear it from here; they reconnect to the new image.
            for client in st.clients.values() {
                client.queue.push_msg(ServerMsg::Bye {
                    reason: ByeReason::Upgrading {
                        exe: exe.to_path_buf(),
                    },
                });
            }
            st.sessions
                .iter()
                .flat_map(|s| s.terminals.values().cloned())
                .collect()
        };

        let flush = tokio::time::Instant::now() + WRITER_FLUSH;
        while self.writers.load(std::sync::atomic::Ordering::Acquire) > 0 {
            if tokio::time::timeout_at(flush, self.writers_done.notified())
                .await
                .is_err()
            {
                break;
            }
        }

        let input = tokio::time::Instant::now() + INPUT_WAIT;
        while handles.iter().any(|h| h.pending_input() > 0) && tokio::time::Instant::now() < input {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        for h in &handles {
            let left = h.pending_input();
            if left > 0 {
                tracing::warn!(terminal = %h.id, "dropping {left} bytes of input that did not reach the program");
            }
        }

        let pausing = handles.clone();
        let paused = tokio::task::spawn_blocking(move || pause_all(&pausing))
            .await
            .map_err(|e| format!("the pause did not finish: {e}"))
            .and_then(|r| r);
        match paused {
            Ok(paused) => Ok(Frozen { handles, paused }),
            Err(why) => Err((why, handles)),
        }
    }

    /// Gives the state back after a handoff that did not happen: the threads go on and
    /// requests are served again.
    pub(crate) fn thaw(self: &Arc<Self>, handles: &[Arc<TermHandle>]) {
        for h in handles {
            h.resume_reader();
            h.resume_waiter();
        }
        let mut st = self.st();
        st.upgrading = false;
        st.frozen = false;
    }

    /// Saves `state.json` now, so an adoption that fails outright restores the layout.
    pub(crate) fn save_now(&self) {
        // The shutdown flag may be set already: the layout is written all the same.
        self.save(true);
    }

    /// The handoff file for the frozen state: the model, every terminal's snapshot and the
    /// rest the new image cannot learn from a descriptor.
    pub(crate) fn collect(
        self: &Arc<Self>,
        frozen: &Frozen,
        signals: Vec<String>,
        inherited: &Inherited,
    ) -> Result<HandoffFile, String> {
        let st = self.st();
        let now = self.clock.now();
        let mut sessions = Vec::new();
        let mut terminals = Vec::new();
        for sess in &st.sessions {
            sessions.push(SessionRec {
                model: sess.model.clone(),
                env: sess.env.clone(),
                viewport: sess.viewport,
                cell: sess.cell,
                last_attach: sess.last_attach,
                unseen: sess.unseen.iter().copied().collect(),
            });
            for (tid, h) in &sess.terminals {
                let kept_exit = model_terminal(&sess.model, *tid)
                    .and_then(|t| t.exit_status.filter(|_| t.keep));
                let exited = h.finished().filter(|_| kept_exit.is_some());
                let paused = frozen.paused.get(tid);
                let (snapshot, (cols, rows), cell) = h
                    .with_emu(|e| (e.snapshot(), e.size(), e.cell()))
                    .ok_or_else(|| format!("terminal {tid} has no emulator"))?;
                let (pid, fd) = if exited.is_some() {
                    (None, None)
                } else {
                    let fd = h
                        .master_fd()
                        .ok_or_else(|| format!("terminal {tid} has no PTY descriptor"))?;
                    (h.pid, Some(fd))
                };
                terminals.push(TermRec {
                    id: *tid,
                    session: sess.model.id,
                    program: h.program.clone(),
                    pid,
                    fd,
                    cols,
                    rows,
                    cell,
                    reports_cwd: h.reports_cwd(),
                    reaped: paused.and_then(|p| p.reaped),
                    exited,
                    started_nanos: nanos(h.started),
                    last_data_nanos: nanos(h.last_output()),
                    snapshot,
                    held: paused.map(|p| p.held.clone()).unwrap_or_default(),
                });
            }
        }
        let exits = st
            .exits
            .export(now)
            .into_iter()
            .map(|(terminal, code, age)| ExitRec {
                terminal,
                code,
                age_nanos: nanos(age),
            })
            .collect();
        let notices = st
            .notices
            .iter()
            .map(|(level, text)| NoticeRec {
                level: *level,
                text: text.clone(),
            })
            .collect();
        Ok(HandoffFile {
            handoff_version: HANDOFF_VERSION,
            protocol_version: self.version,
            pid: std::process::id(),
            generation: 0,
            clock_nanos: nanos(now),
            next_id: st.next_id,
            tick: st.tick,
            sessions,
            notices,
            exits,
            signals,
            terminals,
            listener_fd: inherited.listener,
            lock_fd: inherited.lock,
            state_lock_fd: inherited.state_lock,
        })
    }

    /// The new image's half: rebuilds the sessions and their terminals from the handoff
    /// file. A terminal that does not come over is dropped (its master closes, so its
    /// program is hung up), logged and reported in a notice; the others go on. Blocking.
    pub(crate) fn adopt(self: &Arc<Self>, file: HandoffFile) -> Adopted {
        let now = self.clock.now();
        let mut failed: Vec<(SessionId, TerminalId, String)> = Vec::new();
        let mut live = Vec::new();
        let mut by_session: HashMap<SessionId, HashMap<TerminalId, Arc<TermHandle>>> =
            HashMap::new();

        let mut st = self.st();
        st.next_id = file.next_id;
        st.tick = file.tick;
        for rec in file.terminals {
            let (id, session) = (rec.id, rec.session);
            match self.adopt_terminal(rec) {
                Ok(handle) => {
                    if !handle.has_exited() {
                        live.push(Arc::clone(&handle));
                    }
                    by_session.entry(session).or_default().insert(id, handle);
                }
                Err(why) => failed.push((session, id, why)),
            }
        }
        for rec in file.sessions {
            let mut model = rec.model;
            let gone: Vec<TerminalId> = failed
                .iter()
                .filter(|(s, ..)| *s == model.id)
                .map(|(_, t, _)| *t)
                .collect();
            for tid in gone {
                drop_from_model(&mut model, tid);
            }
            st.sessions.push(SessionRt {
                terminals: by_session.remove(&model.id).unwrap_or_default(),
                model,
                env: rec.env,
                viewport: rec.viewport,
                cell: rec.cell,
                title_throttle: Throttle::new(TITLE_VIEW_INTERVAL),
                last_attach: rec.last_attach,
                startup: None,
                unseen: rec.unseen.into_iter().collect(),
            });
        }
        st.exits.import(
            file.exits
                .into_iter()
                .map(|e| (e.terminal, e.code, Duration::from_nanos(e.age_nanos))),
            now,
        );
        st.notices
            .extend(file.notices.into_iter().map(|n| (n.level, n.text)));
        drop(st);

        let dropped = failed.len();
        for (_, id, why) in failed {
            self.notify(
                NoticeLevel::Warning,
                format!("terminal {id} could not be kept through the upgrade: {why}"),
            );
        }
        Adopted {
            live,
            dropped,
            launches: self.pending_launches(),
        }
    }

    fn adopt_terminal(self: &Arc<Self>, rec: TermRec) -> Result<Arc<TermHandle>, String> {
        let seed = Seed {
            cols: rec.cols,
            rows: rec.rows,
            cell: rec.cell,
            snapshot: rec.snapshot,
            held: rec.held,
            reaped: rec.reaped,
            reports_cwd: rec.reports_cwd,
            started: Duration::from_nanos(rec.started_nanos),
            last_data: Duration::from_nanos(rec.last_data_nanos),
            finished: rec.exited,
        };
        let parts = match (rec.fd, rec.pid, rec.exited) {
            (_, _, Some(_)) => Parts::finished(rec.cols, rec.rows, rec.cell),
            (Some(fd), Some(pid), None) => Parts::adopted(fd, pid)?,
            _ => return Err("the handoff file has no PTY for it".into()),
        };
        terminal::adopt(self, rec.id, rec.session, &rec.program, parts, seed)
    }
}

fn model_terminal(model: &Session, id: TerminalId) -> Option<&Terminal> {
    model
        .projects
        .iter()
        .flat_map(|p| &p.panels)
        .flat_map(|p| &p.terminals)
        .find(|t| t.id == id)
}

fn nanos(d: Duration) -> u64 {
    u64::try_from(d.as_nanos()).unwrap_or(u64::MAX)
}

/// Removes a terminal that did not come over from the model, and its panel when it was the
/// last one in it.
fn drop_from_model(model: &mut Session, tid: TerminalId) {
    let mut emptied = None;
    if let Some(panel) = panel_of_terminal(model, tid) {
        panel.remove_terminal(tid);
        if panel.terminals.is_empty() {
            emptied = Some(panel.id);
        }
    }
    if let Some(panel) = emptied {
        model.remove_panel(panel);
    }
}

/// Pauses every terminal's reader and wait thread, all at once: each waits up to a poll
/// interval for its thread, so one after the other would add up.
fn pause_all(handles: &[Arc<TermHandle>]) -> Result<HashMap<TerminalId, Paused>, String> {
    let results: Vec<(TerminalId, Result<Paused, String>)> = std::thread::scope(|scope| {
        let workers: Vec<_> = handles
            .iter()
            .map(|h| {
                scope.spawn(move || {
                    let held = h.pause_reader();
                    let reaped = h.pause_waiter();
                    let paused = match (held, reaped) {
                        (Ok(held), Ok(reaped)) => Ok(Paused { held, reaped }),
                        _ => Err(format!("terminal {} did not stop in time", h.id)),
                    };
                    (h.id, paused)
                })
            })
            .collect();
        workers.into_iter().filter_map(|w| w.join().ok()).collect()
    });
    if results.len() != handles.len() {
        return Err("a thread that pauses a terminal failed".into());
    }
    let mut paused = HashMap::new();
    for (id, result) in results {
        paused.insert(id, result?);
    }
    Ok(paused)
}
