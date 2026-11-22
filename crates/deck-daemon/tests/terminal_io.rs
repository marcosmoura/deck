//! The control requests that type into a terminal, read its screen and wait for it.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use deck_core::config::Config;
use deck_core::keys::KeySpec;
use deck_core::model::{LaunchSpec, TerminalId};
use deck_core::protocol::{ByeReason, Captured, Request, Response, ServerMsg, WaitFor, Waited};
use deck_daemon::{ManualClock, Options};
use support::{Attach, Client, TestDaemon, config};

fn launch(script: &str) -> Option<LaunchSpec> {
    Some(LaunchSpec {
        program: Some("/bin/sh".into()),
        args: vec!["-c".into(), script.into()],
        cwd: PathBuf::new(),
        env: Vec::new(),
    })
}

async fn attach(daemon: &TestDaemon) -> Client {
    Client::attach(&daemon.paths.socket, Attach::default()).await
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

async fn error(c: &mut Client, request: Request) -> String {
    match c.request(request).await {
        Response::Error(text) => text,
        other => panic!("expected an error, got {other:?}"),
    }
}

async fn ok(c: &mut Client, request: Request) {
    assert_eq!(c.request(request).await, Response::Ok);
}

async fn capture(
    c: &mut Client,
    terminal: TerminalId,
    lines: Option<u32>,
    history: bool,
) -> Captured {
    match c
        .request(Request::Capture {
            terminal,
            lines,
            history,
            ansi: false,
        })
        .await
    {
        Response::Captured(captured) => captured,
        other => panic!("expected a capture, got {other:?}"),
    }
}

async fn wait(
    c: &mut Client,
    terminal: TerminalId,
    until: WaitFor,
    timeout_ms: Option<u64>,
) -> Response {
    c.request(Request::Wait {
        terminal,
        until,
        timeout_ms,
    })
    .await
}

/// Waits until a line of the terminal's screen contains `text`.
async fn screen_has(c: &mut Client, terminal: TerminalId, text: &str) -> String {
    match wait(c, terminal, WaitFor::Text(text.into()), Some(4000)).await {
        Response::Waited(Waited::Text(line)) => line,
        other => panic!("{text:?} did not appear: {other:?}"),
    }
}

fn key(text: &str) -> KeySpec {
    KeySpec::parse(text).unwrap()
}

/// A program that turns off the terminal's own handling and shows the bytes it receives.
const SHOW_BYTES: &str = "stty -echo -icanon -icrnl; echo ready; exec cat -v";

#[tokio::test(flavor = "multi_thread")]
async fn typed_text_reaches_another_terminal_and_capture_shows_it() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon).await;
    let first = c.terminal();
    let other = new_terminal(&mut c, "echo ready; exec sh", false).await;
    screen_has(&mut c, other, "ready").await;
    ok(
        &mut c,
        Request::Send {
            terminal: other,
            bytes: b"echo $((40+2))\r".to_vec(),
            paste: false,
        },
    )
    .await;
    assert_eq!(screen_has(&mut c, other, "42").await, "42");
    let shot = capture(&mut c, other, None, false).await;
    assert!(shot.lines.iter().any(|l| l == "42"), "{:?}", shot.lines);
    assert!(shot.size.0 > 0 && shot.size.1 > 0);
    assert!(!shot.alternate_screen && shot.truncated == 0);
    // Nothing reached the first terminal.
    assert!(
        !capture(&mut c, first, None, false)
            .await
            .lines
            .iter()
            .any(|l| l == "42")
    );
    // Unknown and gone terminals.
    let unknown = TerminalId(9999);
    assert_eq!(
        error(
            &mut c,
            Request::Capture {
                terminal: unknown,
                lines: None,
                history: false,
                ansi: false
            }
        )
        .await,
        "no terminal 9999"
    );
    assert_eq!(
        error(
            &mut c,
            Request::Send {
                terminal: unknown,
                bytes: Vec::new(),
                paste: false
            }
        )
        .await,
        "no terminal 9999"
    );
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_paste_is_wrapped_only_when_the_terminal_asked_for_it() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon).await;
    let on = new_terminal(
        &mut c,
        &format!("printf '\\033[?2004h'; {SHOW_BYTES}"),
        false,
    )
    .await;
    let off = new_terminal(&mut c, SHOW_BYTES, false).await;
    for t in [on, off] {
        screen_has(&mut c, t, "ready").await;
        ok(
            &mut c,
            Request::Send {
                terminal: t,
                bytes: b"ab".to_vec(),
                paste: true,
            },
        )
        .await;
        ok(
            &mut c,
            Request::Send {
                terminal: t,
                bytes: b"\r".to_vec(),
                paste: false,
            },
        )
        .await;
    }
    assert_eq!(screen_has(&mut c, on, "^M").await, "^[[200~ab^[[201~^M");
    assert_eq!(screen_has(&mut c, off, "^M").await, "ab^M");
    // The end marker inside pasted text cannot end the paste early.
    ok(
        &mut c,
        Request::Send {
            terminal: on,
            bytes: b"x\x1b[201~y".to_vec(),
            paste: true,
        },
    )
    .await;
    // Every ESC is gone from the text; the rest of the line may wrap.
    let line = screen_has(&mut c, on, "^[[200~x[201~y").await;
    assert!(line.ends_with("^[[200~x[201~y^[[201") || line.ends_with("^[[200~x[201~y^[[201~"));
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn keys_are_encoded_for_the_terminals_modes() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon).await;
    let app = new_terminal(&mut c, &format!("printf '\\033[?1h'; {SHOW_BYTES}"), false).await;
    let normal = new_terminal(&mut c, SHOW_BYTES, false).await;
    for t in [app, normal] {
        screen_has(&mut c, t, "ready").await;
        ok(
            &mut c,
            Request::SendKeys {
                terminal: t,
                keys: vec![key("Up"), key("C-a"), key("M-x"), key("Enter")],
            },
        )
        .await;
    }
    assert_eq!(screen_has(&mut c, app, "^M").await, "^[OA^A^[x^M");
    assert_eq!(screen_has(&mut c, normal, "^M").await, "^[[A^A^[x^M");
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn ctrl_c_interrupts_a_running_command() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon).await;
    let t = new_terminal(&mut c, "exec sh -i", false).await;
    screen_has(&mut c, t, "$").await;
    ok(
        &mut c,
        Request::Send {
            terminal: t,
            bytes: b"sleep 100\r".to_vec(),
            paste: false,
        },
    )
    .await;
    // The command line was echoed and nothing else comes: the sleep runs.
    assert_eq!(
        wait(&mut c, t, WaitFor::Idle { quiet_ms: 300 }, Some(4000)).await,
        Response::Waited(Waited::Idle)
    );
    // The shell would be busy for 100 s; after the interrupt it runs the next command.
    ok(
        &mut c,
        Request::SendKeys {
            terminal: t,
            keys: vec![key("C-c")],
        },
    )
    .await;
    // Once the shell is quiet again it took the interrupt and shows its prompt.
    assert_eq!(
        wait(&mut c, t, WaitFor::Idle { quiet_ms: 300 }, Some(4000)).await,
        Response::Waited(Waited::Idle)
    );
    ok(
        &mut c,
        Request::Send {
            terminal: t,
            bytes: b"echo $((40+2))\r".to_vec(),
            paste: false,
        },
    )
    .await;
    assert_eq!(screen_has(&mut c, t, "42").await, "42");
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn typing_into_an_exited_kept_terminal_fails_but_it_can_be_read_and_waited_on() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon).await;
    let t = new_terminal(&mut c, "echo bye; exit 4", true).await;
    assert_eq!(
        wait(&mut c, t, WaitFor::Exit, Some(4000)).await,
        Response::Waited(Waited::Exit(4))
    );
    let text = format!("terminal {t} has exited");
    assert_eq!(
        error(
            &mut c,
            Request::Send {
                terminal: t,
                bytes: b"x".to_vec(),
                paste: false
            }
        )
        .await,
        text
    );
    assert_eq!(
        error(
            &mut c,
            Request::SendKeys {
                terminal: t,
                keys: vec![key("Enter")]
            }
        )
        .await,
        text
    );
    assert_eq!(capture(&mut c, t, None, false).await.lines, ["bye"]);
    assert_eq!(screen_has(&mut c, t, "bye").await, "bye");
    // Another wait on it answers at once.
    assert_eq!(
        wait(&mut c, t, WaitFor::Exit, None).await,
        Response::Waited(Waited::Exit(4))
    );
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn waiting_for_exit_reports_the_code_and_a_signal_as_128_plus_n() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon).await;
    for (script, code) in [("exit 124", 124), ("exit 0", 0), ("kill -TERM $$", 143)] {
        let t = new_terminal(&mut c, &format!("sleep 0.3; {script}"), false).await;
        assert_eq!(
            wait(&mut c, t, WaitFor::Exit, Some(4000)).await,
            Response::Waited(Waited::Exit(code)),
            "{script}"
        );
        // The terminal left the daemon: the code is still answered.
        assert_eq!(
            wait(&mut c, t, WaitFor::Exit, None).await,
            Response::Waited(Waited::Exit(code)),
            "{script}, late"
        );
    }
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_closed_terminal_still_answers_with_the_status_its_hang_up_produced() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon).await;
    let t = new_terminal(&mut c, "sleep 100", false).await;
    let mut waiter = control(&daemon).await;
    send_wait(&mut waiter, t, WaitFor::Exit, None).await;
    ok(&mut c, Request::Close { id: t.0 }).await;
    // SIGHUP.
    assert_eq!(
        answer(&mut waiter, Duration::from_secs(5)).await,
        Some(Response::Waited(Waited::Exit(129)))
    );
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn wait_text_answers_with_the_first_line_and_on_the_alternate_screen() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon).await;
    let shown = new_terminal(
        &mut c,
        "echo one; echo ready now; echo ready later; sleep 30",
        false,
    )
    .await;
    assert_eq!(screen_has(&mut c, shown, "ready").await, "ready now");
    let alt = new_terminal(
        &mut c,
        "printf '\\033[?1049hon the alternate screen'; sleep 30",
        false,
    )
    .await;
    assert_eq!(
        screen_has(&mut c, alt, "alternate").await,
        "on the alternate screen"
    );
    // A later appearance wakes the wait.
    let late = new_terminal(&mut c, "sleep 0.5; echo arrived; sleep 30", false).await;
    let started = Instant::now();
    assert_eq!(screen_has(&mut c, late, "arrived").await, "arrived");
    assert!(started.elapsed() >= Duration::from_millis(300));
    // Text never spans lines.
    assert_eq!(
        wait(&mut c, shown, WaitFor::Text("one\nready".into()), Some(0)).await,
        Response::Waited(Waited::TimedOut)
    );
    daemon.stop().await.unwrap();
}

