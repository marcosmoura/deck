//! The attach client, end to end: the real `deck` in a PTY, through the harness.

mod support;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

use alacritty_terminal::vte::ansi::Color;
use support::{Harness, TIMEOUT, TtyMode, wait_until};

const SIZE: (u16, u16) = (80, 24);
const DECK: &str = env!("CARGO_BIN_EXE_deck");
/// The status bar's hint for the default prefix and `detach`.
const HINT: &str = if cfg!(target_os = "macos") {
    "⌥Space d detach"
} else {
    "M-Space d detach"
};

/// `deck` in an 80×24 terminal, once its first prompt is inside the frame.
fn attached() -> Harness {
    let h = Harness::deck(SIZE);
    h.wait_for_text("│$ ", TIMEOUT);
    h
}

/// The prefix, then `key`.
fn prefixed(h: &mut Harness, key: &str) {
    h.send_bytes(b"\x1b ");
    h.send_keys(key);
}

fn rows(h: &Harness) -> Vec<String> {
    h.screen_text().lines().map(str::to_string).collect()
}

fn row_of(h: &Harness, needle: &str) -> usize {
    rows(h)
        .iter()
        .position(|r| r.contains(needle))
        .unwrap_or_else(|| panic!("no row with {needle:?}:\n{}", h.screen_text()))
}

/// A second client on the first one's daemon.
fn second_client(first: &Harness, size: (u16, u16)) -> Harness {
    let path = first.home().path().to_string_lossy().into_owned();
    Harness::builder(size).env("DECK_HOME", &path).deck()
}

#[test]
fn the_frame_and_status_bar_appear() {
    let h = attached();
    h.wait_for_text(HINT, TIMEOUT);
    let path = h.capture("attached");
    let rows = rows(&h);
    assert!(rows[0].starts_with("╭─ sh ─"), "{}", rows[0]);
    assert!(rows[0].ends_with('╮'));
    assert!(rows[1].starts_with("│$ ") && rows[1].ends_with('│'));
    assert!(rows[22].starts_with('╰') && rows[22].ends_with('╯'));
    assert!(rows[23].starts_with(" main "), "{}", rows[23]);
    assert!(rows[23].ends_with(HINT), "{}", rows[23]);
    assert!(path.is_file());
}

#[test]
fn typing_reaches_the_shell() {
    let mut h = attached();
    h.send_keys("echo hello\r");
    h.wait_for_text("│hello", TIMEOUT);
}

#[test]
fn detaching_keeps_the_shell_and_deck_comes_back_to_it() {
    let mut h = attached();
    h.send_keys("echo hello\r");
    h.wait_for_text("│hello", TIMEOUT);
    prefixed(&mut h, "d");
    let status = h.wait_for_exit(TIMEOUT);
    assert!(status.success(), "{}", h.screen_text());
    h.wait_for_text("[detached from main]", TIMEOUT);
    assert_eq!(h.tty_mode(), TtyMode::Cooked);
    let raw = h.raw_output();
    let leave = b"\x1b[?1049l";
    assert!(raw.windows(leave.len()).any(|w| w == leave));

    // The daemon notices the closed connection a moment later.
    wait_until("the session to be detached", TIMEOUT, || {
        let ls = String::from_utf8_lossy(&h.run_deck(&["ls"]).stdout).into_owned();
        ls.starts_with("main") && ls.contains("1 terminal") && !ls.contains("(attached)")
    });

    let again = second_client(&h, SIZE);
    again.wait_for_text("│hello", TIMEOUT);
    again.capture("reattached");
    assert!(again.screen_text().contains(HINT));
}

#[test]
fn the_shell_follows_the_window_size() {
    let mut h = attached();
    h.send_keys("stty size\r");
    h.wait_for_text("21 78", TIMEOUT);
    h.resize(100, 30);
    // The frame follows once the daemon has resized the shell.
    wait_until("the frame at the new size", TIMEOUT, || {
        rows(&h)[0].chars().count() == 100
    });
    h.send_keys("stty size\r");
    h.wait_for_text("27 98", TIMEOUT);
    let rows = rows(&h);
    assert!(
        rows[0].ends_with('╮') && rows[0].chars().count() == 100,
        "{}",
        rows[0]
    );
    assert!(rows[28].starts_with('╰'), "{}", rows[28]);
}

