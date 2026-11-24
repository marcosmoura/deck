//! The input thread: it waits for the outer terminal itself and calls crossterm only to
//! drain what is ready, so a terminal that went away ends it instead of spinning in crossterm.

use std::io::{self, Read};
use std::os::fd::BorrowedFd;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use ratatui::crossterm::event as ct;
use tokio::sync::mpsc::UnboundedSender;

use crate::INPUT_POLL;
use crate::colors::LateAnswers;

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

/// One ready event from crossterm, `None` when nothing is ready (it never waits).
fn ready_event() -> io::Result<Option<ct::Event>> {
    if ct::poll(Duration::ZERO)? {
        ct::read().map(Some)
    } else {
        Ok(None)
    }
}

/// The reader thread's body, with the real crossterm. The main loop writes to `wake` on
/// `SIGWINCH`: crossterm also watches that signal, and when it and input are both pending in
/// one of its polls it can drop the input, so the thread is woken to take the signal first.
pub fn run(
    wait: WaitStdin,
    wake: &UnixStream,
    late: LateAnswers,
    stop: &AtomicBool,
    tx: &UnboundedSender<ct::Event>,
) {
    run_with(&*wait, wake, ready_event, late, stop, tx);
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
    wake: &UnixStream,
    mut next: impl FnMut() -> io::Result<Option<ct::Event>>,
    mut late: LateAnswers,
    stop: &AtomicBool,
    tx: &UnboundedSender<ct::Event>,
) {
    // The last wait said `Ready`: if the drain then finds nothing, crossterm used up the
    // readiness without reading the bytes, and trusting the wait again would spin.
    let mut ready_last = false;
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
        if ready_last && !got {
            ready_last = false;
            std::thread::sleep(INPUT_POLL);
            continue;
        }
        let state = wait(INPUT_POLL, fd);
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

    #[test]
    fn a_hang_up_during_the_wait_ends_the_thread() {
        let (sent, waits, closed) = drive(&[StdinState::Idle, StdinState::HungUp], Vec::new());
        assert!(sent.is_empty());
        assert_eq!(waits, [Duration::ZERO, INPUT_POLL]);
        assert!(closed);
    }

    #[test]
    fn crossterm_is_drained_before_the_wait() {
        let events = vec![key('a'), key('b'), key('c')];
        let (sent, waits, closed) = drive(&[StdinState::Idle, StdinState::HungUp], events.clone());
        assert_eq!(sent, events);
        assert_eq!(waits, [Duration::ZERO, INPUT_POLL]);
        assert!(closed);
    }

    #[test]
    fn a_ready_wait_goes_straight_to_the_next_round() {
        let (_, waits, _) = drive(
            &[StdinState::Idle, StdinState::Ready, StdinState::HungUp],
            Vec::new(),
        );
        assert_eq!(waits, [Duration::ZERO, INPUT_POLL, Duration::ZERO]);
    }

    #[test]
    fn an_idle_wait_goes_on_to_the_next_round() {
        let (_, waits, _) = drive(
            &[StdinState::Idle, StdinState::Idle, StdinState::HungUp],
            Vec::new(),
        );
        assert_eq!(waits, [Duration::ZERO, INPUT_POLL, Duration::ZERO]);
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
            &wake,
            || Ok(events.borrow_mut().pop_front()),
            LateAnswers::new(0, Instant::now()),
            &stop,
            &tx,
        );
    }
}