/// A daemon whose clock moves only when the test says so, and its waiter counter.
async fn manual_daemon() -> (TestDaemon, Arc<ManualClock>, Arc<AtomicUsize>) {
    let clock = Arc::new(ManualClock::new());
    let waiters = Arc::new(AtomicUsize::new(0));
    let daemon = TestDaemon::start_with(
        config(),
        Options {
            clock: clock.clone(),
            waiters: waiters.clone(),
            ..Options::default()
        },
    )
    .await;
    (daemon, clock, waiters)
}

async fn control(daemon: &TestDaemon) -> Client {
    Client::control(&daemon.paths.socket).await
}

async fn send_wait(c: &mut Client, terminal: TerminalId, until: WaitFor, timeout_ms: Option<u64>) {
    c.send(&deck_core::protocol::ClientMsg::Request(Request::Wait {
        terminal,
        until,
        timeout_ms,
    }))
    .await;
}

async fn answer(c: &mut Client, within: Duration) -> Option<Response> {
    let deadline = Instant::now() + within;
    loop {
        let left = deadline.checked_duration_since(Instant::now())?;
        match c.recv(left).await {
            Some(ServerMsg::Response(r)) => return Some(r),
            Some(_) => {}
            None => return None,
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn timeout_and_idle_are_exact_against_the_daemons_clock() {
    let (daemon, clock, waiters) = manual_daemon().await;
    let mut c = attach(&daemon).await;
    let t = c.terminal();
    // Let the shell print its prompt (at clock 0), then watch.
    screen_has(&mut c, t, "$").await;
    let mut a = control(&daemon).await;
    send_wait(&mut a, t, WaitFor::Idle { quiet_ms: 500 }, None).await;
    let mut b = control(&daemon).await;
    send_wait(&mut b, t, WaitFor::Text("never".into()), Some(2000)).await;
    until_waiters(&waiters, 2).await;
    let step = |by: u64| clock.advance(Duration::from_millis(by));

    step(499);
    assert_eq!(
        answer(&mut a, Duration::from_millis(300)).await,
        None,
        "499 ms is not 500"
    );
    step(1);
    assert_eq!(
        answer(&mut a, Duration::from_secs(2)).await,
        Some(Response::Waited(Waited::Idle))
    );
    // 500 ms of the 2000 passed.
    step(1499);
    assert_eq!(
        answer(&mut b, Duration::from_millis(300)).await,
        None,
        "1999 ms is not 2000"
    );
    step(1);
    assert_eq!(
        answer(&mut b, Duration::from_secs(2)).await,
        Some(Response::Waited(Waited::TimedOut))
    );

    // `--idle 0` answers at once, and output restarts the quiet period.
    let mut z = control(&daemon).await;
    send_wait(&mut z, t, WaitFor::Idle { quiet_ms: 0 }, None).await;
    assert_eq!(
        answer(&mut z, Duration::from_secs(2)).await,
        Some(Response::Waited(Waited::Idle))
    );
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn idle_counts_from_the_last_output() {
    let (daemon, clock, waiters) = manual_daemon().await;
    let mut c = attach(&daemon).await;
    let t = c.terminal();
    screen_has(&mut c, t, "$").await;
    let mut a = control(&daemon).await;
    send_wait(&mut a, t, WaitFor::Idle { quiet_ms: 500 }, None).await;
    until_waiters(&waiters, 1).await;
    // The terminal prints at 300 ms: the quiet period ends at 800 ms, not at 500 ms.
    clock.advance(Duration::from_millis(300));
    ok(
        &mut c,
        Request::Send {
            terminal: t,
            bytes: b"echo $((40+2))\r".to_vec(),
            paste: false,
        },
    )
    .await;
    screen_has(&mut c, t, "42").await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    clock.advance(Duration::from_millis(400));
    assert_eq!(
        answer(&mut a, Duration::from_millis(300)).await,
        None,
        "700 ms"
    );
    clock.advance(Duration::from_millis(100));
    assert_eq!(
        answer(&mut a, Duration::from_secs(2)).await,
        Some(Response::Waited(Waited::Idle))
    );
    daemon.stop().await.unwrap();
}

async fn until_waiters(waiters: &AtomicUsize, n: usize) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while waiters.load(Ordering::Acquire) != n {
        assert!(Instant::now() < deadline, "expected {n} waiters");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_exit_code_is_kept_for_sixty_seconds() {
    let (daemon, clock, _) = manual_daemon().await;
    let mut c = attach(&daemon).await;
    let t = new_terminal(&mut c, "exit 4", false).await;
    // Gone from the tree: it was not kept.
    c.wait(|c| {
        c.view
            .as_ref()
            .is_some_and(|v| v.projects[0].panels[0].terminals.iter().all(|x| x.id != t))
    })
    .await;
    clock.advance(Duration::from_secs(59));
    assert_eq!(
        wait(&mut c, t, WaitFor::Exit, None).await,
        Response::Waited(Waited::Exit(4))
    );
    clock.advance(Duration::from_secs(1));
    assert_eq!(
        error(
            &mut c,
            Request::Wait {
                terminal: t,
                until: WaitFor::Exit,
                timeout_ms: None
            }
        )
        .await,
        format!("no terminal {t}")
    );
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_kept_terminals_exit_code_is_kept_for_sixty_seconds_after_its_removal() {
    let (daemon, clock, _) = manual_daemon().await;
    let mut c = attach(&daemon).await;
    let t = new_terminal(&mut c, "exit 4", true).await;
    assert_eq!(
        wait(&mut c, t, WaitFor::Exit, None).await,
        Response::Waited(Waited::Exit(4))
    );
    // Closed long after it exited.
    clock.advance(Duration::from_secs(100));
    ok(&mut c, Request::Close { id: t.0 }).await;
    c.wait(|c| {
        c.view
            .as_ref()
            .is_some_and(|v| v.projects[0].panels[0].terminals.iter().all(|x| x.id != t))
    })
    .await;
    clock.advance(Duration::from_secs(59));
    assert_eq!(
        wait(&mut c, t, WaitFor::Exit, None).await,
        Response::Waited(Waited::Exit(4))
    );
    clock.advance(Duration::from_secs(1));
    assert_eq!(
        error(
            &mut c,
            Request::Wait {
                terminal: t,
                until: WaitFor::Exit,
                timeout_ms: None
            }
        )
        .await,
        format!("no terminal {t}")
    );
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_killed_session_fails_a_wait_for_exit_too() {
    let (daemon, _clock, waiters) = manual_daemon().await;
    let _first = attach(&daemon).await;
    let other = Client::attach(
        &daemon.paths.socket,
        Attach {
            session: Some("other".into()),
            ..Attach::default()
        },
    )
    .await;
    let mut waiting = control(&daemon).await;
    send_wait(&mut waiting, other.terminal(), WaitFor::Exit, None).await;
    until_waiters(&waiters, 1).await;
    let mut admin = control(&daemon).await;
    ok(
        &mut admin,
        Request::KillSession {
            name: "other".into(),
        },
    )
    .await;
    assert_eq!(
        answer(&mut waiting, Duration::from_secs(5)).await,
        Some(Response::Error("session was killed".into()))
    );
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pending_wait_ends_when_its_terminal_session_or_daemon_goes_and_with_its_client() {
    let (daemon, _clock, waiters) = manual_daemon().await;
    let mut c = attach(&daemon).await;

    // The client goes away: no waiter is left.
    let t = new_terminal(&mut c, "sleep 100", false).await;
    let mut gone = control(&daemon).await;
    send_wait(&mut gone, t, WaitFor::Text("never".into()), None).await;
    until_waiters(&waiters, 1).await;
    drop(gone);
    until_waiters(&waiters, 0).await;

    // The terminal is closed: text and idle fail, exit answers.
    let mut text = control(&daemon).await;
    send_wait(&mut text, t, WaitFor::Text("never".into()), None).await;
    let mut idle = control(&daemon).await;
    send_wait(&mut idle, t, WaitFor::Idle { quiet_ms: 10_000 }, None).await;
    let mut exit = control(&daemon).await;
    send_wait(&mut exit, t, WaitFor::Exit, None).await;
    until_waiters(&waiters, 3).await;
    ok(&mut c, Request::Close { id: t.0 }).await;
    let closed = Response::Error(format!("terminal {t} was closed"));
    assert_eq!(
        answer(&mut text, Duration::from_secs(5)).await,
        Some(closed.clone())
    );
    assert_eq!(
        answer(&mut idle, Duration::from_secs(5)).await,
        Some(closed)
    );
    assert_eq!(
        answer(&mut exit, Duration::from_secs(5)).await,
        Some(Response::Waited(Waited::Exit(129)))
    );
    assert_eq!(waiters.load(Ordering::Acquire), 0);

    // The session is killed.
    let other = Client::attach(
        &daemon.paths.socket,
        Attach {
            session: Some("other".into()),
            ..Attach::default()
        },
    )
    .await;
    let t2 = other.terminal();
    let mut killed = control(&daemon).await;
    send_wait(&mut killed, t2, WaitFor::Text("never".into()), None).await;
    until_waiters(&waiters, 1).await;
    let mut admin = control(&daemon).await;
    ok(
        &mut admin,
        Request::KillSession {
            name: "other".into(),
        },
    )
    .await;
    assert_eq!(
        answer(&mut killed, Duration::from_secs(5)).await,
        Some(Response::Error("session was killed".into()))
    );

    // The daemon stops.
    let mut stopping = control(&daemon).await;
    send_wait(
        &mut stopping,
        c.terminal(),
        WaitFor::Text("never".into()),
        None,
    )
    .await;
    until_waiters(&waiters, 1).await;
    drop(other);
    let stopped = tokio::spawn(async move {
        let mut seen = None;
        while let Some(msg) = stopping.recv(Duration::from_secs(10)).await {
            match msg {
                ServerMsg::Response(Response::Error(text)) => seen = Some(text),
                ServerMsg::Bye {
                    reason: ByeReason::ServerShutdown,
                } => {
                    seen.get_or_insert_with(|| "deck daemon stopped".to_string());
                }
                _ => {}
            }
        }
        seen
    });
    daemon.stop().await.unwrap();
    assert_eq!(
        stopped.await.unwrap().as_deref(),
        Some("deck daemon stopped")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn capture_reads_history_the_alternate_screen_and_wide_characters() {
    let daemon = TestDaemon::start_with(
        Config {
            scrollback_lines: 30_000,
            ..config()
        },
        Options::default(),
    )
    .await;
    let mut c = attach(&daemon).await;
    let t = new_terminal(&mut c, "seq 1 100; echo done; exec sh", false).await;
    screen_has(&mut c, t, "done").await;
    // Let the shell print its prompt.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let shot = capture(&mut c, t, Some(3), true).await;
    assert_eq!(shot.lines[0], "100", "{:?}", shot.lines);
    assert_eq!(shot.lines[1], "done");
    let all = capture(&mut c, t, None, true).await;
    assert_eq!(all.lines[0], "1");
    assert!(all.lines.len() > 100);
    let screen_only = capture(&mut c, t, None, false).await;
    assert!(screen_only.lines.len() <= usize::from(screen_only.size.1));

    let wide = new_terminal(&mut c, "printf '日本 🙂 e\\314\\201x\\n'; sleep 30", false).await;
    screen_has(&mut c, wide, "日本").await;
    assert_eq!(
        capture(&mut c, wide, None, false).await.lines,
        ["日本 🙂 e\u{301}x"]
    );

    let alt = new_terminal(
        &mut c,
        "seq 1 50; printf '\\033[?1049h\\033[Hless'; sleep 30",
        false,
    )
    .await;
    screen_has(&mut c, alt, "less").await;
    let shot = capture(&mut c, alt, None, true).await;
    assert!(shot.alternate_screen);
    assert_eq!(shot.lines, ["less"]);

    // A capture is capped, and says so.
    let big = new_terminal(&mut c, "seq 1 20000; echo finished; sleep 30", false).await;
    screen_has(&mut c, big, "finished").await;
    let shot = capture(&mut c, big, None, true).await;
    assert_eq!(shot.lines.len(), 10_000);
    // 20000 numbers and `finished`.
    assert_eq!(shot.truncated, 10_001);
    assert_eq!(shot.lines.last().map(String::as_str), Some("finished"));
    daemon.stop().await.unwrap();
}
