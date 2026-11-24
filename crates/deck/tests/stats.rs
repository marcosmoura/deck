//! `deck stats` and the stats log line, with the real binaries.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use std::fs;
use std::time::{Duration, Instant};

use serde_json::Value;
use support::{Harness, Home, TIMEOUT, stderr, stdout, wait_until};

const SIZE: (u16, u16) = (120, 30);

fn attached() -> Harness {
    let h = Harness::builder(SIZE).deck();
    h.wait_for_text("$", TIMEOUT);
    h
}

fn stats_json(home: &Home) -> Value {
    let out = home.run_deck(&["stats", "--json"]);
    assert!(out.status.success(), "{}", stderr(&out));
    serde_json::from_str(&stdout(&out)).unwrap()
}

fn new_terminal(home: &Home, program: &[&str]) -> String {
    let mut args = vec!["new-terminal", "--"];
    args.extend_from_slice(program);
    let out = home.run_deck(&args);
    assert!(out.status.success(), "{}", stderr(&out));
    stdout(&out).trim().to_string()
}

#[test]
fn without_a_daemon_stats_says_so_and_starts_none() {
    let home = Home::new();
    let out = home.run_deck(&["stats"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "no deck daemon running\n");
    let json = home.run_deck(&["stats", "--json"]);
    assert!(json.status.success());
    assert_eq!(stdout(&json).trim(), r#"{"running":false}"#);
    assert!(!home.daemon_running());
}

#[test]
fn stats_show_the_daemon_in_text_and_json() {
    let h = attached();
    let home = h.home();
    let out = home.run_deck(&["stats"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].starts_with("deck daemon pid "), "{text}");
    assert!(lines[0].contains("(protocol "), "{text}");
    assert!(
        lines[1].starts_with("memory ") && lines[1].contains(" MiB  fds "),
        "{text}"
    );
    assert_eq!(lines[2], "1 session, 1 terminal, 1 client", "{text}");
    assert_eq!(lines[4].split_whitespace().next(), Some("terminal"));

    let json = stats_json(home);
    assert_eq!(json["running"], true);
    assert_eq!(json["pid"], home.daemon_pid().unwrap());
    assert_eq!(json["clients"], 1);
    assert!(json["rss_bytes"].as_u64().unwrap() > 1 << 20);
    assert!(json["cpu_user_secs"].as_f64().is_some());
    assert_eq!(json["terminal_list"][0]["session"], "main");
}

#[test]
fn opening_and_closing_terminals_moves_descriptors_and_threads_and_back() {
    let h = attached();
    let home = h.home();
    let figures = |home: &Home| {
        let j = stats_json(home);
        (j["fds"].as_u64().unwrap(), j["threads"].as_u64().unwrap())
    };
    let (fds, threads) = figures(home);
    let ids: Vec<String> = (0..5)
        .map(|_| new_terminal(home, &["sleep", "60"]))
        .collect();
    let (more_fds, more_threads) = figures(home);
    assert!(more_fds > fds, "{more_fds} fds after {fds}");
    assert!(
        more_threads > threads,
        "{more_threads} threads after {threads}"
    );

    for id in &ids {
        assert!(home.run_deck(&["close", id]).status.success());
    }
    // Tokio's blocking pool keeps an idle thread for 10 seconds.
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let (f, t) = figures(home);
        if f.abs_diff(fds) <= 2 && t.abs_diff(threads) <= 2 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "fds {f} (was {fds}), threads {t} (was {threads})"
        );
        std::thread::sleep(Duration::from_millis(250));
    }
}

#[test]
fn a_terminal_that_printed_a_mebibyte_shows_it_as_read() {
    let h = attached();
    let home = h.home();
    let id: u64 = new_terminal(
        home,
        &[
            "sh",
            "-c",
            "head -c 1048576 /dev/zero | tr '\\0' x; sleep 60",
        ],
    )
    .parse()
    .unwrap();
    wait_until("1 MiB read", Duration::from_secs(10), || {
        stats_json(home)["terminal_list"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == id && t["bytes_read"].as_u64().unwrap() >= 1 << 20)
    });
    let text = stdout(&home.run_deck(&["stats"]));
    assert!(text.contains(" MiB\n"), "{text}");
}

#[test]
fn the_daemon_logs_stats_on_a_timer_and_once_more_when_it_stops() {
    let home = Home::with(&[], &[("DECK_TEST_STATS_LOG_MS", Some("200"))]);
    assert!(home.run_deck(&["_probe"]).status.success());
    let count = |home: &Home| {
        fs::read_to_string(home.log_file())
            .unwrap()
            .lines()
            .filter(|l| l.contains("stats: rss="))
            .count()
    };
    // The first lines can come before the probe's session exists, so wait for one that
    // counts it.
    let counted = |home: &Home| {
        fs::read_to_string(home.log_file())
            .unwrap()
            .lines()
            .rfind(|l| l.contains("stats: rss=") && l.contains(" sessions=1 "))
            .map(str::to_owned)
    };
    wait_until("two stats lines", TIMEOUT, || {
        count(&home) >= 2 && counted(&home).is_some()
    });
    let line = counted(&home).unwrap();
    for field in [
        " fds=",
        " threads=",
        " cpu=",
        " sessions=1 terminals=1 clients=0",
    ] {
        assert!(line.contains(field), "{line}");
    }
    assert!(home.run_deck(&["kill-server"]).status.success());
    let log = fs::read_to_string(home.log_file()).unwrap();
    let lines: Vec<&str> = log.lines().collect();
    let last = lines
        .iter()
        .rposition(|l| l.contains("stats: rss="))
        .unwrap();
    let asked = lines
        .iter()
        .position(|l| l.contains("shutting down: kill-server"))
        .unwrap();
    assert!(
        last > asked,
        "no stats line after the shutdown request:\n{log}"
    );
}

#[test]
fn the_daemon_logs_one_stats_line_at_shutdown_without_the_timer() {
    let home = Home::new();
    assert!(home.run_deck(&["_probe"]).status.success());
    assert!(home.run_deck(&["kill-server"]).status.success());
    let log = fs::read_to_string(home.log_file()).unwrap();
    assert_eq!(
        log.lines().filter(|l| l.contains("stats: rss=")).count(),
        1,
        "{log}"
    );
}
