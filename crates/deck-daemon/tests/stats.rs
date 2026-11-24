//! `Request::Stats`: the counts, the per-terminal figures and the bytes read from a PTY.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use deck_core::model::{LaunchSpec, TerminalId};
use deck_core::protocol::{PROTOCOL_VERSION, Request, Response, Stats};
use support::{Attach, Client, TestDaemon};

fn launch(script: &str) -> Option<LaunchSpec> {
    Some(LaunchSpec {
        program: Some("/bin/sh".into()),
        args: vec!["-c".into(), script.into()],
        cwd: PathBuf::new(),
        env: Vec::new(),
    })
}

async fn stats(c: &mut Client) -> Stats {
    match c.request(Request::Stats).await {
        Response::Stats(s) => s,
        other => panic!("expected stats, got {other:?}"),
    }
}

async fn new_terminal(c: &mut Client, script: &str, keep: bool) -> TerminalId {
    let panel = c.view.as_ref().unwrap().projects[0].panels[0].id;
    match c
        .request(Request::NewTerminal {
            panel,
            launch: launch(script),
            keep,
            focus: false,
        })
        .await
    {
        Response::Created { id } => TerminalId(id),
        other => panic!("expected an id, got {other:?}"),
    }
}

#[tokio::test]
async fn stats_count_what_the_daemon_holds_and_not_the_control_connections() {
    let daemon = TestDaemon::start().await;
    let mut control = Client::control(&daemon.paths.socket).await;

    // Nothing yet: no session, no client.
    let empty = stats(&mut control).await;
    assert_eq!(
        (empty.sessions, empty.live, empty.exited, empty.clients),
        (0, 0, 0, 0)
    );
    assert!(empty.per_terminal.is_empty());

    let mut tui = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let kept = new_terminal(&mut tui, "exit 0", true).await;
    let live = new_terminal(&mut tui, "sleep 30", false).await;
    // The kept terminal's program ended; wait until the daemon knows.
    let deadline = Instant::now() + Duration::from_secs(5);
    let s = loop {
        let s = stats(&mut control).await;
        if s.exited == 1 {
            break s;
        }
        assert!(Instant::now() < deadline, "the kept terminal never exited");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };

    assert_eq!(s.pid, std::process::id());
    assert_eq!(s.protocol, PROTOCOL_VERSION);
    assert_eq!(s.version, env!("CARGO_PKG_VERSION"));
    // The shell of the session and `sleep`.
    assert_eq!((s.sessions, s.live, s.exited, s.clients), (1, 2, 1, 1));
    assert!(s.rss_bytes.unwrap() > 1 << 20);
    assert!(s.fds.unwrap() > 3 && s.threads.unwrap() >= 1);
    assert_eq!(s.per_terminal.len(), 3);
    let one = s.per_terminal.iter().find(|t| t.id == live).unwrap();
    assert_eq!(one.session, "main");
    assert!(one.size.0 > 0 && one.size.1 > 0);
    assert!(s.per_terminal.iter().any(|t| t.id == kept));
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn a_terminal_that_printed_a_mebibyte_has_read_a_mebibyte() {
    let daemon = TestDaemon::start().await;
    let mut tui = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let id = new_terminal(
        &mut tui,
        "head -c 1048576 /dev/zero | tr '\\0' x; sleep 30",
        false,
    )
    .await;
    let mut control = Client::control(&daemon.paths.socket).await;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let s = stats(&mut control).await;
        let read = s
            .per_terminal
            .iter()
            .find(|t| t.id == id)
            .unwrap()
            .bytes_read;
        if read >= 1 << 20 {
            break;
        }
        assert!(Instant::now() < deadline, "only {read} bytes were read");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    daemon.stop().await.unwrap();
}
