//! Saving `state.json`, restoring it at start, the shutdown flag, notices and bad files.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use deck_core::action::Action;
use deck_core::layout::Ratios;
use deck_core::model::{NameSource, PanelId, ProjectId, SessionId, SessionView, TerminalId};
use deck_core::protocol::{Request, Response, Role};
use deck_core::state::{PanelState, ProjectState, SessionState, StateFile, TerminalState};
use deck_daemon::{Daemon, DaemonError, DaemonPaths, Options};
use support::{Attach, Client, TIMEOUT, TestDaemon, config, hello, hello_env};

const HANG_UP_PROGRAM: &str = r#"trap "exit 0" HUP; while :; do sleep 1; done"#;

fn state_file(paths: &DaemonPaths) -> PathBuf {
    paths.state_dir.join("state.json")
}

fn read_state(paths: &DaemonPaths) -> Option<StateFile> {
    serde_json::from_slice(&fs::read(state_file(paths)).ok()?).ok()
}

/// Waits until the saved state satisfies `done`.
async fn wait_saved(paths: &DaemonPaths, done: impl Fn(&StateFile) -> bool) -> StateFile {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(file) = read_state(paths)
            && done(&file)
        {
            return file;
        }
        assert!(Instant::now() < deadline, "the state was not saved in time");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn shell(id: u64) -> TerminalState {
    TerminalState {
        id: TerminalId(id),
        program: None,
        args: vec![],
        env: vec![],
        cwd: None,
        keep: false,
        exited: None,
        agent: None,
    }
}

fn program(id: u64, program: &str, args: &[&str]) -> TerminalState {
    TerminalState {
        program: Some(program.into()),
        args: args.iter().map(|a| (*a).into()).collect(),
        ..shell(id)
    }
}

fn panel(id: u64, name: &str, terminals: Vec<TerminalState>) -> PanelState {
    PanelState {
        id: PanelId(id),
        name: name.into(),
        name_source: NameSource::Default,
        focused_terminal: terminals.first().map(|t| t.id),
        ratios: Ratios::default_for(terminals.len()),
        terminals,
    }
}

fn project(id: u64, name: &str, root: &Path, panels: Vec<PanelState>) -> ProjectState {
    ProjectState {
        id: ProjectId(id),
        name: name.into(),
        root: root.to_path_buf(),
        collapsed: false,
        active_panel: panels.first().map(|p| p.id),
        panels,
    }
}

fn session(id: u64, name: &str, projects: Vec<ProjectState>) -> SessionState {
    SessionState {
        id: SessionId(id),
        name: name.into(),
        active_project: projects.first().map(|p| p.id),
        projects,
    }
}

fn file(sessions: Vec<SessionState>) -> StateFile {
    StateFile {
        version: 1,
        next_id: 1,
        last_attached: vec![],
        sessions,
    }
}

/// Writes `state.json` into a home before any daemon runs on it.
fn seed_text(home: &Path, text: &[u8]) -> DaemonPaths {
    let paths = support::paths_in(home);
    fs::create_dir_all(&paths.state_dir).unwrap();
    fs::set_permissions(&paths.state_dir, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(state_file(&paths), text).unwrap();
    paths
}

fn seed(home: &Path, state: &StateFile) -> DaemonPaths {
    seed_text(home, &serde_json::to_vec_pretty(state).unwrap())
}

fn workdir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("pw")
        .tempdir_in("/tmp")
        .unwrap()
}

/// Attaches to the named session without waiting for any frame.
async fn attach_to(daemon: &TestDaemon, name: &str) -> Client {
    let mut c = Client::connect(&daemon.paths.socket).await;
    c.send(&hello(
        Role::Attach {
            session: Some(name.into()),
            create: false,
            cols: 80,
            rows: 24,
        },
        std::env::temp_dir(),
        hello_env(),
        None,
    ))
    .await;
    c.wait(|c| c.view.is_some()).await;
    c
}

fn structure(view: &SessionView) -> SessionView {
    let mut view = view.clone();
    for t in view
        .projects
        .iter_mut()
        .flat_map(|p| &mut p.panels)
        .flat_map(|p| &mut p.terminals)
    {
        t.title = None;
        // The process ids differ after a restart.
        t.pid = None;
    }
    view
}

#[tokio::test(flavor = "multi_thread")]
async fn a_change_is_saved_after_the_window_with_private_modes() {
    let daemon = TestDaemon::start().await;
    assert!(
        read_state(&daemon.paths).is_none(),
        "nothing before a change"
    );
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;

    let saved = wait_saved(&daemon.paths, |f| f.sessions.len() == 1).await;
    assert_eq!(saved.version, 1);
    assert_eq!(saved.sessions[0].name, "main");
    assert_eq!(saved.last_attached, [saved.sessions[0].id]);
    assert!(saved.next_id > saved.sessions[0].projects[0].panels[0].terminals[0].id.0);
    assert_eq!(
        fs::metadata(state_file(&daemon.paths)).unwrap().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(&daemon.paths.state_dir).unwrap().mode() & 0o777,
        0o700
    );
    assert!(
        !state_file(&daemon.paths)
            .with_extension("json.tmp")
            .exists()
    );

    client
        .action(Action::RenameSession {
            name: "renamed".into(),
        })
        .await
        .unwrap();
    wait_saved(&daemon.paths, |f| f.sessions[0].name == "renamed").await;
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn changes_inside_the_window_join_one_save() {
    let daemon = TestDaemon::start().await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    wait_saved(&daemon.paths, |f| f.sessions.len() == 1).await;
    let before = fs::metadata(state_file(&daemon.paths)).unwrap().ino();
    for name in ["a", "b", "c"] {
        client
            .action(Action::RenameSession { name: name.into() })
            .await
            .unwrap();
    }
    let saved = wait_saved(&daemon.paths, |f| f.sessions[0].name == "c").await;
    assert_eq!(saved.sessions[0].name, "c");
    // One rename of the temp file over the old one: a new inode, once.
    let saved_ino = fs::metadata(state_file(&daemon.paths)).unwrap().ino();
    assert_ne!(saved_ino, before);
    // Nothing else writes afterwards: the file stays the one that was saved.
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(
        fs::metadata(state_file(&daemon.paths)).unwrap().ino(),
        saved_ino
    );
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn changes_made_while_saves_run_are_never_lost() {
    let daemon = TestDaemon::start().await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    wait_saved(&daemon.paths, |f| f.sessions.len() == 1).await;
    // Renames spread over several save windows: some land while a save is in flight.
    for n in 0..60 {
        client
            .action(Action::RenameSession {
                name: format!("n{n}"),
            })
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    wait_saved(&daemon.paths, |f| f.sessions[0].name == "n59").await;
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn everything_comes_back_after_a_restart_with_the_same_ids() {
    let work = workdir();
    let second_root = workdir();
    let daemon = TestDaemon::start().await;
    let mut client = Client::attach(
        &daemon.paths.socket,
        Attach {
            cwd: work.path().to_path_buf(),
            ..Attach::default()
        },
    )
    .await;
    client
        .action(Action::NewProject {
            name: "second".into(),
            root: second_root.path().to_path_buf(),
        })
        .await
        .unwrap();
    let second = client.project_id("second");
    client
        .action(Action::NewPanel {
            project: second,
            name: Some("logs".into()),
        })
        .await
        .unwrap();
    let first_panel = client.view.as_ref().unwrap().projects[0].panels[0].id;
    for _ in 0..2 {
        client
            .action(Action::NewTerminal {
                panel: Some(first_panel),
            })
            .await
            .unwrap();
    }
    client
        .wait(|c| {
            c.view.as_ref().unwrap().projects[0].panels[0]
                .terminals
                .len()
                == 3
        })
        .await;
    let mut ratios = client.view.as_ref().unwrap().projects[0].panels[0]
        .ratios
        .clone();
    ratios.main = Some(700);
    client
        .action(Action::SetRatios {
            panel: first_panel,
            n: 3,
            ratios,
        })
        .await
        .unwrap();
    let middle = client.view.as_ref().unwrap().projects[0].panels[0].terminals[1].id;
    client
        .action(Action::FocusTerminal { terminal: middle })
        .await
        .unwrap();
    client
        .action(Action::SetProjectCollapsed {
            project: second,
            collapsed: true,
        })
        .await
        .unwrap();
    client
        .action(Action::RenamePanel {
            panel: first_panel,
            name: "editor".into(),
        })
        .await
        .unwrap();
    client
        .wait(|c| c.view.as_ref().unwrap().projects[0].panels[0].name == "editor")
        .await;
    let mut control = Client::control(&daemon.paths.socket).await;
    let Response::Session { .. } = control
        .request(Request::NewSession {
            name: Some("other".into()),
            cwd: work.path().to_path_buf(),
        })
        .await
    else {
        panic!("a session")
    };
    let before = structure(client.view.as_ref().unwrap());
    assert_eq!(before.projects[0].panels[0].ratios.main, Some(700));
    drop(client);

    let home = daemon.stop_keep().await;
    let daemon = TestDaemon::start_on(home).await;
    let again = attach_to(&daemon, "main").await;
    let after = structure(again.view.as_ref().unwrap());
    assert_eq!(
        after.projects, before.projects,
        "projects, panels, ids, ratios, focus"
    );
    assert_eq!(
        (after.id, after.active_project, after.active_panel),
        (before.id, before.active_project, before.active_panel)
    );

    let mut control = Client::control(&daemon.paths.socket).await;
    let Response::Sessions(sessions) = control.request(Request::ListSessions).await else {
        panic!("sessions")
    };
    let names: Vec<_> = sessions
        .iter()
        .map(|s| (s.name.as_str(), s.terminals))
        .collect();
    assert_eq!(names, [("main", 5), ("other", 1)]);
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_program_that_exits_on_hangup_survives_the_shutdown() {
    let work = workdir();
    let home = TestDaemon::new_home();
    let state = file(vec![session(
        1,
        "main",
        vec![project(
            2,
            "p",
            work.path(),
            vec![panel(
                3,
                "1",
                vec![program(4, "/bin/sh", &["-c", HANG_UP_PROGRAM]), shell(5)],
            )],
        )],
    )]);
    let paths = seed(home.path(), &state);
    let daemon = TestDaemon::start_on(home).await;
    let first = attach_to(&daemon, "main").await;
    assert_eq!(
        first.view.as_ref().unwrap().projects[0].panels[0]
            .terminals
            .len(),
        2
    );
    drop(first);

    // Twice over: the shutdown hangs the program up, and it must stay in the state.
    let home = daemon.stop_keep().await;
    let saved = read_state(&paths).expect("a saved state");
    let terminals = &saved.sessions[0].projects[0].panels[0].terminals;
    assert_eq!(terminals.len(), 2);
    assert_eq!(terminals[0].program.as_deref(), Some("/bin/sh"));
    assert_eq!(terminals[0].args, ["-c", HANG_UP_PROGRAM]);

    let daemon = TestDaemon::start_on(home).await;
    let again = attach_to(&daemon, "main").await;
    assert_eq!(
        again.view.as_ref().unwrap().projects[0].panels[0]
            .terminals
            .len(),
        2
    );
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_killed_session_stays_gone_and_the_last_one_saves_an_empty_state() {
    let daemon = TestDaemon::start().await;
    let _a = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let mut control = Client::control(&daemon.paths.socket).await;
    let Response::Session { .. } = control
        .request(Request::NewSession {
            name: Some("two".into()),
            cwd: std::env::temp_dir(),
        })
        .await
    else {
        panic!("a session")
    };
    assert_eq!(
        control
            .request(Request::KillSession {
                name: "main".into()
            })
            .await,
        Response::Ok
    );
    let home = daemon.stop_keep().await;
    let daemon = TestDaemon::start_on(home).await;
    let paths = daemon.paths.clone();
    let mut control = Client::control(&paths.socket).await;
    let Response::Sessions(sessions) = control.request(Request::ListSessions).await else {
        panic!("sessions")
    };
    assert_eq!(
        sessions.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        ["two"]
    );

    // The last one: the daemon exits, and what it saved is empty.
    assert_eq!(
        control
            .request(Request::KillSession { name: "two".into() })
            .await,
        Response::Ok
    );
    let _home = daemon.wait_exit_keep().await;
    assert_eq!(read_state(&paths).unwrap().sessions.len(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_last_attached_order_survives_a_restart() {
    let daemon = TestDaemon::start().await;
    let mut control = Client::control(&daemon.paths.socket).await;
    for name in ["a", "b"] {
        let Response::Session { .. } = control
            .request(Request::NewSession {
                name: Some(name.into()),
                cwd: std::env::temp_dir(),
            })
            .await
        else {
            panic!("a session")
        };
    }
    drop(attach_to(&daemon, "a").await);
    drop(attach_to(&daemon, "b").await);
    drop(attach_to(&daemon, "a").await);
    let home = daemon.stop_keep().await;
    let daemon = TestDaemon::start_on(home).await;
    let mut control = Client::control(&daemon.paths.socket).await;
    let Response::Sessions(sessions) = control.request(Request::ListSessions).await else {
        panic!("sessions")
    };
    let tick = |name: &str| {
        sessions
            .iter()
            .find(|s| s.name == name)
            .unwrap()
            .last_attached
    };
    assert!(tick("a") > tick("b") && tick("b") > 0);
    // A bare attach joins the most recent one.
    let mut client = Client::connect(&daemon.paths.socket).await;
    client
        .send(&hello(
            Role::Attach {
                session: None,
                create: true,
                cols: 80,
                rows: 24,
            },
            std::env::temp_dir(),
            hello_env(),
            None,
        ))
        .await;
    client.wait(|c| c.view.is_some()).await;
    assert_eq!(client.view.as_ref().unwrap().name, "a");
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_corrupt_file_is_moved_aside_and_the_client_gets_a_notice() {
    let home = TestDaemon::new_home();
    let paths = seed_text(home.path(), b"{ this is not json");
    let daemon = TestDaemon::start_on(home).await;
    let client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    assert_eq!(client.view.as_ref().unwrap().name, "main");
    assert!(
        client.has_notice("previous state could not be read; see"),
        "{:?}",
        client.notices()
    );
    assert!(client.has_notice(paths.log_file.to_str().unwrap()));
    let aside: Vec<_> = fs::read_dir(&paths.state_dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("state.json.bad-"))
        .collect();
    assert_eq!(aside.len(), 1, "{aside:?}");
    assert_eq!(
        fs::read(paths.state_dir.join(&aside[0])).unwrap(),
        b"{ this is not json"
    );
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_newer_file_stops_the_daemon_and_stays_untouched() {
    let home = TestDaemon::new_home();
    let text = br#"{"version": 2, "sessions": "from the future"}"#;
    let paths = seed_text(home.path(), text);
    let err = Daemon::run_with(
        paths.clone(),
        config(),
        std::future::ready(()),
        Options::default(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, DaemonError::NewerState(2)), "{err}");
    assert!(
        err.to_string().contains(
            "state.json was written by a newer deck (version 2); upgrade deck or run \"deck kill-server --forget\""
        ),
        "{err}"
    );
    assert_eq!(fs::read(state_file(&paths)).unwrap(), text);
    assert!(!paths.socket.exists(), "nothing was bound");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_leftover_temp_file_is_deleted_at_start() {
    let home = TestDaemon::new_home();
    let paths = seed(home.path(), &file(vec![]));
    let tmp = paths.state_dir.join("state.json.tmp");
    fs::write(&tmp, b"half").unwrap();
    let daemon = TestDaemon::start_on(home).await;
    assert!(!tmp.exists());
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restored_auto_name_follows_what_runs_now() {
    let work = workdir();
    let home = TestDaemon::new_home();
    let mut saved = panel(3, "nvim", vec![shell(4)]);
    saved.name_source = NameSource::Auto;
    let state = file(vec![session(
        1,
        "main",
        vec![project(2, "p", work.path(), vec![saved])],
    )]);
    seed(home.path(), &state);
    let daemon = TestDaemon::start_on(home).await;
    let mut client = attach_to(&daemon, "main").await;
    client
        .wait(|c| c.view.as_ref().unwrap().projects[0].panels[0].name == "sh")
        .await;
    let view = client.view.as_ref().unwrap();
    assert_eq!(view.projects[0].panels[0].name_source, NameSource::Auto);
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_program_is_dropped_with_its_panel_and_reported() {
    let work = workdir();
    let home = TestDaemon::new_home();
    let state = file(vec![session(
        1,
        "main",
        vec![project(
            2,
            "p",
            work.path(),
            vec![
                panel(
                    3,
                    "gone",
                    vec![program(4, "/nonexistent/deck-test-program", &[])],
                ),
                panel(5, "kept", vec![shell(6)]),
            ],
        )],
    )]);
    seed(home.path(), &state);
    let daemon = TestDaemon::start_on(home).await;
    let mut client = attach_to(&daemon, "main").await;
    client
        .wait(|c| c.has_notice("/nonexistent/deck-test-program"))
        .await;
    client
        .wait(|c| c.view.as_ref().unwrap().projects[0].panels.len() == 1)
        .await;
    let view = client.view.as_ref().unwrap();
    assert_eq!(view.projects[0].panels[0].id, PanelId(5), "the kept panel");
    assert_eq!(view.name, "main", "the session stays");
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restored_terminal_that_fails_at_once_is_removed_and_the_session_stays() {
    let work = workdir();
    let home = TestDaemon::new_home();
    let state = file(vec![session(
        1,
        "main",
        vec![project(
            2,
            "p",
            work.path(),
            vec![panel(
                3,
                "only",
                vec![program(4, "/bin/sh", &["-c", "exit 3"])],
            )],
        )],
    )]);
    seed(home.path(), &state);
    let daemon = TestDaemon::start_on(home).await;
    let mut client = attach_to(&daemon, "main").await;
    client.wait(|c| c.has_notice("exited at once")).await;
    client
        .wait(|c| c.view.as_ref().unwrap().projects[0].panels.is_empty())
        .await;
    assert!(
        client.bye.is_none(),
        "the session is not ended: {:?}",
        client.bye
    );
    let mut control = Client::control(&daemon.paths.socket).await;
    let Response::Sessions(sessions) = control.request(Request::ListSessions).await else {
        panic!("sessions")
    };
    assert_eq!(sessions.len(), 1);
    daemon.stop().await.unwrap();
}

/// Runs `pwd` in the focused terminal and waits for `expected`.
async fn expect_cwd(client: &mut Client, expected: &Path) {
    let expected = fs::canonicalize(expected).unwrap();
    client.type_line("pwd").await;
    client.wait_row(expected.to_str().unwrap()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_terminal_starts_in_the_first_directory_that_exists() {
    let work = workdir();
    let root = work.path().join("root");
    fs::create_dir(&root).unwrap();
    let gone = work.path().join("deleted");
    let home = TestDaemon::new_home();
    let mut saved = shell(4);
    saved.cwd = Some(gone.clone());
    let mut state = file(vec![session(
        1,
        "main",
        vec![project(2, "p", &root, vec![panel(3, "1", vec![saved])])],
    )]);
    seed(home.path(), &state);
    let daemon = TestDaemon::start_on(home).await;
    let mut client = Client::attach(
        &daemon.paths.socket,
        Attach {
            session: Some("main".into()),
            create: false,
            ..Attach::default()
        },
    )
    .await;
    expect_cwd(&mut client, &root).await;
    drop(client);
    let home = daemon.stop_keep().await;

    // The root is gone too: the home directory.
    fs::remove_dir(&root).unwrap();
    let mut saved = shell(4);
    saved.cwd = Some(gone);
    state.sessions[0].projects[0].panels[0].terminals = vec![saved];
    seed(home.path(), &state);
    let daemon = TestDaemon::start_on(home).await;
    let mut client = Client::attach(
        &daemon.paths.socket,
        Attach {
            session: Some("main".into()),
            create: false,
            ..Attach::default()
        },
    )
    .await;
    let home_dir = PathBuf::from(std::env::var_os("HOME").expect("HOME"));
    expect_cwd(&mut client, &home_dir).await;
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restored_terminal_gets_fresh_ids_and_none_of_the_clients_environment() {
    let work = workdir();
    let daemon = TestDaemon::start().await;
    let mut env = hello_env();
    env.push(("CLIENT_ONLY_VAR".into(), "client-secret".into()));
    let mut client = Client::attach(
        &daemon.paths.socket,
        Attach {
            cwd: work.path().to_path_buf(),
            env,
            ..Attach::default()
        },
    )
    .await;
    client.type_line("echo only=$CLIENT_ONLY_VAR").await;
    client.wait_row("only=client-secret").await;
    let terminal = client.terminal();
    let socket = daemon.paths.socket.clone();
    drop(client);

    let home = daemon.stop_keep().await;
    let text = fs::read_to_string(state_file(&support::paths_in(home.path()))).unwrap();
    assert!(
        !text.contains("client-secret") && !text.contains("DECK_"),
        "{text}"
    );

    let daemon = TestDaemon::start_on(home).await;
    let mut client = Client::attach(
        &daemon.paths.socket,
        Attach {
            session: Some("main".into()),
            create: false,
            ..Attach::default()
        },
    )
    .await;
    assert_eq!(client.terminal(), terminal);
    client.type_line("echo only=[$CLIENT_ONLY_VAR]").await;
    client.wait_row("only=[]").await;
    client.type_line("echo id=$DECK_TERMINAL_ID").await;
    client.wait_row(&format!("id={terminal}")).await;
    client.type_line("echo sock=$DECK_SOCKET").await;
    client.wait_row(&format!("sock={}", socket.display())).await;
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_with_duplicates_and_dangling_focus_restores_without_error() {
    let work = workdir();
    let home = TestDaemon::new_home();
    let mut state = file(vec![
        session(
            1,
            "main",
            vec![
                project(
                    2,
                    "same",
                    work.path(),
                    vec![
                        panel(3, "a", vec![shell(4)]),
                        panel(5, "b", vec![shell(4), shell(6)]),
                    ],
                ),
                project(7, "same", work.path(), vec![panel(8, "c", vec![shell(9)])]),
            ],
        ),
        session(10, "main", vec![]),
    ]);
    state.next_id = 2;
    state.sessions[0].projects[0].active_panel = Some(PanelId(404));
    state.sessions[0].active_project = Some(ProjectId(405));
    state.last_attached = vec![SessionId(1), SessionId(406)];
    seed(home.path(), &state);
    let daemon = TestDaemon::start_on(home).await;
    let client = attach_to(&daemon, "main").await;
    let view = client.view.as_ref().unwrap();
    let names: Vec<_> = view.projects.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["same", "same (2)"]);
    // The later duplicate terminal went; its panel keeps the other one.
    assert_eq!(view.projects[0].panels[1].terminals.len(), 1);
    assert_eq!(view.projects[0].panels[1].terminals[0].id, TerminalId(6));
    assert_eq!(view.active_project, Some(ProjectId(2)));
    assert_eq!(view.projects[0].active_panel, Some(PanelId(3)));

    let mut control = Client::control(&daemon.paths.socket).await;
    let Response::Sessions(sessions) = control.request(Request::ListSessions).await else {
        panic!("sessions")
    };
    let names: Vec<_> = sessions.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["main", "main (2)"]);
    // New ids do not collide with saved ones.
    let mut client = client;
    client
        .action(Action::NewTerminal { panel: None })
        .await
        .unwrap();
    client
        .wait(|c| {
            c.view.as_ref().unwrap().projects[0].panels[0]
                .terminals
                .len()
                == 2
        })
        .await;
    let added = client.view.as_ref().unwrap().projects[0].panels[0].terminals[1].id;
    assert!(added.0 > 10, "{added}");
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn clients_connect_while_terminals_are_still_starting() {
    let work = workdir();
    let home = TestDaemon::new_home();
    let terminals: Vec<_> = (10..40).map(shell).collect();
    let state = file(vec![session(
        1,
        "main",
        vec![project(2, "p", work.path(), vec![panel(3, "1", terminals)])],
    )]);
    seed(home.path(), &state);
    let daemon = TestDaemon::start_on(home).await;
    // The socket accepts at once and the session is already there, whole.
    let started = Instant::now();
    let mut control = Client::control(&daemon.paths.socket).await;
    let Response::Sessions(sessions) = control.request(Request::ListSessions).await else {
        panic!("sessions")
    };
    assert_eq!(
        (sessions[0].name.as_str(), sessions[0].terminals),
        ("main", 30)
    );
    assert!(started.elapsed() < TIMEOUT);
    daemon.stop().await.unwrap();
}
