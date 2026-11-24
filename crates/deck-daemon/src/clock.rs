//! Time as the daemon's decisions see it. Rate limits and the exit drain read a [`Clock`],
//! so tests can drive them exactly; real waiting still uses tokio timers.

use std::fmt;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A monotonic clock: the time since some fixed origin.
pub trait Clock: Send + Sync + fmt::Debug {
    fn now(&self) -> Duration;
}

/// The real clock.
#[derive(Debug)]
pub struct SystemClock {
    origin: Instant,
}

impl SystemClock {
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl SystemClock {
    /// A clock that reads `start` now and goes on from there: a new image continues the old
    /// one's time, so instants kept in the model stay valid.
    pub fn starting_at(start: Duration) -> Self {
        let now = Instant::now();
        Self {
            origin: now.checked_sub(start).unwrap_or(now),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
}

/// A clock that only moves when told to.
#[derive(Debug, Default)]
pub struct ManualClock {
    now: Mutex<Duration>,
}

impl ManualClock {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn advance(&self, by: Duration) {
        *self.now.lock().unwrap_or_else(|e| e.into_inner()) += by;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Duration {
        *self.now.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// What to do about a change that is rate limited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Poke {
    /// Act now.
    Now,
    /// Act after this long, by calling [`Throttle::fire`].
    Wait(Duration),
    /// A later action is already scheduled and will cover this change.
    Covered,
}

/// Allows one action per `min` interval, with a trailing action so the last change is
/// never lost.
#[derive(Debug, Clone)]
pub struct Throttle {
    min: Duration,
    last: Option<Duration>,
    pending: bool,
}

impl Throttle {
    pub fn new(min: Duration) -> Self {
        Self {
            min,
            last: None,
            pending: false,
        }
    }

    pub fn poke(&mut self, now: Duration) -> Poke {
        if self.pending {
            return Poke::Covered;
        }
        match self.last {
            Some(last) if now.saturating_sub(last) < self.min => {
                self.pending = true;
                Poke::Wait(self.min - now.saturating_sub(last))
            }
            _ => {
                self.last = Some(now);
                Poke::Now
            }
        }
    }

    /// Called when a [`Poke::Wait`] elapsed, with the clock's time: the action is due once
    /// the interval since the last one has passed on the clock.
    pub fn fire(&mut self, now: Duration) -> Fire {
        if !self.pending {
            return Fire::Idle;
        }
        let since = now.saturating_sub(self.last.unwrap_or_default());
        if since < self.min {
            return Fire::Wait(self.min - since);
        }
        self.pending = false;
        self.last = Some(now);
        Fire::Due
    }
}

/// The answer of [`Throttle::fire`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fire {
    /// Act now.
    Due,
    /// The clock has not moved far enough: ask again after this long.
    Wait(Duration),
    /// Nothing is scheduled.
    Idle,
}

/// How long a terminal's output is read after its process exited: until the end of the
/// stream, or until it has been quiet for `silence`.
#[derive(Debug, Clone, Copy)]
pub struct Drain {
    silence: Duration,
}

impl Drain {
    pub fn new(silence: Duration) -> Self {
        Self { silence }
    }

    /// The time left to wait given the last activity, `None` once the silence is long
    /// enough.
    pub fn remaining(&self, last_activity: Duration, now: Duration) -> Option<Duration> {
        let quiet = now.saturating_sub(last_activity);
        self.silence.checked_sub(quiet).filter(|d| !d.is_zero())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn throttle_limits_a_thousand_changes_in_a_second() {
        let clock = ManualClock::new();
        let mut throttle = Throttle::new(100 * MS);
        let mut actions = 0;
        let mut waits: Vec<Duration> = Vec::new();
        for _ in 0..1000 {
            match throttle.poke(clock.now()) {
                Poke::Now => actions += 1,
                Poke::Wait(d) => waits.push(clock.now() + d),
                Poke::Covered => {}
            }
            clock.advance(MS);
            // Fire scheduled actions whose time came.
            if waits.first().is_some_and(|at| *at <= clock.now()) {
                waits.remove(0);
                if throttle.fire(clock.now()) == Fire::Due {
                    actions += 1;
                }
            }
        }
        // The last change is always covered by a trailing action.
        if !waits.is_empty() {
            clock.advance(100 * MS);
            if throttle.fire(clock.now()) == Fire::Due {
                actions += 1;
            }
        }
        assert!((10..=12).contains(&actions), "{actions}");
    }

    #[test]
    fn throttle_frozen_clock_sends_first_and_one_trailing() {
        let mut t = Throttle::new(100 * MS);
        assert_eq!(t.poke(Duration::ZERO), Poke::Now);
        assert_eq!(t.poke(Duration::ZERO), Poke::Wait(100 * MS));
        assert_eq!(t.poke(Duration::ZERO), Poke::Covered);
        // A frozen clock never makes the trailing action due.
        assert_eq!(t.fire(Duration::ZERO), Fire::Wait(100 * MS));
        assert_eq!(t.fire(60 * MS), Fire::Wait(40 * MS));
        assert_eq!(t.fire(100 * MS), Fire::Due);
        assert_eq!(t.fire(100 * MS), Fire::Idle);
    }

    #[test]
    fn drain_expires_after_the_silence() {
        let drain = Drain::new(200 * MS);
        let last = 1000 * MS;
        assert_eq!(drain.remaining(last, last), Some(200 * MS));
        assert_eq!(drain.remaining(last, last + 150 * MS), Some(50 * MS));
        assert_eq!(drain.remaining(last, last + 200 * MS), None);
        assert_eq!(drain.remaining(last, last + 900 * MS), None);
        // Data after the last look restarts the silence.
        assert_eq!(
            drain.remaining(last + 150 * MS, last + 200 * MS),
            Some(150 * MS)
        );
    }
}
