//! The live upgrade's handoff end to end: the daemon re-executes itself as another copy of
//! the binary and the terminals go on as if nothing happened. Each test starts a real
//! daemon, writes `deck.upgrade` and sends `SIGUSR2`, as `deck upgrade` will.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use deck_core::protocol::{
    ClientMsg, FrameReader, PROTOCOL_VERSION, Request, Response, Role, ServerMsg, encode,
};
use deck_daemon::Marker;
use serde_json::Value;
use support::{Harness, Home, TIMEOUT, stderr, stdout, wait_until};

const LIMIT: Duration = Duration::from_secs(20);

fn ok(home: &Home, args: &[&str]) -> String {
    let out = home.run_deck(args);
    assert!(
        out.status.success(),
        "deck {args:?} failed: {}{}",
        stderr(&out),
        stdout(&out)
    );
    stdout(&out).trim_end().to_string()
}

/// A daemon with a session and no terminal besides the session's first.
fn started(home: &Home) {
    ok(home, &["new", "main", "-d"]);
}

fn script(home: &Home, text: &str) -> String {
    ok(home, &["new-terminal", "--", "sh", "-c", text])
}

fn shows(home: &Home, id: &str, text: &str) -> String {
    ok(
        home,
        &["wait", "--terminal", id, "--text", text, "--timeout", "20"],
    )
}

fn marker_file(home: &Home) -> PathBuf {
    home.path().join("run").join("deck.handoff")
}

fn marker(home: &Home) -> Option<Marker> {
    Marker::read(&marker_file(home))
}

/// A copy of the binary: another file for the daemon to hand over to.
fn copy_of_deck(home: &Home, name: &str) -> PathBuf {
    let to = home.path().join(name);
    fs::copy(env!("CARGO_BIN_EXE_deck"), &to).unwrap();
    fs::set_permissions(&to, fs::Permissions::from_mode(0o755)).unwrap();
    to
}

fn request(home: &Home, content: &str) {
    fs::write(home.path().join("run").join("deck.upgrade"), content).unwrap();
    support::sys::signal(home.daemon_pid().unwrap(), libc::SIGUSR2);
}

fn upgrade_to(home: &Home, exe: &std::path::Path) {
    request(home, &format!("{}\n", exe.display()));
}

/// Waits until the marker says the daemon adopted `generation` handoffs.
fn adopted(home: &Home, generation: u64) -> Marker {
    let mut seen = None;
    wait_until("the handoff to finish", LIMIT, || {
        seen = marker(home);
        seen.as_ref()
            .is_some_and(|m| m.generation >= generation && !m.upgrading)
    });
    seen.unwrap()
}

fn result(home: &Home) -> String {
    let path = home.path().join("run").join("deck.upgrade.result");
    wait_until("the result file", LIMIT, || path.exists());
    fs::read_to_string(path).unwrap().trim().to_string()
}

fn log(home: &Home) -> String {
    fs::read_to_string(home.log_file()).unwrap_or_default()
}

fn first_terminal(home: &Home) -> String {
    let tree: Value = serde_json::from_str(&ok(home, &["ls", "--json"])).unwrap();
    tree["sessions"][0]["spaces"][0]["panels"][0]["terminals"][0]["id"]
        .as_u64()
        .unwrap()
        .to_string()
}

/// The number a shell printed after `prefix` (`echo <prefix>$$`), once it did.
fn shell_pid(home: &Home, id: &str, prefix: &str) -> u32 {
    let mut found = None;
    wait_until("the shell's answer", LIMIT, || {
        let screen = ok(home, &["capture", "--terminal", id, "--history"]);
        found = screen
            .lines()
            .rev()
            .find_map(|l| l.trim().strip_prefix(prefix)?.parse().ok());
        found.is_some()
    });
    found.unwrap()
}

/// A home whose daemon waits this long (milliseconds) before freezing and once frozen.
fn slow_home(prefreeze: u64, frozen: u64) -> Home {
    let (before, after) = (prefreeze.to_string(), frozen.to_string());
    Home::with(
        &[],
        &[
            ("DECK_TEST_PREFREEZE_DELAY_MS", Some(&before)),
            ("DECK_TEST_HANDOFF_DELAY_MS", Some(&after)),
        ],
    )
}

/// Waits until the marker says a handoff is under way.
fn upgrading(home: &Home) {
    wait_until("the handoff to start", LIMIT, || {
        marker(home).is_some_and(|m| m.upgrading)
    });
}

/// A control connection to the daemon, to ask for things during the handoff.
struct Conn(UnixStream);

