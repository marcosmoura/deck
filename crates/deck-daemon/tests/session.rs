//! Sessions, PTYs, frames, viewports, environment, exit and backpressure.

mod support;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use deck_core::action::Action;
use deck_core::protocol::{ByeReason, ClientMsg, Request, Response, Role, ServerMsg};
use deck_daemon::{ManualClock, Options};
use support::{Attach, Client, TIMEOUT, TestDaemon, config, hello, sys, system_clock};

#[tokio::test(flavor = "multi_thread")]
async fn a_new_client_gets_main_with_a_shell_sized_by_its_attach() {
    let daemon = TestDaemon::start().await;
    let cwd = tempfile::Builder::new()
        .prefix("proj")
        .tempdir_in("/tmp")
        .unwrap();
    let mut client = Client::attach(
        &daemon.paths.socket,
        Attach {
            cwd: cwd.path().to_path_buf(),
            ..Attach::default()
        },
    )
    .await;

    // Welcome, the view, then a full frame, in that order.
    assert!(matches!(client.log[0], ServerMsg::Welcome { .. }));
    assert!(matches!(client.log[1], ServerMsg::SessionView(_)));
    let ServerMsg::Frame(first) = &client.log[2] else {
        panic!("a frame third: {:?}", client.log[2])
    };
    assert!(first.full);
    assert_eq!(first.size, (80, 24));

    let view = client.view.clone().unwrap();
    assert_eq!(view.name, "main");
    assert_eq!(view.layout_viewport, Some((80, 24)));
    let project = &view.projects[0];
    assert_eq!(
        project.name,
        cwd.path().file_name().unwrap().to_string_lossy()
    );
    assert_eq!(project.root, cwd.path());
    assert_eq!(project.panels[0].name, "1");

    client.type_line("stty size").await;
    client.wait_row("24 80").await;
    assert!(client.frames().count() > 1);

    let mut control = Client::control(&daemon.paths.socket).await;
    let Response::Sessions(sessions) = control.request(Request::ListSessions).await else {
        panic!("sessions")
    };
    assert_eq!(sessions.len(), 1);
    assert_eq!(
        (
            sessions[0].name.as_str(),
            sessions[0].attached_clients,
            sessions[0].terminals
        ),
        ("main", 1, 1)
    );
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn input_shows_up_in_frames_and_a_new_client_gets_the_screen_in_full() {
    let daemon = TestDaemon::start().await;
    let mut first = Client::attach(&daemon.paths.socket, Attach::default()).await;
    first.type_line("echo hi").await;
    first.wait_row("hi").await;
    drop(first);

    let second = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let ServerMsg::Frame(frame) = &second.log[2] else {
        panic!("a frame third")
    };
    assert!(frame.full);
    assert!(second.has_row("hi"), "{:?}", second.screen());
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_viewport_resizes_the_pty_and_the_view() {
    let daemon = TestDaemon::start().await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    client
        .send(&ClientMsg::Viewport {
            cols: 100,
            rows: 30,
        })
        .await;
    client
        .wait(|c| c.view.as_ref().and_then(|v| v.layout_viewport) == Some((100, 30)))
        .await;
    client.type_line("stty size").await;
    client.wait_row("30 100").await;
    assert!(client.frames().any(|f| f.full && f.size == (100, 30)));
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_pty_follows_the_client_that_was_active_last() {
    let daemon = TestDaemon::start().await;
    let mut a = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let mut b = Client::attach(
        &daemon.paths.socket,
        Attach {
            cols: 60,
            rows: 18,
            ..Attach::default()
        },
    )
    .await;
    // Attaching to an existing session is a viewport claim too.
    assert_eq!(b.view.as_ref().unwrap().layout_viewport, Some((60, 18)));

    b.type_line("stty size").await;
    b.wait_row("18 60").await;
    a.wait_row("18 60").await;
    assert_eq!(
        a.view.as_ref().unwrap().layout_viewport,
        Some((60, 18)),
        "both clients get the new view"
    );
    assert_eq!(b.view.as_ref().unwrap().layout_viewport, Some((60, 18)));

    a.type_line("stty size").await;
    a.wait_row("24 80").await;
    b.wait_row("24 80").await;
    assert_eq!(b.view.as_ref().unwrap().layout_viewport, Some((80, 24)));
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn terminals_get_the_clients_environment_and_nothing_of_the_daemons() {
    let daemon = TestDaemon::start().await;
    let mut env = support::hello_env();
    env.push(("FOO".into(), "bar".into()));
    env.push(("DECK_TERMINAL_ID".into(), "99".into()));
    env.push(("TMUX".into(), "x".into()));
    // Not in the Hello, but certainly in this process's environment.
    assert!(std::env::var_os("HOME").is_some());
    let mut client = Client::attach(
        &daemon.paths.socket,
        Attach {
            env,
            ..Attach::default()
        },
    )
    .await;
    let id = client.terminal();
    client
        .type_line("echo \"F=$FOO T=$DECK_TERMINAL_ID M=${TMUX-unset} H=${HOME-unset} C=$COLORTERM $TERM\"")
        .await;
    client
        .wait_row(&format!(
            "F=bar T={id} M=unset H=unset C=truecolor xterm-256color"
        ))
        .await;
    assert_ne!(id.0, 99);
    client
        .type_line("echo \"S=$DECK_SESSION_ID K=$DECK_SOCKET\"")
        .await;
    let view = client.view.clone().unwrap();
    client
        .wait_row(&format!(
            "S={} K={}",
            view.id,
            daemon.paths.socket.display()
        ))
        .await;
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shell_that_exits_leaves_the_session_with_an_empty_project_after_its_last_output() {
    let daemon = TestDaemon::start().await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    client.type_line("printf 'a\\nb'; exit").await;
    client
        .wait(|c| {
            c.view
                .as_ref()
                .is_some_and(|v| v.projects[0].panels.is_empty())
        })
        .await;
    // The screen as of the last frame before the view has the final output.
    assert!(
        client.screen().iter().any(|r| r.starts_with('b')),
        "{:?}",
        client.screen()
    );
    let view = client.view.clone().unwrap();
    assert_eq!(view.projects.len(), 1);
    assert_eq!(view.active_panel, None);
    assert_eq!(client.bye, None, "the client stays attached");

    let mut control = Client::control(&daemon.paths.socket).await;
    let Response::Sessions(sessions) = control.request(Request::ListSessions).await else {
        panic!("sessions")
    };
    assert_eq!(sessions.len(), 1);
    assert_eq!((sessions[0].terminals, sessions[0].projects), (0, 1));
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_background_job_holding_the_pty_does_not_keep_the_terminal_alive() {
    let daemon = TestDaemon::start().await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    client
        .type_line("sleep 1000 & jobs -p; echo BGSET; exit")
        .await;
    client.wait_row("BGSET").await;
    let bg: i32 = client
        .screen()
        .iter()
        .find_map(|r| {
            let r = r.trim();
            (!r.is_empty() && r.chars().all(|c| c.is_ascii_digit())).then(|| r.parse().ok())?
        })
        .expect("jobs -p prints the pid");
    client
        .wait(|c| {
            c.view
                .as_ref()
                .is_some_and(|v| v.projects[0].panels.is_empty())
        })
        .await;
    assert_eq!(client.bye, None);
    daemon.stop().await.unwrap();
    // Clean up what the test started.
    sys::kill9(bg);
    let deadline = Instant::now() + TIMEOUT;
    while sys::alive(bg) && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!sys::alive(bg));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shell_that_fails_at_once_ends_the_attach_with_its_status() {
    let mut cfg = config();
    cfg.shell_args = vec!["-c".into(), "exit 3".into()];
    let daemon = TestDaemon::start_with(cfg, Options::default()).await;
    let mut client = daemon.connect().await;
    client
        .send(&hello(
            Role::Attach {
                session: None,
                create: true,
                cols: 82,
                rows: 26,
            },
            std::env::temp_dir(),
            support::hello_env(),
            None,
        ))
        .await;
    client.wait(|c| c.bye.is_some()).await;
    let Some(ByeReason::Error(message)) = &client.bye else {
        panic!("{:?}", client.bye)
    };
    assert!(
        message.starts_with("terminal exited at once (status 3); see "),
        "{message}"
    );
    assert!(message.contains(daemon.paths.log_file.to_str().unwrap()));
    daemon.wait_exit().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_cwd_is_an_error_not_a_shell_in_the_home_dir() {
    let daemon = TestDaemon::start().await;
    let mut client = daemon.connect().await;
    client
        .send(&hello(
            Role::Attach {
                session: None,
                create: true,
                cols: 82,
                rows: 26,
            },
            PathBuf::from("/nonexistent/deck-test"),
            support::hello_env(),
            None,
        ))
        .await;
    client.wait(|c| c.bye.is_some()).await;
    let Some(ByeReason::Error(message)) = &client.bye else {
        panic!("{:?}", client.bye)
    };
    assert!(
        message.starts_with("could not start /bin/sh: "),
        "{message}"
    );
    assert!(message.contains("/nonexistent/deck-test"), "{message}");
    // Nothing was created.
    let mut control = Client::control(&daemon.paths.socket).await;
    assert_eq!(
        control.request(Request::ListSessions).await,
        Response::Sessions(vec![])
    );
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn kill_server_hangs_up_the_shells() {
    let daemon = TestDaemon::start().await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    client.type_line("echo PID=$$").await;
    client.wait(|c| c.number_after("PID=").is_some()).await;
    let pid = client.number_after("PID=").unwrap();
    assert!(sys::alive(pid));

    let mut control = Client::control(&daemon.paths.socket).await;
    assert_eq!(control.request(Request::KillServer).await, Response::Ok);
    client.wait(|c| c.bye.is_some()).await;
    assert_eq!(client.bye, Some(ByeReason::ServerShutdown));
    daemon.wait_exit().await.unwrap();
    assert!(!sys::alive(pid), "the shell survived the daemon");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stalled_client_does_not_stop_another_and_resyncs_when_it_reads_again() {
    let daemon = TestDaemon::start().await;
    let mut fast = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let mut stalled = Client::attach(&daemon.paths.socket, Attach::default()).await;
    // `stalled` now stops reading.
    fast.type_line("yes").await;
    let seen = fast.frames().count();
    let deadline = Instant::now() + Duration::from_secs(2);
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        fast.recv(left).await;
    }
    let got = fast.frames().count() - seen;
    assert!(got >= 10, "only {got} frames in 2 s");

    // Stop the flood and change the size; the stalled client then reads again.
    fast.send(&ClientMsg::Input {
        terminal: fast.terminal(),
        bytes: vec![3],
    })
    .await;
    fast.send(&ClientMsg::Viewport {
        cols: 100,
        rows: 30,
    })
    .await;
    stalled
        .wait(|c| c.view.as_ref().and_then(|v| v.layout_viewport) == Some((100, 30)))
        .await;
    stalled
        .wait(|c| c.frames().any(|f| f.full && f.size == (100, 30)))
        .await;
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thousand_title_changes_cause_few_views_and_the_last_title_arrives() {
    // A clock that never moves makes the limit exact: one view at once, one trailing.
    let clock = Arc::new(ManualClock::new());
    let daemon = TestDaemon::start_with(
        config(),
        Options {
            clock: clock.clone(),
            protocol_version: deck_core::protocol::PROTOCOL_VERSION,
        },
    )
    .await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let before = client.views().count();
    client
        .type_line(
            "i=0; while [ $i -lt 1000 ]; do printf '\\033]0;t%d\\007' $i; i=$((i+1)); done; echo DONE",
        )
        .await;
    client.wait_row("DONE").await;
    // The trailing view waits for the clock, not for real time.
    assert_eq!(client.views().count() - before, 1);
    clock.advance(Duration::from_millis(150));
    client
        .wait(|c| {
            c.view
                .as_ref()
                .map(|v| v.projects[0].panels[0].terminals[0].title.clone())
                == Some(Some("t999".to_string()))
        })
        .await;
    let sent = client.views().count() - before;
    assert_eq!(sent, 2, "one view at once, one trailing");
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn input_for_a_terminal_of_another_session_is_ignored() {
    let daemon = TestDaemon::start().await;
    let mut a = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let mut b = Client::attach(
        &daemon.paths.socket,
        Attach {
            session: Some("other".into()),
            ..Attach::default()
        },
    )
    .await;
    assert_ne!(a.view.as_ref().unwrap().id, b.view.as_ref().unwrap().id);
    a.send(&ClientMsg::Input {
        terminal: b.terminal(),
        bytes: b"echo SNEAKY\n".to_vec(),
    })
    .await;
    // The daemon handles one connection's messages in order.
    a.request(Request::ListSessions).await;
    b.type_line("echo OK2").await;
    b.wait_row("OK2").await;
    assert!(!b.has_row("SNEAKY"), "{:?}", b.screen());
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn project_names_follow_the_directory() {
    let daemon = TestDaemon::start().await;
    let root = Client::attach(
        &daemon.paths.socket,
        Attach {
            session: Some("r".into()),
            cwd: "/".into(),
            ..Attach::default()
        },
    )
    .await;
    assert_eq!(root.view.as_ref().unwrap().projects[0].name, "root");

    let home = tempfile::Builder::new()
        .prefix("home")
        .tempdir_in("/tmp")
        .unwrap();
    let mut env = support::hello_env();
    env.push(("HOME".into(), home.path().into()));
    let at_home = Client::attach(
        &daemon.paths.socket,
        Attach {
            session: Some("h".into()),
            cwd: home.path().to_path_buf(),
            env,
            ..Attach::default()
        },
    )
    .await;
    assert_eq!(at_home.view.as_ref().unwrap().projects[0].name, "~");
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn session_selection_follows_the_attach_rules() {
    let daemon = TestDaemon::start_with(
        config(),
        Options {
            clock: system_clock(),
            protocol_version: deck_core::protocol::PROTOCOL_VERSION,
        },
    )
    .await;
    let socket = &daemon.paths.socket;
    let main = Client::attach(socket, Attach::default()).await;
    assert_eq!(main.view.as_ref().unwrap().name, "main");

    // A named session that does not exist is refused without `create`.
    let mut missing = daemon.connect().await;
    missing
        .send(&hello(
            Role::Attach {
                session: Some("nope".into()),
                create: false,
                cols: 80,
                rows: 24,
            },
            std::env::temp_dir(),
            support::hello_env(),
            None,
        ))
        .await;
    missing.wait(|c| c.bye.is_some()).await;
    assert_eq!(
        missing.bye,
        Some(ByeReason::Error("no session named nope".into()))
    );

    // With no name the most recently attached session is used.
    let mut x = Client::attach(
        socket,
        Attach {
            session: Some("x".into()),
            ..Attach::default()
        },
    )
    .await;
    let again = Client::attach(
        socket,
        Attach {
            session: None,
            ..Attach::default()
        },
    )
    .await;
    assert_eq!(again.view.as_ref().unwrap().name, "x");
    drop(again);

    // When that session is gone, the most recently attached one that still exists is used.
    x.send(&ClientMsg::Action(Action::KillSession)).await;
    x.wait(|c| c.bye.is_some()).await;
    let next = Client::attach(
        socket,
        Attach {
            session: None,
            ..Attach::default()
        },
    )
    .await;
    assert_eq!(next.view.as_ref().unwrap().name, "main");

    let mut control = Client::control(socket).await;
    let Response::Sessions(sessions) = control.request(Request::ListSessions).await else {
        panic!("sessions")
    };
    let names: Vec<_> = sessions.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["main"]);
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn extreme_viewports_are_clamped_and_wide_output_still_works() {
    let daemon = TestDaemon::start().await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    client.send(&ClientMsg::Viewport { cols: 3, rows: 3 }).await;
    client
        .wait(|c| c.view.as_ref().and_then(|v| v.layout_viewport) == Some((3, 3)))
        .await;
    client
        .type_line("printf '\\346\\227\\245\\346\\234\\254\\n'")
        .await;
    // Back to a normal size: the terminal still works after wide output in two columns.
    client
        .send(&ClientMsg::Viewport { cols: 82, rows: 26 })
        .await;
    client.type_line("echo ALIVE").await;
    client.wait_row("ALIVE").await;

    client
        .send(&ClientMsg::Viewport {
            cols: u16::MAX,
            rows: u16::MAX,
        })
        .await;
    client
        .wait(|c| c.frames().any(|f| f.full && f.size == (1000, 1000)))
        .await;
    client.type_line("stty size").await;
    client.wait_row("1000 1000").await;
    daemon.stop().await.unwrap();
}
