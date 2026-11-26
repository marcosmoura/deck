//! The input thread: it waits for the outer terminal itself and calls crossterm only to
//! drain what is ready, so a terminal that went away ends it instead of spinning in crossterm.

use std::io::{self, Read};
use std::os::fd::BorrowedFd;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use ratatui::crossterm::event as ct;
use tokio::sync::mpsc::UnboundedSender;

use crate::colors::LateAnswers;
use crate::{INPUT_IDLE, INPUT_POLL};

/// What the outer terminal's input is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StdinState {
    /// Bytes are waiting.
    Ready,
    /// Nothing came within the time.
    Idle,
    /// The terminal is gone (closed, revoked or failing).
    HungUp,
}

/// Waits up to the given time for input on the outer terminal without reading it, or for
/// the second descriptor (the wake-up, see [`run`]) to be readable, which counts as `Ready`.
/// `deck` builds it from `poll(2)`, since `deck-tui` has no `unsafe`.
pub type WaitStdin = Box<dyn Fn(Duration, BorrowedFd<'_>) -> StdinState + Send>;

/// Makes the outer terminal report its waiting input as new: on Linux, re-applying its
/// settings wakes everything that waits on it. `deck` builds it, since `deck-tui` has no
/// `unsafe`.
pub type RearmStdin = Box<dyn Fn() + Send>;

/// One ready event, `None` when nothing is ready (it never waits). crossterm's `poll` also
/// returns false after it skipped an event of its own (a cursor, keyboard or attribute
/// answer) while more events sit in its parser, so only a second poll that finds nothing
/// too ends the drain.
fn ready_from(
    mut poll: impl FnMut() -> io::Result<bool>,
    read: impl FnOnce() -> io::Result<ct::Event>,
) -> io::Result<Option<ct::Event>> {
    if poll()? || poll()? {
        read().map(Some)
    } else {
        Ok(None)
    }
}

/// One ready event from crossterm, `None` when nothing is ready (it never waits).
fn ready_event() -> io::Result<Option<ct::Event>> {
    ready_from(|| ct::poll(Duration::ZERO), ct::read)
}

/// The reader thread's body, with the real crossterm. The main loop writes to `wake` on
/// `SIGWINCH`: crossterm also watches that signal, and when it and input are both pending in
/// one of its polls it can drop the input, so the thread is woken to take the signal first.
pub fn run(
    wait: WaitStdin,
    rearm: RearmStdin,
    wake: &UnixStream,
    late: LateAnswers,
    stop: &AtomicBool,
    tx: &UnboundedSender<ct::Event>,
) {
    run_with(&*wait, &*rearm, wake, ready_event, late, stop, tx);
}

/// Empties the wake-up socket (it is non-blocking).
fn clear(mut wake: &UnixStream) {
    let mut buf = [0u8; 64];
    while matches!(wake.read(&mut buf), Ok(n) if n > 0) {}
}

/// Each round: a zero-time check for a hang-up, a drain of crossterm (its own buffer is
/// invisible to `poll(2)`), then the wait. Returning drops the sender, which ends the main loop.
fn run_with(
    wait: &dyn Fn(Duration, BorrowedFd<'_>) -> StdinState,
    rearm: &dyn Fn(),
    wake: &UnixStream,
    mut next: impl FnMut() -> io::Result<Option<ct::Event>>,
    mut late: LateAnswers,
    stop: &AtomicBool,
    tx: &UnboundedSender<ct::Event>,
) {
    // The last wait said `Ready`: if the drain then finds nothing, crossterm used up the
    // readiness without reading the bytes, and trusting the wait again would spin.
    let mut ready_last = false;
    // Whether the terminal was re-armed since crossterm last gave an event.
    let mut rearmed = false;
    while !stop.load(Ordering::Acquire) {
        let fd = std::os::fd::AsFd::as_fd(wake);
        if wait(Duration::ZERO, fd) == StdinState::HungUp {
            return;
        }
        clear(wake);
        let mut got = false;
        loop {
            match next() {
                Ok(Some(event)) => {
                    got = true;
                    if late
                        .feed(event, Instant::now())
                        .into_iter()
                        .any(|e| tx.send(e).is_err())
                    {
                        return;
                    }
                }
                Ok(None) => break,
                Err(_) => return,
            }
        }
        if got {
            rearmed = false;
        }
        if ready_last && !got {
            ready_last = false;
            // crossterm waits for input edge-triggered and reads only until one event parses,
            // so what it left in the kernel needs new input to be seen. On Linux a terminal
            // holds a burst's sender back until fewer than 128 bytes wait, so none comes: the
            // re-arm gives crossterm the wake-up instead. Once per stall; then the old sleep.
            if !rearmed {
                rearmed = true;
                rearm();
                continue;
            }
            std::thread::sleep(INPUT_POLL);
            continue;
        }
        let timeout = late
            .next_wake(Instant::now())
            .map_or(INPUT_IDLE, |d| d.min(INPUT_IDLE));
        let state = wait(timeout, fd);
        ready_last = state == StdinState::Ready;
        match state {
            StdinState::HungUp => return,
            StdinState::Idle => {
                if late
                    .flush(Instant::now())
                    .into_iter()
                    .any(|e| tx.send(e).is_err())
                {
                    return;
                }
            }
            StdinState::Ready => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;

    use ratatui::crossterm::event::{KeyCode, KeyEvent};
    use tokio::sync::mpsc;

    use super::*;

    fn key(c: char) -> ct::Event {
        ct::Event::Key(KeyEvent::from(KeyCode::Char(c)))
    }

    /// Runs the loop with `answers` for the waits (the last one repeats as `HungUp` once
    /// they run out) and `events` ready for crossterm; returns what was sent, the waits'
    /// timeouts and whether the channel is closed.
    fn drive(
        answers: &[StdinState],
        events: Vec<ct::Event>,
    ) -> (Vec<ct::Event>, Vec<Duration>, bool) {
        let answers = RefCell::new(answers.iter().copied().collect::<VecDeque<_>>());
        let waits = RefCell::new(Vec::new());
        let events = RefCell::new(events.into_iter().collect::<VecDeque<_>>());
        let wait = |t: Duration, _: BorrowedFd<'_>| {
            waits.borrow_mut().push(t);
            answers
                .borrow_mut()
                .pop_front()
                .unwrap_or(StdinState::HungUp)
        };
        let (tx, mut rx) = mpsc::unbounded_channel();
        let stop = AtomicBool::new(false);
        let (wake, _tx) = UnixStream::pair().expect("a socket pair");
        wake.set_nonblocking(true).expect("non-blocking");
        run_with(
            &wait,
            &|| {},
            &wake,
            || Ok(events.borrow_mut().pop_front()),
            LateAnswers::new(0, Instant::now()),
            &stop,
            &tx,
        );
        drop(tx);
        let mut sent = Vec::new();
        while let Ok(e) = rx.try_recv() {
            sent.push(e);
        }
        let closed = rx.try_recv() == Err(mpsc::error::TryRecvError::Disconnected);
        (sent, waits.into_inner(), closed)
    }

    #[test]
    fn a_hang_up_at_the_first_check_ends_the_thread_before_crossterm_is_read() {
        let (sent, waits, closed) = drive(&[StdinState::HungUp], vec![key('a')]);
        assert!(sent.is_empty());
        assert_eq!(waits, [Duration::ZERO]);
        assert!(closed);
    }

    /// Every wait says `Ready` until the answers run out; crossterm gives `events` in turn,
    /// `None` for an empty slot. Returns the waits and the re-arms in order.
    fn drive_stalls(answers: usize, events: Vec<Option<ct::Event>>) -> Vec<&'static str> {
        let log = RefCell::new(Vec::new());
        let left = RefCell::new(answers);
        let events = RefCell::new(events.into_iter().collect::<VecDeque<_>>());
        let wait = |t: Duration, _: BorrowedFd<'_>| {
            if t.is_zero() {
                return StdinState::Idle;
            }
            log.borrow_mut().push("wait");
            let mut left = left.borrow_mut();
            if *left == 0 {
                return StdinState::HungUp;
            }
            *left -= 1;
            StdinState::Ready
        };
        let rearm = || log.borrow_mut().push("rearm");
        let (tx, _rx) = mpsc::unbounded_channel();
        let stop = AtomicBool::new(false);
        let (wake, _tx) = UnixStream::pair().expect("a socket pair");
        wake.set_nonblocking(true).expect("non-blocking");
        run_with(
            &wait,
            &rearm,
            &wake,
            || Ok(events.borrow_mut().pop_front().flatten()),
            LateAnswers::new(0, Instant::now()),
            &stop,
            &tx,
        );
        log.into_inner()
    }

    #[test]
    fn input_crossterm_leaves_unread_is_re_armed_once_per_stall() {
        // Ready, nothing read: one re-arm, then (still nothing) the sleep, then the wait.
        assert_eq!(
            drive_stalls(2, Vec::new()),
            ["wait", "rearm", "wait", "wait"]
        );
        // An event after the re-arm starts a new stall, which is re-armed again.
        assert_eq!(
            drive_stalls(2, vec![None, None, Some(key('a'))]),
            ["wait", "rearm", "wait", "rearm", "wait"]
        );
    }

    #[test]
    fn a_poll_that_skipped_an_internal_event_is_asked_a_second_time() {
        let polls = RefCell::new(VecDeque::from([false, true]));
        let got = ready_from(
            || Ok(polls.borrow_mut().pop_front().unwrap_or(false)),
            || Ok(key('a')),
        );
        assert_eq!(got.unwrap(), Some(key('a')));
        // Two polls that find nothing end the drain, without a read.
        let mut count = 0;
        let got = ready_from(
            || {
                count += 1;
                Ok(false)
            },
            || Err(io::Error::other("read")),
        );
        assert_eq!(got.unwrap(), None);
        assert_eq!(count, 2);
    }

    #[test]
    fn a_hang_up_during_the_wait_ends_the_thread() {
        let (sent, waits, closed) = drive(&[StdinState::Idle, StdinState::HungUp], Vec::new());
        assert!(sent.is_empty());
        assert_eq!(waits, [Duration::ZERO, INPUT_IDLE]);
        assert!(closed);
    }

    #[test]
    fn crossterm_is_drained_before_the_wait() {
        let events = vec![key('a'), key('b'), key('c')];
        let (sent, waits, closed) = drive(&[StdinState::Idle, StdinState::HungUp], events.clone());
        assert_eq!(sent, events);
        assert_eq!(waits, [Duration::ZERO, INPUT_IDLE]);
        assert!(closed);
    }

    #[test]
    fn a_ready_wait_goes_straight_to_the_next_round() {
        let (_, waits, _) = drive(
            &[StdinState::Idle, StdinState::Ready, StdinState::HungUp],
            Vec::new(),
        );
        assert_eq!(waits, [Duration::ZERO, INPUT_IDLE, Duration::ZERO]);
    }

    #[test]
    fn an_idle_wait_goes_on_to_the_next_round() {
        let (_, waits, _) = drive(
            &[StdinState::Idle, StdinState::Idle, StdinState::HungUp],
            Vec::new(),
        );
        assert_eq!(waits, [Duration::ZERO, INPUT_IDLE, Duration::ZERO]);
    }

    #[test]
    fn a_ready_wait_that_drains_nothing_does_not_spin() {
        let count = std::sync::atomic::AtomicUsize::new(0);
        let wait = |_: Duration, _: BorrowedFd<'_>| {
            count.fetch_add(1, Ordering::Relaxed);
            StdinState::Ready
        };
        let (wake, _tx) = UnixStream::pair().expect("a socket pair");
        wake.set_nonblocking(true).expect("non-blocking");
        let (tx, _rx) = mpsc::unbounded_channel();
        let stop = AtomicBool::new(false);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                run_with(
                    &wait,
                    &|| {},
                    &wake,
                    || Ok(None),
                    LateAnswers::new(0, Instant::now()),
                    &stop,
                    &tx,
                );
            });
            std::thread::sleep(Duration::from_millis(300));
            stop.store(true, Ordering::Release);
        });
        assert!(count.load(Ordering::Relaxed) < 30);
    }

    #[test]
    fn a_closed_channel_ends_the_thread() {
        let wait = |_: Duration, _: BorrowedFd<'_>| StdinState::Ready;
        let (tx, rx) = mpsc::unbounded_channel();
        drop(rx);
        let events = RefCell::new(VecDeque::from([key('a')]));
        let stop = AtomicBool::new(false);
        let (wake, _tx) = UnixStream::pair().expect("a socket pair");
        wake.set_nonblocking(true).expect("non-blocking");
        run_with(
            &wait,
            &|| {},
            &wake,
            || Ok(events.borrow_mut().pop_front()),
            LateAnswers::new(0, Instant::now()),
            &stop,
            &tx,
        );
    }
}
