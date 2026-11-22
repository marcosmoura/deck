//! The control requests that list and edit the tree: launches, `keep`, focus, close, rename,
//! and the saved state of kept terminals.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use deck_core::model::{
    LaunchSpec, NameSource, PanelId, ProjectId, SessionView, TerminalId, TerminalView,
};
use deck_core::protocol::{ClientMsg, Request, Response, Role, Target};
use deck_core::state::StateFile;
use support::{Attach, Client, TestDaemon, hello, hello_env};

fn workdir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("pw")
        .tempdir_in("/tmp")
        .unwrap()
}

async fn attach(daemon: &TestDaemon) -> Client {
    Client::attach(&daemon.paths.socket, Attach::default()).await
}

fn view(c: &Client) -> &SessionView {
    c.view.as_ref().unwrap()
}

fn launch(program: &str, args: &[&str]) -> Option<LaunchSpec> {
    Some(LaunchSpec {
        program: Some(program.into()),
        args: args.iter().map(|a| (*a).into()).collect(),
        cwd: PathBuf::new(),
        env: Vec::new(),
    })
}

fn sh(script: &str) -> Option<LaunchSpec> {
    launch("/bin/sh", &["-c", script])
}

async fn created(c: &mut Client, request: Request) -> u64 {
    match c.request(request).await {
        Response::Created { id } => id,
        other => panic!("expected an id, got {other:?}"),
    }
}

async fn error(c: &mut Client, request: Request) -> String {
    match c.request(request).await {
        Response::Error(text) => text,
        other => panic!("expected an error, got {other:?}"),
    }
}

fn terminal(c: &Client, id: u64) -> Option<TerminalView> {
    c.view
        .as_ref()?
        .projects
        .iter()
        .flat_map(|p| &p.panels)
        .flat_map(|p| &p.terminals)
        .find(|t| t.id.0 == id)
        .cloned()
}

fn panel_terminals(c: &Client) -> usize {
    view(c).projects[0].panels[0].terminals.len()
}

fn shows(c: &Client, id: u64, text: &str) -> bool {
    c.screens
        .get(&TerminalId(id))
        .is_some_and(|rows| rows.iter().any(|r| r.trim_end() == text))
}

