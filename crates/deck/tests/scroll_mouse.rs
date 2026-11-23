//! Scrollback, the mouse and focus events, end to end: the real `deck` in a PTY. Mouse and
//! focus events are written to the outer terminal as the escape sequences a terminal sends.

mod support;

use support::{Harness, TIMEOUT, wait_until};

const SIZE: (u16, u16) = (80, 24);
const PAGE_UP: &str = "\x1b[5~";

/// `deck` in an 80×24 terminal, once its first prompt is inside the frame.
fn attached() -> Harness {
    let h = Harness::deck(SIZE);
    h.wait_for_text("$", TIMEOUT);
    h
}

fn prefixed(h: &mut Harness, key: &str) {
    h.send_bytes(b"\x1b ");
    h.send_keys(key);
}

fn rows(h: &Harness) -> Vec<String> {
    h.screen_text().split('\n').map(str::to_string).collect()
}

/// `(offset, history)` of the `[offset/history]` in the floating bar.
fn indicator(h: &Harness) -> Option<(u32, u32)> {
    let bar = rows(h).pop()?;
    let inner = &bar[bar.find('[')? + 1..];
    let (offset, rest) = inner.split_once('/')?;
    let (history, _) = rest.split_once(']')?;
    Some((offset.parse().ok()?, history.parse().ok()?))
}

/// The window's last row, where the floating bar shows.
fn status(h: &Harness) -> String {
    rows(h).pop().unwrap_or_default()
}

/// A wheel notch at the 1-based outer cell (10, 5), well inside the terminal.
fn wheel(h: &mut Harness, up: bool) {
    h.send_keys(if up {
        "\x1b[<64;10;5M"
    } else {
        "\x1b[<65;10;5M"
    });
}

/// Prints 1 to 500 and waits for the last number.
fn with_history() -> Harness {
    let mut h = attached();
    h.send_keys("seq 1 500\r");
    h.wait_for_row_start("500", TIMEOUT);
    h
}

#[test]
fn scroll_mode_pages_back_returns_to_the_bottom_and_leaves() {
    let mut h = with_history();
    prefixed(&mut h, "[");
    h.wait_for_text("SCROLL", TIMEOUT);
    h.send_keys(PAGE_UP);
    wait_until("the indicator", TIMEOUT, || indicator(&h).is_some());
    let (offset, history) = indicator(&h).unwrap();
    assert_eq!(offset, 24, "one page of the 24-row terminal");
    assert!(history > 450, "{history}");
    h.capture("scrolled");
    let screen = rows(&h);
    assert!(
        screen[..screen.len() - 1]
            .iter()
            .all(|r| !r.contains("500")),
        "{}",
        h.screen_text()
    );
    assert!(status(&h).contains(" SCROLL "), "{}", status(&h));
    assert!(
        status(&h).contains("q exit  g top  G bottom"),
        "{}",
        status(&h)
    );
    assert!(
        status(&h).contains(&format!("[{offset}/{history}]")),
        "{}",
        status(&h)
    );

    // G returns to the bottom and stays in the mode.
    h.send_keys("G");
    wait_until("the bottom", TIMEOUT, || indicator(&h).is_none());
    h.wait_for_row_start("500", TIMEOUT);
    assert!(status(&h).contains(" SCROLL "), "{}", status(&h));
    h.capture("bottom");

    // g goes to the top; keys do not reach the shell.
    h.send_keys("g");
    wait_until("the top", TIMEOUT, || {
        indicator(&h).is_some_and(|(o, hist)| o == hist)
    });
    h.send_keys("xyz");
    h.send_keys("q");
    wait_until("scroll mode to end", TIMEOUT, || {
        !status(&h).contains("SCROLL")
    });
    h.wait_for_row_start("500", TIMEOUT);
    assert!(indicator(&h).is_none());
    // What was typed in scroll mode never reached the shell.
    h.send_keys("echo done\r");
    h.wait_for_row_start("done", TIMEOUT);
    assert!(!h.screen_text().contains("xyz"), "{}", h.screen_text());
}

#[test]
fn the_prefix_still_detaches_inside_scroll_mode() {
    let mut h = with_history();
    prefixed(&mut h, "[");
    h.wait_for_text("SCROLL", TIMEOUT);
    prefixed(&mut h, "d");
    let status = h.wait_for_exit(TIMEOUT);
    assert!(status.success(), "{}", h.screen_text());
    h.wait_for_text("[detached from main]", TIMEOUT);
}

#[test]
fn the_wheel_scrolls_a_shell_with_history_and_typing_returns_to_the_bottom() {
    let mut h = with_history();
    wheel(&mut h, true);
    // Three lines back: the last output line and the prompt are off the screen.
    wait_until("the screen to scroll", TIMEOUT, || {
        !rows(&h).iter().any(|r| r.starts_with("500"))
    });
    // The wheel does not enter scroll mode, so no bar shows.
    assert!(!status(&h).contains("SCROLL"), "{}", status(&h));
    h.capture("wheel");

    h.send_keys("Q");
    h.wait_for_text("$ Q", TIMEOUT);
}

