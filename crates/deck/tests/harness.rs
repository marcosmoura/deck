//! The end-to-end harness on its own, with `/bin/sh`.

mod support;

use std::time::Duration;

use alacritty_terminal::vte::ansi::{Color, NamedColor};
use support::{Harness, TIMEOUT, TtyMode, wait_until};

const SIZE: (u16, u16) = (80, 24);

#[test]
fn a_shell_shows_its_prompt() {
    let h = Harness::spawn(SIZE, &["/bin/sh"]);
    h.wait_for_text("$", TIMEOUT);
    assert!(h.screen_text().starts_with("$"), "{}", h.screen_text());
}

#[test]
fn colours_and_styles_are_in_the_cells_and_the_raw_bytes() {
    let mut h = Harness::spawn(SIZE, &["/bin/sh"]);
    h.wait_for_text("$", TIMEOUT);
    h.send_keys("printf '\\033[1;31mred\\033[0m'\r");
    h.wait_for_text("red$", TIMEOUT);

    let row = h
        .screen_text()
        .lines()
        .position(|l| l.starts_with("red"))
        .expect("a row with the output");
    let r = h.cell(row, 0);
    assert_eq!(r.ch, 'r');
    assert_eq!(r.fg, Color::Named(NamedColor::Red));
    assert!(r.bold());
    let after = h.cell(row, 3);
    assert!(!after.bold());

    let raw = h.raw_output();
    let needle = b"\x1b[1;31mred";
    assert!(raw.windows(needle.len()).any(|w| w == needle));
}

#[test]
fn the_tty_mode_follows_stty() {
    // An interactive shell switches its terminal itself, so use a script that leaves it
    // alone until told.
    let mut h = Harness::spawn(SIZE, &["/bin/sh", "-c", "read go; stty raw; read more"]);
    assert_eq!(h.tty_mode(), TtyMode::Cooked);
    h.send_keys("go\n");
    wait_until("raw mode", TIMEOUT, || h.tty_mode() == TtyMode::Raw);
}

#[test]
fn a_resize_reaches_the_program() {
    let mut h = Harness::spawn(SIZE, &["/bin/sh"]);
    h.wait_for_text("$", TIMEOUT);
    h.send_keys("stty size\r");
    h.wait_for_text("24 80", TIMEOUT);
    h.resize(100, 30);
    h.send_keys("stty size\r");
    h.wait_for_text("30 100", TIMEOUT);
}

#[test]
fn exit_is_reported_and_captures_are_written() {
    let mut h = Harness::spawn(SIZE, &["/bin/sh"]);
    h.wait_for_text("$", TIMEOUT);
    let path = h.capture("prompt");
    assert!(std::fs::read_to_string(path).unwrap().starts_with("$"));
    h.send_keys("exit 3\r");
    let status = h.wait_for_exit(Duration::from_secs(5));
    assert_eq!(status.exit_code(), 3);
}
