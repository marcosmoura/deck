//! Copy mode end to end: the real `deck` in a PTY. The copies are read from the OSC 52
//! sequences the client writes to its outer terminal.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use serde_json::Value;
use support::{Color, Harness, TIMEOUT, stderr, stdout, wait_until};

const SIZE: (u16, u16) = (80, 24);
const PAGE_UP: &str = "\x1b[5~";

fn prefixed(h: &mut Harness, key: &str) {
    h.send_bytes(b"\x1b ");
    h.send_keys(key);
}

fn rows(h: &Harness) -> Vec<String> {
    h.screen_text().split('\n').map(str::to_string).collect()
}

fn status(h: &Harness) -> String {
    rows(h).pop().unwrap_or_default()
}

/// `(offset, history)` of the `[offset/history]` in the floating bar.
fn indicator(h: &Harness) -> Option<(u32, u32)> {
    let bar = status(h);
    let inner = &bar[bar.find('[')? + 1..];
    let (offset, rest) = inner.split_once('/')?;
    let (history, _) = rest.split_once(']')?;
    Some((offset.parse().ok()?, history.parse().ok()?))
}

/// `deck` after `seq 1 500`, its prompt on the last row.
fn with_history() -> Harness {
    let mut h = Harness::deck(SIZE);
    h.wait_for_text("$", TIMEOUT);
    h.send_keys("seq 1 500\r");
    h.wait_for_row_start("500", TIMEOUT);
    wait_until("the prompt below 500", TIMEOUT, || {
        rows(&h).iter().any(|r| r.starts_with('$'))
            && rows(&h).iter().position(|r| r.starts_with("500"))
                < rows(&h).iter().rposition(|r| r.starts_with('$'))
    });
    h
}

fn enter_scroll_mode(h: &mut Harness) {
    prefixed(h, "[");
    h.wait_for_text("SCROLL", TIMEOUT);
}

fn base64_decode(text: &str) -> String {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for c in text.bytes().filter(|c| *c != b'=') {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            _ => 63,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((acc >> bits) & 0xFF).unwrap());
        }
    }
    String::from_utf8(out).unwrap()
}

/// The texts of the OSC 52 copies in the client's raw output, oldest first.
fn copies(h: &Harness) -> Vec<String> {
    let raw = String::from_utf8_lossy(&h.raw_output()).into_owned();
    let mut found = Vec::new();
    let mut rest = raw.as_str();
    while let Some(at) = rest.find("\x1b]52;c;") {
        rest = &rest[at + 7..];
        let end = rest.find('\x07').unwrap_or(rest.len());
        found.push(base64_decode(&rest[..end]));
        rest = &rest[end..];
    }
    found
}

fn wait_for_copy(h: &Harness, count: usize) -> String {
    wait_until("the OSC 52 copy", TIMEOUT, || copies(h).len() >= count);
    copies(h)[count - 1].clone()
}

