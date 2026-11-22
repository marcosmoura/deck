//! `deck config default | check | path`, end to end: the real `deck`, no daemon.

mod support;

use std::fs;

use deck_core::config::{Config, DEFAULT_CONFIG_TOML};
use support::{Harness, Home, TIMEOUT, stderr, stdout};

fn write_config(home: &Home, text: &str) -> std::path::PathBuf {
    let path = home.path().join("config.toml");
    fs::write(&path, text).unwrap();
    path
}

#[test]
fn default_saved_as_the_config_checks_ok_and_equals_the_defaults() {
    let home = Home::new();
    let printed = home.run_deck(&["config", "default"]);
    assert!(printed.status.success(), "{}", stderr(&printed));
    assert_eq!(stdout(&printed), DEFAULT_CONFIG_TOML);

    let path = write_config(&home, &stdout(&printed));
    let check = home.run_deck(&["config", "check"]);
    assert!(check.status.success(), "{}", stdout(&check));
    assert_eq!(stdout(&check), format!("ok: {}\n", path.display()));
    assert_eq!(
        Config::parse(&stdout(&printed)).unwrap().config,
        Config::default()
    );
    assert!(!home.daemon_running());
}

#[test]
fn check_reports_each_finding_with_line_and_column_and_exits_1() {
    let home = Home::new();
    let path = write_config(
        &home,
        "prefix = \"C-\"\n\
         bogus = 1\n\
         sidebar = true\n\
         \n\
         [theme]\n\
         accent = \"purple\"\n\
         \n\
         [keys]\n\
         detach = = \"x\"\n",
    );
    let out = home.run_deck(&["config", "check", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1), "{}", stdout(&out));
    let text = stdout(&out);
    let lines: Vec<&str> = text.lines().collect();
    let p = path.display();
    assert_eq!(lines.len(), 4, "{text}");
    assert!(
        lines[0].starts_with(&format!("{p}:1:10: error: ")),
        "{text}"
    );
    assert!(lines[0].contains("`prefix`"), "{text}");
    assert_eq!(
        lines[1],
        format!("{p}:2:1: warning: unknown config key `bogus`")
    );
    assert!(
        lines[2].starts_with(&format!("{p}:6:10: error: ")),
        "{text}"
    );
    assert!(lines[2].contains("`theme.accent`"), "{text}");
    assert!(
        lines[3].starts_with(&format!("{p}:9:10: error: ")),
        "{text}"
    );
    assert!(!home.daemon_running());
}

#[test]
fn a_file_with_only_an_unknown_key_exits_0_with_a_warning() {
    let home = Home::new();
    let path = write_config(&home, "colour = 1\n");
    let out = home.run_deck(&["config", "check"]);
    assert!(out.status.success(), "{}", stdout(&out));
    assert_eq!(
        stdout(&out),
        format!(
            "{p}:1:1: warning: unknown config key `colour`\nok: {p}\n",
            p = path.display()
        )
    );
}

#[test]
fn key_conflicts_warn_for_a_default_and_fail_for_two_user_keys() {
    let home = Home::new();
    write_config(&home, "[keys]\ndetach = \"n\"\n");
    let out = home.run_deck(&["config", "check"]);
    assert!(out.status.success(), "{}", stdout(&out));
    let text = stdout(&out);
    assert!(text.contains(":2:1: warning: "), "{text}");
    for word in ["detach", "next_panel", "\"n\""] {
        assert!(text.contains(word), "{word}: {text}");
    }

    write_config(&home, "[keys]\ndetach = \"n\"\nnext_panel = \"n\"\n");
    let out = home.run_deck(&["config", "check"]);
    assert_eq!(out.status.code(), Some(1), "{}", stdout(&out));
    let text = stdout(&out);
    assert!(text.contains(": error: "), "{text}");
    for word in ["detach", "next_panel", "\"n\""] {
        assert!(text.contains(word), "{word}: {text}");
    }
}

#[test]
fn the_raw_shapes_all_check_ok() {
    let home = Home::new();
    for text in [
        "[keys]\ndetach = \"d\"\n",
        "[keys]\ndetach = [\"d\", \"q\"]\n",
        "[theme]\naccent = 4\n",
        "[theme]\naccent = \"blue\"\n",
    ] {
        write_config(&home, text);
        let out = home.run_deck(&["config", "check"]);
        assert!(out.status.success(), "{text}: {}", stdout(&out));
        assert!(stdout(&out).starts_with("ok: "), "{text}");
    }
}

#[test]
fn a_user_key_taking_a_default_works_and_an_empty_list_shows_as_unbound() {
    let mut h = Harness::builder((100, 24))
        .config("keys", "{ detach = \"n\" }")
        .deck();
    h.wait_for_text("$", TIMEOUT);
    h.send_bytes(b"\x1b ");
    h.send_keys("n");
    h.wait_for_text("[detached from main]", TIMEOUT);
    h.wait_for_exit(TIMEOUT);

    let mut h = Harness::builder((100, 30))
        .config("keys", "{ detach = [] }")
        .deck();
    h.wait_for_text("$", TIMEOUT);
    h.send_bytes(b"\x1b ");
    h.send_keys("?");
    h.wait_for_text("Detach from the session", TIMEOUT);
    h.capture("help-unbound");
    let screen = h.screen_text();
    let row = screen
        .lines()
        .find(|l| l.contains("Detach from the session"))
        .unwrap();
    assert!(row.contains("unbound"), "{row}");
}

#[test]
fn path_prints_the_resolved_file_and_marks_a_missing_one() {
    let home = Home::new();
    let path = home.path().join("config.toml");
    let out = home.run_deck(&["config", "path"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), format!("{}\n", path.display()));

    fs::remove_file(&path).unwrap();
    let out = home.run_deck(&["config", "path"]);
    assert_eq!(stdout(&out), format!("{} (not found)\n", path.display()));

    let xdg = tempfile::tempdir().unwrap();
    let xdg_path = xdg.path().to_string_lossy().into_owned();
    let home = Home::with(
        &[],
        &[("DECK_HOME", None), ("XDG_CONFIG_HOME", Some(&xdg_path))],
    );
    let out = home.run_deck(&["config", "path"]);
    assert_eq!(
        stdout(&out),
        format!("{xdg_path}/deck/config.toml (not found)\n")
    );
    assert!(!home.daemon_running());
}

#[test]
fn check_without_a_file_uses_the_defaults() {
    let home = Home::new();
    let path = home.path().join("config.toml");
    fs::remove_file(&path).unwrap();
    let out = home.run_deck(&["config", "check"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        format!("no config file at {}; using defaults\n", path.display())
    );
    assert!(!home.daemon_running());
}
