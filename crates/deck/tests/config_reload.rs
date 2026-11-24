//! Live reload, end to end: the real `deck` in a PTY while `config.toml` changes under it.

mod support;

use std::fs;
use std::time::Duration;

use alacritty_terminal::vte::ansi::Color;
use support::{Harness, TIMEOUT, wait_until};

const SIZE: (u16, u16) = (100, 24);
const PREFIX: &[u8] = b"\x1b ";
const SHELL: &str = "shell = \"/bin/sh\"\n";
const NO_ARGS: &str = "shell_args = []\n";
// The ANSI colours as the emulator reports them.
const BLACK: Color = Color::Indexed(0);
const RED: Color = Color::Indexed(1);
const GREEN: Color = Color::Indexed(2);
const YELLOW: Color = Color::Indexed(3);
const BLUE: Color = Color::Indexed(4);

/// A `deck` with the sidebar, a short poll interval and these extra config lines (written
/// after the harness's own keys), once its first prompt is up.
fn running(extra: &[(&str, &str)]) -> Harness {
    let mut builder = Harness::builder(SIZE)
        .config_unset("sidebar")
        // A config that cannot be read leaves the shell to the environment.
        .env("SHELL", "/bin/sh")
        .env("DECK_TEST_CONFIG_POLL_MS", "200");
    for (key, value) in extra {
        builder = builder.config(key, value);
    }
    let h = builder.deck();
    h.wait_for_text("│$", TIMEOUT);
    h
}

fn config_path(h: &Harness) -> std::path::PathBuf {
    h.home().path().join("config.toml")
}

/// Replaces the file with the harness's base keys and `extra` lines.
#[expect(
    clippy::unwrap_used,
    reason = "a config write failure fails the integration test"
)]
/// Writes the config the way editors save: a temp file renamed over it, so the daemon's poll
/// never sees it half-written.
fn write(h: &Harness, extra: &str) {
    let path = config_path(h);
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, contents(extra)).unwrap();
    fs::rename(&tmp, path).unwrap();
}

fn contents(extra: &str) -> String {
    let args = if extra.contains("shell_args") {
        ""
    } else {
        NO_ARGS
    };
    format!("{SHELL}{args}{extra}")
}

fn prefixed(h: &mut Harness, key: &str) {
    h.send_bytes(PREFIX);
    h.send_keys(key);
}

/// The first cell of the session name in the sidebar header, which takes the accent colour.
fn accent(h: &Harness) -> Color {
    h.cell(0, 1).fg
}

#[expect(clippy::panic, reason = "missing text fails the integration test")]
fn find(h: &Harness, text: &str) -> (usize, usize) {
    h.screen_text()
        .lines()
        .enumerate()
        .find_map(|(y, r)| r.find(text).map(|i| (y, r[..i].chars().count())))
        .unwrap_or_else(|| panic!("{text:?} is not on the screen:\n{}", h.screen_text()))
}

fn wait_accent(h: &Harness, color: Color) {
    wait_until("the accent colour", TIMEOUT, || accent(h) == color);
}

#[test]
fn a_new_accent_applies_without_reattaching_and_reloaded_flashes() {
    let h = running(&[]);
    h.wait_for_active_row(" ❯  sh", TIMEOUT);
    assert_eq!(accent(&h), BLUE);
    write(&h, "[theme]\naccent = \"red\"\n");
    wait_accent(&h, RED);
    assert_eq!(h.cell(0, 1).fg, RED, "the session name");
    h.wait_for_text("config reloaded", TIMEOUT);
    h.capture("reloaded");
}

#[test]
fn a_rebound_detach_key_works_and_the_old_one_flashes() {
    let mut h = running(&[]);
    write(&h, "[keys]\ndetach = \"q\"\n");
    h.wait_for_text("config reloaded", TIMEOUT);
    prefixed(&mut h, "d");
    h.wait_for_text("no binding for d", TIMEOUT);
    prefixed(&mut h, "q");
    let status = h.wait_for_exit(TIMEOUT);
    assert!(status.success(), "{status}");
}

#[test]
fn a_wider_sidebar_shrinks_the_shells_columns() {
    let mut h = running(&[]);
    h.send_keys("stty size\r");
    h.wait_for_text("24 71", TIMEOUT);
    write(&h, "sidebar_width = 40\n");
    wait_until("the wider sidebar", TIMEOUT, || {
        h.screen_text()
            .lines()
            .nth(5)
            .is_some_and(|r| r.chars().nth(40) == Some('│'))
    });
    h.send_keys("stty size\r");
    h.wait_for_text("24 59", TIMEOUT);
    h.capture("wider-sidebar");
}

#[test]
fn an_invalid_file_shows_its_location_and_keeps_the_previous_config() {
    let h = running(&[("theme", "{ accent = \"green\" }")]);
    wait_accent(&h, GREEN);
    write(&h, "[theme]\naccent = \"purple\"\n");
    h.wait_for_text("config.toml:4:10:", TIMEOUT);
    let (row, col) = find(&h, "config.toml:4:10:");
    assert_eq!(row, 23);
    assert_eq!(h.cell(row, col).fg, RED, "the error colour");
    assert_eq!(accent(&h), GREEN);
    h.capture("invalid");
}

