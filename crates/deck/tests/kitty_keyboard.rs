//! The kitty keyboard protocol end to end (DECK-22 sub-spec 04): a program in a panel pushes
//! flags and reads its keys raw (a `sh` script with `stty raw` and `dd | od`); the harness's
//! outer terminal answers the client's `CSI ? u` query when `kitty_keyboard()` is on, keeps the
//! flag stack the client pushes, and sends the bytes a terminal with the protocol would.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use support::{Harness, TIMEOUT, wait_until};

const SIZE: (u16, u16) = (100, 24);

fn attached(kitty_keyboard: bool) -> Harness {
    let builder = Harness::builder(SIZE);
    let h = if kitty_keyboard {
        builder.kitty_keyboard().deck()
    } else {
        builder.deck()
    };
    h.wait_for_text("$", TIMEOUT);
    h
}

/// Runs a script in the focused terminal's shell. `REA""DY` keeps the typed command line from
/// looking like the marker the script prints.
fn run(h: &mut Harness, script: &str) {
    h.send_keys(&format!("sh -c '{script}'\r"));
}

/// A script that pushes kitty flags (`push`, an escape sequence body such as `>1u`), prints
/// `READY`, reads `count` bytes of raw input, shows them in hex and pops the flags.
fn reader(push: &str, count: usize) -> String {
    format!(
        "stty raw -echo; printf \"\\033[{push}\"; printf \"REA\"\"DY\"; \
         dd bs=1 count={count} 2>/dev/null | od -An -tx1; printf \"\\033[<u\"; stty sane"
    )
}

/// Starts a reader and waits until the client has pushed `flags` on the outer terminal.
fn start_reader(h: &mut Harness, push: &str, count: usize, flags: u8) {
    run(h, &reader(push, count));
    h.wait_for_text("READY", TIMEOUT);
    if h.keyboard_flags() != flags {
        wait_until("the outer flags", TIMEOUT, || h.keyboard_flags() == flags);
    }
}

/// Waits for `text` on the screen with every run of whitespace (the `od` columns) as one space.
fn wait_squeezed(h: &Harness, text: &str) {
    wait_until(text, TIMEOUT, || {
        let screen = h.screen_text();
        screen
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .contains(text)
    });
}

fn prefixed(h: &mut Harness, key: &str) {
    h.send_bytes(b"\x1b ");
    h.send_keys(key);
}

#[test]
fn shift_enter_reaches_a_program_with_flags_as_an_escape_code_when_the_outer_terminal_can_tell() {
    let mut h = attached(true);
    start_reader(&mut h, ">1u", 7, 1);
    // What a terminal with the protocol sends for Shift+Enter.
    h.send_bytes(b"\x1b[13;2u");
    wait_squeezed(&h, "1b 5b 31 33 3b 32 75");
    // The program popped its flags and so did the client.
    wait_until("the flags to go", TIMEOUT, || h.keyboard_flags() == 0);
    h.capture("kitty-shift-enter");
}

#[test]
fn without_the_protocol_outside_shift_enter_is_what_arrived() {
    let mut h = attached(false);
    run(&mut h, &reader(">1u", 1));
    h.wait_for_text("READY", TIMEOUT);
    // The outer terminal cannot tell Shift+Enter from Enter: it sends `\r`.
    h.send_bytes(b"\r");
    wait_squeezed(&h, "READY 0d");
    // The client pushed nothing: the outer terminal does not speak the protocol.
    let raw = h.raw_output();
    assert!(!raw.windows(5).any(|w| w == b"\x1b[>1u"), "pushed anyway");
    assert_eq!(h.keyboard_flags(), 0);
}