#[test]
fn the_prefix_twice_sends_it_to_the_shell_as_pressed() {
    let mut h = attached();
    h.send_keys("stty raw -echo; printf 'REA%s\\r\\n' DY; od -An -tx1 -N3 | tr -d ' '\r");
    h.wait_for_text("READY", TIMEOUT);
    // Option+Space (ESC and a space) twice, then `a`: the second one reaches the shell.
    h.send_bytes(b"\x1b \x1b ");
    h.send_keys("a");
    h.wait_for_text("1b2061", TIMEOUT);
}

#[test]
fn the_non_breaking_space_arms_the_prefix_and_twice_sends_its_own_bytes() {
    let mut h = attached();
    h.send_bytes(b"\xc2\xa0");
    h.wait_for_text("PREFIX", TIMEOUT);
    h.send_keys("x");
    h.wait_for_text("no binding for x", TIMEOUT);
    h.send_keys("stty raw -echo; printf 'REA%s\\r\\n' DY; od -An -tx1 -N3 | tr -d ' '\r");
    h.wait_for_text("READY", TIMEOUT);
    h.send_bytes(b"\xc2\xa0\xc2\xa0");
    h.send_keys("a");
    h.wait_for_text("c2a061", TIMEOUT);
    // And it detaches like Option+Space does.
    let mut h = attached();
    h.send_bytes(b"\xc2\xa0");
    h.send_keys("d");
    assert!(h.wait_for_exit(TIMEOUT).success());
}

#[test]
fn an_unbound_key_after_the_prefix_flashes_and_sends_nothing() {
    let mut h = attached();
    h.send_keys("stty raw -echo; od -An -tx1 -N1 | tr -d ' '\r");
    prefixed(&mut h, "x");
    h.wait_for_text("no binding for x", TIMEOUT);
    h.send_keys("a");
    wait_until("the byte of `a`", TIMEOUT, || {
        rows(&h).iter().any(|r| r.contains("61"))
    });
    assert!(!h.screen_text().contains("78"), "{}", h.screen_text());
}

#[test]
fn every_byte_reaches_a_program_in_raw_mode_unchanged() {
    let mut h = attached();
    let bytes: Vec<u8> = (0x01..=0x7f).filter(|b| *b != 0x1b).collect();
    for (n, chunk) in bytes.chunks(16).enumerate() {
        let count = chunk.len();
        h.send_keys(&format!(
            "stty raw -echo; echo REA''DY{n}; printf 'H:'; \
             dd bs=1 count={count} 2>/dev/null | od -An -tx1 -v | tr -d ' \\n'; \
             stty sane; echo ' DO''NE{n}'\r"
        ));
        h.wait_for_text(&format!("READY{n}"), TIMEOUT);
        for byte in chunk {
            h.send_bytes(&[*byte]);
        }
        let hex: String = chunk.iter().map(|b| format!("{b:02x}")).collect();
        h.wait_for_text(&format!("H:{hex} DONE{n}"), TIMEOUT);
    }
}

#[test]
fn a_configured_detach_key_works_shifted() {
    let mut h = Harness::builder(SIZE)
        .config("keys", "{ detach = \"X\" }")
        .deck();
    h.wait_for_text("│$ ", TIMEOUT);
    prefixed(&mut h, "X");
    let status = h.wait_for_exit(TIMEOUT);
    assert!(status.success(), "{}", h.screen_text());
    h.wait_for_text("[detached from main]", TIMEOUT);
}

#[test]
fn the_prefix_badge_shows_until_the_next_key() {
    let mut h = attached();
    h.send_bytes(b"\x1b ");
    h.wait_for_text("PREFIX", TIMEOUT);
    h.capture("prefix");
    h.send_bytes(&[0x1b]);
    wait_until("the badge to go", TIMEOUT, || {
        !h.screen_text().contains("PREFIX")
    });
    prefixed(&mut h, "x");
    wait_until("the badge to go", TIMEOUT, || {
        !h.screen_text().contains("PREFIX")
    });
    h.wait_for_text("no binding for x", TIMEOUT);
    // The message goes after two seconds and the hints return.
    h.wait_for_text(HINT, Duration::from_secs(6));
}

#[test]
fn exit_in_the_shell_ends_the_session() {
    let mut h = attached();
    h.send_keys("exit\r");
    let status = h.wait_for_exit(TIMEOUT);
    assert!(status.success(), "{}", h.screen_text());
    h.wait_for_text("[session main ended]", TIMEOUT);
}

#[test]
fn kill_server_ends_the_client() {
    let mut h = attached();
    let out = h.run_deck(&["kill-server"]);
    assert!(out.status.success());
    let status = h.wait_for_exit(TIMEOUT);
    assert!(status.success(), "{}", h.screen_text());
    h.wait_for_text("[deck daemon stopped]", TIMEOUT);
}

