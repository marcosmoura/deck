//! The session picker and the session commands, end to end: the real `deck` in a PTY.

mod support;

use support::{Harness, TIMEOUT, stderr, stdout, wait_until};

const SIZE: (u16, u16) = (100, 30);
const PREFIX: &[u8] = b"\x1b ";
const ENTER: &str = "\r";
const CTRL_U: &str = "\x15";
const CTRL_O: &str = "\x0f";
const CTRL_R: &str = "\x12";
const CTRL_X: &str = "\x18";

/// `deck` with the sidebar of the product's defaults, once its first prompt is up.
fn attached() -> Harness {
    let h = Harness::builder(SIZE).config_unset("sidebar").deck();
    h.wait_for_text("│$", TIMEOUT);
    h
}

/// `deck <args>` in a PTY on the same home as `first`.
fn deck_in_pty(first: &Harness, args: &[&str]) -> Harness {
    let path = first.home().path().to_string_lossy().into_owned();
    let mut argv = vec![env!("CARGO_BIN_EXE_deck")];
    argv.extend_from_slice(args);
    Harness::builder(SIZE)
        .config_unset("sidebar")
        .env("DECK_HOME", &path)
        .spawn(&argv)
}

fn prefixed(h: &mut Harness, key: &str) {
    h.send_bytes(PREFIX);
    h.send_keys(key);
}

fn rows(h: &Harness) -> Vec<String> {
    h.screen_text().lines().map(str::to_string).collect()
}

/// Waits until the sidebar header (the first row) names the session.
fn wait_for_session(h: &Harness, name: &str) {
    wait_until(&format!("the session {name}"), TIMEOUT, || {
        // The sidebar header starts with the session's name.
        rows(h)
            .first()
            .is_some_and(|r| r.trim_start().split(' ').next() == Some(name))
    });
}

fn wait_gone(h: &Harness, text: &str) {
    wait_until(&format!("{text:?} to go"), TIMEOUT, || {
        !h.screen_text().contains(text)
    });
}

fn new_detached(h: &Harness, name: &str) {
    let out = h.run_deck(&["new", name, "-d"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), format!("{name}\n"));
}

fn ls(h: &Harness) -> String {
    stdout(&h.run_deck(&["ls"]))
}

fn open_picker(h: &mut Harness) {
    prefixed(h, "s");
    h.wait_for_text("Sessions", TIMEOUT);
}

#[test]
fn the_picker_lists_every_session_with_the_current_one_marked() {
    let mut h = attached();
    new_detached(&h, "other");
    open_picker(&mut h);
    h.wait_for_text("other", TIMEOUT);
    h.wait_for_text("⏎ switch  ^o new  ^r rename  ^x kill  esc close", TIMEOUT);
    let screen = h.screen_text();
    assert!(screen.contains("╭─ Sessions"), "{screen}");
    let main = rows(&h)
        .into_iter()
        .find(|r| r.contains("● main"))
        .expect("the current session is marked");
    assert!(
        main.contains("1 space") && main.contains("1 terminal"),
        "{main}"
    );
    assert!(screen.contains("   other"), "{screen}");
    h.capture("picker");
    h.send_keys("\x1b");
    wait_gone(&h, "Sessions");
}

#[test]
fn filtering_and_enter_switch_the_session_without_leaving_the_screen() {
    let mut h = attached();
    new_detached(&h, "other");
    open_picker(&mut h);
    h.wait_for_text("other", TIMEOUT);
    h.send_keys("oth");
    wait_gone(&h, "● main");
    h.capture("picker-filtered");
    h.send_keys(ENTER);
    wait_for_session(&h, "other");
    h.wait_for_text("│$", TIMEOUT);
    wait_gone(&h, "Sessions");
    let raw = h.raw_output();
    let leave = b"\x1b[?1049l";
    assert!(
        !raw.windows(leave.len()).any(|w| w == leave),
        "the main screen was restored while switching"
    );
    h.capture("switched");
}

