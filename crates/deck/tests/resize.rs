//! Resizing, end to end: the real `deck` in a PTY. The resize keys and their repeat, the
//! reset, dragging a border with the mouse, and the sizes surviving count and window changes.

#![allow(
    clippy::unwrap_used,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use std::thread::sleep;
use std::time::Duration;

use support::{ACCENT, Harness, TIMEOUT, wait_until};

const SIZE: (u16, u16) = (100, 24);
const LEFT: &str = "\x1b[D";
const RIGHT: &str = "\x1b[C";
const DOWN: &str = "\x1b[B";

fn attached(size: (u16, u16)) -> Harness {
    let h = Harness::deck(size);
    h.wait_for_text("$", TIMEOUT);
    h
}

fn prefixed(h: &mut Harness, key: &str) {
    h.send_bytes(b"\x1b ");
    h.send_keys(key);
}

fn rows(h: &Harness) -> Vec<String> {
    h.screen_text().lines().map(str::to_string).collect()
}

/// The (row, column) of the top-left corner of terminal `n`'s box.
fn corner(h: &Harness, n: usize) -> Option<(usize, usize)> {
    let title = format!("╭─ {n} · sh");
    rows(h)
        .iter()
        .enumerate()
        .find_map(|(y, row)| row.find(&title).map(|i| (y, row[..i].chars().count())))
}

fn wait_corner(h: &Harness, n: usize, at: (usize, usize)) {
    wait_until(&format!("terminal {n} at {at:?}"), TIMEOUT, || {
        corner(h, n) == Some(at)
    });
}

/// The window's last row, where the floating bar shows.
fn status(h: &Harness) -> String {
    h.screen_text()
        .split('\n')
        .next_back()
        .unwrap_or_default()
        .to_string()
}

/// Adds terminals until the panel has `n`.
fn grow(h: &mut Harness, n: usize) {
    for count in 2..=n {
        prefixed(h, "t");
        wait_until(&format!("box {count}"), TIMEOUT, || {
            corner(h, count).is_some()
        });
    }
}

/// Moves focus to terminal `n` of three (the layout of the spec's mockup).
fn focus_of_three(h: &mut Harness, n: usize) {
    let accent = |_: &Harness| ACCENT;
    let focused = |h: &Harness| {
        [(0, 0), (0, 50), (12, 50)]
            .iter()
            .position(|&(r, c)| h.cell(r, c).fg == accent(h) && h.cell(r, c).ch == '╭')
    };
    let moves: &[&str] = match n {
        1 => &[LEFT],
        2 => &[LEFT, RIGHT],
        _ => &[],
    };
    for key in moves {
        prefixed(h, key);
    }
    wait_until("focus", TIMEOUT, || focused(h) == Some(n - 1));
}

/// The `(rows, cols)` of the focused terminal's PTY, as its shell sees them.
fn size_of(h: &mut Harness, tag: &str) -> (u16, u16) {
    h.send_keys(&format!("echo {tag}=$(stty size)\r"));
    let marker = format!("{tag}=");
    let find = |h: &Harness| -> Option<(u16, u16)> {
        h.screen_text().lines().find_map(|line| {
            let rest = line.split(&marker).nth(1)?;
            let mut nums = rest
                .split(|c: char| !c.is_ascii_digit())
                .filter(|s| !s.is_empty());
            let rows = nums.next()?.parse().ok()?;
            let cols = nums.next()?.parse().ok()?;
            (rest.starts_with(|c: char| c.is_ascii_digit())).then_some((rows, cols))
        })
    };
    wait_until(&format!("{tag} size"), TIMEOUT, || find(h).is_some());
    find(h).unwrap()
}

#[test]
fn a_resize_key_grows_the_terminal_and_the_next_one_repeats_without_the_prefix() {
    let mut h = attached(SIZE);
    grow(&mut h, 3);
    focus_of_three(&mut h, 1);
    assert_eq!(corner(&h, 2), Some((0, 50)));

    prefixed(&mut h, "L");
    wait_corner(&h, 2, (0, 55));
    wait_until("the resize badge", TIMEOUT, || {
        let status = status(&h);
        status.contains(" RESIZE ") && status.contains("H J K L resize  = reset")
    });
    assert!(
        status(&h).contains("H J K L resize  = reset"),
        "{}",
        status(&h)
    );
    h.capture("repeating");
    // No prefix: within a second the same key resizes again.
    h.send_keys("L");
    wait_corner(&h, 2, (0, 60));
    // Any other key ends the repeat and goes to the terminal.
    h.send_keys("echo OK\r");
    wait_until("the badge to go", TIMEOUT, || {
        !status(&h).contains("RESIZE")
    });
    h.wait_for_text("│OK", TIMEOUT);
    assert_eq!(corner(&h, 2), Some((0, 60)));
    assert_eq!(
        size_of(&mut h, "R1"),
        (22, 58),
        "60% of 100, less the border"
    );
    h.capture("resized");
}

