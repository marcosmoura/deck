//! The `deck` command line, as a real process on a temp `DECK_HOME`.

mod support;

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::time::Duration;

use deck_core::action::Action;
use deck_core::protocol::{ClientMsg, FrameReader, PROTOCOL_VERSION, Role, ServerMsg, encode};
use support::{Harness, Home, TIMEOUT, stderr, stdout, sys, wait_until};

const MISMATCH_HINT: &str = "run \"deck kill-server\" to restart it";

#[test]
fn probe_starts_the_daemon_and_ls_lists_the_session() {
    let home = Home::new();
    assert!(!home.daemon_running());
    let probe = home.run_deck(&["_probe"]);
    assert!(probe.status.success(), "{}", stderr(&probe));
    assert_eq!(stdout(&probe).lines().next(), Some("main"));
    assert!(home.daemon_running());

    let ls = home.run_deck(&["ls"]);
    assert!(ls.status.success(), "{}", stderr(&ls));
    let text = stdout(&ls);
    assert!(
        text.starts_with("main") && text.contains("1 terminal"),
        "{text}"
    );
}

#[test]
fn two_probes_at_once_share_one_daemon() {
    for run in 0..10 {
        let home = Home::new();
        let children: Vec<_> = (0..2)
            .map(|_| {
                home.deck_command(&["_probe"])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap()
            })
            .collect();
        for child in children {
            let out = child.wait_with_output().unwrap();
            assert!(out.status.success(), "run {run}: {}", stderr(&out));
        }
        assert!(home.socket().exists(), "run {run}");
        assert!(home.daemon_running(), "run {run}");
    }
}

#[test]
fn ls_without_a_daemon_starts_none() {
    let home = Home::new();
    let ls = home.run_deck(&["ls"]);
    assert!(ls.status.success(), "{}", stderr(&ls));
    assert_eq!(stdout(&ls), "no sessions\n");
    assert!(!home.daemon_running());
}

#[test]
fn kill_server_stops_the_daemon_and_waits_for_the_lock() {
    let home = Home::new();
    assert!(home.run_deck(&["_probe"]).status.success());
    let pid = home.daemon_pid().unwrap();

    let kill = home.run_deck(&["kill-server"]);
    assert!(kill.status.success(), "{}", stderr(&kill));
    assert_eq!(stdout(&kill), "");
    assert!(!home.daemon_running());
    wait_until("the daemon to exit", TIMEOUT, || !sys::alive(pid));
    let log = fs::read_to_string(home.log_file()).unwrap();
    assert!(
        log.contains("shutting down: kill-server from pid ") && log.contains("(outside deck)"),
        "{log}"
    );

    // The session the probe created was saved when the daemon stopped.
    assert_eq!(
        stdout(&home.run_deck(&["ls"])),
        "no daemon running; 1 saved session will be restored by \"deck\"\n"
    );
    let again = home.run_deck(&["kill-server"]);
    assert!(again.status.success());
    assert_eq!(stdout(&again), "no deck daemon running\n");

    let probe = home.run_deck(&["_probe"]);
    assert!(probe.status.success(), "{}", stderr(&probe));
    assert!(home.daemon_running());
}

#[test]
fn a_daemon_of_another_version_is_reported_and_stopped_by_sigterm() {
    let home = Home::with(&[], &[("DECK_TEST_PROTOCOL_VERSION", Some("999"))]);
    let probe = home.run_deck(&["_probe"]);
    assert_eq!(probe.status.code(), Some(1));
    let err = stderr(&probe);
    assert!(
        err.starts_with("deck: ") && err.contains("999") && err.contains(MISMATCH_HINT),
        "{err}"
    );
    assert_eq!(err.trim_end().lines().count(), 1, "{err}");
    assert!(home.daemon_running());

    let kill = home.run_deck(&["kill-server"]);
    assert!(kill.status.success(), "{}", stderr(&kill));
    assert!(!home.daemon_running());
    let log = fs::read_to_string(home.log_file()).unwrap();
    assert!(log.contains("shutting down: SIGTERM"), "{log}");
}

#[test]
fn kill_server_from_a_deck_terminal_logs_that_terminal() {
    let home = Home::new();
    assert!(home.run_deck(&["_probe"]).status.success());
    let kill = home
        .deck_command(&["kill-server"])
        .env("DECK_SESSION_ID", "1")
        .env("DECK_PANEL_ID", "3")
        .env("DECK_TERMINAL_ID", "4")
        .output()
        .unwrap();
    assert!(kill.status.success(), "{}", stderr(&kill));
    let log = fs::read_to_string(home.log_file()).unwrap();
    assert!(
        log.contains("shutting down: kill-server from pid ")
            && log.contains("(session 1, panel 3, terminal 4)"),
        "{log}"
    );
}