async fn new_terminal(
    c: &mut Client,
    panel: PanelId,
    launch: Option<LaunchSpec>,
    keep: bool,
    focus: bool,
) -> u64 {
    created(
        c,
        Request::NewTerminal {
            panel,
            launch,
            keep,
            focus,
        },
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn the_tree_lists_pids_and_attached_clients() {
    let daemon = TestDaemon::start().await;
    let mut control = Client::control(&daemon.paths.socket).await;
    let Response::Tree(none) = control.request(Request::Tree { session: None }).await else {
        panic!("a tree")
    };
    assert!(none.is_empty());

    let attached = attach(&daemon).await;
    let Response::Tree(all) = control.request(Request::Tree { session: None }).await else {
        panic!("a tree")
    };
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].attached_clients, 1);
    let t = &all[0].projects[0].panels[0].terminals[0];
    assert!(t.pid.is_some_and(|p| p > 1) && t.exit_status.is_none() && !t.keep);

    let Response::Tree(one) = control
        .request(Request::Tree {
            session: Some("main".into()),
        })
        .await
    else {
        panic!("a tree")
    };
    assert_eq!(one, all);
    assert_eq!(
        error(
            &mut control,
            Request::Tree {
                session: Some("nope".into())
            }
        )
        .await,
        "no session named \"nope\""
    );
    drop(attached);
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_launched_terminal_runs_its_program_and_the_daemon_owns_the_deck_variables() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon).await;
    let panel = view(&c).active_panel.unwrap();
    let first = c.active_terminal();
    let mut spec =
        sh("echo id=$DECK_TERMINAL_ID extra=$EXTRA panel=$DECK_PANEL_ID; sleep 30").unwrap();
    spec.env = vec![
        ("DECK_TERMINAL_ID".into(), "1".into()),
        ("DECK_PANEL_ID".into(), "2".into()),
        ("EXTRA".into(), "yes".into()),
    ];
    let id = new_terminal(&mut c, panel, Some(spec), false, false).await;
    let line = format!("id={id} extra=yes panel={panel}");
    c.wait(|c| shows(c, id, &line)).await;

    // No focus: the calling terminal stays focused.
    assert_eq!(view(&c).projects[0].panels[0].focused_terminal, Some(first));
    assert_eq!(panel_terminals(&c), 2);
    let t = terminal(&c, id).unwrap();
    assert_eq!(t.program, "sh");
    assert!(t.pid.is_some());
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_kept_terminal_stays_with_its_exit_code_and_drops_input() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon).await;
    let panel = view(&c).active_panel.unwrap();

    let exited = new_terminal(&mut c, panel, sh("echo done; exit 3"), true, false).await;
    c.wait(|c| terminal(c, exited).is_some_and(|t| t.exit_status == Some(3)))
        .await;
    let t = terminal(&c, exited).unwrap();
    assert!(t.keep && t.pid.is_none());
    // Its screen stays.
    assert!(shows(&c, exited, "done"));

    // A death by signal is 128 + n.
    let killed = new_terminal(&mut c, panel, sh("kill -TERM $$"), true, false).await;
    c.wait(|c| terminal(c, killed).is_some_and(|t| t.exit_status == Some(143)))
        .await;

    // Input is dropped, and nothing breaks.
    c.send(&ClientMsg::Input {
        terminal: TerminalId(exited),
        bytes: b"echo hi\n".to_vec(),
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!shows(&c, exited, "hi"));
    assert!(terminal(&c, exited).is_some());

    // Without `keep` a terminal that exits goes away.
    let gone = new_terminal(&mut c, panel, sh("exit 0"), false, false).await;
    c.wait(|c| terminal(c, gone).is_none() && panel_terminals(c) == 3)
        .await;

    // Closing a kept terminal removes it; the panel follows the usual rules.
    let Response::Ok = c.request(Request::Close { id: exited }).await else {
        panic!("closed")
    };
    c.wait(|c| terminal(c, exited).is_none()).await;
    let Response::Ok = c.request(Request::Close { id: killed }).await else {
        panic!("closed")
    };
    c.wait(|c| panel_terminals(c) == 1).await;
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn focus_moves_only_when_asked() {
    let daemon = TestDaemon::start().await;
    let work = workdir();
    let mut c = attach(&daemon).await;
    let session = view(&c).id;
    let panel = view(&c).active_panel.unwrap();
    let first = c.active_terminal();

    // A terminal without focus, then with.
    let quiet = new_terminal(&mut c, panel, sh("sleep 30"), false, false).await;
    c.wait(|c| terminal(c, quiet).is_some()).await;
    assert_eq!(c.active_terminal(), first);
    let loud = new_terminal(&mut c, panel, sh("sleep 30"), false, true).await;
    c.wait(|c| terminal(c, loud).is_some() && c.active_terminal().0 == loud)
        .await;

    // A panel without focus keeps the active one; with focus it takes over.
    let project = Target::Id(view(&c).projects[0].id.0);
    let quiet_panel = created(
        &mut c,
        Request::NewPanel {
            session,
            project: project.clone(),
            name: Some("quiet".into()),
            launch: sh("sleep 30"),
            focus: false,
        },
    )
    .await;
    c.wait(|c| view(c).projects[0].panels.len() == 2).await;
    assert_eq!(view(&c).active_panel, Some(panel));
    let q = &view(&c).projects[0].panels[1];
    assert_eq!((q.id.0, q.name.as_str()), (quiet_panel, "quiet"));
    assert_eq!(q.name_source, NameSource::Manual);

    let loud_panel = created(
        &mut c,
        Request::NewPanel {
            session,
            project,
            name: None,
            launch: None,
            focus: true,
        },
    )
    .await;
    c.wait(|c| view(c).active_panel == Some(PanelId(loud_panel)))
        .await;

    // A project without focus leaves the active project; with focus it becomes active.
    let before = view(&c).active_project;
    let quiet_project = created(
        &mut c,
        Request::NewProject {
            session,
            name: "quiet".into(),
            root: work.path().to_path_buf(),
            focus: false,
        },
    )
    .await;
    c.wait(|c| view(c).projects.len() == 2).await;
    assert_eq!(view(&c).active_project, before);
    let loud_project = created(
        &mut c,
        Request::NewProject {
            session,
            name: "loud".into(),
            root: work.path().to_path_buf(),
            focus: true,
        },
    )
    .await;
    c.wait(|c| view(c).active_project == Some(ProjectId(loud_project)))
        .await;
    assert_ne!(quiet_project, loud_project);

    // Focus by id: a terminal brings its panel and project, a panel its project.
    let Response::Ok = c.request(Request::Focus { id: quiet }).await else {
        panic!("focused")
    };
    c.wait(|c| view(c).active_panel == Some(panel) && c.active_terminal().0 == quiet)
        .await;
    assert_eq!(view(&c).active_project, Some(view(&c).projects[0].id));
    let Response::Ok = c.request(Request::Focus { id: quiet_panel }).await else {
        panic!("focused")
    };
    c.wait(|c| view(c).active_panel == Some(PanelId(quiet_panel)))
        .await;
    assert_eq!(
        error(&mut c, Request::Focus { id: quiet_project }).await,
        format!("{quiet_project} is not a terminal or panel")
    );
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn close_and_rename_work_on_ids_and_report_errors() {
    let daemon = TestDaemon::start().await;
    let work = workdir();
    let mut c = attach(&daemon).await;
    let session = view(&c).id;
    let panel = view(&c).active_panel.unwrap();
    let project = created(
        &mut c,
        Request::NewProject {
            session,
            name: "api".into(),
            root: work.path().to_path_buf(),
            focus: false,
        },
    )
    .await;
    let extra = created(
        &mut c,
        Request::NewPanel {
            session,
            project: Target::Name("api".into()),
            name: None,
            launch: None,
            focus: false,
        },
    )
    .await;
    c.wait(|c| view(c).projects.len() == 2 && view(c).projects[1].panels.len() == 2)
        .await;

    let Response::Ok = c
        .request(Request::RenamePanel {
            panel: PanelId(extra),
            name: "build".into(),
        })
        .await
    else {
        panic!("renamed")
    };
    c.wait(|c| view(c).projects[1].panels[1].name == "build")
        .await;
    assert_eq!(
        view(&c).projects[1].panels[1].name_source,
        NameSource::Manual
    );
    assert_eq!(
        error(
            &mut c,
            Request::RenamePanel {
                panel: PanelId(9999),
                name: "x".into()
            }
        )
        .await,
        "no panel 9999"
    );
    assert_eq!(
        error(
            &mut c,
            Request::RenamePanel {
                panel,
                name: "  ".into()
            }
        )
        .await,
        "name must not be empty"
    );

    // A panel closes with its terminals; a project with its panels.
    let Response::Ok = c.request(Request::Close { id: extra }).await else {
        panic!("closed")
    };
    c.wait(|c| view(c).projects[1].panels.len() == 1).await;
    let Response::Ok = c.request(Request::Close { id: project }).await else {
        panic!("closed")
    };
    c.wait(|c| view(c).projects.len() == 1).await;
    assert_eq!(
        error(&mut c, Request::Close { id: 9999 }).await,
        "9999 is not a terminal, panel or project"
    );

    // Names and targets are checked.
    assert_eq!(
        error(
            &mut c,
            Request::NewPanel {
                session,
                project: Target::Name("nope".into()),
                name: None,
                launch: None,
                focus: false
            }
        )
        .await,
        "no project named \"nope\" in session \"main\""
    );
    assert_eq!(
        error(
            &mut c,
            Request::NewPanel {
                session,
                project: Target::Id(9999),
                name: None,
                launch: None,
                focus: false
            }
        )
        .await,
        "no project 9999"
    );
    assert_eq!(
        error(
            &mut c,
            Request::NewProject {
                session,
                name: "main".into(),
                root: Path::new("/nonexistent-dir").to_path_buf(),
                focus: false
            }
        )
        .await,
        "\"/nonexistent-dir\" is not a directory"
    );
    assert_eq!(
        error(
            &mut c,
            Request::NewTerminal {
                panel: PanelId(9999),
                launch: None,
                keep: false,
                focus: false
            }
        )
        .await,
        "no panel 9999"
    );
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_terminal_obeys_the_room_check_and_the_launch_directory() {
    let daemon = TestDaemon::start().await;
    let work = workdir();
    let mut c = attach(&daemon).await;
    let panel = view(&c).active_panel.unwrap();

    // The directory of the launch spec; one that is no directory is an error.
    let mut spec = sh("pwd; sleep 30").unwrap();
    spec.cwd = work.path().canonicalize().unwrap();
    let id = new_terminal(&mut c, panel, Some(spec.clone()), false, false).await;
    let expect = spec.cwd.display().to_string();
    c.wait(|c| shows(c, id, &expect)).await;
    spec.cwd = work.path().join("missing");
    let text = error(
        &mut c,
        Request::NewTerminal {
            panel,
            launch: Some(spec),
            keep: false,
            focus: false,
        },
    )
    .await;
    assert!(text.contains("is not a directory"), "{text}");

    // Terminals until there is no room: the error is the DECK-03 one.
    let deadline = Instant::now() + Duration::from_secs(20);
    let text = loop {
        match c
            .request(Request::NewTerminal {
                panel,
                launch: sh("sleep 30"),
                keep: false,
                focus: false,
            })
            .await
        {
            Response::Created { .. } => {}
            Response::Error(text) => break text,
            other => panic!("unexpected {other:?}"),
        }
        assert!(Instant::now() < deadline, "no room error in time");
    };
    assert_eq!(text, "not enough room for another terminal");
    daemon.stop().await.unwrap();
}

fn read_state(daemon: &TestDaemon) -> Option<StateFile> {
    serde_json::from_slice(&fs::read(daemon.paths.state_dir.join("state.json")).ok()?).ok()
}

#[tokio::test(flavor = "multi_thread")]
async fn keep_is_saved_and_a_restart_restores_only_the_running_kept_terminal() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon).await;
    let panel = view(&c).active_panel.unwrap();
    let exited = new_terminal(&mut c, panel, sh("exit 3"), true, false).await;
    let running = new_terminal(&mut c, panel, sh("sleep 300"), true, false).await;
    c.wait(|c| terminal(c, exited).is_some_and(|t| t.exit_status == Some(3)))
        .await;

    // Both are in the saved state with `keep`; the exited one with its code.
    let deadline = Instant::now() + Duration::from_secs(5);
    let saved = loop {
        if let Some(file) = read_state(&daemon)
            && file.sessions[0].projects[0].panels[0]
                .terminals
                .iter()
                .any(|t| t.id.0 == exited && t.exited == Some(3))
        {
            break file;
        }
        assert!(Instant::now() < deadline, "the state was not saved");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let terminals = &saved.sessions[0].projects[0].panels[0].terminals;
    let kept = |id: u64| terminals.iter().find(|t| t.id.0 == id).unwrap();
    assert!(kept(exited).keep && kept(running).keep);
    assert_eq!(kept(running).exited, None);
    drop(c);

    let home = daemon.stop_keep().await;
    let daemon = TestDaemon::start_on(home).await;
    let mut again = Client::connect(&daemon.paths.socket).await;
    again
        .send(&hello(
            Role::Attach {
                session: Some("main".into()),
                create: false,
                cols: 80,
                rows: 25,
            },
            std::env::temp_dir(),
            hello_env(),
            None,
        ))
        .await;
    again.wait(|c| terminal(c, running).is_some()).await;
    assert!(terminal(&again, exited).is_none(), "the exited one is gone");
    let back = terminal(&again, running).unwrap();
    assert!(back.keep && back.exit_status.is_none());
    assert_eq!(panel_terminals(&again), 2);
    daemon.stop().await.unwrap();
}

#[test]
fn a_state_file_without_keep_still_parses() {
    let old = r#"{"id":4,"program":null,"args":[],"env":[],"cwd":null}"#;
    let t: deck_core::state::TerminalState = serde_json::from_str(old).unwrap();
    assert!(!t.keep && t.exited.is_none());
}
