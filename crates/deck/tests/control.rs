//! The control API end to end: the real `deck` in a PTY, driven from outside with
//! `deck new-terminal`, `ls --json` and the other tree commands.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use std::fs;
use std::time::Duration;

use serde_json::Value;
use support::{Harness, Home, TIMEOUT, stderr, stdout, wait_until};

const SIZE: (u16, u16) = (120, 30);

fn attached() -> Harness {
    let h = Harness::builder(SIZE).config_unset("sidebar").deck();
    h.wait_for_text("$", TIMEOUT);
    h
}

fn prefixed(h: &mut Harness, key: &str) {
    h.send_bytes(b"\x1b ");
    h.send_keys(key);
}

/// Runs `deck <args>`: `(success, stdout, stderr)`.
fn deck(home: &Home, args: &[&str]) -> (bool, String, String) {
    let out = home.run_deck(args);
    (out.status.success(), stdout(&out), stderr(&out))
}

fn ok(home: &Home, args: &[&str]) -> String {
    let (good, out, err) = deck(home, args);
    assert!(good, "deck {args:?} failed: {err}{out}");
    out.trim().to_string()
}

fn id_of(home: &Home, args: &[&str]) -> u64 {
    ok(home, args).parse().unwrap()
}

fn tree(home: &Home) -> Value {
    serde_json::from_str(&ok(home, &["ls", "--json"])).unwrap()
}

fn session<'a>(tree: &'a Value, name: &str) -> &'a Value {
    tree["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == name)
        .unwrap()
}

/// Every terminal of session `main`, flattened.
fn terminals(home: &Home) -> Vec<Value> {
    let t = tree(home);
    session(&t, "main")["projects"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|p| p["panels"].as_array().unwrap().clone())
        .flat_map(|p| p["terminals"].as_array().unwrap().clone())
        .collect()
}

fn terminal(home: &Home, id: u64) -> Option<Value> {
    terminals(home).into_iter().find(|t| t["id"] == id)
}

fn first_panel(home: &Home) -> u64 {
    let t = tree(home);
    session(&t, "main")["projects"][0]["panels"][0]["id"]
        .as_u64()
        .unwrap()
}

fn first_terminal(home: &Home) -> u64 {
    terminals(home)[0]["id"].as_u64().unwrap()
}

/// How many `"keep": true` the saved state has.
fn keep_count(text: &str) -> usize {
    text.split_whitespace()
        .collect::<String>()
        .matches("\"keep\":true")
        .count()
}

fn error_of(home: &Home, args: &[&str]) -> String {
    let (good, _, err) = deck(home, args);
    assert!(!good, "deck {args:?} should fail");
    err.trim().to_string()
}

#[test]
fn a_new_terminal_runs_a_program_without_taking_focus() {
    let h = attached();
    let home = h.home();
    let first = first_terminal(home);
    let id = id_of(
        home,
        &[
            "new-terminal",
            "--",
            "sh",
            "-c",
            r#"printf '\033]7;file://localhost%s\007' "$PWD"; echo ready; sleep 30"#,
        ],
    );
    h.wait_for_text("ready", TIMEOUT);
    wait_until("the directory report", TIMEOUT, || {
        terminal(home, id).is_some_and(|t| t["cwd"].is_string())
    });
    let t = terminal(home, id).unwrap();
    assert_eq!(t["program"], "sh");
    assert!(t["pid"].as_u64().unwrap() > 0);
    assert!(t["cwd"].is_string());
    assert_eq!(t["exited"], Value::Null);
    assert_eq!(t["keep"], false);
    // Focus stayed where it was.
    assert_eq!(terminal(home, first).unwrap()["focused"], true);
    assert_eq!(t["focused"], false);
    h.capture("new-terminal");

    let json = ok(home, &["new-terminal", "--json", "--", "sleep", "30"]);
    assert!(
        serde_json::from_str::<Value>(&json).unwrap()["id"]
            .as_u64()
            .is_some()
    );
}