impl Conn {
    fn open(home: &Home) -> Self {
        let mut stream = UnixStream::connect(home.socket()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(20)))
            .unwrap();
        let hello = ClientMsg::Hello {
            version: PROTOCOL_VERSION,
            role: Role::Control,
            cwd: std::env::temp_dir(),
            env: Vec::new(),
            colors: None,
        };
        stream.write_all(&encode(&hello).unwrap()).unwrap();
        Self(stream)
    }

    /// Sends `KillServer` and waits for the answer.
    fn kill_server(&mut self) -> Response {
        let request = ClientMsg::Request(Request::KillServer);
        self.0.write_all(&encode(&request).unwrap()).unwrap();
        let mut reader = FrameReader::new();
        let mut buf = [0u8; 4096];
        loop {
            while let Some(msg) = reader.next_message::<ServerMsg>().unwrap() {
                if let ServerMsg::Response(r) = msg {
                    return r;
                }
            }
            let n = self.0.read(&mut buf).unwrap();
            assert!(n > 0, "the daemon hung up without an answer");
            reader.push(&buf[..n]);
        }
    }
}

/// A terminal that writes `<home>/hup` when it is hung up, as a normal shutdown does.
fn hup_trap(home: &Home) -> (String, PathBuf) {
    let file = home.path().join("hup");
    let id = script(
        home,
        &format!(
            "trap 'echo hup > {}; exit 0' HUP; echo trap-ready; while :; do sleep 0.1; done",
            file.display()
        ),
    );
    shows(home, &id, "trap-ready");
    (id, file)
}

/// The position of `needle` in the log, after `from`.
fn after(log: &str, from: usize, needle: &str) -> usize {
    let found = log[from..].find(needle);
    assert!(
        found.is_some(),
        "no `{needle}` after byte {from} in:\n{log}"
    );
    from + found.unwrap()
}

#[test]
fn the_terminals_keep_their_process_and_screen_and_the_daemon_its_pid() {
    let home = Home::new();
    started(&home);
    // A background process of the terminal, 300 lines of history and a shell on top.
    let id = script(
        &home,
        "sleep 600 & echo bg=$!; seq 1 300; echo before-upgrade; exec sh -i",
    );
    shows(&home, &id, "before-upgrade");
    ok(
        &home,
        &["send", "--terminal", &id, "--enter", "echo shell=$$"],
    );
    let shell = shell_pid(&home, &id, "shell=");
    let screen = ok(&home, &["capture", "--terminal", &id]);
    let history = ok(&home, &["capture", "--terminal", &id, "--history"]);
    let background: i32 = history
        .lines()
        .find_map(|l| l.strip_prefix("bg="))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(support::sys::alive(background));
    let tree = ok(&home, &["ls", "--json"]);
    let pid = home.daemon_pid().unwrap();
    let marker_before = marker(&home);
    assert_eq!(marker_before.map(|m| m.generation), Some(0));

    let next = copy_of_deck(&home, "deck-next");
    upgrade_to(&home, &next);
    let after = adopted(&home, 1);

    assert_eq!(after.pid, u32::try_from(pid).unwrap());
    assert_eq!(home.daemon_pid(), Some(pid));
    assert_eq!(after.terminals, 2);
    assert_eq!(ok(&home, &["ls", "--json"]), tree);
    assert_eq!(ok(&home, &["capture", "--terminal", &id]), screen);
    assert_eq!(
        ok(&home, &["capture", "--terminal", &id, "--history"]),
        history
    );
    assert!(
        support::sys::alive(background),
        "the terminal's other process was killed"
    );
    let log = log(&home);
    assert!(log.contains("upgrading: "), "{log}");
    let listening = log.rfind("deck daemon listening").unwrap();
    let adopted_at = log.find("adopted 2 terminals from pid").unwrap();
    assert!(adopted_at > listening, "{log}");

    // The same shell answers.
    ok(
        &home,
        &["send", "--terminal", &id, "--enter", "echo again=$$"],
    );
    assert_eq!(shell_pid(&home, &id, "again="), shell);
    assert!(!home.path().join("run").join("deck.upgrade").exists());
}

#[test]
fn an_upgrade_makes_the_state_directory_the_log_and_the_locks_private() {
    let home = Home::new();
    started(&home);
    let mode = |path: &std::path::Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
    // What a deck that did not set the modes left behind.
    let state = home.path().join("state");
    let files = [
        home.log_file().to_path_buf(),
        state.join("state.lock"),
        home.path().join("run").join("deck.lock"),
    ];
    fs::set_permissions(&state, fs::Permissions::from_mode(0o755)).unwrap();
    for file in &files {
        fs::set_permissions(file, fs::Permissions::from_mode(0o644)).unwrap();
    }

    upgrade_to(&home, &copy_of_deck(&home, "deck-next"));
    adopted(&home, 1);
    assert_eq!(mode(&state), 0o700);
    for file in &files {
        assert_eq!(mode(file), 0o600, "{}", file.display());
    }
}

