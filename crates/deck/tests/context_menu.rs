//! The context menus, end to end: the real `deck` in a PTY. Right presses are written to the
//! outer terminal as SGR sequences (1-based cells).

#![allow(
    clippy::unwrap_used,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use support::{Harness, TIMEOUT, wait_until};

const SIZE: (u16, u16) = (100, 24);

fn attached() -> Harness {
    let h = Harness::builder(SIZE).config_unset("sidebar").deck();
    h.wait_for_text("│$", TIMEOUT);
    h
}

/// A right click on the 0-based cell (`column`, `row`).
fn right_click(h: &mut Harness, column: u16, row: u16) {
    h.send_keys(&format!("\x1b[<2;{};{}M", column + 1, row + 1));
    h.send_keys(&format!("\x1b[<2;{};{}m", column + 1, row + 1));
}

/// The sidebar rows of panels: the lines of the first column that are not headers.
fn sidebar_rows(h: &Harness) -> Vec<String> {
    h.screen_text()
        .lines()
        .map(|l| l.chars().take(28).collect::<String>())
        .collect()
}

fn panel_rows(h: &Harness) -> usize {
    sidebar_rows(h)
        .iter()
        .skip(3)
        .filter(|r| !r.trim().is_empty() && !r.contains("New") && !r.contains("Resume"))
        .count()
}

#[test]
fn a_space_row_menu_starts_a_panel_through_the_new_menu() {
    let mut h = attached();
    assert_eq!(panel_rows(&h), 1, "{}", h.screen_text());
    // The space row is the third line.
    right_click(&mut h, 10, 2);
    h.wait_for_text("New panel", TIMEOUT);
    h.capture("space-menu");
    h.send_keys("2");
    h.wait_for_text("New in", TIMEOUT);
    h.capture("space-new-menu");
    h.send_keys("1");
    wait_until("a second panel row", TIMEOUT, || panel_rows(&h) == 2);
}

#[test]
fn a_terminal_menu_adds_a_terminal() {
    let mut h = attached();
    right_click(&mut h, 60, 10);
    h.wait_for_text("Close terminal", TIMEOUT);
    h.capture("terminal-menu");
    h.send_keys("1");
    // Two terminals are two bordered boxes side by side.
    wait_until("two terminals", TIMEOUT, || {
        h.screen_text()
            .lines()
            .next()
            .is_some_and(|l| l.matches('╮').count() == 2)
    });
    h.capture("terminal-added");
}

#[test]
fn a_panel_row_menu_closes_the_panel_after_the_question() {
    let mut h = attached();
    // A second panel first, so closing one leaves a space with a panel.
    h.send_bytes(b"\x1b ");
    h.send_keys("c");
    wait_until("two panel rows", TIMEOUT, || panel_rows(&h) == 2);
    right_click(&mut h, 10, 3);
    h.wait_for_text("Rename", TIMEOUT);
    h.send_keys("2");
    h.wait_for_text("Close panel", TIMEOUT);
    h.capture("panel-close-question");
    h.send_keys("y");
    wait_until("one panel row", TIMEOUT, || panel_rows(&h) == 1);
}