#[test]
fn ctrl_o_creates_a_session_from_the_filter_and_switches_to_it() {
    let mut h = attached();
    open_picker(&mut h);
    h.send_keys("third");
    h.send_keys(CTRL_O);
    wait_for_session(&h, "third");
    h.wait_for_text("│$", TIMEOUT);
    let text = ls(&h);
    assert!(text.contains("third") && text.contains("main"), "{text}");

    // With an empty filter it asks for a name.
    open_picker(&mut h);
    h.send_keys(CTRL_O);
    h.wait_for_text("New session", TIMEOUT);
    h.capture("new-session-prompt");
    h.send_keys("fourth");
    h.send_keys(ENTER);
    wait_for_session(&h, "fourth");
}

#[test]
fn ctrl_r_renames_the_selected_session_and_a_duplicate_shows_the_error() {
    let mut h = attached();
    new_detached(&h, "other");
    open_picker(&mut h);
    h.wait_for_text("other", TIMEOUT);
    h.send_keys("oth");
    wait_gone(&h, "● main");
    h.send_keys(CTRL_R);
    h.wait_for_text("Rename session", TIMEOUT);
    h.wait_for_text("│ other█", TIMEOUT);
    h.send_keys(CTRL_U);
    h.send_keys("main");
    h.send_keys(ENTER);
    h.wait_for_text("a session named \"main\" already exists", TIMEOUT);
    h.wait_for_text("Rename session", TIMEOUT);
    h.capture("rename-error");
    h.send_keys(CTRL_U);
    h.send_keys("second");
    h.send_keys(ENTER);
    wait_until("the new name in the list", TIMEOUT, || {
        let t = ls(&h);
        t.contains("second") && !t.contains("other")
    });
    h.send_keys(CTRL_U);
    h.wait_for_text("   second", TIMEOUT);
}

#[test]
fn ctrl_x_kills_the_current_session_and_the_client_moves_to_the_previous_one() {
    let mut h = attached();
    new_detached(&h, "other");
    open_picker(&mut h);
    h.send_keys("oth");
    wait_gone(&h, "● main");
    h.send_keys(ENTER);
    wait_for_session(&h, "other");
    h.wait_for_text("│$", TIMEOUT);

    open_picker(&mut h);
    h.send_keys(CTRL_X);
    h.wait_for_text("Kill session \"other\" and its 1 terminal?", TIMEOUT);
    h.capture("kill-confirm");
    h.send_keys("y");
    wait_for_session(&h, "main");
    h.wait_for_text("│$", TIMEOUT);
    assert!(!ls(&h).contains("other"));

    // The last session ends the client.
    open_picker(&mut h);
    h.send_keys(CTRL_X);
    h.wait_for_text("y yes  n no", TIMEOUT);
    h.send_keys("y");
    h.wait_for_text("[session main ended]", TIMEOUT);
    assert!(h.wait_for_exit(TIMEOUT).success());
}

#[test]
fn killing_another_session_from_the_picker_keeps_the_picker_open() {
    let mut h = attached();
    new_detached(&h, "other");
    open_picker(&mut h);
    h.send_keys("oth");
    wait_gone(&h, "● main");
    h.send_keys(CTRL_X);
    h.wait_for_text("Kill session \"other\"", TIMEOUT);
    h.send_keys("y");
    wait_until("other to be gone", TIMEOUT, || !ls(&h).contains("other"));
    h.wait_for_text("no sessions match", TIMEOUT);
}

#[test]
fn the_rename_binding_renames_the_current_session() {
    let mut h = attached();
    prefixed(&mut h, "$");
    h.wait_for_text("Rename session", TIMEOUT);
    h.wait_for_text("│ main█", TIMEOUT);
    h.send_keys(CTRL_U);
    h.send_keys("primary");
    h.send_keys(ENTER);
    wait_for_session(&h, "primary");
    h.wait_for_text(" primary ", TIMEOUT);
    assert!(ls(&h).starts_with("primary"));
    h.capture("renamed");
}