#[test]
fn ls_shows_the_tree_as_json_text_and_the_old_summary() {
    let h = attached();
    let home = h.home();
    id_of(home, &["new-terminal", "--", "sleep", "30"]);
    let t = tree(home);
    let main = session(&t, "main");
    assert_eq!(main["attached"], true);
    assert_eq!(main["projects"][0]["panels"][0]["name_source"], "default");

    let text = ok(home, &["ls", "--tree"]);
    assert!(text.starts_with("session main ["), "{text}");
    assert!(text.contains("(attached)") && text.contains("  project "));
    assert!(
        text.contains("    panel ") && text.contains("sleep"),
        "{text}"
    );
    h.capture("ls-tree");

    let plain = ok(home, &["ls"]);
    assert!(
        plain.contains("main") && plain.contains("2 terminals"),
        "{plain}"
    );
    let (good, _, err) = deck(home, &["ls", "--json", "--tree"]);
    assert!(!good && !err.is_empty());
}

#[test]
fn a_kept_terminal_stays_with_its_exit_code_and_closes_without_asking() {
    let mut h = attached();
    let home = h.home();
    let id = id_of(
        home,
        &[
            "new-terminal",
            "--keep",
            "--focus",
            "--",
            "sh",
            "-c",
            "exit 3",
        ],
    );
    h.wait_for_text("exited 3", TIMEOUT);
    wait_until("the exit to be reported", TIMEOUT, || {
        terminal(home, id).is_some_and(|t| t["exited"] == 3)
    });
    let t = terminal(home, id).unwrap();
    assert_eq!(t["keep"], true);
    assert_eq!(t["pid"], Value::Null);
    h.capture("exited");
    assert!(ok(home, &["ls", "--tree"]).contains("exited 3"));

    // Closing it needs no confirmation; a running terminal still asks.
    prefixed(&mut h, "x");
    wait_until("the terminal to be gone", TIMEOUT, || {
        terminal(h.home(), id).is_none()
    });
    assert!(!h.screen_text().contains("Close terminal"));
    prefixed(&mut h, "x");
    h.wait_for_text("Close terminal", TIMEOUT);
}

#[test]
fn a_terminal_killed_by_a_signal_reports_128_plus_the_signal() {
    let h = attached();
    let home = h.home();
    let id = id_of(
        home,
        &["new-terminal", "--keep", "--", "sh", "-c", "kill -TERM $$"],
    );
    wait_until("the exit code", TIMEOUT, || {
        terminal(home, id).is_some_and(|t| t["exited"] == 143)
    });
}

#[test]
fn a_program_cannot_override_the_deck_variables() {
    let h = attached();
    let home = h.home();
    let id = id_of(
        home,
        &[
            "new-terminal",
            "--env",
            "DECK_TERMINAL_ID=1",
            "--env",
            "FOO=bar",
            "--",
            "sh",
            "-c",
            "echo id=$DECK_TERMINAL_ID foo=$FOO; sleep 30",
        ],
    );
    h.wait_for_text(&format!("id={id} foo=bar"), TIMEOUT);
}

