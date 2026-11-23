//! Several terminals in a panel, end to end: the real `deck` in a PTY. Each terminal's shell
//! reports its size with `stty size`; the screen shows the grid.

#![allow(
    clippy::unwrap_used,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use support::{ACCENT, Harness, TIMEOUT, wait_until};

/// The window of the spec's mockup: a 100×24 panel area, every row of the window.
const SIZE: (u16, u16) = (100, 24);

fn attached(size: (u16, u16)) -> Harness {
    let h = Harness::deck(size);
    h.wait_for_text("$", TIMEOUT);
    h
}

fn prefixed(h: &mut Harness, key: &str) {
    h.send_bytes(b"\x1b ");
    h.send_keys(key);
}

const LEFT: &str = "\x1b[D";
const RIGHT: &str = "\x1b[C";
const UP: &str = "\x1b[A";
const DOWN: &str = "\x1b[B";

fn rows(h: &Harness) -> Vec<String> {
    h.screen_text().lines().map(str::to_string).collect()
}

/// Adds terminals until the panel has `n`, waiting for the border of the last one.
fn grow(h: &mut Harness, n: usize) {
    for count in 2..=n {
        prefixed(h, "t");
        let title = format!("╭─ {count} · ");
        wait_until(&title, TIMEOUT, || h.screen_text().contains(&title));
    }
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
            // The echoed command line has `$(stty` after the marker, not numbers.
            (rest.starts_with(|c: char| c.is_ascii_digit())).then_some((rows, cols))
        })
    };
    wait_until(&format!("{tag} size"), TIMEOUT, || find(h).is_some());
    find(h).unwrap()
}

/// The focused terminal, found by its border's colour: the theme's accent. `corners` are the (row, column) of the top-left corners of the boxes.
fn focused_box(h: &Harness, corners: &[(usize, usize)]) -> Option<usize> {
    let accent = ACCENT;
    corners
        .iter()
        .position(|&(r, c)| h.cell(r, c).fg == accent && h.cell(r, c).ch == '╭')
}

/// The prefix and a focus key, then waits until the box at `corners[to]` is the focused one.
fn focus(h: &mut Harness, key: &str, corners: &[(usize, usize)], to: usize) {
    prefixed(h, key);
    wait_until("focus to move", TIMEOUT, || {
        focused_box(h, corners) == Some(to)
    });
}

/// The (row, column) of `text` on the screen, in cells.
fn find(h: &Harness, text: &str) -> Option<(usize, usize)> {
    rows(h)
        .iter()
        .enumerate()
        .find_map(|(y, row)| row.find(text).map(|i| (y, row[..i].chars().count())))
}

#[test]
fn three_four_and_five_terminals_get_the_grid_and_their_own_pty_sizes() {
    let mut h = attached(SIZE);
    assert_eq!(
        size_of(&mut h, "S0"),
        (24, 100),
        "one terminal: the panel area"
    );

    prefixed(&mut h, "t");
    prefixed(&mut h, "t");
    wait_until("three boxes", TIMEOUT, || {
        h.screen_text().contains("╭─ 3 · sh")
    });
    let screen = rows(&h);
    assert!(screen[0].starts_with("╭─ 1 · sh ─"), "{}", screen[0]);
    assert!(screen[0].contains("╮╭─ 2 · sh ─"), "{}", screen[0]);
    assert_eq!(screen[0].chars().count(), 100);
    // Terminal 3 starts under terminal 2 on the right half.
    assert_eq!(find(&h, "╭─ 3 · sh"), Some((12, 50)));

    // The new terminal has focus: its PTY is the lower right box, 12 rows less the border.
    assert_eq!(size_of(&mut h, "S3"), (10, 48));
    h.capture("three");
    let three = [(0, 0), (0, 50), (12, 50)];
    focus(&mut h, LEFT, &three, 0);
    assert_eq!(size_of(&mut h, "S1"), (22, 48));
    focus(&mut h, RIGHT, &three, 1);
    assert_eq!(size_of(&mut h, "S2"), (10, 48));

    // A fourth gives the 2x2 grid.
    prefixed(&mut h, "t");
    wait_until("four boxes", TIMEOUT, || {
        h.screen_text().contains("╭─ 4 · sh")
    });
    assert_eq!(find(&h, "╭─ 4 · sh"), Some((12, 50)));
    assert_eq!(find(&h, "╭─ 3 · sh"), Some((12, 0)));
    assert_eq!(size_of(&mut h, "S4"), (10, 48));
    h.capture("four");

    // A fifth: terminal 1 at full height, then a 2x2 grid of half widths.
    prefixed(&mut h, "t");
    wait_until("five boxes", TIMEOUT, || {
        h.screen_text().contains("╭─ 5 · sh")
    });
    assert_eq!(find(&h, "╭─ 2 · sh"), Some((0, 50)));
    assert_eq!(find(&h, "╭─ 3 · sh"), Some((0, 75)));
    assert_eq!(find(&h, "╭─ 4 · sh"), Some((12, 50)));
    assert_eq!(find(&h, "╭─ 5 · sh"), Some((12, 75)));
    assert_eq!(size_of(&mut h, "S5"), (10, 23));
    h.capture("five");
    let five = [(0, 0), (0, 50), (0, 75), (12, 50), (12, 75)];
    focus(&mut h, LEFT, &five, 3);
    focus(&mut h, LEFT, &five, 0);
    assert_eq!(size_of(&mut h, "T1"), (22, 48));
}