#[test]
fn sigterm_restores_the_outer_terminal() {
    let mut h = attached();
    h.terminate();
    h.wait_for_exit(TIMEOUT);
    assert_eq!(h.tty_mode(), TtyMode::Cooked);
    let raw = h.raw_output();
    let leave = b"\x1b[?1049l";
    assert!(raw.windows(leave.len()).any(|w| w == leave));
    let show = b"\x1b[?25h";
    assert!(raw.windows(show.len()).any(|w| w == show));
    assert!(!h.screen_text().contains('╭'), "{}", h.screen_text());
}

#[test]
fn a_tiny_window_shows_only_a_message_and_the_shell_keeps_its_size() {
    let mut h = attached();
    h.send_keys("stty size\r");
    h.wait_for_text("21 78", TIMEOUT);
    h.resize(15, 4);
    h.wait_for_text("too small", TIMEOUT);
    let text = h.screen_text();
    assert!(
        text.contains("deck: window") && !text.contains('╭'),
        "{text}"
    );
    h.capture("too-small");
    // Typed blind: the shell's size while the window is small. A `Viewport` sent for the
    // small window would have changed it.
    let file = h.home().path().join("size.txt");
    h.send_keys(&format!("stty size > {}\r", file.display()));
    wait_until("the size file", TIMEOUT, || {
        fs::read_to_string(&file).is_ok_and(|t| !t.trim().is_empty())
    });
    assert_eq!(fs::read_to_string(&file).unwrap().trim(), "21 78");
    h.resize(80, 24);
    h.wait_for_text(HINT, TIMEOUT);
    h.send_keys("clear; stty size\r");
    wait_until("the size back", TIMEOUT, || {
        rows(&h).iter().any(|r| r.starts_with("│21 78"))
    });
    prefixed(&mut h, "d");
    assert!(h.wait_for_exit(TIMEOUT).success());
}

#[test]
fn a_tiny_window_still_detaches() {
    let mut h = attached();
    h.resize(15, 4);
    h.wait_for_text("too small", TIMEOUT);
    prefixed(&mut h, "d");
    assert!(h.wait_for_exit(TIMEOUT).success());
}

#[test]
fn colours_and_attributes_reach_the_screen() {
    let mut h = attached();
    h.send_keys("printf '\\033[1;31mred\\033[0m \\033[7mrev\\033[0m'\r");
    h.wait_for_text("│red rev", TIMEOUT);
    let row = row_of(&h, "│red rev");
    let r = h.cell(row, 1);
    assert_eq!(r.ch, 'r');
    // Deck sets palette colours by index (`38;5;1`), as crossterm writes them.
    assert_eq!(r.fg, Color::Indexed(1));
    assert!(r.bold());
    assert!(!h.cell(row, 4).bold());
    assert!(h.cell(row, 5).inverse());
    assert!(!h.cell(row, 4).inverse());
}

#[test]
fn wide_and_accented_characters_keep_their_columns() {
    let mut h = attached();
    h.send_keys("printf '日本 🙂 éx'\r");
    h.wait_for_text("│日本 🙂 éx", TIMEOUT);
    h.capture("wide");
    let row = row_of(&h, "│日本");
    let chars: Vec<char> = [1, 3, 6, 9, 10, 79]
        .iter()
        .map(|c| h.cell(row, *c).ch)
        .collect();
    assert_eq!(chars, ['日', '本', '🙂', 'é', 'x', '│']);
}

#[test]
fn programs_get_the_outer_terminals_colours() {
    let query = "stty -icanon -echo; printf '\\033]11;?\\007'; \
                 dd bs=1 count=24 2>/dev/null | tr -d '\\033\\007'; echo\r";
    let mut h = Harness::builder(SIZE)
        .colors((0, 0, 0), (255, 255, 255))
        .deck();
    h.wait_for_text("│$ ", TIMEOUT);
    h.send_keys(query);
    h.wait_for_text("]11;rgb:ffff/ffff/ffff", TIMEOUT);

    // A terminal that never answers: the client attaches in time and the fallback is used.
    // (A second client on a running daemon, so only the client's own wait is measured.)
    let first = attached();
    let path = first.home().path().to_string_lossy().into_owned();
    let start = Instant::now();
    let mut h = Harness::builder(SIZE)
        .env("DECK_HOME", &path)
        .silent_colors()
        .deck();
    h.wait_for_text("│$ ", Duration::from_secs(1));
    assert!(start.elapsed() < Duration::from_secs(1));
    h.send_keys(query);
    h.wait_for_text("]11;rgb:0000/0000/0000", TIMEOUT);
}

