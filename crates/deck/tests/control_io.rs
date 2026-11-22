//! The terminal I/O commands end to end: the real `deck` in a PTY, driven from outside with
//! `deck send`, `send-keys`, `capture` and `wait`.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use std::io::Write;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;
use support::{Harness, Home, TIMEOUT, stderr, stdout};

const SIZE: (u16, u16) = (120, 30);

/// Shows the bytes a program receives, and turns the terminal's own handling off.
const SHOW_BYTES: &str = "stty -echo -icanon -icrnl; echo ready; exec cat -v";

fn attached_with(config: &[(&str, &str)]) -> Harness {
    let mut builder = Harness::builder(SIZE).config_unset("sidebar");
    for (key, value) in config {
        builder = builder.config(key, value);
    }
    let h = builder.deck();
    h.wait_for_text("$", TIMEOUT);
    h
}

fn attached() -> Harness {
    attached_with(&[])
}

fn deck(home: &Home, args: &[&str]) -> Output {
    home.run_deck(args)
}

fn ok(home: &Home, args: &[&str]) -> String {
    let out = deck(home, args);
    assert!(
        out.status.success(),
        "deck {args:?} failed: {}{}",
        stderr(&out),
        stdout(&out)
    );
    stdout(&out).trim_end().to_string()
}

fn fails(home: &Home, args: &[&str]) -> String {
    let out = deck(home, args);
    assert!(!out.status.success(), "deck {args:?} should fail");
    stderr(&out).trim().to_string()
}

/// A new terminal running `program`; its id.
fn terminal(home: &Home, extra: &[&str], program: &[&str]) -> String {
    let mut args = vec!["new-terminal"];
    args.extend_from_slice(extra);
    args.push("--");
    args.extend_from_slice(program);
    ok(home, &args)
}

fn script(home: &Home, extra: &[&str], script: &str) -> String {
    terminal(home, extra, &["sh", "-c", script])
}

fn first_terminal(home: &Home) -> String {
    let tree: Value = serde_json::from_str(&ok(home, &["ls", "--json"])).unwrap();
    tree["sessions"][0]["projects"][0]["panels"][0]["terminals"][0]["id"]
        .as_u64()
        .unwrap()
        .to_string()
}

/// Waits for `text` on a terminal's screen, as the user of the control API does.
fn shows(home: &Home, id: &str, text: &str) -> String {
    ok(
        home,
        &["wait", "--terminal", id, "--text", text, "--timeout", "5"],
    )
}

fn quiet(home: &Home, id: &str) {
    ok(
        home,
        &["wait", "--terminal", id, "--idle", "300", "--timeout", "5"],
    );
}

fn lines(text: &str) -> Vec<&str> {
    text.lines().collect()
}

#[test]
fn text_typed_into_another_terminal_runs_and_capture_shows_it_with_the_prompt() {
    let h = attached();
    let home = h.home();
    let id = terminal(home, &[], &["sh", "-i"]);
    quiet(home, &id);
    ok(
        home,
        &["send", "--terminal", &id, "--enter", "echo $((40+2))"],
    );
    assert_eq!(shows(home, &id, "42"), "42");
    quiet(home, &id);
    let screen = ok(home, &["capture", "--terminal", &id]);
    let rows = lines(&screen);
    assert!(rows.contains(&"42"), "{screen}");
    assert!(rows.last().unwrap().ends_with('$'), "{screen}");
    // The first terminal did not get it.
    let first = ok(home, &["capture", "--terminal", &first_terminal(home)]);
    assert!(!lines(&first).contains(&"42"), "{first}");
    h.capture("send-to-another-terminal");
}

