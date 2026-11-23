//! Projects, panels and sessions: actions, requests, lifecycle, focus and frame streaming.

#![allow(
    clippy::unwrap_used,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use deck_core::action::{Action, ActionError};
use deck_core::protocol::{ByeReason, ClientMsg, Request, Response, ServerMsg};
use support::{Attach, Client, TestDaemon, hello, sys};
use tokio::time::sleep;

fn dir(prefix: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in("/tmp")
        .unwrap()
}

/// A path whose `pwd` output is the same on macOS, where `/tmp` is a link.
fn real(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().canonicalize().unwrap()
}

async fn attach(daemon: &TestDaemon) -> Client {
    Client::attach(&daemon.paths.socket, Attach::default()).await
}

async fn new_panel(c: &mut Client, project: &str) {
    let project = c.project_id(project);
    c.action(Action::NewPanel {
        project,
        name: None,
    })
    .await
    .unwrap();
}

async fn wait_gone(pid: i32) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while sys::alive(pid) && Instant::now() < deadline {
        sleep(Duration::from_millis(20)).await;
    }
    assert!(!sys::alive(pid), "process {pid} is still running");
}

/// Types `echo PID=$$` in the active terminal and returns the shell's pid.
async fn shell_pid(c: &mut Client, tag: &str) -> i32 {
    c.type_line(&format!("echo {tag}=$$")).await;
    c.wait(|c| c.number_after(&format!("{tag}=")).is_some())
        .await;
    c.number_after(&format!("{tag}=")).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn invalid_names_duplicates_and_missing_directories_are_refused() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon).await;
    let root = dir("api");
    let new = |name: &str, root: PathBuf| Action::NewProject {
        name: name.into(),
        root,
    };

    assert_eq!(
        c.action(new("  ", real(&root))).await,
        Err(ActionError::EmptyName)
    );
    assert_eq!(
        c.action(new("a/b", real(&root))).await,
        Err(ActionError::NameHasSlash)
    );
    assert_eq!(
        c.action(new("nope", "/nope".into())).await,
        Err(ActionError::NotADirectory("/nope".into()))
    );
    c.action(new(" api ", real(&root))).await.unwrap();
    let err = c.action(new("api", real(&root))).await.unwrap_err();
    assert_eq!(err.to_string(), "a space named \"api\" already exists");
    let api = c.project_id("api");
    assert_eq!(
        c.action(Action::RenameProject {
            project: api,
            name: "x".repeat(65)
        })
        .await,
        Err(ActionError::NameTooLong)
    );
    assert_eq!(
        c.action(Action::RenameSession { name: "a/b".into() }).await,
        Err(ActionError::NameHasSlash)
    );
    assert_eq!(
        c.action(Action::RenameSession { name: "".into() }).await,
        Err(ActionError::EmptyName)
    );
    // Nothing of that changed the tree.
    c.wait(|c| c.tree().len() == 2).await;
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_project_gets_panel_1_with_a_shell_in_its_root_and_focus() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon).await;
    let root = dir("proj");
    c.action(Action::NewProject {
        name: "api".into(),
        root: real(&root),
    })
    .await
    .unwrap();
    c.wait(|c| c.view.as_ref().is_some_and(|v| v.projects.len() == 2))
        .await;

    let view = c.view.clone().unwrap();
    let api = &view.projects[1];
    assert_eq!(api.name, "api");
    assert_eq!(api.root, real(&root));
    assert_eq!(api.panels.len(), 1);
    assert_eq!(api.panels[0].name, "sh", "named after its program");
    assert_eq!(
        api.panels[0].name_source,
        deck_core::model::NameSource::Auto
    );
    assert_eq!(view.active_project, Some(api.id));
    assert_eq!(view.active_panel, Some(api.panels[0].id));
    assert_eq!(api.panels[0].terminals[0].program, "sh");

    c.type_line("echo \"HERE=$(pwd)\"").await;
    c.wait_row(&format!("HERE={}", real(&root).display())).await;
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn panels_get_the_lowest_unused_number() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon).await;
    let main = c.tree()[0].0.clone();
    new_panel(&mut c, &main).await;
    new_panel(&mut c, &main).await;
    // Panels are named after their shell at once; the numbers never show.
    assert_eq!(c.tree()[0].1, ["sh", "sh", "sh"]);

    let [one, two, three] = [1, 2, 3].map(|n| c.panel_at(&main, n));
    c.action(Action::ClosePanel { panel: two }).await.unwrap();
    assert_eq!(c.tree()[0].1, ["sh", "sh"]);
    new_panel(&mut c, &main).await;
    let ids: Vec<_> = c.view.as_ref().unwrap().projects[0]
        .panels
        .iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(ids[..2], [one, three]);

    let view = c.view.clone().unwrap();
    let last = view.projects[0].panels.last().unwrap();
    assert_eq!(view.active_panel, Some(last.id), "the new panel is active");

    // A manual name is kept and does not take a number.
    let project = c.project_id(&main);
    c.action(Action::NewPanel {
        project,
        name: Some(" logs ".into()),
    })
    .await
    .unwrap();
    let view = c.view.clone().unwrap();
    let logs = view.projects[0].panels.last().unwrap();
    assert_eq!(logs.name, "logs");
    assert_eq!(logs.name_source, deck_core::model::NameSource::Manual);
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn closing_a_panel_kills_its_process_and_moves_focus() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon).await;
    let main = c.tree()[0].0.clone();
    new_panel(&mut c, &main).await;
    new_panel(&mut c, &main).await;
    let active = |c: &Client| c.view.as_ref().unwrap().active_panel;

    // The middle panel closes: focus goes to the next one.
    let (p1, p2, p3) = (
        c.panel_at(&main, 1),
        c.panel_at(&main, 2),
        c.panel_at(&main, 3),
    );
    c.action(Action::FocusPanel { panel: p2 }).await.unwrap();
    let pid = shell_pid(&mut c, "P2").await;
    c.action(Action::ClosePanel { panel: p2 }).await.unwrap();
    wait_gone(pid).await;
    assert_eq!(active(&c), Some(p3), "next");

    // The last panel closes: focus goes to the previous one.
    c.action(Action::ClosePanel { panel: p3 }).await.unwrap();
    assert_eq!(active(&c), Some(p1), "previous");

    // The only panel closes: the project stays with no active panel.
    c.action(Action::ClosePanel { panel: p1 }).await.unwrap();
    assert_eq!(active(&c), None, "none");
    let view = c.view.clone().unwrap();
    assert_eq!(view.projects.len(), 1);
    assert!(view.projects[0].panels.is_empty());
    assert_eq!(view.active_project, Some(view.projects[0].id));
    assert_eq!(
        c.action(Action::ClosePanel { panel: p1 }).await,
        Err(ActionError::NoSuchPanel(p1))
    );
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn closing_a_project_kills_its_processes_and_moves_focus() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon).await;
    let root = dir("two");
    c.action(Action::NewProject {
        name: "two".into(),
        root: real(&root),
    })
    .await
    .unwrap();
    let pid = shell_pid(&mut c, "P").await;
    let (main, two) = (c.project_id(&c.tree()[0].0.clone()), c.project_id("two"));
    c.action(Action::CloseProject { project: two })
        .await
        .unwrap();
    wait_gone(pid).await;
    assert_eq!(
        c.view.as_ref().unwrap().active_project,
        Some(main),
        "previous"
    );
    c.action(Action::CloseProject { project: main })
        .await
        .unwrap();
    let view = c.view.clone().unwrap();
    assert!(view.projects.is_empty());
    assert_eq!((view.active_project, view.active_panel), (None, None));
    // An empty session is a valid view and stays.
    let mut control = Client::control(&daemon.paths.socket).await;
    let Response::Sessions(s) = control.request(Request::ListSessions).await else {
        panic!("sessions")
    };
    assert_eq!((s.len(), s[0].projects, s[0].terminals), (1, 0, 0));
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn exit_in_a_panels_only_terminal_removes_the_panel_and_keeps_the_rest() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon).await;
    let main = c.tree()[0].0.clone();
    new_panel(&mut c, &main).await;
    c.type_line("exit").await;
    c.wait(|c| c.tree()[0].1.len() == 1).await;
    let view = c.view.clone().unwrap();
    assert_eq!(view.active_panel, Some(view.projects[0].panels[0].id));
    assert_eq!(c.bye, None);

    c.type_line("echo STILL").await;
    c.wait_row("STILL").await;
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn cycling_follows_the_sidebar_order_and_skips_collapsed_projects() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon).await;
    let main = c.tree()[0].0.clone();
    new_panel(&mut c, &main).await; // main/2
    let a = dir("a");
    let b = dir("b");
    for (name, d) in [("a", &a), ("b", &b)] {
        c.action(Action::NewProject {
            name: name.into(),
            root: real(d),
        })
        .await
        .unwrap();
    }
    new_panel(&mut c, "a").await; // a/2
    let a_project = c.project_id("a");
    c.action(Action::SetProjectCollapsed {
        project: a_project,
        collapsed: true,
    })
    .await
    .unwrap();
    assert!(c.view.as_ref().unwrap().projects[1].collapsed);

    let expect = [
        c.panel_at(&main, 1),
        c.panel_at(&main, 2),
        c.panel_at("b", 1),
    ];
    let first = c.panel_at(&main, 1);
    c.action(Action::FocusPanel { panel: first }).await.unwrap();
    let mut seen = vec![first];
    for _ in 0..3 {
        c.action(Action::CyclePanel { delta: 1 }).await.unwrap();
        seen.push(c.view.as_ref().unwrap().active_panel.unwrap());
    }
    assert_eq!(seen, [expect[0], expect[1], expect[2], expect[0]]);
    c.action(Action::CyclePanel { delta: -1 }).await.unwrap();
    assert_eq!(c.view.as_ref().unwrap().active_panel, Some(expect[2]));

    // FocusProject returns to a project's last active panel.
    let main_project = c.project_id(&main);
    c.action(Action::FocusProject {
        project: main_project,
    })
    .await
    .unwrap();
    assert_eq!(c.view.as_ref().unwrap().active_panel, Some(expect[0]));
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn after_focus_panel_only_the_new_panels_full_frames_follow() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon).await;
    let main = c.tree()[0].0.clone();
    let first = c.active_terminal();
    new_panel(&mut c, &main).await;
    let second = c.active_terminal();
    assert_ne!(first, second);
    c.send(&ClientMsg::Viewport { cols: 60, rows: 18 }).await;
    c.wait(|c| c.view.as_ref().and_then(|v| v.layout_viewport) == Some((60, 18)))
        .await;
    c.type_line("echo ON2").await;
    // The prompt after the answer is the second panel's last output; no frame of it may
    // come after the focus change.
    c.wait(|c| {
        c.screens.get(&second).is_some_and(|rows| {
            let rows: Vec<&str> = rows.iter().map(|r| r.trim_end()).collect();
            rows.windows(2)
                .any(|w| matches!(w[0], "ON2" | "$ ON2") && w[1] == "$")
        })
    })
    .await;
    // Until the connection is quiet: a frame already on its way would land after the focus.
    while c.recv(Duration::from_millis(300)).await.is_some() {}

    let before = c.log.len();
    let p1 = c.panel_at(&main, 1);
    c.action(Action::FocusPanel { panel: p1 }).await.unwrap();
    let frames: Vec<_> = c.log[before..]
        .iter()
        .filter_map(|m| match m {
            ServerMsg::Frame(f) => Some(f.clone()),
            _ => None,
        })
        .collect();
    assert!(!frames.is_empty());
    assert!(
        frames.iter().all(|f| f.terminal == first && f.full),
        "first {first:?}, second {second:?}: {:?}",
        frames
            .iter()
            .map(|f| (f.terminal, f.full, f.size))
            .collect::<Vec<_>>()
    );
    assert!(frames.iter().all(|f| f.size == (60, 18)), "{frames:?}");

    // Input for the inactive panel still runs but is no longer streamed.
    c.send(&ClientMsg::Input {
        terminal: second,
        bytes: b"echo HIDDEN\n".to_vec(),
    })
    .await;
    c.type_line("echo VISIBLE").await;
    c.wait_row("VISIBLE").await;
    sleep(Duration::from_millis(150)).await;
    c.request(Request::ListSessions).await;
    let hidden = c.log[before..]
        .iter()
        .any(|m| matches!(m, ServerMsg::Frame(f) if f.terminal == second));
    assert!(!hidden, "frames of the inactive panel were streamed");

    // Coming back shows what it printed meanwhile.
    let p2 = c.panel_at(&main, 2);
    c.action(Action::FocusPanel { panel: p2 }).await.unwrap();
    c.wait_row("HIDDEN").await;
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn every_client_of_a_session_gets_the_new_view() {
    let daemon = TestDaemon::start().await;
    let mut a = attach(&daemon).await;
    let mut b = attach(&daemon).await;
    let main = a.tree()[0].0.clone();

    new_panel(&mut a, &main).await;
    b.wait(|c| c.tree()[0].1.len() == 2).await;
    let project = b.project_id(&main);
    b.action(Action::RenameProject {
        project,
        name: "renamed".into(),
    })
    .await
    .unwrap();
    a.wait(|c| c.tree()[0].0 == "renamed").await;

    let mut control = Client::control(&daemon.paths.socket).await;
    let response = control
        .request(Request::RenameSession {
            from: "main".into(),
            to: "work".into(),
        })
        .await;
    assert_eq!(response, Response::Ok);
    a.wait(|c| c.view.as_ref().unwrap().name == "work").await;
    b.wait(|c| c.view.as_ref().unwrap().name == "work").await;

    a.action(Action::RenameSession {
        name: "again".into(),
    })
    .await
    .unwrap();
    b.wait(|c| c.view.as_ref().unwrap().name == "again").await;
    // Both see the same focus.
    assert_eq!(
        a.view.as_ref().unwrap().active_panel,
        b.view.as_ref().unwrap().active_panel
    );
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn kill_session_ends_one_session_and_the_last_one_stops_the_daemon() {
    let daemon = TestDaemon::start().await;
    let socket = daemon.paths.socket.clone();
    let mut a = attach(&daemon).await;
    let mut b = Client::attach(
        &socket,
        Attach {
            session: Some("other".into()),
            ..Attach::default()
        },
    )
    .await;
    let pid = shell_pid(&mut b, "B").await;

    b.send(&ClientMsg::Action(Action::KillSession)).await;
    b.wait(|c| c.bye.is_some()).await;
    assert_eq!(b.bye, Some(ByeReason::SessionEnded));
    wait_gone(pid).await;
    a.type_line("echo A_LIVES").await;
    a.wait_row("A_LIVES").await;

    let mut control = Client::control(&socket).await;
    assert_eq!(
        control
            .request(Request::KillSession {
                name: "nope".into()
            })
            .await,
        Response::Error("no session named \"nope\"".into())
    );
    assert_eq!(
        control
            .request(Request::KillSession {
                name: "main".into()
            })
            .await,
        Response::Ok
    );
    a.wait(|c| c.bye.is_some()).await;
    assert_eq!(a.bye, Some(ByeReason::SessionEnded));
    daemon.wait_exit().await.unwrap();
    assert!(!socket.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn new_session_names_are_unique_and_default_to_main_n() {
    let daemon = TestDaemon::start().await;
    let mut control = Client::control(&daemon.paths.socket).await;
    let ns = dir("ns");
    let cwd = real(&ns);
    let new = |name: Option<&str>| Request::NewSession {
        name: name.map(Into::into),
        cwd: cwd.clone(),
    };

    let Response::Session { name, .. } = control.request(new(None)).await else {
        panic!("a session")
    };
    assert_eq!(name, "main");
    let Response::Session { name, .. } = control.request(new(None)).await else {
        panic!("a session")
    };
    assert_eq!(name, "main-2");
    let Response::Session { name, .. } = control.request(new(Some(" work "))).await else {
        panic!("a session")
    };
    assert_eq!(name, "work");
    assert_eq!(
        control.request(new(Some("work"))).await,
        Response::Error("a session named \"work\" already exists".into())
    );
    assert_eq!(
        control.request(new(Some("a/b"))).await,
        Response::Error("name must not contain /".into())
    );
    assert_eq!(
        control
            .request(Request::NewSession {
                name: None,
                cwd: "/nope".into()
            })
            .await,
        Response::Error("\"/nope\" is not a directory".into())
    );

    let Response::Sessions(list) = control.request(Request::ListSessions).await else {
        panic!("sessions")
    };
    let names: Vec<_> = list.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["main", "main-2", "work"]);
    assert!(list.iter().all(|s| s.projects == 1 && s.terminals == 1));

    // A session made this way can be attached by name, and it was never attached before.
    assert!(list.iter().all(|s| s.last_attached == 0));
    let work = Client::attach(
        &daemon.paths.socket,
        Attach {
            session: Some("work".into()),
            create: false,
            ..Attach::default()
        },
    )
    .await;
    assert_eq!(work.view.as_ref().unwrap().name, "work");
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn projects_for_the_root_and_the_home_directory_get_their_default_names() {
    let daemon = TestDaemon::start().await;
    let mut control = Client::control(&daemon.paths.socket).await;
    let Response::Session { .. } = control
        .request(Request::NewSession {
            name: Some("r".into()),
            cwd: "/".into(),
        })
        .await
    else {
        panic!("a session")
    };
    let r = Client::attach(
        &daemon.paths.socket,
        Attach {
            session: Some("r".into()),
            create: false,
            ..Attach::default()
        },
    )
    .await;
    assert_eq!(r.tree()[0].0, "root");

    let home_dir = dir("home");
    let home = real(&home_dir);
    let mut env = support::hello_env();
    env.push(("HOME".into(), home.clone().into()));
    let mut ctl = daemon.connect().await;
    ctl.send(&hello(
        deck_core::protocol::Role::Control,
        std::env::temp_dir(),
        env,
        None,
    ))
    .await;
    let Response::Session { .. } = ctl
        .request(Request::NewSession {
            name: Some("h".into()),
            cwd: home,
        })
        .await
    else {
        panic!("a session")
    };
    let h = Client::attach(
        &daemon.paths.socket,
        Attach {
            session: Some("h".into()),
            create: false,
            ..Attach::default()
        },
    )
    .await;
    assert_eq!(h.tree()[0].0, "~");
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn actions_before_attaching_are_refused() {
    let daemon = TestDaemon::start().await;
    let mut control = Client::control(&daemon.paths.socket).await;
    assert_eq!(
        control.action(Action::CyclePanel { delta: 1 }).await,
        Err(ActionError::NotAttached)
    );
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn killing_the_last_session_kills_a_shell_that_ignores_sighup() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon).await;
    c.type_line("trap '' HUP; echo TRAPPED").await;
    c.wait_row("TRAPPED").await;
    let pid = shell_pid(&mut c, "S").await;
    c.send(&ClientMsg::Action(Action::KillSession)).await;
    c.wait(|c| c.bye.is_some()).await;
    daemon.wait_exit().await.unwrap();
    wait_gone(pid).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_requester_of_a_kill_gets_its_answer_before_the_bye() {
    let daemon = TestDaemon::start().await;
    let mut a = attach(&daemon).await;
    let mut b = Client::attach(
        &daemon.paths.socket,
        Attach {
            session: Some("other".into()),
            ..Attach::default()
        },
    )
    .await;

    let before = a.log.len();
    a.send(&ClientMsg::Action(Action::KillSession)).await;
    a.wait(|c| c.bye.is_some()).await;
    let tail: Vec<_> = a.log[before..]
        .iter()
        .filter(|m| matches!(m, ServerMsg::ActionResult(_) | ServerMsg::Bye { .. }))
        .collect();
    assert_eq!(
        tail,
        [
            &ServerMsg::ActionResult(Ok(())),
            &ServerMsg::Bye {
                reason: ByeReason::SessionEnded
            }
        ]
    );

    let before = b.log.len();
    b.send(&ClientMsg::Request(Request::KillSession {
        name: "other".into(),
    }))
    .await;
    b.wait(|c| c.bye.is_some()).await;
    let tail: Vec<_> = b.log[before..]
        .iter()
        .filter(|m| matches!(m, ServerMsg::Response(_) | ServerMsg::Bye { .. }))
        .collect();
    assert_eq!(
        tail,
        [
            &ServerMsg::Response(Response::Ok),
            &ServerMsg::Bye {
                reason: ByeReason::SessionEnded
            }
        ]
    );
    daemon.wait_exit().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn relative_directories_and_bad_attach_names_are_refused() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon).await;
    assert_eq!(
        c.action(Action::NewProject {
            name: "rel".into(),
            root: "tmp".into()
        })
        .await,
        Err(ActionError::NotAbsolute("tmp".into()))
    );
    let mut control = Client::control(&daemon.paths.socket).await;
    assert_eq!(
        control
            .request(Request::NewSession {
                name: None,
                cwd: "".into()
            })
            .await,
        Response::Error("\"\" is not an absolute path".into())
    );

    let mut bad = daemon.connect().await;
    bad.send(&hello(
        deck_core::protocol::Role::Attach {
            session: Some("a/b".into()),
            create: true,
            cols: 80,
            rows: 24,
        },
        std::env::temp_dir(),
        support::hello_env(),
        None,
    ))
    .await;
    bad.wait(|c| c.bye.is_some()).await;
    assert_eq!(
        bad.bye,
        Some(ByeReason::Error("name must not contain /".into()))
    );
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unsized_session_starts_its_shell_for_an_80_by_24_viewport() {
    let daemon = TestDaemon::start().await;
    let mut control = Client::control(&daemon.paths.socket).await;
    let Response::Session { .. } = control
        .request(Request::NewSession {
            name: Some("quiet".into()),
            cwd: "/tmp".into(),
        })
        .await
    else {
        panic!("a session")
    };
    // Attaching with that same size claims nothing new, so the shell keeps the size the
    // daemon gave it: the whole 80x24 viewport.
    let c = Client::attach(
        &daemon.paths.socket,
        Attach {
            session: Some("quiet".into()),
            create: false,
            cols: 80,
            rows: 24,
            ..Attach::default()
        },
    )
    .await;
    assert_eq!(c.view.as_ref().unwrap().layout_viewport, Some((80, 24)));
    let first = c.frames().next().unwrap();
    assert!(first.full);
    assert_eq!(first.size, (80, 24));
    daemon.stop().await.unwrap();
}

/// The projects' ids in order, and each one's panel ids.
fn order(c: &Client) -> (Vec<u64>, Vec<Vec<u64>>) {
    let view = c.view.as_ref().unwrap();
    (
        view.projects.iter().map(|p| p.id.0).collect(),
        view.projects
            .iter()
            .map(|p| p.panels.iter().map(|x| x.id.0).collect())
            .collect(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn moves_reorder_spaces_and_panels_and_keep_the_rest() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon).await;
    let main = c.tree()[0].0.clone();
    new_panel(&mut c, &main).await;
    new_panel(&mut c, &main).await;
    let (a, b) = (dir("a"), dir("b"));
    for (name, d) in [("a", &a), ("b", &b)] {
        c.action(Action::NewProject {
            name: name.into(),
            root: real(d),
        })
        .await
        .unwrap();
    }
    let (projects, panels) = order(&c);
    let (m, pa, pb) = (projects[0], projects[1], projects[2]);
    let [p1, p2, p3] = [panels[0][0], panels[0][1], panels[0][2]];
    let active = c.view.as_ref().unwrap().clone();
    let (active_project, active_panel) = (active.active_project, active.active_panel);
    c.action(Action::SetProjectCollapsed {
        project: c.project_id("a"),
        collapsed: true,
    })
    .await
    .unwrap();

    let mv = |project, to| Action::MoveProject {
        project: deck_core::model::ProjectId(project),
        to,
    };
    let mp = |panel, to| Action::MovePanel {
        panel: deck_core::model::PanelId(panel),
        to,
    };
    // To the front, to the back, to the same place, and past the end (clamped).
    c.action(mv(pb, 0)).await.unwrap();
    assert_eq!(order(&c).0, [pb, m, pa]);
    c.action(mv(pb, 2)).await.unwrap();
    assert_eq!(order(&c).0, [m, pa, pb]);
    c.action(mv(pa, 1)).await.unwrap();
    assert_eq!(order(&c).0, [m, pa, pb]);
    c.action(mv(m, 99)).await.unwrap();
    assert_eq!(order(&c).0, [pa, pb, m]);
    c.action(mp(p3, 0)).await.unwrap();
    assert_eq!(order(&c).1[2], [p3, p1, p2]);
    c.action(mp(p3, 99)).await.unwrap();
    assert_eq!(order(&c).1[2], [p1, p2, p3]);
    c.action(mp(p2, 1)).await.unwrap();
    assert_eq!(order(&c).1[2], [p1, p2, p3]);

    // Active space and panel, and the collapsed state, are untouched.
    let view = c.view.as_ref().unwrap();
    assert_eq!(
        (view.active_project, view.active_panel),
        (active_project, active_panel)
    );
    assert!(view.projects[0].collapsed);

    // Unknown ids are refused.
    assert!(matches!(
        c.action(mv(999, 0)).await,
        Err(ActionError::NoSuchProject(_))
    ));
    assert!(matches!(
        c.action(mp(999, 0)).await,
        Err(ActionError::NoSuchPanel(_))
    ));

    // The order is saved.
    let before = order(&c);
    drop(c);
    let home = daemon.stop_keep().await;
    let daemon = TestDaemon::start_on(home).await;
    let again = attach(&daemon).await;
    assert_eq!(order(&again), before);
    daemon.stop().await.unwrap();
}
