//! `wait`: the daemon answers when the condition holds, so a client never polls.
//!
//! A pending wait checks its condition under the state lock, then sleeps until something
//! changed (`Shared::bump`) or, when a time limit or a quiet period is running, until it may
//! have passed. Time is read from the daemon's clock, so tests can drive it exactly; the
//! sleeps are capped so a clock that moves by hand is noticed soon.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use deck_core::model::TerminalId;
use deck_core::protocol::{Response, WaitFor, Waited};

use super::Shared;
use crate::terminal::TermHandle;

/// How long the exit code of a terminal that ended is kept for a late `wait --exit`.
const KEEP_EXITS: Duration = Duration::from_secs(60);
/// At most this many exit codes are kept; the oldest go first.
const MAX_EXITS: usize = 256;
/// The longest a wait sleeps while it depends on a clock that does not run by itself.
const NAP_CAP: Duration = Duration::from_millis(50);

/// The exit codes of terminals that ended and left the daemon.
#[derive(Debug, Default)]
pub(super) struct Exits(VecDeque<(TerminalId, i32, Duration)>);

impl Exits {
    pub(super) fn record(&mut self, id: TerminalId, code: i32, now: Duration) {
        self.prune(now);
        self.0.retain(|(other, ..)| *other != id);
        self.0.push_back((id, code, now));
        while self.0.len() > MAX_EXITS {
            self.0.pop_front();
        }
    }

    pub(super) fn get(&mut self, id: TerminalId, now: Duration) -> Option<i32> {
        self.prune(now);
        self.0
            .iter()
            .find(|(other, ..)| *other == id)
            .map(|(_, code, _)| *code)
    }

    /// The codes with their age at `now`, for a live upgrade.
    pub(super) fn export(&self, now: Duration) -> Vec<(TerminalId, i32, Duration)> {
        self.0
            .iter()
            .map(|(id, code, at)| (*id, *code, now.saturating_sub(*at)))
            .collect()
    }

    /// The reverse of [`Exits::export`].
    pub(super) fn import(
        &mut self,
        codes: impl IntoIterator<Item = (TerminalId, i32, Duration)>,
        now: Duration,
    ) {
        for (id, code, age) in codes {
            self.record(id, code, now.saturating_sub(age));
        }
    }

    fn prune(&mut self, now: Duration) {
        self.0
            .retain(|(.., at)| now.saturating_sub(*at) < KEEP_EXITS);
    }
}

/// Counts a pending wait until it ends or is dropped (its client went away).
struct WaiterGuard(Arc<AtomicUsize>);

impl WaiterGuard {
    fn new(count: &Arc<AtomicUsize>) -> Self {
        count.fetch_add(1, Ordering::AcqRel);
        Self(Arc::clone(count))
    }
}

impl Drop for WaiterGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

enum Step {
    Done(Response),
    /// Not yet; the condition may hold after this long even if nothing changes.
    Pending(Option<Duration>),
}

fn done(waited: Waited) -> Step {
    Step::Done(Response::Waited(waited))
}

fn failed(text: impl Into<String>) -> Step {
    Step::Done(Response::Error(text.into()))
}

impl Shared {
    /// Wakes the pending waits: a terminal printed, ended or left, a session changed or went,
    /// or the daemon is stopping.
    pub(super) fn bump(&self) {
        self.changes.send_modify(|n| *n = n.wrapping_add(1));
    }

    /// Answers `Wait`; dropping the future is how a client that went away cancels it.
    pub async fn wait(
        self: &Arc<Self>,
        terminal: TerminalId,
        until: WaitFor,
        timeout_ms: Option<u64>,
    ) -> Response {
        let _guard = WaiterGuard::new(&self.waiters);
        let mut changes = self.changes.subscribe();
        let started = self.clock.now();
        let deadline = timeout_ms.map(|ms| started.saturating_add(Duration::from_millis(ms)));
        let mut handle = None;
        loop {
            let mut nap = match self.wait_step(terminal, &until, started, &mut handle) {
                Step::Done(response) => return response,
                Step::Pending(nap) => nap,
            };
            if let Some(deadline) = deadline {
                let left = deadline.saturating_sub(self.clock.now());
                if left.is_zero() {
                    return Response::Waited(Waited::TimedOut);
                }
                nap = Some(nap.map_or(left, |n| n.min(left)));
            }
            match nap {
                Some(nap) => {
                    tokio::select! {
                        _ = changes.changed() => {}
                        () = tokio::time::sleep(if self.clock.runs() { nap } else { nap.min(NAP_CAP) }) => {}
                    }
                }
                None => {
                    let _ = changes.changed().await;
                }
            }
        }
    }