#[test]
fn a_terminal_never_types_into_itself_by_accident() {
    let h = attached();
    let home = h.home();
    let own = first_terminal(home);
    let tree: Value = serde_json::from_str(&ok(home, &["ls", "--json"])).unwrap();
    let session = tree["sessions"][0]["id"].as_u64().unwrap().to_string();
    let panel = tree["sessions"][0]["projects"][0]["panels"][0]["id"]
        .as_u64()
        .unwrap()
        .to_string();
    // The environment of a program running in the first terminal.
    let inside = |args: &[&str]| -> Output {
        let mut cmd: Command = home.deck_command(args);
        cmd.env("DECK_SESSION_ID", &session)
            .env("DECK_PANEL_ID", &panel)
            .env("DECK_TERMINAL_ID", &own);
        cmd.output().unwrap()
    };
    let refuse = "refusing to act on your own terminal; pass --terminal <id>";
    for args in [
        &["send", "x"][..],
        &["send-keys", "a"][..],
        &["wait", "--idle", "10"][..],
        &["wait", "--text", "x", "--timeout", "1"][..],
        &["wait", "--exit"][..],
    ] {
        let out = inside(args);
        assert!(
            !out.status.success() && stderr(&out).trim() == refuse,
            "{args:?}: {}",
            stderr(&out)
        );
    }
    // Named explicitly it is allowed, except waiting for its exit.
    let named = inside(&["send", "--terminal", &own, "--enter", "echo $((40+2))"]);
    assert!(named.status.success(), "{}", stderr(&named));
    h.wait_for_text("42", TIMEOUT);
    let out = inside(&["wait", "--terminal", &own, "--exit"]);
    assert_eq!(
        stderr(&out).trim(),
        "cannot wait for your own terminal to exit"
    );
    // Capture reads the caller's own screen by default.
    let own_screen = inside(&["capture"]);
    assert!(own_screen.status.success());
    assert!(
        stdout(&own_screen).contains("42"),
        "{}",
        stdout(&own_screen)
    );
}