#[test]
fn a_programs_flags_query_is_answered_with_its_flags() {
    let mut h = attached(false);
    // Query, push 1, query, push 3, query, pop, query, pop, query.
    run(
        &mut h,
        "stty raw -echo; printf \"\\033[?u\\033[>1u\\033[?u\\033[>3u\\033[?u\\033[<u\\033[?u\\033[<u\\033[?u\"; \
         dd bs=1 count=25 2>/dev/null | od -An -c; stty sane",
    );
    // The replies are written to the program's input, which `dd` reads: 5 bytes each.
    let want = "033 [ ? 0 u 033 [ ? 1 u 033 [ ? 3 u 033 [ ? 1 u 033 [ ? 0 u";
    wait_squeezed(&h, want);
}

#[test]
fn flags_are_pushed_and_popped_as_focus_and_the_session_move() {
    let mut h = attached(true);
    run(
        &mut h,
        "printf \"\\033[>1u\"; printf \"REA\"\"DY\"; sleep 30",
    );
    h.wait_for_text("READY", TIMEOUT);
    wait_until("the push", TIMEOUT, || h.keyboard_flags() == 1);
    // A new terminal gets the focus: a plain shell, no flags.
    prefixed(&mut h, "t");
    wait_until("the pop", TIMEOUT, || h.keyboard_flags() == 0);
    let raw = h.raw_output();
    assert!(raw.windows(4).any(|w| w == b"\x1b[<u"), "no pop");
    // Back to the program: pushed again.
    prefixed(&mut h, "h");
    wait_until("the push again", TIMEOUT, || h.keyboard_flags() == 1);
    // An overlay takes the keyboard: popped, and pushed again when it closes.
    prefixed(&mut h, "?");
    h.wait_for_text("Help", TIMEOUT);
    wait_until("the pop for the overlay", TIMEOUT, || {
        h.keyboard_flags() == 0
    });
    h.send_keys("\x1b");
    wait_until("the push after the overlay", TIMEOUT, || {
        h.keyboard_flags() == 1
    });
    // Detaching pops them for good, before the alternate screen is left.
    prefixed(&mut h, "d");
    h.wait_for_exit(TIMEOUT);
    assert_eq!(h.keyboard_flags(), 0);
    let raw = h.raw_output();
    let pop = raw.windows(4).rposition(|w| w == b"\x1b[<u").unwrap();
    let leave = raw.windows(8).rposition(|w| w == b"\x1b[?1049l").unwrap();
    assert!(pop < leave, "the flags are popped on the alternate screen");
}

#[test]
fn releases_reach_a_program_only_with_event_types() {
    // Flags 1 and 2: the client asks the outer terminal for both, which reports the release.
    let mut h = attached(true);
    start_reader(&mut h, ">3u", 10, 3);
    h.send_bytes(b"a\x1b[97;1:3u");
    wait_squeezed(&h, "61 1b 5b 39 37 3b 31 3a 33 75");
    h.capture("kitty-release");
    wait_until("the flags to go", TIMEOUT, || h.keyboard_flags() == 0);
    // Flag 1 alone: the outer terminal sends no release, and a stray one never arrives.
    start_reader(&mut h, ">1u", 2, 1);
    h.send_bytes(b"a\x1b[97;1:3ub");
    wait_squeezed(&h, "61 62");
}

#[test]
fn the_prefix_works_in_both_encodings_with_flags_and_twice_it_goes_to_the_program() {
    for prefix in [&b"\x1b[32;3u"[..], b"\x1b "] {
        let mut h = attached(true);
        start_reader(&mut h, ">1u", 7, 1);
        // Once: the prefix, and the key after it is deck's.
        h.send_bytes(prefix);
        h.send_keys("?");
        h.wait_for_text("Help", TIMEOUT);
        h.send_keys("\x1b");
        wait_until("the help to close", TIMEOUT, || {
            !h.screen_text().contains("Help")
        });
        // Twice: the key goes to the program in its encoding.
        h.send_bytes(prefix);
        h.send_bytes(prefix);
        wait_squeezed(&h, "1b 5b 33 32 3b 33 75");
    }
}