#[test]
fn attach_needs_the_session_unless_asked_to_create_it() {
    let h = attached();
    let mut missing = deck_in_pty(&h, &["attach", "nope"]);
    missing.wait_for_text("no session named nope", TIMEOUT);
    assert!(!missing.wait_for_exit(TIMEOUT).success());
    assert!(!ls(&h).contains("nope"));

    let created = deck_in_pty(&h, &["attach", "-c", "nope"]);
    wait_for_session(&created, "nope");
    assert!(ls(&h).contains("nope"));
}

#[test]
fn kill_session_and_rename_session_work_from_a_second_shell() {
    let h = attached();
    new_detached(&h, "other");
    let out = h.run_deck(&["rename-session", "main", "primary"]);
    assert!(out.status.success(), "{}", stderr(&out));
    wait_for_session(&h, "primary");
    let out = h.run_deck(&["rename-session", "primary", "other"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("already exists"), "{}", stderr(&out));
    let out = h.run_deck(&["kill-session", "other"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = ls(&h);
    assert!(
        text.starts_with("primary") && !text.contains("other"),
        "{text}"
    );
    let out = h.run_deck(&["kill-session", "other"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("no session named \"other\""),
        "{}",
        stderr(&out)
    );
}

#[test]
fn killing_the_attached_session_from_a_shell_moves_the_client_on() {
    let h = attached();
    new_detached(&h, "other");
    let out = h.run_deck(&["kill-session", "main"]);
    assert!(out.status.success(), "{}", stderr(&out));
    wait_for_session(&h, "other");
    h.wait_for_text("│$", TIMEOUT);
    h.capture("moved-on");
}

#[test]
fn without_a_daemon_kill_session_and_rename_session_say_so() {
    let home = support::Home::new();
    for args in [
        &["kill-session", "x"][..],
        &["rename-session", "x", "y"][..],
    ] {
        let out = home.run_deck(args);
        assert_eq!(out.status.code(), Some(1));
        assert_eq!(stderr(&out), "no deck daemon running\n");
    }
    assert!(!home.daemon_running());
}

#[test]
fn new_inside_deck_refuses_unless_detached() {
    let h = attached();
    let inside = |args: &[&str]| {
        h.home()
            .deck_command(args)
            .env("DECK", "1")
            .output()
            .expect("run deck")
    };
    let out = inside(&["new", "x"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("already inside deck"),
        "{}",
        stderr(&out)
    );
    assert!(!ls(&h).contains('x'));
    let out = inside(&["new", "-d", "x"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "x\n");
    assert!(ls(&h).contains("x  "));
}

#[test]
fn new_never_attaches_to_an_existing_session_and_reports_a_bad_directory() {
    let h = attached();
    let mut dup = deck_in_pty(&h, &["new", "main"]);
    dup.wait_for_text("a session named \"main\" already exists", TIMEOUT);
    assert!(!dup.wait_for_exit(TIMEOUT).success());
    assert!(!dup.screen_text().contains("│$"), "{}", dup.screen_text());

    let mut bad = deck_in_pty(&h, &["new", "-c", "/nope", "x"]);
    bad.wait_for_text("/nope", TIMEOUT);
    assert!(!bad.wait_for_exit(TIMEOUT).success());
    assert!(!ls(&h).contains("x  "));

    // A fresh name creates the session and attaches to it.
    let fresh = deck_in_pty(&h, &["new", "fresh"]);
    wait_for_session(&fresh, "fresh");
    fresh.wait_for_text("│$", TIMEOUT);
}

#[test]
fn switching_to_a_session_killed_meanwhile_stays_in_the_current_one_with_the_error() {
    let mut h = attached();
    new_detached(&h, "other");
    open_picker(&mut h);
    h.send_keys("oth");
    wait_gone(&h, "● main");
    // The picker's list is stale from here on.
    let out = h.run_deck(&["kill-session", "other"]);
    assert!(out.status.success(), "{}", stderr(&out));
    h.send_keys(ENTER);
    h.wait_for_text("no session named", TIMEOUT);
    wait_for_session(&h, "main");
    h.wait_for_text("│$", TIMEOUT);
    h.capture("refused-switch");
    // The client is alive and typing works.
    h.send_keys("echo STILL\r");
    h.wait_for_text("│STILL", TIMEOUT);
}