#[test]
fn the_wheel_does_nothing_in_a_fresh_shell() {
    let mut h = attached();
    wheel(&mut h, true);
    wheel(&mut h, true);
    // Output after the wheel shows that it was handled; nothing scrolled.
    h.send_keys("echo sync\r");
    h.wait_for_row_start("sync", TIMEOUT);
    assert!(indicator(&h).is_none(), "{}", h.screen_text());
}

#[test]
fn background_output_grows_the_history_while_scrolled_back() {
    let mut h = with_history();
    h.send_keys("(sleep 1; seq 1 50) &\r");
    prefixed(&mut h, "[");
    h.wait_for_text("SCROLL", TIMEOUT);
    h.send_keys(PAGE_UP);
    wait_until("the indicator", TIMEOUT, || indicator(&h).is_some());
    let (_, before) = indicator(&h).unwrap();
    wait_until("the history to grow", TIMEOUT, || {
        indicator(&h).is_some_and(|(offset, history)| offset > 0 && history >= before + 20)
    });
    assert!(status(&h).contains(" SCROLL "), "{}", status(&h));
    h.capture("history-grew");
}

#[test]
fn the_wheel_moves_through_a_file_in_less() {
    let mut h = attached();
    h.send_keys("seq 1 200 > numbers.txt; LESS= less numbers.txt\r");
    wait_until("less", TIMEOUT, || {
        rows(&h).first().is_some_and(|r| r.trim() == "1")
    });
    wheel(&mut h, false);
    wait_until("three lines down", TIMEOUT, || {
        rows(&h).first().is_some_and(|r| r.trim() == "4")
    });
    wheel(&mut h, true);
    wait_until("back up", TIMEOUT, || {
        rows(&h).first().is_some_and(|r| r.trim() == "1")
    });
    // The wheel moved the program, not deck's history.
    assert!(indicator(&h).is_none());
    h.send_keys("q");
    h.wait_for_text("$", TIMEOUT);
}

/// Runs a shell snippet in the terminal that prints `READY` (built so that the echoed command
/// line does not contain it) once it is waiting for input.
fn start_reader(h: &mut Harness, setup: &str, count: usize) {
    h.send_keys(&format!(
        "set +m; {setup}; stty raw -echo; printf 'REA%s\\r\\n' DY; \
         x=$(dd bs=1 count={count} 2>/dev/null | od -An -tx1 -v | tr -d ' \\n'); \
         stty sane; printf 'GOT:%s\\r\\n' \"$x\"; sleep 30\r"
    ));
    h.wait_for_text("READY", TIMEOUT);
}

#[test]
fn the_wheel_on_the_alternate_screen_sends_cursor_keys_until_1007_is_off() {
    let mut h = attached();
    // Application cursor mode on the alternate screen.
    start_reader(&mut h, "printf '\\033[?1049h\\033[?1h'", 9);
    wheel(&mut h, true);
    h.wait_for_text("GOT:1b4f411b4f411b4f41", TIMEOUT);
    h.capture("alt-scroll");
    // Ctrl+C ends the sleep; a fresh program with `?1007` turned off.
    h.send_bytes(&[3]);
    // Back on the main screen before the next reader, or its `READY` could be the first
    // reader's, and the wheel would still go to that program as cursor keys.
    h.send_keys("printf '\\033[?1049l'; echo BA''CK\r");
    h.wait_for_text("BACK", TIMEOUT);
    h.send_keys("clear\r");
    start_reader(&mut h, "printf '\\033[?1049h\\033[?1007l'", 1);
    wheel(&mut h, true);
    wheel(&mut h, false);
    // Only the key that follows reaches the program.
    h.send_keys("Z");
    h.wait_for_text("GOT:5a", TIMEOUT);
}

#[test]
fn a_program_with_sgr_mouse_mode_receives_a_click_in_terminal_coordinates() {
    let mut h = attached();
    start_reader(&mut h, "printf '\\033[?1000h\\033[?1006h'", 9);
    // The cell (6, 4), 1-based, is the terminal's own column 6, row 4: no border to take off.
    h.send_keys("\x1b[<0;6;4M");
    h.wait_for_text("GOT:1b5b3c303b363b344d", TIMEOUT);
    h.capture("mouse");
    // The hex is `ESC[<0;6;4M`.
}

#[test]
fn a_program_with_focus_reporting_receives_focus_in_and_out() {
    let mut h = attached();
    start_reader(&mut h, "printf '\\033[?1004h'", 6);
    h.send_keys("\x1b[I");
    h.send_keys("\x1b[O");
    h.wait_for_text("GOT:1b5b491b5b4f", TIMEOUT);
    h.capture("focus");
}