#[test]
fn the_key_moves_the_boundary_on_the_side_it_points_to() {
    let mut h = attached(SIZE);
    grow(&mut h, 3);
    focus_of_three(&mut h, 2);
    // J on terminal 2 moves the border between 2 and 3 down.
    assert_eq!(corner(&h, 3), Some((12, 50)));
    prefixed(&mut h, "J");
    wait_corner(&h, 3, (13, 50));
    // L on terminal 2: its left edge is the main border, which moves right (2 shrinks).
    prefixed(&mut h, "L");
    wait_corner(&h, 2, (0, 55));
    wait_corner(&h, 3, (13, 55));
    // J on terminal 3: the row border moves down (3 shrinks).
    focus_of_three_by_keys(&mut h);
    prefixed(&mut h, "J");
    wait_corner(&h, 3, (14, 55));
    h.capture("moved");
}

fn focus_of_three_by_keys(h: &mut Harness) {
    prefixed(h, DOWN);
    wait_until("focus on 3", TIMEOUT, || {
        let accent = ACCENT;
        let (r, c) = corner(h, 3).unwrap();
        h.cell(r, c).fg == accent
    });
}

#[test]
fn resizing_stops_at_the_minimum_without_errors_and_keys_never_leak_to_the_terminal() {
    let mut h = attached(SIZE);
    grow(&mut h, 2);
    prefixed(&mut h, LEFT);
    wait_until("focus on 1", TIMEOUT, || {
        let accent = ACCENT;
        h.cell(0, 0).fg == accent
    });
    prefixed(&mut h, "L");
    for _ in 0..14 {
        sleep(Duration::from_millis(60));
        h.send_keys("L");
    }
    // Terminal 2 keeps 12 columns: 10 inside.
    wait_corner(&h, 2, (0, 88));
    sleep(Duration::from_millis(200));
    assert!(!status(&h).contains("layout"), "{}", status(&h));
    h.capture("minimum");
    // Wait out the repeat, then ask the shell: the Ls were all keys, none text.
    wait_until("the repeat to end", TIMEOUT, || {
        !status(&h).contains("RESIZE")
    });
    assert_eq!(size_of(&mut h, "M1"), (22, 86));
    assert!(!h.screen_text().contains("LL"), "{}", h.screen_text());
}

#[test]
fn a_paste_during_the_repeat_ends_it_and_reaches_the_terminal() {
    let mut h = attached(SIZE);
    grow(&mut h, 2);
    prefixed(&mut h, LEFT);
    prefixed(&mut h, "L");
    wait_corner(&h, 2, (0, 55));
    h.send_bytes(b"\x1b[200~PASTED\x1b[201~");
    h.wait_for_text("PASTED", TIMEOUT);
    assert!(!status(&h).contains("RESIZE"), "{}", status(&h));
    // The repeat is over: the key is text now.
    h.send_keys("L");
    h.wait_for_text("PASTEDL", TIMEOUT);
    assert_eq!(corner(&h, 2), Some((0, 55)));
}

#[test]
fn the_reset_restores_the_default_split() {
    let mut h = attached(SIZE);
    grow(&mut h, 3);
    focus_of_three(&mut h, 1);
    prefixed(&mut h, "L");
    wait_corner(&h, 2, (0, 55));
    h.send_keys("L");
    wait_corner(&h, 2, (0, 60));
    prefixed(&mut h, "=");
    wait_corner(&h, 2, (0, 50));
    assert_eq!(size_of(&mut h, "D1"), (22, 48));
    h.capture("reset");
}