#[test]
fn a_long_output_goes_on_without_a_gap() {
    let home = Home::new();
    started(&home);
    let id = script(&home, "seq 1 200000; echo done-seq; exec sleep 600");
    let next = copy_of_deck(&home, "deck-next");
    upgrade_to(&home, &next);
    adopted(&home, 1);
    shows(&home, &id, "done-seq");
    let all = ok(&home, &["capture", "--terminal", &id, "--history"]);
    let numbers: Vec<u64> = all.lines().filter_map(|l| l.trim().parse().ok()).collect();
    assert_eq!(numbers.last(), Some(&200_000), "the end is missing");
    for pair in numbers.windows(2) {
        assert_eq!(pair[0] + 1, pair[1], "a gap in the output near {}", pair[0]);
    }
}

#[test]
fn a_program_that_writes_through_the_handoff_loses_nothing() {
    // The handoff waits 300 ms before it freezes and 300 ms once frozen, and the program
    // writes 6,000 lines in about three seconds, so it is writing before, during and after.
    let home = slow_home(300, 300);
    started(&home);
    let id = script(
        &home,
        "i=1; while [ $i -le 6000 ]; do echo $i; i=$((i+1)); \
         if [ $((i % 100)) -eq 0 ]; then sleep 0.05; fi; done; echo done-paced; exec sleep 600",
    );
    let next = copy_of_deck(&home, "deck-next");
    upgrade_to(&home, &next);
    adopted(&home, 1);
    shows(&home, &id, "done-paced");
    let all = ok(&home, &["capture", "--terminal", &id, "--history"]);
    let numbers: Vec<u64> = all.lines().filter_map(|l| l.trim().parse().ok()).collect();
    assert_eq!(numbers.first(), Some(&1), "the start is missing");
    assert_eq!(numbers.last(), Some(&6000), "the end is missing");
    for pair in numbers.windows(2) {
        assert_eq!(pair[0] + 1, pair[1], "a gap in the output near {}", pair[0]);
    }
}

#[test]
fn a_full_screen_program_keeps_its_screen_and_the_history_behind_it() {
    let home = Home::new();
    started(&home);
    let id = script(
        &home,
        "seq 1 50; \
         redraw() { printf '\\033[2J\\033[Halt-screen'; }; \
         printf '\\033[?1049h'; redraw; trap redraw WINCH; \
         exec 3<&0; dd bs=1 count=1 of=/dev/null <&3 2>/dev/null & \
         reader=$!; while kill -0 $reader 2>/dev/null; do wait $reader; done; \
         printf '\\033[?1049l'; echo left-alt; exec sleep 600",
    );
    shows(&home, &id, "alt-screen");
    let before = ok(&home, &["capture", "--terminal", &id]);
    assert_eq!(before, "alt-screen");

    let next = copy_of_deck(&home, "deck-next");
    upgrade_to(&home, &next);
    adopted(&home, 1);

    // The nudge made it redraw, and it draws what it drew.
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(ok(&home, &["capture", "--terminal", &id]), before);
    // Leaving the alternate screen shows the history that was behind it.
    ok(&home, &["send", "--terminal", &id, "--enter", "x"]);
    shows(&home, &id, "left-alt");
    let history = ok(&home, &["capture", "--terminal", &id, "--history"]);
    let numbers: Vec<u64> = history
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect();
    assert_eq!(numbers.first(), Some(&1), "{history}");
    assert_eq!(numbers.last(), Some(&50), "{history}");
}

#[test]
fn a_program_is_told_to_redraw() {
    let home = Home::new();
    started(&home);
    let redraw = script(
        &home,
        "trap 'echo winch-seen' WINCH; echo ready; while :; do sleep 0.1; done",
    );
    shows(&home, &redraw, "ready");
    let next = copy_of_deck(&home, "deck-next");
    upgrade_to(&home, &next);
    adopted(&home, 1);
    shows(&home, &redraw, "winch-seen");
}

#[test]
fn a_status_reaped_before_the_pause_survives_the_handoff() {
    // The program ends while the handoff waits to freeze: its wait thread reaps it, and the
    // pause hands the status over in the handoff file.
    let home = slow_home(1500, 0);
    started(&home);
    let id = script(&home, "sleep 0.5; exit 7");
    let next = copy_of_deck(&home, "deck-next");
    upgrade_to(&home, &next);
    adopted(&home, 1);
    let out = ok(
        &home,
        &["wait", "--terminal", &id, "--exit", "--timeout", "20"],
    );
    assert!(!log(&home).contains("it ended during the handoff"));
    assert_eq!(out, "7");
}

#[test]
fn a_program_that_ends_while_frozen_is_reaped_by_the_new_image() {
    // The wait thread is paused when the program ends, so the zombie crosses the exec and
    // the new image reaps it.
    let home = slow_home(0, 2500);
    started(&home);
    let id = script(&home, "sleep 1.5; exit 7");
    let next = copy_of_deck(&home, "deck-next");
    upgrade_to(&home, &next);
    adopted(&home, 1);
    let out = ok(
        &home,
        &["wait", "--terminal", &id, "--exit", "--timeout", "20"],
    );
    let log = log(&home);
    assert!(log.contains("it ended during the handoff"), "{log}");
    assert_eq!(out, "7");
}