#[test]
fn focus_keys_follow_the_layout_and_typing_reaches_only_the_focused_terminal() {
    let mut h = attached(SIZE);
    grow(&mut h, 3);
    let boxes = [(0, 0), (0, 50), (12, 50)];
    let say = |h: &mut Harness, word: &str| {
        // The shell prints the word; the echoed command line has quotes inside it.
        let (a, b) = word.split_at(2);
        h.send_keys(&format!("echo {a}''{b}\r"));
        wait_until(word, TIMEOUT, || {
            rows(h).iter().any(|r| r.contains(&format!("│{word}")))
        });
        find_output(h, word)
    };
    fn find_output(h: &Harness, word: &str) -> (usize, usize) {
        find(h, &format!("│{word}")).expect("the word on the screen")
    }

    assert_eq!(focused_box(&h, &boxes), Some(2), "the new terminal");
    // From the lower right: Left goes to the big terminal, Right returns to terminal 2.
    for (key, to, word, origin) in [
        (LEFT, 0, "AAAA", (1, 0)),
        (RIGHT, 1, "BBBB", (1, 50)),
        (DOWN, 2, "CCCC", (13, 50)),
        (UP, 1, "DDDD", (1, 50)),
    ] {
        prefixed(&mut h, key);
        wait_until("focus to move", TIMEOUT, || {
            focused_box(&h, &boxes) == Some(to)
        });
        let (row, col) = say(&mut h, word);
        // The word is on a row inside the focused box, never in another one.
        assert!(
            col >= origin.1 && col < origin.1 + 50,
            "{word} at {row},{col}"
        );
        assert!(row >= origin.0, "{word} at {row},{col}");
        // And only once: the other terminals did not get the keys.
        assert_eq!(
            rows(&h)
                .iter()
                .filter(|r| r.contains(&format!("│{word}")))
                .count(),
            1
        );
    }
    // The letters work as well, and nothing moves past the edge.
    for (key, to) in [
        ("j", 2),
        ("j", 2),
        ("k", 1),
        ("k", 1),
        ("h", 0),
        ("h", 0),
        ("l", 1),
    ] {
        prefixed(&mut h, key);
        wait_until("focus to move", TIMEOUT, || {
            focused_box(&h, &boxes) == Some(to)
        });
    }
    h.capture("focus");
}

#[test]
fn closing_a_terminal_asks_then_reflows_and_focus_follows() {
    let mut h = attached(SIZE);
    grow(&mut h, 3);
    prefixed(&mut h, "x");
    h.wait_for_text("Close terminal 3?", TIMEOUT);
    h.capture("confirm");
    // `n` keeps it.
    h.send_keys("n");
    wait_until("the question to go", TIMEOUT, || {
        !h.screen_text().contains("Close terminal")
    });
    assert!(h.screen_text().contains("╭─ 3 · sh"));

    prefixed(&mut h, "x");
    h.wait_for_text("Close terminal 3?", TIMEOUT);
    h.send_keys("y");
    wait_until("two boxes", TIMEOUT, || {
        let text = h.screen_text();
        !text.contains("Close terminal") && !text.contains("╭─ 3 · sh")
    });
    assert_eq!(find(&h, "╭─ 2 · sh"), Some((0, 50)));
    // Terminal 2 took focus and grew to the full height of the right half.
    assert_eq!(size_of(&mut h, "C2"), (22, 48));
    h.capture("closed");
    focus(&mut h, LEFT, &[(0, 0), (0, 50)], 0);
    assert_eq!(size_of(&mut h, "C1"), (22, 48));
}

#[test]
fn exit_in_one_of_three_reflows_and_one_of_two_leaves_a_single_borderless_terminal() {
    let mut h = attached(SIZE);
    grow(&mut h, 3);
    h.send_keys("exit\r");
    wait_until("two boxes", TIMEOUT, || {
        !h.screen_text().contains("╭─ 3 · sh")
    });
    assert_eq!(find(&h, "╭─ 2 · sh"), Some((0, 50)));
    h.capture("two");
    assert_eq!(size_of(&mut h, "E2"), (22, 48));

    prefixed(&mut h, "x");
    h.wait_for_text("Close terminal 2?", TIMEOUT);
    h.send_keys("y");
    wait_until("no borders", TIMEOUT, || !h.screen_text().contains('╭'));
    assert!(!h.screen_text().contains('│'), "{}", h.screen_text());
    assert_eq!(size_of(&mut h, "E1"), (24, 100));
    h.capture("single");
}