#[test]
fn dragging_a_border_moves_it_with_the_pointer_and_the_program_sees_none_of_it() {
    let mut h = attached(SIZE);
    grow(&mut h, 3);
    focus_of_three(&mut h, 1);
    // Terminal 1's program asks for the mouse and waits for one 9-byte click.
    h.send_keys(
        "set +m; printf '\\033[?1000h\\033[?1002h\\033[?1006h'; stty raw -echo; printf 'REA%s\\r\\n' DY; \
         x=$(dd bs=1 count=9 2>/dev/null | od -An -tx1 -v | tr -d ' \\n'); \
         stty sane; printf 'GOT:%s\\r\\n' \"$x\"; sleep 30\r",
    );
    h.wait_for_text("READY", TIMEOUT);

    // Press on terminal 1's right border column (1-based 50), then move and release.
    h.send_keys("\x1b[<0;50;6M");
    sleep(Duration::from_millis(50));
    h.send_keys("\x1b[<32;60;6M");
    wait_corner(&h, 2, (0, 60));
    sleep(Duration::from_millis(50));
    h.send_keys("\x1b[<32;70;6M");
    wait_corner(&h, 2, (0, 70));
    h.capture("dragging");
    h.send_keys("\x1b[<0;70;6m");
    sleep(Duration::from_millis(200));
    assert_eq!(corner(&h, 2), Some((0, 70)));
    assert!(!h.screen_text().contains("GOT:1b"), "{}", h.screen_text());

    // The program does get a click inside its terminal: cell (5, 3).
    h.send_keys("\x1b[<0;6;4M");
    h.wait_for_text("GOT:1b5b3c303b353b334d", TIMEOUT);
}

#[test]
fn the_border_in_motion_is_drawn_in_the_accent() {
    let mut h = attached(SIZE);
    grow(&mut h, 3);
    focus_of_three(&mut h, 2);
    let accent = ACCENT;
    // Terminal 1 is not focused: its border is not the accent until it is dragged.
    assert_ne!(h.cell(5, 49).fg, accent);
    h.send_keys("\x1b[<0;50;6M");
    wait_until("the accent border", TIMEOUT, || h.cell(5, 49).fg == accent);
    h.send_keys("\x1b[<0;50;6m");
    wait_until("the border back", TIMEOUT, || h.cell(5, 49).fg != accent);
}

#[test]
fn custom_sizes_survive_adding_and_closing_terminals() {
    let mut h = attached(SIZE);
    grow(&mut h, 2);
    prefixed(&mut h, LEFT);
    prefixed(&mut h, "L");
    wait_corner(&h, 2, (0, 55));
    h.send_keys("L");
    wait_corner(&h, 2, (0, 60));
    // A third terminal: the big one keeps 60%.
    prefixed(&mut h, "t");
    wait_until("three boxes", TIMEOUT, || corner(&h, 3).is_some());
    assert_eq!(corner(&h, 2), Some((0, 60)));
    h.capture("added");
    // Closing it returns to 60/40.
    prefixed(&mut h, "x");
    h.wait_for_text("Close terminal 3?", TIMEOUT);
    h.send_keys("y");
    wait_until("two boxes", TIMEOUT, || corner(&h, 3).is_none());
    wait_corner(&h, 2, (0, 60));
}

#[test]
fn default_sizes_follow_the_shape_rule_with_no_resize_at_all() {
    let mut h = attached(SIZE);
    grow(&mut h, 5);
    assert_eq!(
        corner(&h, 2),
        Some((0, 50)),
        "terminal 1 has half the width"
    );
    assert_eq!(corner(&h, 3), Some((0, 75)));
}

#[test]
fn a_narrow_window_draws_a_clamped_copy_and_the_exact_sizes_come_back() {
    let mut h = attached((200, 24));
    grow(&mut h, 2);
    prefixed(&mut h, LEFT);
    prefixed(&mut h, "L");
    wait_corner(&h, 2, (0, 110));
    for at in [120, 130, 140, 150, 160, 170, 180] {
        sleep(Duration::from_millis(60));
        h.send_keys("L");
        wait_corner(&h, 2, (0, at));
    }
    // 90/10: terminal 2 is 20 columns, 18 inside.
    wait_corner(&h, 2, (0, 180));
    wait_until("the repeat to end", TIMEOUT, || {
        !status(&h).contains("RESIZE")
    });
    prefixed(&mut h, RIGHT);
    wait_until("focus on 2", TIMEOUT, || {
        let accent = ACCENT;
        h.cell(0, 180).fg == accent
    });
    assert_eq!(size_of(&mut h, "N1"), (22, 18));

    h.resize(40, 24);
    wait_until("the narrow layout", TIMEOUT, || {
        corner(&h, 2).is_some_and(|(_, c)| c < 40)
    });
    let (_, narrow) = corner(&h, 2).unwrap();
    assert!(40 - narrow - 2 >= 10, "terminal 2 keeps its 10 columns");
    h.capture("narrow");
    let (_, cols) = size_of(&mut h, "N2");
    assert!(cols >= 10, "{cols}");

    h.resize(200, 24);
    wait_corner(&h, 2, (0, 180));
    assert_eq!(size_of(&mut h, "N3"), (22, 18));
}
