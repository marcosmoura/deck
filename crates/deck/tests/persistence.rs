//! Sessions surviving the daemon, end to end: the real `deck` and its daemon as processes.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

use deck_core::layout::Ratios;
use support::{Harness, Home, TIMEOUT, stderr, stdout, sys, wait_until};

const SIZE: (u16, u16) = (160, 40);
const HANG_UP_PROGRAM: &str = r#"trap "exit 0" HUP; while :; do sleep 1; done"#;
const NEWER_LINE: &str = "state.json was written by a newer deck (version 2); upgrade deck or run \"deck kill-server --forget\"";

// state.json by hand.

fn ratios(n: usize) -> String {
    let r = Ratios::default_for(n);
    let main = r.main.map_or("null".to_string(), |m| m.to_string());
    format!(
        r#"{{"main":{main},"cols":{:?},"rows":{:?}}}"#,
        r.cols, r.rows
    )
}

/// A terminal that is a shell, or a program with arguments: its id and its JSON.
fn terminal(id: u64, program: Option<(&str, &[&str])>) -> (u64, String) {
    let (program, args) = match program {
        Some((p, a)) => (
            format!("{p:?}"),
            format!(
                "[{}]",
                a.iter()
                    .map(|a| format!("{a:?}"))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        ),
        None => ("null".into(), "[]".into()),
    };
    (
        id,
        format!(r#"{{"id":{id},"program":{program},"args":{args},"env":[],"cwd":null}}"#),
    )
}

fn panel(id: u64, name: &str, terminals: &[(u64, String)]) -> String {
    let json: Vec<&str> = terminals.iter().map(|(_, j)| j.as_str()).collect();
    format!(
        r#"{{"id":{id},"name":"{name}","name_source":"Default","focused_terminal":{},"ratios":{},"terminals":[{}]}}"#,
        terminals[0].0,
        ratios(terminals.len()),
        json.join(",")
    )
}

fn project(id: u64, name: &str, root: &Path, first_panel: u64, panels: &[String]) -> String {
    format!(
        r#"{{"id":{id},"name":"{name}","root":{:?},"collapsed":false,"active_panel":{first_panel},"panels":[{}]}}"#,
        root.display().to_string(),
        panels.join(",")
    )
}

fn session(id: u64, name: &str, first_project: u64, projects: &[String]) -> String {
    format!(
        r#"{{"id":{id},"name":"{name}","active_project":{first_project},"projects":[{}]}}"#,
        projects.join(",")
    )
}

fn state(sessions: &[String]) -> String {
    format!(
        r#"{{"version":1,"next_id":100,"last_attached":[],"sessions":[{}]}}"#,
        sessions.join(",")
    )
}

/// Two one-terminal sessions, `one` and `two`.
fn two_sessions(root: &Path) -> String {
    let make = |base: u64, name: &str| {
        session(
            base,
            name,
            base + 1,
            &[project(
                base + 1,
                "p",
                root,
                base + 2,
                &[panel(base + 2, "1", &[terminal(base + 3, None)])],
            )],
        )
    };
    state(&[make(10, "one"), make(20, "two")])
}

/// A session `main` whose panel holds a program that exits on `SIGHUP`, and a shell.
fn hangup_state(root: &Path) -> String {
    state(&[session(
        1,
        "main",
        2,
        &[project(
            2,
            "p",
            root,
            3,
            &[panel(
                3,
                "1",
                &[
                    terminal(4, Some(("/bin/sh", &["-c", HANG_UP_PROGRAM]))),
                    terminal(5, None),
                ],
            )],
        )],
    )])
}

fn state_path(home: &Home) -> std::path::PathBuf {
    home.path().join("state").join("state.json")
}

fn seed(home: &Home, text: &str) {
    let dir = home.path().join("state");
    fs::create_dir_all(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(state_path(home), text).unwrap();
}

fn ls(home: &Home) -> String {
    stdout(&home.run_deck(&["ls"]))
}

/// Starts the daemon (and so restores) without a terminal.
fn start_daemon(home: &Home) {
    let probe = home.run_deck(&["_probe"]);
    assert!(probe.status.success(), "{}", stderr(&probe));
}

fn wait_stopped(home: &Home) {
    wait_until("the daemon to stop", TIMEOUT, || !home.daemon_running());
}

/// `deck` in a PTY that only starts once `release` is called, so a test can seed
/// `state.json` into the home it already has.
fn deck_after_seed(size: (u16, u16)) -> Harness {
    Harness::builder(size).config_unset("sidebar").spawn(&[
        "/bin/sh",
        "-c",
        r#"while [ ! -e "$DECK_HOME/go" ]; do sleep 0.05; done; exec "$0""#,
        env!("CARGO_BIN_EXE_deck"),
    ])
}

fn release(h: &Harness) {
    fs::write(h.home().path().join("go"), b"").unwrap();
}

/// `deck <args>` in a PTY on the home of `first`.
fn deck_in_pty(first: &Harness, args: &[&str], vars: &[(&str, Option<&str>)]) -> Harness {
    let path = first.home().path().to_string_lossy().into_owned();
    let mut argv = vec![env!("CARGO_BIN_EXE_deck")];
    argv.extend_from_slice(args);
    let mut builder = Harness::builder(SIZE)
        .config_unset("sidebar")
        .env("DECK_HOME", &path);
    for (k, v) in vars {
        builder = match v {
            Some(v) => builder.env(k, v),
            None => builder.env_remove(k),
        };
    }
    builder.spawn(&argv)
}

fn prefixed(h: &mut Harness, key: &str) {
    h.send_bytes(b"\x1b ");
    h.send_keys(key);
}

fn add_terminals(h: &mut Harness, up_to: usize) {
    for count in 2..=up_to {
        prefixed(h, "t");
        h.wait_for_text(&format!("╭─ {count} · "), TIMEOUT);
    }
}

#[test]
fn the_sessions_panels_and_grid_come_back_after_kill_server() {
    let mut h = Harness::builder(SIZE).config_unset("sidebar").deck();
    h.wait_for_text("$", TIMEOUT);
    // Panel 1 has one terminal, the second three, a renamed third five. New panels are
    // named after their shell at once.
    prefixed(&mut h, "c");
    wait_until("a second panel", TIMEOUT, || {
        h.is_active_row(4)
            && h.screen_text()
                .lines()
                .nth(4)
                .is_some_and(|r| r.starts_with("   ❯  sh"))
    });
    add_terminals(&mut h, 3);
    prefixed(&mut h, "c");
    wait_until("a third panel", TIMEOUT, || {
        h.is_active_row(5)
            && h.screen_text()
                .lines()
                .nth(5)
                .is_some_and(|r| r.starts_with("   ❯  sh"))
    });
    add_terminals(&mut h, 5);
    prefixed(&mut h, ",");
    h.send_keys("\x15build\r");
    h.wait_for_active_row("   ❯  build", TIMEOUT);
    let other = h.run_deck(&["new", "other", "-d"]);
    assert!(other.status.success(), "{}", stderr(&other));
    wait_until("the grid to settle", TIMEOUT, || {
        let screen = h.screen_text();
        // Every shell has drawn its prompt, or `before` is a half-drawn screen.
        screen.matches('╭').count() >= 5 && screen.matches("│$").count() >= 5
    });
    let before = h.screen_text();
    h.capture("before");

    let stop = h.run_deck(&["kill-server"]);
    assert!(stop.status.success(), "{}", stderr(&stop));
    h.wait_for_exit(TIMEOUT);
    assert!(
        stdout(&h.run_deck(&["ls"]))
            .starts_with("no daemon running; 2 saved sessions will be restored by \"deck\""),
        "{}",
        stdout(&h.run_deck(&["ls"]))
    );

    let again = deck_in_pty(&h, &[], &[]);
    wait_until("the same screen again", TIMEOUT, || {
        again.screen_text() == before
    });
    again.capture("after");
    let listed = ls(h.home());
    assert!(
        listed.contains("main") && listed.contains("other"),
        "{listed}"
    );
    assert!(listed.contains("9 terminals"), "{listed}");
}

fn program_survives(stop: impl Fn(&Home)) {
    let home = Home::new();
    let root = home.path().to_path_buf();
    seed(&home, &hangup_state(&root));
    start_daemon(&home);
    assert!(ls(&home).contains("2 terminals"), "{}", ls(&home));

    stop(&home);
    wait_stopped(&home);
    let text = fs::read_to_string(state_path(&home)).unwrap();
    assert!(text.contains("while :; do sleep 1; done"), "{text}");

    start_daemon(&home);
    assert!(ls(&home).contains("2 terminals"), "{}", ls(&home));
}

#[test]
fn a_program_that_exits_on_hangup_is_kept_by_kill_server() {
    program_survives(|home| {
        assert!(home.run_deck(&["kill-server"]).status.success());
    });
}

#[test]
fn a_program_that_exits_on_hangup_is_kept_by_sigterm() {
    program_survives(|home| sys::signal(home.daemon_pid().unwrap(), libc::SIGTERM));
}

#[test]
fn a_program_that_exits_on_hangup_is_kept_by_sighup() {
    program_survives(|home| sys::signal(home.daemon_pid().unwrap(), libc::SIGHUP));
}

#[test]
fn a_change_from_over_a_second_ago_survives_kill_9() {
    let home = Home::new();
    start_daemon(&home);
    assert!(home.run_deck(&["new", "work", "-d"]).status.success());
    wait_until("the change to be saved", Duration::from_secs(5), || {
        fs::read_to_string(state_path(&home)).is_ok_and(|t| t.contains("work"))
    });
    sys::signal(home.daemon_pid().unwrap(), libc::SIGKILL);
    wait_stopped(&home);
    assert_eq!(
        ls(&home),
        "no daemon running; 2 saved sessions will be restored by \"deck\"\n"
    );
    start_daemon(&home);
    assert!(ls(&home).contains("work"), "{}", ls(&home));
}

#[test]
fn forget_clears_the_state_with_or_without_a_daemon() {
    let home = Home::new();
    // No daemon: the files go and it says so.
    seed(&home, &two_sessions(home.path()));
    fs::write(home.path().join("state").join("state.json.tmp"), b"x").unwrap();
    let out = home.run_deck(&["kill-server", "--forget"]);
    assert_eq!(stdout(&out), "state cleared\n");
    assert!(!state_path(&home).exists());
    assert!(!home.path().join("state").join("state.json.tmp").exists());

    // With a daemon: it stops, saves, and then the state is deleted.
    start_daemon(&home);
    assert!(home.run_deck(&["new", "x", "-d"]).status.success());
    let out = home.run_deck(&["kill-server", "--forget"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(!home.daemon_running());
    assert!(!state_path(&home).exists());
    assert_eq!(ls(&home), "no sessions\n");
    let probe = stdout(&home.run_deck(&["_probe"]));
    assert!(probe.starts_with("main\n"), "{probe}");
}

#[test]
fn a_killed_session_does_not_come_back() {
    let home = Home::new();
    start_daemon(&home);
    for name in ["a", "b"] {
        assert!(home.run_deck(&["new", name, "-d"]).status.success());
    }
    assert!(home.run_deck(&["kill-session", "a"]).status.success());
    assert!(home.run_deck(&["kill-server"]).status.success());
    start_daemon(&home);
    let listed = ls(&home);
    assert!(
        listed.contains('b') && !listed.starts_with("a "),
        "{listed}"
    );
    assert!(!listed.lines().any(|l| l.starts_with("a ")), "{listed}");
}

#[test]
fn ls_without_a_daemon_counts_the_saved_sessions_and_starts_nothing() {
    let home = Home::new();
    assert_eq!(ls(&home), "no sessions\n");
    seed(&home, &two_sessions(home.path()));
    assert_eq!(
        ls(&home),
        "no daemon running; 2 saved sessions will be restored by \"deck\"\n"
    );
    assert!(!home.daemon_running());
    seed(&home, &state(&[]));
    assert_eq!(ls(&home), "no sessions\n");
}

#[test]
fn a_corrupt_state_file_is_set_aside_and_the_client_shows_a_notice() {
    let h = deck_after_seed(SIZE);
    seed(h.home(), "{ not json");
    release(&h);
    h.wait_for_text("previous state could not be read", TIMEOUT);
    h.wait_for_text("$", TIMEOUT);
    h.capture("notice");
    let aside = fs::read_dir(h.home().path().join("state"))
        .unwrap()
        .filter_map(Result::ok)
        .any(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("state.json.bad-")
        });
    assert!(aside);
    assert!(h.screen_text().contains("main"));
}

#[test]
fn a_state_file_from_a_newer_deck_stops_deck_and_stays_as_it_is() {
    let mut h = deck_after_seed(SIZE);
    let text = r#"{"version":2,"sessions":[]}"#;
    seed(h.home(), text);
    release(&h);
    let status = h.wait_for_exit(Duration::from_secs(10));
    assert_eq!(status.exit_code(), 1);
    assert!(h.screen_text().contains(NEWER_LINE), "{}", h.screen_text());
    assert_eq!(fs::read_to_string(state_path(h.home())).unwrap(), text);
    assert!(!h.home().daemon_running());
}

#[test]
fn a_program_that_is_gone_is_reported_in_a_notice() {
    let h = deck_after_seed(SIZE);
    let root = h.home().path().to_path_buf();
    seed(
        h.home(),
        &state(&[session(
            1,
            "main",
            2,
            &[project(
                2,
                "p",
                &root,
                3,
                &[
                    panel(
                        3,
                        "gone",
                        &[terminal(4, Some(("/nonexistent/deck-prog", &[])))],
                    ),
                    panel(5, "kept", &[terminal(6, None)]),
                ],
            )],
        )]),
    );
    release(&h);
    h.wait_for_text("/nonexistent/deck-prog", TIMEOUT);
    h.wait_for_text("$", TIMEOUT);
    h.capture("missing-program");
}

#[test]
fn the_clients_environment_is_not_restored() {
    let mut first = Harness::builder(SIZE)
        .config_unset("sidebar")
        .env("PERSIST_FOO", "1")
        .deck();
    first.wait_for_text("$", TIMEOUT);
    first.send_keys("echo has=[$PERSIST_FOO]\r");
    first.wait_for_text("has=[1]", TIMEOUT);
    let stop = first.run_deck(&["kill-server"]);
    assert!(stop.status.success(), "{}", stderr(&stop));
    first.wait_for_exit(TIMEOUT);
    assert!(
        !fs::read_to_string(state_path(first.home()))
            .unwrap()
            .contains("PERSIST_FOO")
    );

    let mut again = deck_in_pty(&first, &[], &[("PERSIST_FOO", None)]);
    again.wait_for_text("$", TIMEOUT);
    again.send_keys("echo now=[$PERSIST_FOO] id=$DECK_TERMINAL_ID\r");
    again.wait_for_text("now=[] id=", TIMEOUT);
}

#[test]
fn a_daemon_of_another_protocol_version_saves_and_the_next_deck_restores() {
    let home = Home::with(&[], &[("DECK_TEST_PROTOCOL_VERSION", Some("999"))]);
    seed(&home, &two_sessions(home.path()));
    // The old daemon starts (the probe is refused), restoring the file.
    let probe = home.run_deck(&["_probe"]);
    assert_eq!(probe.status.code(), Some(1));
    assert!(home.daemon_running());

    // The current build stops it through `SIGTERM`, which saves.
    let mut kill = home.deck_command(&["kill-server"]);
    kill.env_remove("DECK_TEST_PROTOCOL_VERSION");
    let out = kill.output().unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    wait_stopped(&home);
    assert!(
        fs::read_to_string(state_path(&home))
            .unwrap()
            .contains("\"two\"")
    );

    let mut ls = home.deck_command(&["_probe"]);
    ls.env_remove("DECK_TEST_PROTOCOL_VERSION");
    let out = ls.output().unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let mut list = home.deck_command(&["ls"]);
    list.env_remove("DECK_TEST_PROTOCOL_VERSION");
    let listed = stdout(&list.output().unwrap());
    assert!(listed.contains("one") && listed.contains("two"), "{listed}");
}

// Working directories.

/// The `"cwd"` field `state.json` holds for a path.
fn cwd_json(path: &Path) -> String {
    format!(r#""cwd":{:?}"#, path.display().to_string())
}

fn wait_saved_cwd(home: &Home, path: &Path) {
    let field = cwd_json(path);
    wait_until("the directory to be saved", Duration::from_secs(5), || {
        saved_has_field(home, &field)
    });
}

/// The file is pretty-printed: compare without whitespace.
fn saved_has_field(home: &Home, field: &str) -> bool {
    fs::read_to_string(state_path(home)).is_ok_and(|t| {
        let compact: String = t.chars().filter(|c| !c.is_whitespace()).collect();
        compact.contains(field)
    })
}

fn saved_has_cwd(home: &Home, path: &Path) -> bool {
    saved_has_field(home, &cwd_json(path))
}

const REPORT: &str = r#"printf '\033]7;file://localhost%s\007' "$PWD""#;

/// Types a line that prints `marker` when the shell is in `dir`, and waits for it.
fn expect_in(h: &mut Harness, dir: &Path, marker: &str) {
    let dir = fs::canonicalize(dir).unwrap();
    h.send_keys(&format!(
        "[ \"$(pwd -P)\" = \"{}\" ] && echo {marker}-$((6*7))\r",
        dir.display()
    ));
    h.wait_for_text(&format!("{marker}-42"), TIMEOUT);
}

#[test]
fn a_shell_that_reports_osc7_comes_back_in_its_directory() {
    let mut h = Harness::builder(SIZE).config_unset("sidebar").deck();
    h.wait_for_text("$", TIMEOUT);
    h.send_keys(&format!("cd /tmp && {REPORT}\r"));
    wait_saved_cwd(h.home(), Path::new("/tmp"));

    // A long command elsewhere does not move what the shell reported.
    h.send_keys("(cd /var && sleep 1); echo job-$((1+1))\r");
    h.wait_for_text("job-2", TIMEOUT);
    assert!(saved_has_cwd(h.home(), Path::new("/tmp")));

    assert!(h.run_deck(&["kill-server"]).status.success());
    h.wait_for_exit(TIMEOUT);
    let mut again = deck_in_pty(&h, &[], &[]);
    again.wait_for_text("$", TIMEOUT);
    expect_in(&mut again, Path::new("/tmp"), "restored");
    again.capture("restored-in-tmp");
}

#[test]
fn a_shell_without_osc7_is_followed_through_the_os() {
    let mut h = Harness::builder(SIZE)
        .config_unset("sidebar")
        .env("DECK_TEST_CWD_POLL_MS", "200")
        .deck();
    h.wait_for_text("$", TIMEOUT);
    h.send_keys("cd /tmp\r");
    let tmp = fs::canonicalize("/tmp").unwrap();
    wait_saved_cwd(h.home(), &tmp);

    // A foreground job's own directory is not the shell's.
    h.send_keys("(cd /var && sleep 1); echo job-$((1+1))\r");
    h.wait_for_text("job-2", TIMEOUT);
    assert!(saved_has_cwd(h.home(), &tmp));

    assert!(h.run_deck(&["kill-server"]).status.success());
    h.wait_for_exit(TIMEOUT);
    let mut again = deck_in_pty(&h, &[], &[]);
    again.wait_for_text("$", TIMEOUT);
    expect_in(&mut again, &tmp, "restored");
}

#[test]
fn a_new_terminal_opens_in_the_directory_of_the_focused_one() {
    let mut h = Harness::builder(SIZE)
        .config_unset("sidebar")
        .env("DECK_TEST_CWD_POLL_MS", "200")
        .deck();
    h.wait_for_text("$", TIMEOUT);
    h.send_keys("cd /tmp\r");
    let tmp = fs::canonicalize("/tmp").unwrap();
    wait_saved_cwd(h.home(), &tmp);
    prefixed(&mut h, "t");
    h.wait_for_text("╭─ 2 · ", TIMEOUT);
    expect_in(&mut h, &tmp, "inherited");
    h.capture("new-terminal-in-tmp");
}

#[test]
fn a_directory_reported_two_seconds_before_kill_9_is_restored() {
    let mut h = Harness::builder(SIZE).config_unset("sidebar").deck();
    h.wait_for_text("$", TIMEOUT);
    h.send_keys(&format!("cd /tmp && {REPORT}\r"));
    wait_saved_cwd(h.home(), Path::new("/tmp"));
    std::thread::sleep(Duration::from_secs(2));
    sys::signal(h.home().daemon_pid().unwrap(), libc::SIGKILL);
    h.wait_for_exit(TIMEOUT);
    wait_stopped(h.home());

    let mut again = deck_in_pty(&h, &[], &[]);
    again.wait_for_text("$", TIMEOUT);
    expect_in(&mut again, Path::new("/tmp"), "restored");
}