#[test]
fn attaching_is_refused_inside_deck_and_without_a_terminal() {
    let home = Home::new();
    for args in [&[][..], &["attach"][..]] {
        let nested = home.deck_command(args).env("DECK", "1").output().unwrap();
        assert_eq!(nested.status.code(), Some(1));
        assert_eq!(
            stderr(&nested),
            "deck: already inside deck (unset DECK to nest)\n"
        );

        let file = fs::File::create(home.path().join("out.txt")).unwrap();
        let piped = home.deck_command(args).stdout(file).output().unwrap();
        assert_eq!(piped.status.code(), Some(1));
        assert_eq!(
            stderr(&piped),
            "deck: attach needs a terminal (stdin and stdout must be a TTY)\n"
        );
    }
    assert!(!home.daemon_running());
}

#[test]
fn deck_in_a_terminal_starts_the_daemon_and_attaches() {
    let mut h = Harness::deck((80, 24));
    h.wait_for_text("$", TIMEOUT);
    assert!(h.home().daemon_running());
    h.send_bytes(b"\x1b ");
    h.send_keys("d");
    let status = h.wait_for_exit(TIMEOUT);
    assert!(status.success(), "{}", h.screen_text());
}

#[test]
fn an_unsafe_runtime_dir_is_refused() {
    let home = Home::new();
    let run = home.path().join("run");
    fs::create_dir(&run).unwrap();
    fs::set_permissions(&run, fs::Permissions::from_mode(0o755)).unwrap();
    for args in [&["_probe"][..], &["ls"][..], &["kill-server"][..]] {
        let out = home.run_deck(args);
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        let err = stderr(&out);
        assert!(
            err.starts_with("deck: unsafe runtime directory") && err.contains("0755"),
            "{err}"
        );
    }
    assert!(!home.daemon_running());
}

/// Attaches to the running daemon and kills its session, then reads until the daemon says
/// goodbye: the last session has ended and the daemon is shutting down.
#[expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "this integration-test helper requires a successful protocol exchange"
)]
fn end_the_last_session(home: &Home) {
    let mut stream = UnixStream::connect(home.socket()).unwrap();
    stream.set_read_timeout(Some(TIMEOUT)).unwrap();
    let hello = ClientMsg::Hello {
        version: PROTOCOL_VERSION,
        role: Role::Attach {
            session: None,
            create: true,
            cols: 80,
            rows: 24,
        },
        cwd: std::env::temp_dir(),
        env: vec![("PATH".into(), std::env::var_os("PATH").unwrap_or_default())],
        colors: None,
    };
    stream.write_all(&encode(&hello).unwrap()).unwrap();
    let mut reader = FrameReader::new();
    let mut buf = [0u8; 16 * 1024];
    let mut typed = false;
    loop {
        while let Some(msg) = reader.next_message::<ServerMsg>().unwrap() {
            match msg {
                ServerMsg::SessionView(_) if !typed => {
                    let kill = ClientMsg::Action(Action::KillSession);
                    stream.write_all(&encode(&kill).unwrap()).unwrap();
                    typed = true;
                }
                ServerMsg::Bye { .. } => return,
                _ => {}
            }
        }
        match stream.read(&mut buf).unwrap() {
            0 => return,
            n => reader.push(&buf[..n]),
        }
    }
}

#[test]
fn a_start_right_after_the_last_session_ended_waits_for_the_old_daemon() {
    let home = Home::new();
    for round in 0..3 {
        let probe = home.run_deck(&["_probe"]);
        assert!(probe.status.success(), "round {round}: {}", stderr(&probe));
        end_the_last_session(&home);
    }
    let probe = home.run_deck(&["_probe"]);
    assert!(probe.status.success(), "{}", stderr(&probe));
}

#[test]
fn a_start_while_the_lock_is_held_briefly_still_succeeds() {
    let home = Home::new();
    let run = home.path().join("run");
    fs::DirBuilder::new().mode(0o700).create(&run).unwrap();
    let file = fs::File::create(home.lock_file()).unwrap();
    file.lock().unwrap();
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(600));
        drop(file);
    });
    let probe = home.run_deck(&["_probe"]);
    releaser.join().unwrap();
    assert!(probe.status.success(), "{}", stderr(&probe));
    assert!(home.daemon_running());
}

const LEAK_OUT: &str = "E2E_LEAK_OUT";

/// Runs twice: the test starts itself as a child that starts a daemon and exits without
/// dropping its `Home`, as a test killed by nextest does.
#[test]
fn a_killed_test_leaves_no_daemon() {
    if let Ok(out) = std::env::var(LEAK_OUT) {
        let home = Home::new();
        assert!(home.run_deck(&["_probe"]).status.success());
        fs::write(out, home.path().to_string_lossy().as_bytes()).unwrap();
        std::process::exit(0);
    }

    let out = tempfile::tempdir().unwrap();
    let out_file = out.path().join("home");
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "a_killed_test_leaves_no_daemon", "--nocapture"])
        .env(LEAK_OUT, &out_file)
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());

    let home = std::path::PathBuf::from(fs::read_to_string(&out_file).unwrap());
    let lock = home.join("run").join("deck.lock");
    wait_until(
        "the orphaned daemon to stop",
        Duration::from_secs(5),
        || !support::lock_held(&lock),
    );
    let _ = fs::remove_dir_all(home);
}
