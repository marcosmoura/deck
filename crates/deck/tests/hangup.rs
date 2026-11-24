//! A client whose terminal goes away ends, and the session stays.

mod support;

use std::time::{Duration, Instant};

use support::{Harness, TIMEOUT};

const SIZE: (u16, u16) = (80, 24);

/// `deck` attached with `env` set, its shell showing the first prompt.
fn attached(env: &[(&str, &str)]) -> Harness {
    let mut builder = Harness::builder(SIZE);
    for (key, value) in env {
        builder = builder.env(key, value);
    }
    let h = builder.deck();
    h.wait_for_text("$", TIMEOUT);
    h
}

/// Closes the terminal, waits for the client to end and checks that the daemon still lists
/// the session.
fn close_and_check(mut h: Harness) {
    h.close_master();
    let started = Instant::now();
    h.wait_for_exit(TIMEOUT);
    assert!(started.elapsed() < TIMEOUT);
    let ls = h.run_deck(&["ls"]);
    let text = String::from_utf8_lossy(&ls.stdout).into_owned();
    assert!(text.contains("main"), "deck ls says: {text}");
}

#[test]
fn closing_the_terminal_ends_the_client_and_keeps_the_session() {
    let mut h = attached(&[("DECK_TEST_IGNORE_SIGHUP", "1")]);
    h.send_keys("while true; do echo tick; sleep 0.02; done\r");
    h.wait_for_text("tick", TIMEOUT);
    close_and_check(h);
}

#[test]
fn closing_the_terminal_of_an_idle_session_ends_the_client() {
    let h = attached(&[("DECK_TEST_IGNORE_SIGHUP", "1")]);
    close_and_check(h);
}

#[test]
fn closing_the_terminal_ends_the_client_through_sighup() {
    let mut h = attached(&[]);
    h.send_keys("while true; do echo tick; sleep 0.02; done\r");
    h.wait_for_text("tick", TIMEOUT);
    close_and_check(h);
}

#[test]
fn sigterm_ends_a_client_whose_reader_is_stuck() {
    let mut h = attached(&[("DECK_TEST_STUCK_READER", "1")]);
    h.terminate();
    h.wait_for_exit(TIMEOUT);
}

#[test]
fn typed_ahead_keys_arrive_together() {
    let mut h = attached(&[]);
    let word = "k".repeat(40);
    h.send_bytes(format!("echo {word}\r").as_bytes());
    h.wait_for_row_start(&word, Duration::from_secs(3));
}