/// The descriptors a program started in a new terminal has open.
fn open_descriptors(home: &Home) -> Vec<u32> {
    let id = script(
        home,
        "echo FDS=$(ls /dev/fd | tr '\\n' ' ')END; exec sleep 600",
    );
    let line = shows(home, &id, "END");
    line.split("FDS=")
        .nth(1)
        .unwrap()
        .trim_end_matches("END")
        .split_whitespace()
        .filter_map(|n| n.parse().ok())
        .collect()
}

#[test]
fn new_terminals_after_the_upgrade_inherit_no_descriptors() {
    let home = Home::new();
    started(&home);
    let before = open_descriptors(&home);
    let next = copy_of_deck(&home, "deck-next");
    upgrade_to(&home, &next);
    adopted(&home, 1);
    // The same as with a daemon that never upgraded: what the shell itself opens.
    assert_eq!(open_descriptors(&home), before);
    assert!(before.iter().all(|fd| *fd <= 4), "{before:?}");
}

#[test]
fn a_client_that_connects_during_the_handoff_is_answered_by_the_new_daemon() {
    let home = Home::new();
    started(&home);
    let next = copy_of_deck(&home, "deck-next");
    upgrade_to(&home, &next);
    let started_at = Instant::now();
    let out = home.run_deck(&["ls", "--json"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(started_at.elapsed() < LIMIT);
    adopted(&home, 1);
}

#[test]
fn a_termination_during_the_handoff_is_acted_on_after_it() {
    let home = slow_home(1000, 0);
    started(&home);
    let (_, hup) = hup_trap(&home);
    let pid = home.daemon_pid().unwrap();
    let next = copy_of_deck(&home, "deck-next");
    upgrade_to(&home, &next);
    upgrading(&home);
    support::sys::terminate(pid);
    wait_until("the daemon to stop", LIMIT, || !home.daemon_running());
    assert!(!marker_file(&home).exists());
    assert!(!home.socket().exists());
    let log = log(&home);
    let adopted_at = after(&log, 0, "adopted ");
    after(&log, adopted_at, "shutting down: SIGTERM");
    assert!(hup.exists(), "the terminal was not hung up normally");
}

#[test]
fn kill_server_during_the_handoff_is_answered_by_the_new_daemon_which_then_stops() {
    let home = slow_home(0, 1500);
    started(&home);
    let (_, hup) = hup_trap(&home);
    let next = copy_of_deck(&home, "deck-next");
    upgrade_to(&home, &next);
    upgrading(&home);
    // This request is queued on the socket until the new image accepts it. The command
    // itself (`deck kill-server`, which waits the handoff out) is the two tests below.
    assert_eq!(Conn::open(&home).kill_server(), Response::Ok);
    wait_until("the daemon to stop", LIMIT, || !home.daemon_running());
    assert!(!marker_file(&home).exists());
    let log = log(&home);
    let adopted_at = after(&log, 0, "adopted ");
    after(&log, adopted_at, "shutting down: kill-server");
    assert!(hup.exists(), "the terminal was not hung up normally");
}

/// `deck kill-server` while a 1.5 s handoff is held, with nothing but `DECK_HOME` in its
/// environment, so its `Hello` fits the socket's buffer and cannot block the write (a large
/// environment did, on macOS, and hid that the command gave up after 1 s).
#[test]
fn kill_server_with_a_minimal_environment_during_the_handoff_is_answered_by_the_new_daemon() {
    let home = slow_home(0, 1500);
    started(&home);
    let (_, hup) = hup_trap(&home);
    let next = copy_of_deck(&home, "deck-next");
    upgrade_to(&home, &next);
    upgrading(&home);
    let out = home
        .deck_command(&["kill-server"])
        .env_clear()
        .env("DECK_HOME", home.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    wait_until("the daemon to stop", LIMIT, || !home.daemon_running());
    assert!(!marker_file(&home).exists());
    let log = log(&home);
    let adopted_at = after(&log, 0, "adopted ");
    after(&log, adopted_at, "shutting down: kill-server");
    assert!(hup.exists(), "the terminal was not hung up normally");
}

#[test]
fn deck_kill_server_waits_out_a_long_handoff_and_is_answered_by_the_new_daemon() {
    // Frozen past the second `deck kill-server` used to wait for an answer: the command now
    // waits the handoff out and asks the new image, so no `SIGTERM` is sent.
    let home = slow_home(0, 2500);
    started(&home);
    let (_, hup) = hup_trap(&home);
    let next = copy_of_deck(&home, "deck-next");
    upgrade_to(&home, &next);
    upgrading(&home);
    ok(&home, &["kill-server"]);
    wait_until("the daemon to stop", LIMIT, || !home.daemon_running());
    assert!(!marker_file(&home).exists());
    let log = log(&home);
    let adopted_at = after(&log, 0, "adopted ");
    after(&log, adopted_at, "shutting down: kill-server");
    assert!(hup.exists(), "the terminal was not hung up normally");
}

#[test]
fn a_shutdown_asked_over_an_open_connection_during_the_handoff_is_not_lost() {
    let home = slow_home(2000, 0);
    started(&home);
    let (_, hup) = hup_trap(&home);
    let mut conn = Conn::open(&home);
    let next = copy_of_deck(&home, "deck-next");
    upgrade_to(&home, &next);
    upgrading(&home);
    // The old image answers, and the new one must still shut down once it has adopted.
    assert_eq!(conn.kill_server(), Response::Ok);
    wait_until("the daemon to stop", LIMIT, || !home.daemon_running());
    assert!(!marker_file(&home).exists());
    let log = log(&home);
    let adopted_at = after(&log, 0, "adopted ");
    after(&log, adopted_at, "shutting down: a shutdown request");
    assert!(hup.exists(), "the terminal was not hung up normally");
}

#[test]
fn a_shutdown_asked_during_a_handoff_that_fails_ends_the_daemon_after_the_rollback() {
    let home = Home::with(
        &[],
        &[
            ("DECK_TEST_PREFREEZE_DELAY_MS", Some("1500")),
            ("DECK_TEST_EXEC_FAIL", Some("1")),
        ],
    );
    started(&home);
    let mut conn = Conn::open(&home);
    let next = copy_of_deck(&home, "deck-next");
    upgrade_to(&home, &next);
    upgrading(&home);
    assert_eq!(conn.kill_server(), Response::Ok);
    wait_until("the daemon to stop", LIMIT, || !home.daemon_running());
    assert!(result(&home).starts_with("failed: "));
    assert!(!marker_file(&home).exists());
}

#[test]
fn kill_server_after_an_upgrade_shuts_down_and_removes_the_marker() {
    let home = Home::new();
    started(&home);
    let next = copy_of_deck(&home, "deck-next");
    upgrade_to(&home, &next);
    adopted(&home, 1);
    ok(&home, &["kill-server"]);
    wait_until("the daemon to stop", LIMIT, || !home.daemon_running());
    assert!(!marker_file(&home).exists());
    assert!(!home.socket().exists());
}

/// Runs `deck daemon --adopt <fd>` the way a new image would find a bad handoff, with
/// `setup` giving it the descriptor (or none), after a session was saved; the daemon must
/// start over from the saved state and serve.
fn starts_over_from_state(setup: &str, fd: &str) {
    let home = Home::new();
    started(&home);
    ok(&home, &["kill-server"]);
    wait_until("the first daemon to stop", LIMIT, || !home.daemon_running());

    let garbage = home.path().join("garbage");
    fs::write(&garbage, "this is not json").unwrap();
    let mut child = std::process::Command::new("sh")
        .args(["-c", setup, "sh"])
        .arg(&garbage)
        .arg(env!("CARGO_BIN_EXE_deck"))
        .arg(fd)
        .env_clear()
        .envs(home.env().iter().map(|(k, v)| (k, v)))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    // Not before it listens: a `deck` that finds no daemon starts one of its own.
    wait_until("the daemon to listen", LIMIT, || home.socket().exists());
    wait_until("the daemon to answer", LIMIT, || {
        home.run_deck(&["ls", "--json"]).status.success()
    });
    let tree: Value = serde_json::from_str(&ok(&home, &["ls", "--json"])).unwrap();
    assert_eq!(tree["sessions"][0]["name"], "main", "{tree}");
    let log = log(&home);
    assert!(
        log.contains("the upgrade could not hand the terminals over"),
        "{log}"
    );
    assert!(!log.contains("panicked"), "{log}");
    ok(&home, &["kill-server"]);
    let status = child.wait().unwrap();
    assert!(status.success(), "the daemon crashed: {status}");
}

#[test]
fn a_new_image_with_a_descriptor_that_is_not_open_starts_over_from_the_saved_state() {
    starts_over_from_state(r#"exec "$2" daemon --adopt "$3""#, "99");
}

#[test]
fn a_new_image_with_a_handoff_file_that_is_garbage_starts_over_from_the_saved_state() {
    starts_over_from_state(r#"exec 5<"$1"; exec "$2" daemon --adopt "$3""#, "5");
}

#[test]
fn the_generation_grows_with_each_handoff() {
    let home = Home::new();
    started(&home);
    let next = copy_of_deck(&home, "deck-next");
    upgrade_to(&home, &next);
    assert_eq!(adopted(&home, 1).generation, 1);
    let again = copy_of_deck(&home, "deck-again");
    upgrade_to(&home, &again);
    assert_eq!(adopted(&home, 2).generation, 2);
    let text = fs::read_to_string(marker_file(&home)).unwrap();
    assert!(text.starts_with("handoff 1\npid "), "{text}");
    let mode = fs::metadata(marker_file(&home))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn a_request_that_cannot_be_honoured_is_refused_and_nothing_changes() {
    let home = Home::new();
    started(&home);
    let id = first_terminal(&home);
    let pid = home.daemon_pid().unwrap();

    let plain = home.path().join("plain");
    fs::write(&plain, "#!/bin/sh\n").unwrap();
    fs::set_permissions(&plain, fs::Permissions::from_mode(0o644)).unwrap();
    let failing = home.path().join("failing");
    fs::write(&failing, "#!/bin/sh\necho no thanks >&2\nexit 1\n").unwrap();
    fs::set_permissions(&failing, fs::Permissions::from_mode(0o755)).unwrap();
    let cases = [
        ("relative/deck".to_string(), "absolute"),
        ("/nonexistent/deck".to_string(), "cannot inspect"),
        (home.path().display().to_string(), "regular file"),
        (plain.display().to_string(), "not executable"),
        (failing.display().to_string(), "cannot take over"),
    ];
    for (content, why) in cases {
        let results = home.path().join("run").join("deck.upgrade.result");
        let _ = fs::remove_file(&results);
        request(&home, &content);
        let line = result(&home);
        assert!(line.starts_with("refused: "), "{line}");
        assert!(line.contains(why), "{line}");
        assert!(!home.path().join("run").join("deck.upgrade").exists());
        assert_eq!(home.daemon_pid(), Some(pid));
        assert_eq!(marker(&home).unwrap().generation, 0);
        assert!(!marker(&home).unwrap().upgrading);
    }
    let log = log(&home);
    assert!(log.contains("upgrade refused: "), "{log}");
    // Still the same daemon, still serving.
    ok(
        &home,
        &["send", "--terminal", &id, "--enter", "echo still-here"],
    );
    shows(&home, &id, "still-here");
}

#[test]
fn a_request_with_no_file_is_refused() {
    let home = Home::new();
    started(&home);
    support::sys::signal(home.daemon_pid().unwrap(), libc::SIGUSR2);
    let line = result(&home);
    assert!(line.starts_with("refused: "), "{line}");
}

#[test]
fn a_failed_exec_rolls_back_and_the_terminals_go_on() {
    let home = Home::with(&[], &[("DECK_TEST_EXEC_FAIL", Some("1"))]);
    started(&home);
    let id = script(&home, "echo before; exec sh -i");
    shows(&home, &id, "before");
    let tree = ok(&home, &["ls", "--json"]);
    let pid = home.daemon_pid().unwrap();
    let next = copy_of_deck(&home, "deck-next");
    upgrade_to(&home, &next);
    let line = result(&home);
    assert!(line.starts_with("failed: "), "{line}");

    assert_eq!(home.daemon_pid(), Some(pid));
    let m = marker(&home).unwrap();
    assert!(!m.upgrading);
    assert_eq!(m.generation, 0);
    assert_eq!(ok(&home, &["ls", "--json"]), tree);
    ok(
        &home,
        &["send", "--terminal", &id, "--enter", "echo rolled-back"],
    );
    shows(&home, &id, "rolled-back");
    // New work is accepted again.
    script(&home, "echo fresh; exec sleep 600");
    let log = log(&home);
    assert!(log.contains("upgrade failed: "), "{log}");
}

#[test]
fn the_preflight_answers_by_version() {
    let home = Home::new();
    let check = |n: &str| {
        home.run_deck(&["daemon", "--handoff-check", n])
            .status
            .code()
    };
    assert_eq!(check("1"), Some(0));
    assert_eq!(check("99"), Some(1));
    assert_eq!(check("0"), Some(1));
}

/// 30 terminals with 10,000 lines each: the whole handoff takes under 10 s.
#[test]
#[ignore = "slow; run with `cargo nextest run -p deck --test upgrade --run-ignored all`"]
fn thirty_terminals_of_ten_thousand_lines_are_adopted_in_under_ten_seconds() {
    let home = Home::new();
    started(&home);
    let mut panels = Vec::new();
    for _ in 0..30 {
        panels.push(ok(
            &home,
            &[
                "new-panel",
                "--",
                "sh",
                "-c",
                "seq 1 10000; echo full; exec sleep 600",
            ],
        ));
    }
    let tree: Value = serde_json::from_str(&ok(&home, &["ls", "--json"])).unwrap();
    let ids: Vec<String> = tree["sessions"][0]["spaces"][0]["panels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| panels.contains(&p["id"].as_u64().unwrap().to_string()))
        .map(|p| p["terminals"][0]["id"].as_u64().unwrap().to_string())
        .collect();
    assert_eq!(ids.len(), 30);
    for id in &ids {
        shows(&home, id, "full");
    }
    let next = copy_of_deck(&home, "deck-next");
    let began = Instant::now();
    upgrade_to(&home, &next);
    adopted(&home, 1);
    let took = began.elapsed();
    eprintln!("handoff of 30 terminals x 10000 lines: {took:?}");
    assert!(took < Duration::from_secs(10), "took {took:?}");
}

// `deck upgrade` and the commands around it.

/// The protocol version of the older daemon the tests start.
const OLDER: &str = "25";

fn older_home() -> Home {
    Home::with(&[], &[("DECK_TEST_PROTOCOL_VERSION", Some(OLDER))])
}

/// Starts a daemon that speaks the older protocol: `_probe` finds it and fails on the
/// mismatch.
fn older_daemon(home: &Home) {
    let out = home.run_deck(&["_probe"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("protocol 25"), "{}", stderr(&out));
    assert!(home.daemon_running());
}

#[test]
fn deck_upgrade_with_the_same_build_brings_the_attached_client_back_to_its_session() {
    let mut h = Harness::deck((80, 24));
    h.wait_for_text("$", TIMEOUT);
    h.send_keys("sleep 600 & echo bg=$!\r");
    h.wait_for_row_start("bg=", TIMEOUT);
    let background: i32 = h
        .screen_text()
        .lines()
        .find_map(|l| l.strip_prefix("bg="))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let home = h.home();
    let pid = home.daemon_pid().unwrap();
    let before = h.screen_text();

    let out = home.run_deck(&["upgrade"]);
    assert!(out.status.success(), "{}{}", stderr(&out), stdout(&out));
    assert_eq!(
        stdout(&out).trim_end(),
        format!("deck daemon restarted in place (protocol {PROTOCOL_VERSION}, 1 terminal kept)")
    );
    assert_eq!(marker(home).map(|m| m.generation), Some(1));
    assert_eq!(home.daemon_pid(), Some(pid));
    assert!(support::sys::alive(background));

    // The client executed `deck attach main` in the same terminal: same session, same screen.
    wait_until("the client to reattach", LIMIT, || {
        home.run_deck(&["ls"]).status.success()
            && stdout(&home.run_deck(&["ls"])).contains("(attached)")
    });
    h.wait_for_text("bg=", TIMEOUT);
    wait_until("the same screen", LIMIT, || h.screen_text() == before);
    h.capture("upgraded");
    h.send_keys("echo still-$((1+1))\r");
    h.wait_for_row_start("still-2", TIMEOUT);
}

#[test]
fn deck_upgrade_against_an_older_daemon_names_both_protocols() {
    let home = older_home();
    older_daemon(&home);
    // The error of another command points at the upgrade.
    let out = home.run_deck(&["ls"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("run \"deck upgrade\""),
        "{}",
        stderr(&out)
    );

    let out = home.run_deck(&["upgrade"]);
    assert!(out.status.success(), "{}{}", stderr(&out), stdout(&out));
    assert_eq!(
        stdout(&out).trim_end(),
        format!(
            "deck daemon upgraded (protocol {OLDER} \u{2192} {PROTOCOL_VERSION}, 0 terminals kept)"
        )
    );
    ok(&home, &["new", "main", "-d"]);
    assert!(ok(&home, &["ls"]).starts_with("main"));
}

#[test]
fn deck_upgrade_without_a_usable_marker_says_so_and_signals_nothing() {
    let home = Home::new();
    started(&home);
    let pid = home.daemon_pid().unwrap();
    let said = "the running daemon cannot be upgraded in place; run \"deck kill-server\" once";

    // No marker.
    let kept = fs::read_to_string(marker_file(&home)).unwrap();
    fs::remove_file(marker_file(&home)).unwrap();
    let out = home.run_deck(&["upgrade"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stderr(&out).trim_end(), said);

    // A marker of another daemon.
    let other = Marker {
        pid: u32::try_from(pid).unwrap() + 1,
        ..Marker::parse(&kept).unwrap()
    };
    other.write(&marker_file(&home)).unwrap();
    let out = home.run_deck(&["upgrade"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stderr(&out).trim_end(), said);

    // Nothing was written or signalled.
    assert!(!home.path().join("run").join("deck.upgrade").exists());
    assert!(!home.path().join("run").join("deck.upgrade.result").exists());
    assert!(!log(&home).contains("upgrad"));
    assert_eq!(home.daemon_pid(), Some(pid));
    assert!(home.daemon_running());
    ok(&home, &["ls"]);
}

#[test]
fn deck_upgrade_to_a_binary_that_fails_its_check_prints_why_and_exits_1() {
    let home = Home::with(&[], &[("DECK_TEST_HANDOFF_CHECK_FAIL", Some("1"))]);
    started(&home);
    let pid = home.daemon_pid().unwrap();
    let out = home.run_deck(&["upgrade"]);
    assert_eq!(out.status.code(), Some(1));
    let text = stderr(&out);
    assert!(text.starts_with("upgrade refused: "), "{text}");
    assert!(text.contains("cannot take over"), "{text}");
    assert!(text.contains("DECK_TEST_HANDOFF_CHECK_FAIL"), "{text}");
    assert_eq!(home.daemon_pid(), Some(pid));
    assert_eq!(marker(&home).unwrap().generation, 0);
}

#[test]
fn deck_upgrade_with_no_daemon_says_so() {
    let home = Home::new();
    let out = home.run_deck(&["upgrade"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(stdout(&out).trim_end(), "no deck daemon running");
}

#[test]
fn deck_meeting_an_older_daemon_upgrades_it_and_attaches() {
    let mut h = Harness::builder((80, 24))
        .env("DECK_TEST_PROTOCOL_VERSION", OLDER)
        .deck();
    h.wait_for_text("$", TIMEOUT);
    h.send_keys("echo up-$((20+1))\r");
    h.wait_for_row_start("up-21", TIMEOUT);
    assert_eq!(marker(h.home()).map(|m| m.generation), Some(1));
    // The line went to stderr before the screen was taken over.
    let raw = String::from_utf8_lossy(&h.raw_output()).into_owned();
    assert!(
        raw.contains(&format!(
            "upgrading the deck daemon (protocol {OLDER} \u{2192} {PROTOCOL_VERSION})"
        )),
        "{raw:?}"
    );
    // The daemon speaks this deck's protocol now.
    ok(h.home(), &["ls"]);
    h.capture("auto-upgraded");
}

#[test]
fn deck_meeting_an_older_daemon_without_a_marker_prints_the_mismatch() {
    let deck = env!("CARGO_BIN_EXE_deck");
    let script = format!(
        "{deck} _probe >/dev/null 2>&1; rm -f \"$DECK_HOME/run/deck.handoff\"; exec {deck}"
    );
    let h = Harness::builder((100, 24))
        .env("DECK_TEST_PROTOCOL_VERSION", OLDER)
        .spawn(&["sh", "-c", &script]);
    h.wait_for_text("deck kill-server", TIMEOUT);
    assert!(!h.screen_text().contains("deck upgrade"));
}

#[test]
fn a_wait_pending_during_the_upgrade_ends_with_the_upgrading_message() {
    let home = slow_home(1500, 0);
    started(&home);
    let id = script(&home, "exec sleep 600");
    let waiting = home
        .deck_command(&["wait", "--terminal", &id, "--exit", "--timeout", "30"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    // It is connected and waiting well before the daemon freezes.
    std::thread::sleep(Duration::from_millis(400));
    let out = home.run_deck(&["upgrade"]);
    assert!(out.status.success(), "{}{}", stderr(&out), stdout(&out));
    let waited = waiting.wait_with_output().unwrap();
    assert_eq!(waited.status.code(), Some(1));
    assert!(
        stderr(&waited).contains("the deck daemon is upgrading; try again"),
        "{}",
        stderr(&waited)
    );
}

/// A reattaching client (`_probe` starts the way `deck attach` does) and `deck ls`, both
/// started once the handoff runs, wait for the new image and start no other daemon.
fn clients_wait_out_the_handoff(home: &Home) {
    let pid = home.daemon_pid().unwrap();
    let next = copy_of_deck(home, "deck-next");
    upgrade_to(home, &next);
    upgrading(home);
    let began = Instant::now();
    let piped = |args: &[&str]| {
        home.deck_command(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    };
    // Both are running while the handoff is.
    let ls = piped(&["ls"]);
    let probe = piped(&["_probe", "--session", "main"]);
    let ls = ls.wait_with_output().unwrap();
    assert!(ls.status.success(), "{}", stderr(&ls));
    let probe = probe.wait_with_output().unwrap();
    assert!(probe.status.success(), "{}", stderr(&probe));
    assert!(stdout(&probe).starts_with("main"), "{}", stdout(&probe));
    assert!(began.elapsed() < LIMIT);
    adopted(home, 1);
    assert_eq!(home.daemon_pid(), Some(pid));
    let log = log(home);
    assert_eq!(log.matches("deck daemon listening").count(), 2, "{log}");
    assert!(!log.contains("already running"), "{log}");
}

#[test]
fn clients_started_during_a_handoff_wait_for_it() {
    let home = slow_home(0, 2500);
    started(&home);
    clients_wait_out_the_handoff(&home);
}

#[test]
#[ignore = "slow; run with `cargo nextest run -p deck --test upgrade --run-ignored all`"]
fn clients_started_during_a_handoff_of_thirty_big_terminals_wait_for_it() {
    let home = Home::new();
    started(&home);
    for _ in 0..30 {
        ok(
            &home,
            &[
                "new-panel",
                "--",
                "sh",
                "-c",
                "seq 1 10000; echo full; exec sleep 600",
            ],
        );
    }
    clients_wait_out_the_handoff(&home);
}