#[test]
fn panels_and_projects_are_created_focused_renamed_and_closed() {
    let h = attached();
    let home = h.home();
    let old_panel = first_panel(home);
    let panel = id_of(home, &["new-panel", "--name", "tests", "--focus"]);
    wait_until("the panel to show", TIMEOUT, || {
        h.screen_text().contains("tests")
    });
    let t = tree(home);
    let panels = session(&t, "main")["projects"][0]["panels"].clone();
    let made = panels
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == panel)
        .unwrap();
    assert_eq!(made["name"], "tests");
    assert_eq!(made["name_source"], "manual");
    assert_eq!(made["active"], true);

    ok(
        home,
        &["rename-panel", "--panel", &old_panel.to_string(), "build"],
    );
    let t = tree(home);
    let renamed = &session(&t, "main")["projects"][0]["panels"][0];
    assert_eq!(renamed["name"], "build");
    assert_eq!(renamed["name_source"], "manual");

    let project = id_of(home, &["new-project", "api", "--dir", "/tmp"]);
    let t = tree(home);
    let projects = session(&t, "main")["projects"].as_array().unwrap().clone();
    assert_eq!(projects.len(), 2);
    assert_eq!(projects[1]["name"], "api");
    assert_eq!(projects[1]["id"], project);
    h.capture("new-project");

    ok(home, &["close", &panel.to_string()]);
    let t = tree(home);
    assert_eq!(
        session(&t, "main")["projects"][0]["panels"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    ok(home, &["close", &project.to_string()]);
    let t = tree(home);
    assert_eq!(session(&t, "main")["projects"].as_array().unwrap().len(), 1);
}

#[test]
fn focus_from_outside_changes_what_the_client_shows() {
    let h = attached();
    let home = h.home();
    let other = id_of(
        home,
        &["new-terminal", "--", "sh", "-c", "echo second; sleep 30"],
    );
    h.wait_for_text("second", TIMEOUT);
    let first = first_terminal(home);
    ok(home, &["focus", &other.to_string()]);
    wait_until("the focus to move", TIMEOUT, || {
        terminal(home, other).is_some_and(|t| t["focused"] == true)
    });
    assert_eq!(terminal(home, first).unwrap()["focused"], false);
    ok(home, &["focus", &first.to_string()]);
    wait_until("the focus to return", TIMEOUT, || {
        terminal(home, first).is_some_and(|t| t["focused"] == true)
    });
}

#[test]
fn errors_are_plain_text_or_json_and_exit_one() {
    let h = attached();
    let home = h.home();
    assert_eq!(
        error_of(home, &["close", "99999"]),
        "99999 is not a terminal, panel or project"
    );
    assert_eq!(
        error_of(home, &["focus", "99999"]),
        "99999 is not a terminal or panel"
    );
    assert!(!error_of(home, &["ls", "--session", "nope", "--tree"]).is_empty());
    assert_eq!(
        error_of(home, &["new-terminal", "--session", "nope"]),
        "no session named \"nope\""
    );
    assert_eq!(
        error_of(home, &["new-panel", "--project", "ghost"]),
        "no project named \"ghost\" in session \"main\""
    );

    let out = home.run_deck(&["close", "--json", "99999"]);
    assert_eq!(out.status.code(), Some(1));
    let v: Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(v["error"], "99999 is not a terminal, panel or project");

    // A stale id from the environment is an error, never replaced.
    let mut cmd = home.deck_command(&["new-terminal"]);
    cmd.env("DECK_PANEL_ID", "99999");
    let out = cmd.output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stderr(&out).trim(), "this panel (99999) no longer exists");
}

#[test]
fn the_session_flag_targets_that_sessions_active_panel() {
    let h = attached();
    let home = h.home();
    ok(home, &["new", "other", "-d"]);
    let id = id_of(
        home,
        &["new-terminal", "--session", "other", "--", "sleep", "30"],
    );
    let t = tree(home);
    let in_other = session(&t, "other")["projects"][0]["panels"][0]["terminals"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["id"] == id);
    assert!(in_other);
}

#[test]
fn without_a_daemon_or_a_session_the_commands_say_so() {
    let home = Home::new();
    // Nothing to list is not an error.
    let sessions = tree(&home);
    assert_eq!(sessions["sessions"].as_array().unwrap().len(), 0);
    assert!(!home.daemon_running(), "a control command started a daemon");
    let (good, _, err) = deck(&home, &["new-terminal"]);
    assert!(!good);
    assert_eq!(err.trim(), "no deck daemon running");
    assert!(!home.daemon_running());

    // A daemon with no session.
    ok(&home, &["new", "tmp", "-d"]);
    ok(&home, &["kill-session", "tmp"]);
    // The daemon leaves with its last session, so it may already be gone.
    let text = error_of(&home, &["new-terminal"]);
    assert!(
        text == "no session" || text == "no deck daemon running",
        "{text}"
    );
    let _ = deck(&home, &["kill-server"]);
}

#[test]
fn a_full_panel_refuses_another_terminal() {
    let h = attached();
    let home = h.home();
    let mut refused = None;
    for _ in 0..80 {
        let (good, _, err) = deck(home, &["new-terminal", "--", "sleep", "30"]);
        if !good {
            refused = Some(err.trim().to_string());
            break;
        }
    }
    let text = refused.expect("a room check");
    assert!(text.contains("room") || text.contains("fit"), "{text}");
}