/// The history of the first terminal, `deck capture --history`, one string per row.
fn history(h: &Harness) -> Vec<String> {
    let home = h.home();
    let out = home.run_deck(&["ls", "--json"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let tree: Value = serde_json::from_str(&stdout(&out)).unwrap();
    let id = tree["sessions"][0]["spaces"][0]["panels"][0]["terminals"][0]["id"]
        .as_u64()
        .unwrap()
        .to_string();
    let out = home.run_deck(&["capture", "--terminal", &id, "--history"]);
    assert!(out.status.success(), "{}", stderr(&out));
    stdout(&out).lines().map(str::to_string).collect()
}

/// A mouse event at the 1-based window cell, as an SGR sequence.
fn mouse(h: &mut Harness, code: u8, col: u16, row: u16, release: bool) {
    let end = if release { 'm' } else { 'M' };
    h.send_keys(&format!("\x1b[<{code};{col};{row}{end}"));
}

/// The size of the selection the bar shows while selecting (`3 lines`), in rows.
fn selected_lines(h: &Harness) -> Option<u32> {
    let bar = status(h);
    let at = bar.find("SELECT")?;
    let rest = bar[at + 6..].trim_start();
    rest.split_once(" line")?.0.parse().ok()
}

fn wheel(h: &mut Harness, up: bool) {
    mouse(h, if up { 64 } else { 65 }, 10, 5, false);
}

#[test]
fn v_then_thirty_k_past_the_top_copies_thirty_one_lines() {
    let mut h = with_history();
    enter_scroll_mode(&mut h);
    h.capture("scroll-mode-hints");
    assert!(
        status(&h).contains("q exit  v select  / search  y copy"),
        "{}",
        status(&h)
    );
    // From the start of the prompt row up 30 rows, scrolling past the top of the screen.
    h.send_keys("0v");
    h.send_keys(&"k".repeat(30));
    wait_until("31 lines", TIMEOUT, || selected_lines(&h) == Some(31));
    h.capture("selecting");
    assert!(
        status(&h).contains("y copy  o other end  Esc clear"),
        "{}",
        status(&h)
    );
    h.send_keys("y");
    let text = wait_for_copy(&h, 1);
    let lines: Vec<&str> = text.split('\n').collect();
    assert_eq!(lines.len(), 31, "{text}");
    let all = history(&h);
    let end = all.len();
    // The same rows as `capture --history`; the last one is cut at the cursor's column 0.
    assert_eq!(&lines[..30], &all[end - 31..end - 1], "{text}");
    assert_eq!(lines[30], &all[end - 1][..1]);
    assert_eq!(lines[0], "471", "{text}");
    // The selection is cleared and scroll mode stays.
    wait_until("the selection to clear", TIMEOUT, || {
        !status(&h).contains("SELECT")
    });
    assert!(status(&h).contains("SCROLL"), "{}", status(&h));
}

#[test]
fn capital_v_then_g_copies_to_the_bottom_and_o_then_k_grows_the_other_end() {
    let mut h = with_history();
    enter_scroll_mode(&mut h);
    h.send_keys(PAGE_UP);
    wait_until("a page back", TIMEOUT, || {
        indicator(&h).is_some_and(|(o, _)| o == 24)
    });
    h.send_keys("HVG");
    wait_until("48 lines", TIMEOUT, || selected_lines(&h) == Some(48));
    h.send_keys("y");
    let text = wait_for_copy(&h, 1);
    let all = history(&h);
    let end = all.len();
    // One page back, the screen's top row is 47 rows above the last one.
    let want = &all[end - 48..end];
    assert_eq!(text.split('\n').collect::<Vec<_>>(), want, "{text}");
    wait_until("the selection to clear", TIMEOUT, || {
        !status(&h).contains("SELECT")
    });

    // `o` goes to the other end (off screen: the view follows), `k` grows it by a row.
    h.send_keys("HVGok");
    wait_until("25 lines", TIMEOUT, || selected_lines(&h) == Some(25));
    h.capture("other-end-grown");
    h.send_keys("y");
    let text = wait_for_copy(&h, 2);
    let all = history(&h);
    let end = all.len();
    assert_eq!(text.split('\n').collect::<Vec<_>>(), &all[end - 25..end]);
}

#[test]
fn y_without_a_selection_copies_the_line_and_escape_clears_then_leaves() {
    let mut h = with_history();
    enter_scroll_mode(&mut h);
    h.send_keys("ky");
    assert_eq!(wait_for_copy(&h, 1), "500");
    // Esc clears a selection and stays; the second one leaves.
    h.send_keys("v");
    wait_until("the badge", TIMEOUT, || status(&h).contains("SELECT"));
    h.send_keys("\x1b");
    wait_until("the selection to clear", TIMEOUT, || {
        status(&h).contains("SCROLL") && !status(&h).contains("SELECT")
    });
    h.send_keys("\x1b");
    wait_until("scroll mode to end", TIMEOUT, || {
        !status(&h).contains("SCROLL")
    });
}

#[test]
fn k_at_the_top_edge_scrolls_exactly_one_line() {
    let mut h = with_history();
    enter_scroll_mode(&mut h);
    h.send_keys(PAGE_UP);
    wait_until("a page back", TIMEOUT, || {
        indicator(&h).is_some_and(|(o, _)| o == 24)
    });
    // Inside the screen the cursor moves and the view stays.
    h.send_keys("Hjkv");
    wait_until("the selection", TIMEOUT, || selected_lines(&h) == Some(1));
    h.send_keys("v");
    wait_until("no selection", TIMEOUT, || indicator(&h).is_some());
    assert_eq!(indicator(&h).map(|i| i.0), Some(24), "{}", status(&h));
    h.send_keys("k");
    wait_until("one line", TIMEOUT, || {
        indicator(&h).is_some_and(|(o, _)| o == 25)
    });
    h.send_keys("k");
    wait_until("one more line", TIMEOUT, || {
        indicator(&h).is_some_and(|(o, _)| o == 26)
    });
}

#[test]
fn a_mouse_selection_survives_scrolling_away_and_back_and_copies_the_same_text() {
    let mut h = with_history();
    enter_scroll_mode(&mut h);
    // Rows 5 to 7 of the screen, by the mouse (1-based cells).
    mouse(&mut h, 0, 1, 5, false);
    mouse(&mut h, 32, 3, 7, false);
    mouse(&mut h, 0, 3, 7, true);
    let first = wait_for_copy(&h, 1);
    assert_eq!(first.split('\n').count(), 3, "{first}");
    let inverse = |h: &Harness, row: usize| h.cell(row, 0).inverse();
    wait_until("the selection to be drawn", TIMEOUT, || {
        inverse(&h, 4) && inverse(&h, 5) && inverse(&h, 6)
    });
    // Three notches back: the rows move down nine lines with the text.
    for _ in 0..3 {
        wheel(&mut h, true);
    }
    wait_until("the view to move", TIMEOUT, || inverse(&h, 13));
    assert!(inverse(&h, 13) && inverse(&h, 14) && inverse(&h, 15));
    assert!(!inverse(&h, 4));
    h.capture("selection-scrolled-away");
    for _ in 0..3 {
        wheel(&mut h, false);
    }
    wait_until("the rows to return", TIMEOUT, || inverse(&h, 4));
    assert!(inverse(&h, 4) && inverse(&h, 6));
    assert!(!inverse(&h, 13));
    h.send_keys("y");
    assert_eq!(wait_for_copy(&h, 2), first);
}

#[test]
fn dragging_past_the_top_edge_scrolls_and_extends_the_selection() {
    let mut h = with_history();
    enter_scroll_mode(&mut h);
    mouse(&mut h, 0, 1, 10, false);
    // The pointer on the screen's top row, held there while the view scrolls.
    mouse(&mut h, 32, 1, 1, false);
    wait_until("the view to scroll", TIMEOUT, || {
        selected_lines(&h).is_some_and(|n| n >= 30)
    });
    h.capture("drag-past-the-top");
    mouse(&mut h, 0, 1, 1, true);
    let text = wait_for_copy(&h, 1);
    let lines: Vec<&str> = text.split('\n').collect();
    // From the first row of the view as it ended, down to row 10 of the first view.
    assert!(lines.len() > 24, "{} rows", lines.len());
    // The last row is cut at the anchor's column.
    let numbers: Vec<u32> = lines[..lines.len() - 1]
        .iter()
        .filter_map(|l| l.parse().ok())
        .collect();
    assert_eq!(numbers.len(), lines.len() - 1, "{text}");
    assert!(numbers.windows(2).all(|w| w[1] == w[0] + 1), "{text}");
}

#[test]
fn a_wrapped_line_copies_the_same_from_the_screen_and_after_scrolling_partly_away() {
    let mut h = Harness::deck(SIZE);
    h.wait_for_text("$", TIMEOUT);
    h.send_keys("seq 1 100; printf '%0200d\\n' 0\r");
    wait_until("the wrapped line", TIMEOUT, || {
        let r = rows(&h);
        r.iter().filter(|r| r.starts_with("0000000")).count() >= 3
            && r.iter().rposition(|r| r.starts_with('$'))
                > r.iter().rposition(|r| r.starts_with("0000000"))
    });
    enter_scroll_mode(&mut h);
    let top = rows(&h)
        .iter()
        .position(|r| r.starts_with("0000000"))
        .unwrap();
    let row = |n: usize| u16::try_from(top + n).unwrap();
    // The three rows of the wrapped line, by the mouse: copied from the screen.
    mouse(&mut h, 0, 1, row(1), false);
    mouse(&mut h, 32, 80, row(3), false);
    mouse(&mut h, 0, 80, row(3), true);
    let on_screen = wait_for_copy(&h, 1);
    assert_eq!(on_screen, "0".repeat(200));
    // One notch back moves its rows three lines down: two of them are below the screen.
    wheel(&mut h, true);
    wait_until("the view to move", TIMEOUT, || {
        h.cell(top + 3, 0).inverse() && !h.cell(top + 2, 0).inverse()
    });
    h.capture("wrapped-partly-off-screen");
    h.send_keys("y");
    let again = wait_for_copy(&h, 2);
    assert_eq!(again, on_screen);
}

#[test]
fn another_clients_scroll_clamps_the_cursor_and_g_returns_to_the_bottom() {
    let mut a = with_history();
    enter_scroll_mode(&mut a);
    let path = a.home().path().to_string_lossy().into_owned();
    let mut b = Harness::builder(SIZE).env("DECK_HOME", &path).deck();
    b.wait_for_text("500", TIMEOUT);
    enter_scroll_mode(&mut b);
    b.send_keys(PAGE_UP);
    b.send_keys(PAGE_UP);
    // A follows the shared view and stays in scroll mode.
    wait_until("A to follow", TIMEOUT, || {
        indicator(&a).is_some_and(|(o, _)| o == 48)
    });
    assert!(status(&a).contains("SCROLL"), "{}", status(&a));
    a.capture("other-client-scrolled");
    // A's cursor sits on the last row of the view; one `k` makes it the row above, which A
    // shows on its second to last row.
    a.send_keys("ky");
    let text = wait_for_copy(&a, 1);
    let shown = rows(&a);
    assert_eq!(text, shown[SIZE.1 as usize - 2].trim_end(), "{text}");
    // `G` returns to the bottom and A stays in scroll mode.
    a.send_keys("G");
    wait_until("the bottom", TIMEOUT, || indicator(&a).is_none());
    assert!(status(&a).contains("SCROLL"), "{}", status(&a));
    a.capture("back-at-the-bottom");
}

/// `deck` after 4000 numbered lines, two of them (2000 and 2100) with the given word after the
/// number. The command line does not contain the word whole, so it is not found in the echo.
fn with_words(first: &str, second: &str) -> Harness {
    with_words_at(first, second, 2100)
}

/// As `with_words`, the second word on line `at`.
fn with_words_at(first: &str, second: &str, at: u32) -> Harness {
    let split = |w: &str| format!("\"{}\" \"{}\"", &w[..1], &w[1..]);
    let mut h = Harness::deck(SIZE);
    h.wait_for_text("$", TIMEOUT);
    h.send_keys(&format!(
        "seq 1 4000 | awk '$1==2000{{print $1 \" \" {}; next}} $1=={at}{{print $1 \" \" {}; next}} {{print}}'\r",
        split(first),
        split(second)
    ));
    h.wait_for_row_start("4000", TIMEOUT);
    wait_until("the prompt below 4000", TIMEOUT, || {
        rows(&h).iter().rposition(|r| r.starts_with('$'))
            > rows(&h).iter().position(|r| r.starts_with("4000"))
    });
    h
}

/// The screen row that starts with `prefix`.
fn row_of(h: &Harness, prefix: &str) -> Option<usize> {
    rows(h).iter().position(|r| r.starts_with(prefix))
}

fn flashed(h: &Harness, text: &str) -> bool {
    status(h).contains(text)
}

#[test]
fn question_mark_finds_error_far_up_and_n_and_capital_n_move_between_matches() {
    let mut h = with_words("error", "error");
    enter_scroll_mode(&mut h);
    h.send_keys("?error");
    wait_until("the prompt", TIMEOUT, || status(&h).contains("?error"));
    h.capture("search-prompt");
    h.send_keys("\r");
    // Searching up from the bottom finds the nearest one first, about 1900 lines up.
    h.wait_for_row_start("2100 error", TIMEOUT);
    wait_until("the prompt to close", TIMEOUT, || {
        !status(&h).contains("?error")
    });
    h.capture("search-found");
    // The copy cursor is on the match's start; the matches are drawn in the search colours.
    let at = row_of(&h, "2100 error").unwrap();
    let cursor = h.cell(at, 5);
    assert_eq!(
        cursor.bg,
        Color::Indexed(4),
        "the cursor, in the accent colour"
    );
    for col in 6..10 {
        let cell = h.cell(at, col);
        assert_eq!(
            (cell.fg, cell.bg),
            (Color::Indexed(0), Color::Indexed(11)),
            "col {col}"
        );
        assert!(cell.bold(), "the current match is bold");
    }
    let before = h.cell(at, 4);
    assert_ne!(before.bg, Color::Indexed(11));
    h.send_keys("n");
    h.wait_for_row_start("2000 error", TIMEOUT);
    assert_eq!(row_of(&h, "2100 error"), None);
    h.send_keys("N");
    h.wait_for_row_start("2100 error", TIMEOUT);
}

#[test]
fn all_matches_on_screen_are_drawn_and_the_current_one_apart() {
    let mut h = Harness::deck(SIZE);
    h.wait_for_text("$", TIMEOUT);
    // Two matches on one screen: a row with the word twice.
    h.send_keys("clear; echo 'a error b error c'\r");
    h.wait_for_row_start("a error b error c", TIMEOUT);
    enter_scroll_mode(&mut h);
    h.send_keys("?error\r");
    wait_until("a match", TIMEOUT, || {
        row_of(&h, "a error b error c").is_some_and(|r| h.cell(r, 12).bg == Color::Indexed(11))
    });
    let at = row_of(&h, "a error b error c").unwrap();
    h.capture("search-two-matches");
    // The nearer (last) is the current; the first is a plain match.
    for col in 2..7 {
        assert_eq!(h.cell(at, col).bg, Color::Indexed(3), "col {col}");
        assert!(!h.cell(at, col).bold());
    }
    assert_ne!(h.cell(at, 8).bg, Color::Indexed(3));
}

#[test]
fn a_search_past_the_oldest_match_wraps_and_a_missing_word_is_flashed() {
    let mut h = with_words("error", "error");
    enter_scroll_mode(&mut h);
    h.send_keys("?error\r");
    h.wait_for_row_start("2100 error", TIMEOUT);
    h.send_keys("n");
    h.wait_for_row_start("2000 error", TIMEOUT);
    // The 2000 one is the oldest: the next `n` wraps to the last.
    h.send_keys("n");
    wait_until("the wrap", TIMEOUT, || flashed(&h, "Search wrapped"));
    h.capture("search-wrapped");
    h.wait_for_text("2100 error", TIMEOUT);
    h.send_keys("/zzzz\r");
    wait_until("not found", TIMEOUT, || flashed(&h, "Not found: zzzz"));
    h.capture("search-not-found");
    // Escape cancels a prompt without leaving scroll mode.
    h.send_keys("/abc");
    wait_until("the prompt", TIMEOUT, || status(&h).contains("/abc"));
    h.send_keys("\x1b");
    wait_until("the prompt to close", TIMEOUT, || {
        status(&h).contains("SCROLL") && !status(&h).contains("/abc")
    });
}

#[test]
fn smart_case_a_capital_matches_only_that_case_and_lower_case_both() {
    // The two words are 3 rows apart, so both are on screen together.
    let mut h = with_words_at("Error", "error", 2003);
    enter_scroll_mode(&mut h);
    h.send_keys("g/Error\r");
    h.wait_for_row_start("2000 Error", TIMEOUT);
    let (first, second) = (
        row_of(&h, "2000 Error").unwrap(),
        row_of(&h, "2003 error").unwrap(),
    );
    wait_until("the highlight", TIMEOUT, || {
        h.cell(first, 7).bg == Color::Indexed(11)
    });
    // `error` is no match of `Error`: not drawn as one, nor as the current one.
    for col in 5..10 {
        let bg = h.cell(second, col).bg;
        assert_ne!(bg, Color::Indexed(3), "col {col}");
        assert_ne!(bg, Color::Indexed(11), "col {col}");
    }
    // The only `Error`: `n` wraps around to it again.
    h.send_keys("n");
    wait_until("the wrap", TIMEOUT, || flashed(&h, "Search wrapped"));
    // Lower case finds both: the next one down from `Error` is `error`.
    h.send_keys("/error\r");
    wait_until("the current match", TIMEOUT, || {
        row_of(&h, "2003 error").is_some_and(|r| h.cell(r, 7).bg == Color::Indexed(11))
    });
    // Both are drawn now, the other one as a plain match.
    let first = row_of(&h, "2000 Error").unwrap();
    assert_eq!(h.cell(first, 7).bg, Color::Indexed(3));
    h.send_keys("n");
    wait_until("the wrap to the first", TIMEOUT, || {
        row_of(&h, "2000 Error").is_some_and(|r| h.cell(r, 7).bg == Color::Indexed(11))
    });
}

#[test]
fn v_after_a_search_then_e_and_y_copies_the_matched_word() {
    let mut h = with_words("error", "error");
    enter_scroll_mode(&mut h);
    h.send_keys("?error\r");
    h.wait_for_row_start("2100 error", TIMEOUT);
    wait_until("the prompt to close", TIMEOUT, || {
        !status(&h).contains("?error")
    });
    h.send_keys("ve");
    wait_until("a selection", TIMEOUT, || status(&h).contains("SELECT"));
    h.send_keys("y");
    assert_eq!(wait_for_copy(&h, 1), "error");
}

#[test]
fn escape_clears_the_highlights_before_it_leaves_scroll_mode() {
    let mut h = with_words("error", "error");
    enter_scroll_mode(&mut h);
    h.send_keys("?error\r");
    h.wait_for_row_start("2100 error", TIMEOUT);
    let at = row_of(&h, "2100 error").unwrap();
    wait_until("the highlight", TIMEOUT, || {
        h.cell(at, 7).bg == Color::Indexed(11)
    });
    h.send_keys("\x1b");
    wait_until("the highlight to go", TIMEOUT, || {
        h.cell(at, 7).bg != Color::Indexed(11)
    });
    assert!(status(&h).contains("SCROLL"), "{}", status(&h));
    h.send_keys("\x1b");
    wait_until("scroll mode to end", TIMEOUT, || {
        !status(&h).contains("SCROLL")
    });
}
