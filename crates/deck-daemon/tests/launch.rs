//! `Action::NewPanelWith`: a panel that runs an agent's `command` directly, the program
//! lookup in the session's `PATH`, and what is saved. Agents are fake `#!/bin/sh` scripts in
//! temp directories; no real agent runs.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use deck_core::action::{Action, ActionError, PanelLaunch};
use deck_core::agents::AgentKind;
use deck_core::config::Config;
use deck_core::protocol::Request;
use serde_json::Value;
use support::{Attach, Client, TestDaemon};

fn dir(prefix: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in("/tmp")
        .unwrap()
}

fn real(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().canonicalize().unwrap()
}

fn make_executable(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

/// A fake agent in `dir`: records its directory, arguments and `DECK_PANEL_ID` (one per
/// line) to `<name>.rec`, then waits for a line on stdin.
fn fake(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    let rec = dir.join(format!("{name}.rec"));
    // Shell built-ins only: the fake's `PATH` is the test's. One write, so a reader never
    // sees half of it.
    fs::write(
        &path,
        format!(
            "#!/bin/sh\nout=\"$(pwd -P)\n$DECK_PANEL_ID\"\nfor a; do out=\"$out\n$a\"; done\necho \"$out\" > '{rec}'\nread line\n",
            rec = rec.display()
        ),
    )
    .unwrap();
    make_executable(&path, 0o755);
    path
}

async fn recorded(dir: &Path, name: &str) -> Vec<String> {
    let rec = dir.join(format!("{name}.rec"));
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Ok(text) = fs::read_to_string(&rec)
            && !text.is_empty()
        {
            return text.lines().map(str::to_string).collect();
        }
        assert!(Instant::now() < deadline, "{name} did not run");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// An attach whose environment has exactly `path` as its `PATH`.
fn attach_with(path: &Path, cwd: &Path) -> Attach {
    Attach {
        cwd: cwd.to_path_buf(),
        env: vec![
            (OsString::from("PS1"), OsString::from("$ ")),
            (OsString::from("PATH"), path.as_os_str().to_os_string()),
        ],
        ..Attach::default()
    }
}

fn panel_count(c: &Client) -> usize {
    c.view
        .as_ref()
        .unwrap()
        .projects
        .iter()
        .map(|p| p.panels.len())
        .sum()
}

async fn launch(c: &mut Client, kind: AgentKind) -> Result<(), ActionError> {
    let project = c.view.as_ref().unwrap().projects[0].id;
    c.action(Action::NewPanelWith {
        project,
        launch: PanelLaunch::Agent(kind),
    })
    .await
}

async fn started(config: Config) -> TestDaemon {
    TestDaemon::start_with(config, deck_daemon::Options::default()).await
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_panel_runs_the_program_in_the_project_root_and_is_active() {
    let bin = dir("bin");
    fake(bin.path(), "claude");
    let root = dir("root");
    let daemon = started(support::config()).await;
    let mut c = Client::attach(&daemon.paths.socket, attach_with(bin.path(), &real(&root))).await;
    assert_eq!(panel_count(&c), 1);

    launch(&mut c, AgentKind::Claude).await.unwrap();
    c.wait(|c| panel_count(c) == 2).await;

    let view = c.view.clone().unwrap();
    let panel = &view.projects[0].panels[1];
    assert_eq!(view.active_panel, Some(panel.id), "the new panel is active");
    assert_eq!(panel.terminals[0].program, "claude");
    assert_eq!(panel.focused_terminal, Some(panel.terminals[0].id));
    assert_eq!(
        panel.name_source,
        deck_core::model::NameSource::Auto,
        "named at once after its program"
    );
    assert_eq!(panel.name, "claude");

    // Directly, with no shell, in the project's root, with deck's variables.
    let lines = recorded(bin.path(), "claude").await;
    assert_eq!(lines[0], real(&root).display().to_string());
    assert_eq!(lines[1], panel.id.to_string());
    assert_eq!(lines.len(), 2, "no arguments");

    // `deck ls --json` shows the program too.
    let tree = c.request(Request::Tree { session: None }).await;
    assert!(format!("{tree:?}").contains("program: \"claude\""));
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_command_arguments_are_passed() {
    let bin = dir("bin");
    fake(bin.path(), "claude");
    let root = dir("root");
    let mut config = support::config();
    config.agents.claude.command = "claude --model fast".into();
    let daemon = started(config).await;
    let mut c = Client::attach(&daemon.paths.socket, attach_with(bin.path(), &real(&root))).await;

    launch(&mut c, AgentKind::Claude).await.unwrap();
    let lines = recorded(bin.path(), "claude").await;
    assert_eq!(lines[2..], ["--model", "fast"]);
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_program_is_refused_and_nothing_is_created() {
    let bin = dir("bin");
    fake(bin.path(), "claude");
    let root = dir("root");
    let daemon = started(support::config()).await;
    let mut c = Client::attach(&daemon.paths.socket, attach_with(bin.path(), &real(&root))).await;
    let before = c.view.clone().unwrap();

    let err = launch(&mut c, AgentKind::OpenCode).await.unwrap_err();
    assert_eq!(
        err,
        ActionError::ProgramNotFound {
            program: "opencode".into(),
            kind: AgentKind::OpenCode,
            key: "command".into(),
        }
    );
    assert_eq!(
        err.to_string(),
        "\"opencode\" not found in PATH (set [agents.opencode] command)"
    );
    // A request after the refusal sees the same tree and focus.
    c.request(Request::Tree { session: None }).await;
    assert_eq!(panel_count(&c), panel_count_of(&before));
    assert_eq!(c.view.as_ref().unwrap().active_panel, before.active_panel);
    daemon.stop().await.unwrap();
}

fn panel_count_of(view: &deck_core::model::SessionView) -> usize {
    view.projects.iter().map(|p| p.panels.len()).sum()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_project_is_refused() {
    let bin = dir("bin");
    fake(bin.path(), "claude");
    let root = dir("root");
    let daemon = started(support::config()).await;
    let mut c = Client::attach(&daemon.paths.socket, attach_with(bin.path(), &real(&root))).await;
    let nope = deck_core::model::ProjectId(9999);
    let panels = panel_count(&c);
    let active = c.view.as_ref().unwrap().active_panel;
    assert_eq!(
        c.action(Action::NewPanelWith {
            project: nope,
            launch: PanelLaunch::Agent(AgentKind::Claude),
        })
        .await,
        Err(ActionError::NoSuchProject(nope))
    );
    c.request(Request::Tree { session: None }).await;
    assert_eq!(panel_count(&c), panels);
    assert_eq!(c.view.as_ref().unwrap().active_panel, active);
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_path_searched_is_the_one_of_the_client_that_created_the_session() {
    let creator = dir("creator");
    let attacher = dir("attacher");
    fake(creator.path(), "claude");
    fake(attacher.path(), "opencode");
    let root = dir("root");
    // `sh` exists on the daemon's own PATH (this process's), and on no other here.
    assert!(
        std::env::split_paths(&std::env::var_os("PATH").unwrap()).any(|d| d.join("sh").is_file()),
        "the daemon's PATH has sh"
    );
    let mut config = support::config();
    config.agents.codex.command = "sh".into();
    let daemon = started(config).await;
    let session = "shared".to_string();
    let mut first = Client::attach(
        &daemon.paths.socket,
        Attach {
            session: Some(session.clone()),
            ..attach_with(creator.path(), &real(&root))
        },
    )
    .await;

    // Only on the creating client's PATH: found.
    launch(&mut first, AgentKind::Claude).await.unwrap();
    recorded(creator.path(), "claude").await;
    // Only on the daemon's PATH: not found.
    assert!(matches!(
        launch(&mut first, AgentKind::Codex).await,
        Err(ActionError::ProgramNotFound { program, .. }) if program == "sh"
    ));

    // A later attacher with another PATH changes nothing: its own programs are not found.
    let mut second = Client::attach(
        &daemon.paths.socket,
        Attach {
            session: Some(session),
            create: false,
            ..attach_with(attacher.path(), &real(&root))
        },
    )
    .await;
    assert!(matches!(
        launch(&mut second, AgentKind::OpenCode).await,
        Err(ActionError::ProgramNotFound { program, .. }) if program == "opencode"
    ));
    assert!(!attacher.path().join("opencode.rec").exists());
    // And the creator's still are.
    launch(&mut second, AgentKind::Claude).await.unwrap();
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_relative_path_entry_is_taken_from_the_project_root() {
    let root = dir("root");
    let bin = real(&root).join("bin");
    fs::create_dir(&bin).unwrap();
    fake(&bin, "claude");
    let daemon = started(support::config()).await;
    let mut c = Client::attach(
        &daemon.paths.socket,
        attach_with(Path::new("bin:/usr/bin"), &real(&root)),
    )
    .await;

    launch(&mut c, AgentKind::Claude).await.unwrap();
    let lines = recorded(&bin, "claude").await;
    assert_eq!(lines[0], real(&root).display().to_string());
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_absolute_command_must_be_an_executable_file() {
    let bin = dir("bin");
    let plain = bin.path().join("claude");
    fs::write(&plain, "#!/bin/sh\nexit 0\n").unwrap();
    make_executable(&plain, 0o644);
    let root = dir("root");
    let mut config = support::config();
    config.agents.claude.command = plain.display().to_string();
    let daemon = started(config).await;
    let mut c = Client::attach(&daemon.paths.socket, attach_with(bin.path(), &real(&root))).await;
    let active = c.view.as_ref().unwrap().active_panel;

    assert!(matches!(
        launch(&mut c, AgentKind::Claude).await,
        Err(ActionError::ProgramNotFound { program, .. }) if program == plain.display().to_string()
    ));
    assert_eq!(panel_count(&c), 1);
    assert_eq!(c.view.as_ref().unwrap().active_panel, active);

    // A directory is no program either.
    let mut config = support::config();
    config.agents.claude.command = bin.path().display().to_string();
    let other = started(config).await;
    let mut c2 = Client::attach(&other.paths.socket, attach_with(bin.path(), &real(&root))).await;
    assert!(matches!(
        launch(&mut c2, AgentKind::Claude).await,
        Err(ActionError::ProgramNotFound { .. })
    ));
    other.stop().await.unwrap();
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_launch_that_cannot_start_fails_and_creates_nothing() {
    let bin = dir("bin");
    fake(bin.path(), "claude");
    let root = dir("root");
    let gone = dir("gone");
    let daemon = started(support::config()).await;
    let mut c = Client::attach(&daemon.paths.socket, attach_with(bin.path(), &real(&root))).await;
    c.action(Action::NewProject {
        name: "gone".into(),
        root: real(&gone),
    })
    .await
    .unwrap();
    c.wait(|c| c.view.as_ref().is_some_and(|v| v.projects.len() == 2))
        .await;
    let project = c.project_id("gone");
    let panels = panel_count(&c);
    let active = c.view.as_ref().unwrap().active_panel;
    // The project's directory vanishes: the program is found but cannot start there.
    drop(gone);

    match c
        .action(Action::NewPanelWith {
            project,
            launch: PanelLaunch::Agent(AgentKind::Claude),
        })
        .await
    {
        Err(ActionError::LaunchFailed(text)) => {
            assert!(text.contains("could not start claude"), "{text}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(panel_count(&c), panels);
    assert_eq!(c.view.as_ref().unwrap().active_panel, active);
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_terminal_is_saved_as_an_explicit_program() {
    let bin = dir("bin");
    fake(bin.path(), "claude");
    let root = dir("root");
    let mut config = support::config();
    config.agents.claude.command = "claude --model fast".into();
    let daemon = started(config).await;
    let mut c = Client::attach(&daemon.paths.socket, attach_with(bin.path(), &real(&root))).await;
    launch(&mut c, AgentKind::Claude).await.unwrap();
    recorded(bin.path(), "claude").await;
    drop(c);

    let state = daemon.paths.state_dir.join("state.json");
    let _home = daemon.stop_keep().await;
    let doc: Value = serde_json::from_slice(&fs::read(state).unwrap()).unwrap();
    let terminals: Vec<&Value> = doc["sessions"][0]["projects"][0]["panels"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|p| p["terminals"].as_array().unwrap())
        .collect();
    let agent = terminals
        .iter()
        .find(|t| t["program"] == "claude")
        .expect("the agent terminal is saved");
    assert_eq!(agent["args"], serde_json::json!(["--model", "fast"]));
}