#[test]
fn a_window_without_room_refuses_the_fifth_terminal() {
    // 40x12: the panel area is 40x11, and five terminals would leave the right-hand grid
    // 10 columns per terminal (8 inside).
    let mut h = attached((40, 12));
    grow(&mut h, 4);
    prefixed(&mut h, "t");
    h.wait_for_text("not enough room for another terminal", TIMEOUT);
    h.capture("refused");
    assert!(!h.screen_text().contains("╭─ 5 · "), "{}", h.screen_text());
    assert_eq!(rows(&h).len(), 12);
}

#[test]
fn the_wheel_scrolls_an_unfocused_terminal_without_moving_focus() {
    let mut h = attached(SIZE);
    grow(&mut h, 2);
    let boxes = [(0, 0), (0, 50)];
    prefixed(&mut h, LEFT);
    wait_until("focus on 1", TIMEOUT, || focused_box(&h, &boxes) == Some(0));
    h.send_keys("seq 1 500\r");
    wait_until("500", TIMEOUT, || {
        rows(&h).iter().any(|r| r.starts_with("│500"))
    });
    prefixed(&mut h, RIGHT);
    wait_until("focus on 2", TIMEOUT, || focused_box(&h, &boxes) == Some(1));

    // A wheel notch over terminal 1 (cell 10,5, 1-based).
    h.send_keys("\x1b[<64;10;5M");
    wait_until("terminal 1 to scroll back", TIMEOUT, || {
        !rows(&h).iter().any(|r| r.starts_with("│500"))
    });
    assert_eq!(focused_box(&h, &boxes), Some(1), "focus did not move");
    let bar = rows(&h).pop().unwrap_or_default();
    assert!(!bar.contains("SCROLL"), "{bar}");
    h.capture("wheel");
}

/// Runs a shell snippet that prints `READY` once it waits for `count` bytes, then prints
/// them in hex after `GOT:`.
fn start_reader(h: &mut Harness, setup: &str, count: usize) {
    h.send_keys(&format!(
        "set +m; {setup}; stty raw -echo; printf 'REA%s\\r\\n' DY; \
         x=$(dd bs=1 count={count} 2>/dev/null | od -An -tx1 -v | tr -d ' \\n'); \
         stty sane; printf 'GOT:%s\\r\\n' \"$x\"; sleep 30\r"
    ));
    h.wait_for_text("READY", TIMEOUT);
}

#[test]
fn a_click_focuses_an_unfocused_terminal_and_only_the_next_one_reaches_the_program() {
    let mut h = attached(SIZE);
    grow(&mut h, 2);
    let boxes = [(0, 0), (0, 50)];
    prefixed(&mut h, LEFT);
    wait_until("focus on 1", TIMEOUT, || focused_box(&h, &boxes) == Some(0));
    start_reader(&mut h, "printf '\\033[?1000h\\033[?1006h'", 9);
    prefixed(&mut h, RIGHT);
    wait_until("focus on 2", TIMEOUT, || focused_box(&h, &boxes) == Some(1));

    // Press and release at (6, 4) of the window, inside terminal 1.
    h.send_keys("\x1b[<0;6;4M");
    h.send_keys("\x1b[<0;6;4m");
    wait_until("focus on 1", TIMEOUT, || focused_box(&h, &boxes) == Some(0));
    h.capture("clicked");
    assert!(
        !h.screen_text().contains("GOT:1b"),
        "the first click is not forwarded"
    );

    // The second click is the program's: terminal 1's screen starts at (2, 2), so the
    // cell is (5, 3) of it.
    h.send_keys("\x1b[<0;6;4M");
    h.wait_for_text("GOT:1b5b3c303b353b334d", TIMEOUT);
}

#[test]
fn a_smaller_client_sees_the_same_focus_and_the_layout_cropped() {
    let mut big = attached(SIZE);
    grow(&mut big, 3);
    let boxes = [(0, 0), (0, 50), (12, 50)];
    prefixed(&mut big, LEFT);
    prefixed(&mut big, RIGHT);
    wait_until("focus on 2", TIMEOUT, || {
        focused_box(&big, &boxes) == Some(1)
    });

    let path = big.home().path().to_string_lossy().into_owned();
    let small = Harness::builder((70, 12)).env("DECK_HOME", &path).deck();
    wait_until("the grid in the small window", TIMEOUT, || {
        small.screen_text().contains("╭─ 2 · sh")
    });
    // The newest client sizes the terminals; the big one types, so they are sized for it
    // again, and the small window shows that layout cropped.
    let mut big = big;
    assert_eq!(size_of(&mut big, "B2"), (10, 48));
    wait_until("the cropped layout", TIMEOUT, || {
        find(&small, "╭─ 2 · sh") == Some((0, 50))
    });
    small.capture("cropped");
    // The same focus, the daemon's layout: terminal 2 starts at column 50, cut at 70.
    assert_eq!(focused_box(&small, &boxes[..2]), Some(1));
    let top = rows(&small);
    assert_eq!(top.len(), 12);
    assert!(top[0].starts_with("╭─ 1 · sh"), "{}", top[0]);
    assert_eq!(find(&small, "╭─ 2 · sh"), Some((0, 50)));
}