#[test]
fn a_paste_to_a_program_with_bracketed_paste_on_arrives_wrapped() {
    let h = attached();
    let home = h.home();
    let id = script(home, &[], &format!("printf '\\033[?2004h'; {SHOW_BYTES}"));
    shows(home, &id, "ready");
    let mut child = home
        .deck_command(&["send", "--terminal", &id, "--paste", "--enter", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"one\ntwo").unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    shows(home, &id, "^M");
    let screen = ok(home, &["capture", "--terminal", &id]);
    assert!(
        lines(&screen).ends_with(&["^[[200~one", "two^[[201~^M"]),
        "{screen}"
    );
    h.capture("paste");

    // Two MiB are too much.
    let mut child = home
        .deck_command(&["send", "--terminal", &id, "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || {
        // The command may stop reading once it has seen enough.
        let _ = stdin.write_all(&vec![b'x'; 2 * 1024 * 1024]);
    });
    let out = child.wait_with_output().unwrap();
    writer.join().unwrap();
    assert!(!out.status.success());
    assert_eq!(stderr(&out).trim(), "input too large");
}

#[test]
fn keys_interrupt_a_command_and_follow_the_terminals_modes() {
    let h = attached();
    let home = h.home();
    let id = terminal(home, &[], &["sh", "-i"]);
    quiet(home, &id);
    ok(home, &["send", "--terminal", &id, "--enter", "sleep 100"]);
    quiet(home, &id);
    ok(home, &["send-keys", "--terminal", &id, "C-c"]);
    quiet(home, &id);
    ok(
        home,
        &["send", "--terminal", &id, "--enter", "echo $((40+2))"],
    );
    assert_eq!(shows(home, &id, "42"), "42");

    // A key that does not parse is named, and nothing is sent: not even the `a` before it.
    quiet(home, &id);
    let err = fails(home, &["send-keys", "--terminal", &id, "a", "X-q"]);
    assert!(err.contains("X-q"), "{err}");
    quiet(home, &id);
    let screen = ok(home, &["capture", "--terminal", &id]);
    assert!(lines(&screen).last().unwrap().ends_with('$'), "{screen}");

    // Application cursor mode changes what `Up` is.
    let app = script(home, &[], &format!("printf '\\033[?1h'; {SHOW_BYTES}"));
    let normal = script(home, &[], SHOW_BYTES);
    for id in [&app, &normal] {
        shows(home, id, "ready");
        ok(home, &["send-keys", "--terminal", id, "Up", "Enter"]);
    }
    assert_eq!(shows(home, &app, "^M"), "^[OA^M");
    assert_eq!(shows(home, &normal, "^M"), "^[[A^M");
}

#[test]
fn typing_into_an_exited_kept_terminal_fails() {
    let h = attached();
    let home = h.home();
    let id = script(home, &["--keep"], "exit 4");
    assert_eq!(ok(home, &["wait", "--terminal", &id, "--exit"]), "4");
    let err = format!("terminal {id} has exited");
    assert_eq!(fails(home, &["send", "--terminal", &id, "x"]), err);
    assert_eq!(fails(home, &["send-keys", "--terminal", &id, "Enter"]), err);
    // It can still be read.
    ok(home, &["capture", "--terminal", &id]);
}

#[test]
fn wait_exit_reports_the_exit_code_as_output_with_or_without_keep() {
    let h = attached();
    let home = h.home();
    for extra in [&["--keep"][..], &[][..]] {
        let id = script(home, extra, "sleep 1; exit 4");
        let started = Instant::now();
        let out = deck(home, &["wait", "--terminal", &id, "--exit"]);
        let took = started.elapsed();
        assert!(out.status.success(), "{}", stderr(&out));
        assert_eq!(stdout(&out).trim(), "4");
        assert!(took >= Duration::from_millis(800), "{took:?}");
        assert!(took < Duration::from_secs(5), "{took:?}");
        let json = ok(home, &["wait", "--terminal", &id, "--exit", "--json"]);
        assert_eq!(json, r#"{"exit":4}"#);
        if extra.is_empty() {
            // The terminal is gone; its code is still known for a while.
            std::thread::sleep(Duration::from_secs(5));
            assert_eq!(ok(home, &["wait", "--terminal", &id, "--exit"]), "4");
        }
    }
    // An exit code of 124 is output, not a timeout; so is 1.
    for code in ["124", "1"] {
        let id = script(home, &[], &format!("sleep 0.3; exit {code}"));
        let out = deck(home, &["wait", "--terminal", &id, "--exit"]);
        assert!(out.status.success());
        assert_eq!(stdout(&out).trim(), code);
    }
}

#[test]
fn wait_text_returns_when_the_text_appears_or_is_there_and_times_out_with_124() {
    let h = attached();
    let home = h.home();
    let id = script(home, &[], "sleep 1; echo ready; sleep 60");
    let started = Instant::now();
    assert_eq!(shows(home, &id, "ready"), "ready");
    assert!(started.elapsed() >= Duration::from_millis(800));
    // Already on screen: at once.
    let again = Instant::now();
    assert_eq!(shows(home, &id, "ready"), "ready");
    assert!(again.elapsed() < Duration::from_secs(2));

    let never = Instant::now();
    let out = deck(
        home,
        &[
            "wait",
            "--terminal",
            &id,
            "--text",
            "no such text",
            "--timeout",
            "5",
        ],
    );
    let took = never.elapsed();
    assert_eq!(out.status.code(), Some(124));
    assert_eq!(stderr(&out).trim(), "timed out");
    assert!(
        took >= Duration::from_secs(5) && took < Duration::from_secs(15),
        "{took:?}"
    );
}

#[test]
fn wait_idle_answers_after_the_output_stopped() {
    let h = attached();
    let home = h.home();
    let id = script(home, &[], "sleep 1; echo busy; sleep 60");
    shows(home, &id, "busy");
    let started = Instant::now();
    let out = deck(home, &["wait", "--terminal", &id, "--idle", "500"]);
    let took = started.elapsed();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        took >= Duration::from_millis(450) && took < Duration::from_secs(3),
        "{took:?}"
    );
    assert_eq!(
        ok(home, &["wait", "--terminal", &id, "--idle", "0", "--json"]),
        r#"{"idle":true}"#
    );
}

#[test]
fn a_pending_wait_ends_with_an_error_when_its_terminal_or_the_daemon_goes() {
    let h = attached();
    let home = h.home();
    let spawn_wait = |id: &str| {
        let child = home
            .deck_command(&["wait", "--terminal", id, "--text", "never appears"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // Give it time to reach the daemon.
        std::thread::sleep(Duration::from_secs(1));
        child
    };
    let id = terminal(home, &[], &["sleep", "60"]);
    let child = spawn_wait(&id);
    ok(home, &["close", &id]);
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stderr(&out).trim(), format!("terminal {id} was closed"));

    let id = terminal(home, &[], &["sleep", "60"]);
    let child = spawn_wait(&id);
    ok(home, &["kill-server"]);
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stderr(&out).trim(), "deck daemon stopped");
}

#[test]
fn capture_prints_the_history_and_ignores_it_on_the_alternate_screen() {
    let h = attached();
    let home = h.home();
    let id = script(home, &[], "seq 1 100; exec sh -i");
    shows(home, &id, "$");
    quiet(home, &id);
    let tail = ok(
        home,
        &["capture", "--terminal", &id, "--history", "--lines", "3"],
    );
    let rows = lines(&tail);
    assert_eq!(&rows[..2], ["99", "100"], "{tail}");
    assert!(rows[2].ends_with('$') && rows.len() == 3, "{tail}");
    let all = ok(home, &["capture", "--terminal", &id, "--history"]);
    assert_eq!(lines(&all)[0], "1");

    let file = h.home().path().join("pager.txt");
    std::fs::write(
        &file,
        (1..=200)
            .map(|n| format!("pager line {n}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let pager = terminal(home, &[], &["less", file.to_str().unwrap()]);
    shows(home, &pager, "pager line 1");
    let json: Value = serde_json::from_str(&ok(
        home,
        &["capture", "--terminal", &pager, "--history", "--json"],
    ))
    .unwrap();
    assert_eq!(json["alternate_screen"], true);
    assert_eq!(json["terminal"].as_u64().unwrap().to_string(), pager);
    assert!(json["lines"].as_array().unwrap().len() <= json["rows"].as_u64().unwrap() as usize);
    assert_eq!(json["lines"][0], "pager line 1");
    h.capture("less-and-history");
}

#[test]
fn wide_characters_and_combining_marks_are_captured_as_printed() {
    let h = attached();
    let home = h.home();
    let id = script(home, &[], "printf '日本 🙂 e\\314\\201x\\n'; sleep 60");
    shows(home, &id, "日本");
    assert_eq!(
        ok(home, &["capture", "--terminal", &id]),
        "日本 🙂 e\u{301}x"
    );
    let ansi = script(home, &[], "printf 'a\\033[1;31mb\\033[0m\\n'; sleep 60");
    shows(home, &ansi, "ab");
    assert_eq!(
        ok(home, &["capture", "--terminal", &ansi, "--ansi"]),
        "a\x1b[0;1;31mb\x1b[0m"
    );
}

#[test]
fn capture_json_matches_the_plain_output() {
    let h = attached();
    let home = h.home();
    let id = script(home, &[], "echo first; echo second; sleep 60");
    shows(home, &id, "second");
    let plain = ok(home, &["capture", "--terminal", &id]);
    let json: Value =
        serde_json::from_str(&ok(home, &["capture", "--terminal", &id, "--json"])).unwrap();
    let json_lines: Vec<&str> = json["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap())
        .collect();
    assert_eq!(json_lines, lines(&plain));
    assert_eq!(json_lines, ["first", "second"]);
    assert_eq!(json["truncated"], false);
    assert_eq!(json["alternate_screen"], false);
    assert!(json["cols"].as_u64().unwrap() > 0 && json["rows"].as_u64().unwrap() > 0);
    assert_eq!(json["cursor"]["row"], 2);
    assert_eq!(json["cursor"]["col"], 0);
    // `--ansi` and `--json` do not mix.
    assert!(fails(home, &["capture", "--ansi", "--json"]).contains("--json"));
}

#[test]
fn a_huge_capture_is_capped_and_says_so() {
    let h = attached_with(&[("scrollback_lines", "30000")]);
    let home = h.home();
    let id = script(home, &[], "seq 1 20000; echo finished; sleep 60");
    ok(
        home,
        &[
            "wait",
            "--terminal",
            &id,
            "--text",
            "finished",
            "--timeout",
            "15",
        ],
    );
    let text = ok(home, &["capture", "--terminal", &id, "--history"]);
    let rows = lines(&text);
    assert_eq!(rows[0], "[truncated: 10001 earlier lines]");
    assert_eq!(rows.len(), 10_001);
    assert_eq!(rows.last().copied(), Some("finished"));
    let json: Value = serde_json::from_str(&ok(
        home,
        &["capture", "--terminal", &id, "--history", "--json"],
    ))
    .unwrap();
    assert_eq!(json["truncated"], true);
    assert_eq!(json["lines"].as_array().unwrap().len(), 10_000);
}

#[test]
fn control_io_commands_need_a_running_daemon() {
    let home = Home::new();
    for args in [
        &["send", "--terminal", "1", "x"][..],
        &["send-keys", "--terminal", "1", "Enter"][..],
        &["capture", "--terminal", "1"][..],
        &["wait", "--terminal", "1", "--exit"][..],
    ] {
        assert_eq!(fails(&home, args), "no deck daemon running");
    }
    let err = deck(&home, &["wait", "--terminal", "1"]);
    assert_eq!(err.status.code(), Some(2), "one condition is required");
}