    /// Checks the condition once.
    fn wait_step(
        &self,
        terminal: TerminalId,
        until: &WaitFor,
        started: Duration,
        handle: &mut Option<Arc<TermHandle>>,
    ) -> Step {
        let mut st = self.st();
        let now = self.clock.now();
        if st.shutting_down {
            return failed("deck daemon stopped");
        }
        let h = match handle {
            Some(h) => Arc::clone(h),
            None => {
                // A terminal that was closed a moment ago is still being hung up: its exit
                // code is on its way.
                let found = st
                    .sessions
                    .iter()
                    .find_map(|s| s.terminals.get(&terminal).cloned())
                    .or_else(|| st.hanging.iter().find(|h| h.id == terminal).cloned());
                let Some(found) = found else {
                    if matches!(until, WaitFor::Exit)
                        && let Some(code) = st.exits.get(terminal, now)
                    {
                        return done(Waited::Exit(code));
                    }
                    return failed(format!("no terminal {terminal}"));
                };
                *handle = Some(Arc::clone(&found));
                found
            }
        };
        // A killed session answers the same error whatever the terminal's exit was.
        let Some(sess) = st.session(h.session) else {
            return failed("session was killed");
        };
        let in_model = sess.terminals.contains_key(&h.id);
        // The scan below can be long; it must not hold the state lock.
        drop(st);
        if matches!(until, WaitFor::Exit)
            && let Some(code) = h.finished()
        {
            return done(Waited::Exit(code));
        }
        if !in_model {
            return match until {
                // Its hang-up produces the exit code.
                WaitFor::Exit => Step::Pending(None),
                _ => failed(format!("terminal {terminal} was closed")),
            };
        }
        match until {
            WaitFor::Exit => Step::Pending(None),
            WaitFor::Text(text) => match h.with_emu({
                let text = text.clone();
                move |e| e.find_line(&text)
            }) {
                Some(Some(line)) => done(Waited::Text(line)),
                Some(None) => Step::Pending(None),
                None => failed(format!("terminal {terminal} was closed")),
            },
            WaitFor::Idle { quiet_ms } => {
                let due = started
                    .max(h.last_output())
                    .saturating_add(Duration::from_millis(*quiet_ms));
                if now >= due {
                    done(Waited::Idle)
                } else {
                    Step::Pending(Some(due - now))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn exit_codes_are_kept_for_a_minute() {
        let mut exits = Exits::default();
        exits.record(TerminalId(1), 4, secs(10));
        assert_eq!(exits.get(TerminalId(1), secs(69)), Some(4));
        assert_eq!(exits.get(TerminalId(1), secs(70)), None);
        assert_eq!(exits.get(TerminalId(2), secs(10)), None);
    }

    #[test]
    fn at_most_256_exit_codes_are_kept_oldest_first() {
        let mut exits = Exits::default();
        for id in 0..300u64 {
            exits.record(TerminalId(id), 1, secs(1));
        }
        assert_eq!(exits.get(TerminalId(43), secs(2)), None);
        assert_eq!(exits.get(TerminalId(44), secs(2)), Some(1));
        assert_eq!(exits.get(TerminalId(299), secs(2)), Some(1));
        // A terminal recorded again keeps one entry, the newest.
        exits.record(TerminalId(299), 7, secs(3));
        assert_eq!(exits.get(TerminalId(299), secs(3)), Some(7));
        assert_eq!(exits.0.len(), 256);
    }
}