#[test]
fn a_save_through_a_rename_is_picked_up_and_deleting_returns_the_defaults() {
    let h = running(&[]);
    let tmp = h.home().path().join("config.toml.swp");
    fs::write(&tmp, contents("[theme]\naccent = \"red\"\n")).unwrap();
    fs::rename(&tmp, config_path(&h)).unwrap();
    wait_accent(&h, RED);
    fs::remove_file(config_path(&h)).unwrap();
    wait_accent(&h, BLUE);
    h.wait_for_text("config reloaded", TIMEOUT);
}

#[test]
fn an_edit_of_the_same_size_right_after_the_last_one_is_picked_up() {
    let h = running(&[("theme", "{ accent = \"green\" }")]);
    wait_accent(&h, GREEN);
    write(&h, "[theme]\naccent = \"white\"\n");
    write(&h, "[theme]\naccent = \"black\"\n");
    wait_until("the last edit", TIMEOUT, || accent(&h) == BLACK);
}

#[test]
fn new_shell_args_reach_new_terminals_only() {
    let mut h = running(&[]);
    write(&h, "shell_args = [\"-c\", \"echo NEWSHELL; exec sh\"]\n");
    h.wait_for_text("config reloaded", TIMEOUT);
    // The running shell is the same one and answers.
    h.send_keys("echo alive$((1+1))\r");
    h.wait_for_text("alive2", TIMEOUT);
    assert!(!h.screen_text().contains("NEWSHELL"));
    prefixed(&mut h, "c");
    h.wait_for_text("NEWSHELL", TIMEOUT);
}

#[test]
fn key_conflicts_are_an_error_or_a_warning_naming_both_actions() {
    let h = running(&[]);
    write(&h, "[keys]\ndetach = \"q\"\nscroll = \"q\"\n");
    h.wait_for_text("bound to both", TIMEOUT);
    h.capture("conflict-error");

    write(&h, "[keys]\ndetach = \"n\"\n");
    // It waits its turn behind the error flash (6 s) and `config reloaded` (2 s).
    h.wait_for_text("replaces next_panel", Duration::from_secs(15));
    let (row, col) = find(&h, "replaces next_panel");
    assert_eq!(row, 23);
    assert_eq!(h.cell(row, col).fg, YELLOW, "the warning colour");
    h.capture("conflict-warning");
}

#[test]
fn an_invalid_file_at_start_runs_with_the_defaults_and_shows_the_error() {
    // Not `running`: with the defaults the shell is the login shell, with its own prompt.
    let h = Harness::builder(SIZE)
        .config_unset("sidebar")
        .config("theme", "{ accent = \"purple\" }")
        .env("SHELL", "/bin/sh")
        .deck();
    h.wait_for_text("config.toml:3:20:", TIMEOUT);
    wait_accent(&h, BLUE);
    h.capture("start-invalid");
}

#[test]
fn the_first_shell_starts_at_its_final_size() {
    let script = "trap 'echo RESIZED' WINCH; stty size; while :; do sleep 0.1; done";
    let h = Harness::builder(SIZE)
        .config_unset("sidebar")
        .config("sidebar_width", "40")
        .config("shell_args", &format!("[\"-c\", \"{script}\"]"))
        .deck();
    // 100 - 41 columns, 24 rows: no border, no status row.
    h.wait_for_text("24 59", TIMEOUT);
    std::thread::sleep(Duration::from_millis(1500));
    assert!(!h.screen_text().contains("RESIZED"), "{}", h.screen_text());
    h.capture("first-size");
}

#[test]
fn a_space_gap_puts_a_blank_row_between_spaces_without_reattaching() {
    let h = running(&[]);
    let out = h.run_deck(&["new-space", "api", "--dir", "/tmp"]);
    assert!(out.status.success(), "{}", support::stderr(&out));
    h.wait_for_text("api", TIMEOUT);
    let before = find_opt(&h, "api").expect("api in the sidebar");
    write(&h, "sidebar_space_gap = 1\n");
    h.wait_for_text("config reloaded", TIMEOUT);
    wait_until("the blank row", TIMEOUT, || {
        find_opt(&h, "api") == Some(before + 1)
    });
    let text = h.screen_text();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[before].chars().take(28).all(|c| c == ' '), "{text}");
    h.capture("space-gap");
    write(&h, "sidebar_space_gap = 0\n");
    wait_until("no blank row", TIMEOUT, || {
        find_opt(&h, "api") == Some(before)
    });
}

fn find_opt(h: &Harness, text: &str) -> Option<usize> {
    h.screen_text()
        .lines()
        .position(|r| r.chars().take(28).collect::<String>().contains(text))
}
