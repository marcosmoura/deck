//! A pause switch for a thread that loops: it checks the gate once per round, and whoever
//! wants it stopped waits until it has parked there.

use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

/// The thread did not stop within the time given.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the thread did not stop in time")]
pub struct PauseTimeout;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum State {
    #[default]
    Running,
    /// A pause was asked for; the thread has not reached the gate yet.
    Requested,
    /// The thread waits at the gate.
    Parked,
    /// The thread ended, so there is nothing to wait for.
    Done,
}

#[derive(Debug, Default)]
struct Inner {
    state: State,
    /// What the thread handed over when it parked.
    pending: Vec<u8>,
    /// `poke` was called and `sleep` has not seen it yet.
    poked: bool,
}

#[derive(Debug, Default)]
pub struct Gate {
    inner: Mutex<Inner>,
    changed: Condvar,
}

impl Gate {
    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// For the looping thread, once per round: parks here while a pause is on. `pending`
    /// runs when it parks, and its bytes are what `pause` returns.
    pub fn checkpoint(&self, pending: impl FnOnce() -> Vec<u8>) {
        let mut inner = self.inner();
        if inner.state == State::Requested {
            inner.pending = pending();
            inner.state = State::Parked;
            self.changed.notify_all();
        }
        while inner.state == State::Parked {
            inner = self
                .changed
                .wait(inner)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Runs `f` unless a pause is on, and tells whether it did. A pause cannot be asked for
    /// while `f` runs.
    pub fn unless_requested(&self, f: impl FnOnce()) -> bool {
        let inner = self.inner();
        if matches!(inner.state, State::Requested | State::Parked) {
            return false;
        }
        f();
        true
    }

    /// For the looping thread: sleeps up to `time`, and wakes at once when a pause is asked
    /// for.
    /// Returns whether it was cut short by `poke`.
    pub fn sleep(&self, time: Duration) -> bool {
        let inner = self.inner();
        let (mut inner, _) = self
            .changed
            .wait_timeout_while(inner, time, |i| i.state == State::Running && !i.poked)
            .unwrap_or_else(PoisonError::into_inner);
        std::mem::take(&mut inner.poked)
    }

    /// Ends the thread's current or next `sleep` at once.
    pub fn poke(&self) {
        self.inner().poked = true;
        self.changed.notify_all();
    }

    /// For the looping thread, when it ends: nobody has to wait for it any more.
    pub fn finish(&self) {
        let mut inner = self.inner();
        inner.state = State::Done;
        inner.pending.clear();
        self.changed.notify_all();
    }

    /// Asks the thread to stop and waits until it has (or ended), at most `limit`. Returns
    /// what the thread handed over when it parked.
    pub fn pause(&self, limit: Duration) -> Result<Vec<u8>, PauseTimeout> {
        self.pause_waking(limit, || {})
    }

    /// Like `pause`, and runs `wake` once the pause is requested, for a thread that waits
    /// on something other than this gate.
    pub fn pause_waking(
        &self,
        limit: Duration,
        wake: impl FnOnce(),
    ) -> Result<Vec<u8>, PauseTimeout> {
        let mut inner = self.inner();
        if inner.state == State::Running {
            inner.state = State::Requested;
            self.changed.notify_all();
        }
        wake();
        let (inner, _) = self
            .changed
            .wait_timeout_while(inner, limit, |i| i.state == State::Requested)
            .unwrap_or_else(PoisonError::into_inner);
        if inner.state == State::Requested {
            return Err(PauseTimeout);
        }
        Ok(inner.pending.clone())
    }

    /// Lets a paused thread go on.
    pub fn resume(&self) {
        let mut inner = self.inner();
        if matches!(inner.state, State::Requested | State::Parked) {
            inner.state = State::Running;
            inner.pending.clear();
            self.changed.notify_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_poke_ends_the_sleep_at_once_and_only_once() {
        let gate = Gate::default();
        gate.poke();
        let start = std::time::Instant::now();
        assert!(gate.sleep(Duration::from_secs(5)));
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(!gate.sleep(Duration::from_millis(5)));
    }
}
