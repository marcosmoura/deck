//! The hidden `deck daemon` command, run as a real process.

use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use deck_core::protocol::{
    ByeReason, ClientMsg, FrameReader, PROTOCOL_VERSION, Role, ServerMsg, encode,
};

const TIMEOUT: Duration = Duration::from_secs(5);

fn wait_until(what: &str, limit: Duration, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + limit;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

struct Daemon {
    home: tempfile::TempDir,
    child: Child,
}

impl Daemon {
    fn socket(&self) -> PathBuf {
        self.home.path().join("run").join("deck.sock")
    }

    fn wait_for_socket(&mut self) {
        let socket = self.socket();
        wait_until("the socket", TIMEOUT, || {
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "the daemon exited early"
            );
            UnixStream::connect(&socket).is_ok()
        });
    }

    fn wait_exit(&mut self, limit: Duration) -> ExitStatus {
        let mut status = None;
        wait_until("the daemon to exit", limit, || {
            status = self.child.try_wait().unwrap();
            status.is_some()
        });
        status.unwrap()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn command(home: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_deck"));
    cmd.arg("daemon")
        .env("DECK_HOME", home)
        .env_remove("XDG_RUNTIME_DIR")
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    cmd
}

fn start(parent: u32, stderr: Stdio) -> Daemon {
    let home = tempfile::Builder::new()
        .prefix("dk")
        .tempdir_in("/tmp")
        .unwrap();
    start_in(home, parent, stderr)
}

fn start_in(home: tempfile::TempDir, parent: u32, stderr: Stdio) -> Daemon {
    let child = command(home.path())
        .env("DECK_TEST_PARENT_PID", parent.to_string())
        .stderr(stderr)
        .spawn()
        .unwrap();
    let mut daemon = Daemon { home, child };
    daemon.wait_for_socket();
    daemon
}

#[test]
fn the_daemon_shuts_down_when_its_parent_is_gone() {
    let mut parent = Command::new("sleep").arg("60").spawn().unwrap();
    let mut daemon = start(parent.id(), Stdio::null());
    parent.kill().unwrap();
    parent.wait().unwrap();
    let status = daemon.wait_exit(Duration::from_secs(3));
    assert!(status.success(), "{status}");
    assert!(!daemon.socket().exists(), "the socket was left behind");
}

#[test]
fn an_oversized_log_is_rotated_at_start_and_sigterm_stops_the_daemon() {
    let home = tempfile::Builder::new()
        .prefix("dk")
        .tempdir_in("/tmp")
        .unwrap();
    let state = home.path().join("state");
    fs::create_dir_all(&state).unwrap();
    let log = state.join("daemon.log");
    fs::File::create(&log)
        .unwrap()
        .set_len(11 * 1024 * 1024)
        .unwrap();

    let mut daemon = start_in(home, std::process::id(), Stdio::null());
    assert_eq!(
        fs::metadata(state.join("daemon.log.1")).unwrap().len(),
        11 * 1024 * 1024
    );
    // A fresh log, written by the daemon itself, has the startup line.
    wait_until("the startup line", TIMEOUT, || {
        fs::read_to_string(&log).is_ok_and(|t| t.contains("deck daemon listening"))
    });
    assert!(fs::metadata(&log).unwrap().len() < 1024 * 1024);

    let pid = daemon.child.id().to_string();
    let killed = Command::new("kill").args(["-TERM", &pid]).status().unwrap();
    assert!(killed.success());
    let status = daemon.wait_exit(TIMEOUT);
    assert!(status.success(), "{status}");
    assert!(!daemon.socket().exists());
}

#[test]
fn a_second_daemon_says_it_is_already_running() {
    let first = start(std::process::id(), Stdio::null());
    let second = command(first.home.path())
        .env("DECK_TEST_PARENT_PID", std::process::id().to_string())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert_eq!(second.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&second.stderr).contains("deck daemon is already running"),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    // The lock holds the first daemon's pid.
    let pid = fs::read_to_string(first.home.path().join("run").join("deck.lock")).unwrap();
    assert_eq!(pid, first.child.id().to_string());
}

#[test]
fn a_shell_that_fails_at_once_is_logged_with_its_status() {
    let home = tempfile::Builder::new()
        .prefix("dk")
        .tempdir_in("/tmp")
        .unwrap();
    fs::write(
        home.path().join("config.toml"),
        "shell = \"/bin/sh\"\nshell_args = [\"-c\", \"exit 3\"]\n",
    )
    .unwrap();
    let log = home.path().join("state").join("daemon.log");
    let mut daemon = start_in(home, std::process::id(), Stdio::null());

    let mut stream = UnixStream::connect(daemon.socket()).unwrap();
    stream.set_read_timeout(Some(TIMEOUT)).unwrap();
    let hello = ClientMsg::Hello {
        version: PROTOCOL_VERSION,
        role: Role::Attach {
            session: None,
            create: true,
            cols: 82,
            rows: 26,
        },
        cwd: std::env::temp_dir(),
        env: vec![("PATH".into(), std::env::var_os("PATH").unwrap_or_default())],
        colors: None,
    };
    stream.write_all(&encode(&hello).unwrap()).unwrap();

    let mut reader = FrameReader::new();
    let mut buf = [0u8; 4096];
    let message = 'read: loop {
        while let Some(msg) = reader.next_message::<ServerMsg>().unwrap() {
            if let ServerMsg::Bye { reason } = msg {
                break 'read reason;
            }
        }
        let n = stream.read(&mut buf).unwrap();
        assert!(n > 0, "closed without a Bye");
        reader.push(&buf[..n]);
    };
    let ByeReason::Error(text) = message else {
        panic!("{message:?}")
    };
    assert!(
        text.contains("terminal exited at once (status 3)"),
        "{text}"
    );

    // The last session ended, so the daemon stops by itself; its log has the status.
    let status = daemon.wait_exit(TIMEOUT);
    assert!(status.success(), "{status}");
    let logged = fs::read_to_string(&log).unwrap();
    assert!(logged.contains("status=3"), "{logged}");
}

#[test]
fn a_second_daemon_does_not_rotate_the_running_daemons_log() {
    let first = start(std::process::id(), Stdio::null());
    let state = first.home.path().join("state");
    let log = state.join("daemon.log");
    fs::OpenOptions::new()
        .write(true)
        .open(&log)
        .unwrap()
        .set_len(11 * 1024 * 1024)
        .unwrap();
    let second = command(first.home.path())
        .env("DECK_TEST_PARENT_PID", std::process::id().to_string())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert_eq!(second.status.code(), Some(1));
    assert!(!state.join("daemon.log.1").exists(), "the log was rotated");
    assert!(fs::metadata(&log).unwrap().len() >= 11 * 1024 * 1024);
}