#[test]
fn kept_terminals_are_saved_and_the_exited_one_is_gone_after_a_restart() {
    let mut h = attached();
    let path = h.home().path().to_string_lossy().into_owned();
    let home = h.home();
    let running = id_of(
        home,
        &["new-terminal", "--keep", "--", "sh", "-c", "sleep 60"],
    );
    let spent = id_of(
        home,
        &["new-terminal", "--keep", "--", "sh", "-c", "exit 3"],
    );
    wait_until("the exit", TIMEOUT, || {
        terminal(home, spent).is_some_and(|t| t["exited"] == 3)
    });
    let state = home.path().join("state").join("state.json");
    wait_until("the state to be saved", Duration::from_secs(5), || {
        fs::read_to_string(&state).is_ok_and(|s| keep_count(&s) >= 2)
    });
    let text = fs::read_to_string(&state).unwrap();
    assert!(keep_count(&text) >= 2, "{text}");

    ok(home, &["kill-server"]);
    h.wait_for_exit(TIMEOUT);
    let again = Harness::builder(SIZE)
        .config_unset("sidebar")
        .env("DECK_HOME", &path)
        .deck();
    again.wait_for_text("$", TIMEOUT);
    let home = again.home();
    assert!(terminal(home, spent).is_none());
    let kept = terminals(home)
        .into_iter()
        .find(|t| t["program"] == "sh" && t["keep"] == true)
        .unwrap();
    assert_eq!(kept["keep"], true);
    let _ = running;
}

/// `deck <args>` run as the first terminal of `main` would: with its `DECK_*` ids.
fn deck_inside(home: &Home, args: &[&str]) -> (bool, String, String) {
    let t = tree(home);
    let main = session(&t, "main");
    let panel = main["projects"][0]["panels"][0]["id"].as_u64().unwrap();
    let term = main["projects"][0]["panels"][0]["terminals"][0]["id"]
        .as_u64()
        .unwrap();
    let mut cmd = home.deck_command(args);
    cmd.env("DECK_SESSION_ID", main["id"].as_u64().unwrap().to_string())
        .env("DECK_PANEL_ID", panel.to_string())
        .env("DECK_TERMINAL_ID", term.to_string());
    let out = cmd.output().unwrap();
    (out.status.success(), stdout(&out), stderr(&out))
}

#[test]
fn a_caller_inside_deck_acts_on_its_own_panel_unless_a_session_is_named() {
    let h = attached();
    let home = h.home();
    let own_panel = first_panel(home);

    // No --panel: the caller's panel.
    let (good, out, err) = deck_inside(home, &["new-terminal", "--", "sleep", "30"]);
    assert!(good, "{err}");
    let id: u64 = out.trim().parse().unwrap();
    assert_eq!(terminals(home).iter().filter(|t| t["id"] == id).count(), 1);
    let t = tree(home);
    let panel = &session(&t, "main")["projects"][0]["panels"][0];
    assert_eq!(panel["id"], own_panel);
    assert!(
        panel["terminals"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == id)
    );

    let (good, _, err) = deck_inside(home, &["rename-panel", "build"]);
    assert!(good, "{err}");
    let t = tree(home);
    let panel = &session(&t, "main")["projects"][0]["panels"][0];
    assert_eq!(
        (panel["name"].as_str(), panel["name_source"].as_str()),
        (Some("build"), Some("manual"))
    );

    // --session turns the environment off: the other session's active panel.
    ok(home, &["new", "other", "-d"]);
    let (good, out, err) = deck_inside(
        home,
        &["new-terminal", "--session", "other", "--", "sleep", "30"],
    );
    assert!(good, "{err}");
    let id: u64 = out.trim().parse().unwrap();
    let t = tree(home);
    assert!(
        session(&t, "other")["projects"][0]["panels"][0]["terminals"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == id)
    );

    // An id of another session is refused with --session.
    let own = first_terminal(home).to_string();
    let (good, _, err) = deck(home, &["focus", "--session", "other", &own]);
    assert!(!good);
    assert_eq!(err.trim(), format!("{own} is not in session \"other\""));
    let (good, _, err) = deck(home, &["close", "--session", "other", &own]);
    assert!(!good, "{err}");
    assert!(terminal(home, first_terminal(home)).is_some());
}

#[test]
fn ls_shows_a_directory_as_soon_as_a_terminal_starts() {
    let h = attached();
    let home = h.home();
    let id = id_of(
        home,
        &["new-terminal", "--cwd", "/tmp", "--", "sleep", "30"],
    );
    let t = terminal(home, id).unwrap();
    assert!(t["cwd"].as_str().is_some_and(|c| c.ends_with("tmp")), "{t}");
}