#[test]
fn a_program_copy_reaches_the_outer_clipboard() {
    let mut h = attached();
    h.send_keys("printf '\\033]52;c;aGk=\\007'\r");
    wait_until("the OSC 52 copy", TIMEOUT, || {
        let raw = h.raw_output();
        let needle = b"\x1b]52;c;aGk=\x07";
        raw.windows(needle.len()).any(|w| w == needle)
    });
}

#[test]
fn a_panic_restores_the_outer_terminal_before_its_message() {
    let mut h = Harness::builder(SIZE).env("DECK_TEST_PANIC", "1").deck();
    let status = h.wait_for_exit(TIMEOUT);
    assert!(!status.success());
    assert_eq!(h.tty_mode(), TtyMode::Cooked);
    let raw = h.raw_output();
    let tail = b"\x1b[?1049l\x1b[?25h";
    let at = raw
        .windows(tail.len())
        .rposition(|w| w == tail)
        .expect("leaving the alternate screen and showing the cursor");
    let after = String::from_utf8_lossy(&raw[at + tail.len()..]).into_owned();
    assert!(after.contains("panicked"), "{after}");
}

#[test]
fn a_shell_started_by_deck_is_a_login_shell() {
    let dir = tempfile::Builder::new()
        .prefix("dks")
        .tempdir_in("/tmp")
        .unwrap();
    let args = dir.path().join("args.txt");
    let script = dir.path().join("shell.sh");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\necho \"$@\" > {}\nexec /bin/sh\n",
            args.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let mut h = Harness::builder(SIZE)
        .config("shell", &format!("\"{}\"", script.display()))
        .config_unset("shell_args")
        .deck();
    h.wait_for_text("│sh", TIMEOUT);
    assert_eq!(fs::read_to_string(&args).unwrap().trim(), "-l");
    prefixed(&mut h, "d");
    h.wait_for_exit(TIMEOUT);
}

#[test]
fn a_terminal_that_exits_at_once_is_reported() {
    let mut h = Harness::builder(SIZE)
        .config("shell_args", "[\"-c\", \"exit 3\"]")
        .deck();
    let status = h.wait_for_exit(TIMEOUT);
    assert_eq!(status.exit_code(), 1);
    let text = h.screen_text().replace('\n', "");
    let want = format!(
        "deck: terminal exited at once (status 3); see {}",
        h.home().log_file().display()
    );
    assert!(text.contains(&want), "{text}");
}

#[test]
fn a_named_session_must_exist() {
    let mut h = Harness::spawn(SIZE, &[DECK, "attach", "nosuch"]);
    let status = h.wait_for_exit(TIMEOUT);
    assert_eq!(status.exit_code(), 1);
    assert!(
        h.screen_text().contains("deck: no session named nosuch"),
        "{}",
        h.screen_text()
    );
    assert_eq!(h.tty_mode(), TtyMode::Cooked);
}

#[test]
fn a_named_session_is_attached_by_name() {
    let mut first = attached();
    first.send_keys("echo named\r");
    first.wait_for_text("│named", TIMEOUT);
    let path = first.home().path().to_string_lossy().into_owned();
    let second = Harness::builder(SIZE)
        .env("DECK_HOME", &path)
        .spawn(&[DECK, "attach", "main"]);
    second.wait_for_text("│named", TIMEOUT);
}

#[test]
fn a_smaller_client_draws_the_larger_layout_cropped() {
    let mut big = attached();
    let small = second_client(&big, (40, 12));
    // The daemon sized the shell for the small client: its layout, with the bottom border.
    small.wait_for_text("│$ ", TIMEOUT);
    wait_until("the small layout", TIMEOUT, || {
        rows(&small).get(10).is_some_and(|r| r.starts_with('╰'))
    });
    // The big client types, so its size is the one the shell has.
    big.send_keys("\r");
    wait_until("the big layout back", TIMEOUT, || {
        rows(&big)[0].chars().count() == 80
    });
    big.send_keys("echo cropped\r");
    small.wait_for_text("│cropped", TIMEOUT);
    small.capture("cropped");
    let rows = rows(&small);
    assert_eq!(rows.len(), 12);
    assert!(rows[0].starts_with("╭─ sh ─"), "{}", rows[0]);
    assert!(rows[11].starts_with(" main "), "{}", rows[11]);
    assert!(rows[1..11].iter().all(|r| r.starts_with('│')));
}
