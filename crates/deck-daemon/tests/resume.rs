//! Saving the agent of a terminal, ignoring agent changes while shutting down, and the two
//! ways a restored terminal reopens its agent. Agents and shells are fake `#!/bin/sh`
//! scripts in a temp directory that record their arguments.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use deck_core::agents::{AgentKind, AgentResume};
use deck_core::config::Config;
use deck_core::layout::Ratios;
use deck_core::model::{NameSource, PanelId, ProjectId, SessionId, TerminalId};
use deck_core::protocol::{Request, Response};
use deck_core::state::{PanelState, ProjectState, SessionState, StateFile, TerminalState};
use deck_daemon::{DaemonPaths, ForegroundQuery, Options};
use support::{Attach, Client, TestDaemon};

fn workdir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("rs")
        .tempdir_in("/tmp")
        .unwrap()
}

/// Fake `claude` and shell scripts; each writes its working directory and arguments, one per
/// line, to a file, then waits for a line on stdin.
struct Fakes {
    dir: tempfile::TempDir,
}

impl Fakes {
    fn new() -> Self {
        let dir = workdir();
        let fakes = Self { dir };
        fakes.script("claude");
        fakes.script("shell");
        fakes
    }

    fn script(&self, name: &str) {
        let path = self.dir.path().join(name);
        let rec = self.record_path(name);
        fs::write(
            &path,
            format!(
                "#!/bin/sh\n{{ pwd -P; for a in \"$@\"; do echo \"$a\"; done; }} > '{rec}.tmp'\nmv '{rec}.tmp' '{rec}'\nread line\n",
                rec = rec.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn path(&self, name: &str) -> String {
        self.dir.path().join(name).display().to_string()
    }

    fn record_path(&self, name: &str) -> PathBuf {
        self.dir.path().join(format!("{name}.rec"))
    }

    /// The directory and arguments the script recorded, once it ran.
    async fn recorded(&self, name: &str) -> Vec<String> {
        let rec = self.record_path(name);
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if let Ok(text) = fs::read_to_string(&rec) {
                return text.lines().map(str::to_string).collect();
            }
            assert!(Instant::now() < deadline, "{name} did not run");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn ran(&self, name: &str) -> bool {
        self.record_path(name).exists()
    }

    /// A config whose shell is the fake shell with `-i`, and whose resume templates run the
    /// fake claude by its path.
    fn config(&self) -> Config {
        let mut config = support::config();
        config.shell = Some(self.path("shell"));
        config.shell_args = vec!["-i".into()];
        let claude = self.path("claude");
        config.agents.claude.resume = format!("{claude} --resume {{session_id}}");
        config.agents.claude.resume_latest = format!("{claude} --continue");
        config
    }
}

fn terminal(
    program: Option<&str>,
    args: &[&str],
    cwd: &Path,
    agent: Option<AgentResume>,
) -> TerminalState {
    TerminalState {
        id: TerminalId(4),
        program: program.map(Into::into),
        args: args.iter().map(|a| (*a).into()).collect(),
        env: vec![],
        cwd: Some(cwd.to_path_buf()),
        keep: false,
        exited: None,
        agent,
    }
}

fn claude(id: Option<&str>) -> Option<AgentResume> {
    Some(AgentResume {
        kind: AgentKind::Claude,
        session_id: id.map(Into::into),
    })
}

fn state(root: &Path, terminal: TerminalState) -> StateFile {
    StateFile {
        version: 1,
        next_id: 1,
        last_attached: vec![],
        sessions: vec![SessionState {
            id: SessionId(1),
            name: "main".into(),
            active_project: Some(ProjectId(2)),
            projects: vec![ProjectState {
                id: ProjectId(2),
                name: "p".into(),
                root: root.to_path_buf(),
                collapsed: false,
                active_panel: Some(PanelId(3)),
                panels: vec![PanelState {
                    id: PanelId(3),
                    name: "1".into(),
                    name_source: NameSource::Default,
                    focused_terminal: Some(TerminalId(4)),
                    ratios: Ratios::default_for(1),
                    terminals: vec![terminal],
                }],
            }],
        }],
    }
}

fn seed(home: &Path, text: &[u8]) -> DaemonPaths {
    let paths = support::paths_in(home);
    fs::create_dir_all(&paths.state_dir).unwrap();
    fs::set_permissions(&paths.state_dir, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(paths.state_dir.join("state.json"), text).unwrap();
    paths
}

/// Starts a daemon that restores `file`.
async fn restore(file: &StateFile, config: Config) -> TestDaemon {
    let home = TestDaemon::new_home();
    seed(home.path(), &serde_json::to_vec_pretty(file).unwrap());
    TestDaemon::start_on_config(home, config, Options::default()).await
}

fn real(path: &Path) -> String {
    path.canonicalize().unwrap().display().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shell_terminal_resumes_the_agent_then_runs_the_shell_with_its_args() {
    let fakes = Fakes::new();
    let work = workdir();
    let file = state(
        work.path(),
        terminal(None, &[], work.path(), claude(Some("abc"))),
    );
    let daemon = restore(&file, fakes.config()).await;

    assert_eq!(
        fakes.recorded("claude").await,
        [real(work.path()).as_str(), "--resume", "abc"]
    );
    assert!(!fakes.ran("shell"), "the shell waits for the agent");

    // The agent quits: the user's shell takes its place with its configured arguments.
    let mut control = Client::control(&daemon.paths.socket).await;
    let sent = control
        .request(Request::Send {
            terminal: TerminalId(4),
            bytes: b"\n".to_vec(),
            paste: false,
        })
        .await;
    assert!(matches!(sent, Response::Ok), "{sent:?}");
    assert_eq!(
        fakes.recorded("shell").await,
        [real(work.path()).as_str(), "-i"]
    );
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_usable_session_id_the_latest_conversation_resumes() {
    for id in [
        None,
        Some("abc; rm -rf ~"),
        Some("-p"),
        Some("a b"),
        Some(""),
    ] {
        let fakes = Fakes::new();
        let work = workdir();
        let file = state(work.path(), terminal(None, &[], work.path(), claude(id)));
        let daemon = restore(&file, fakes.config()).await;
        assert_eq!(
            fakes.recorded("claude").await,
            [real(work.path()).as_str(), "--continue"],
            "{id:?}"
        );
        daemon.stop().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_custom_template_is_used() {
    let fakes = Fakes::new();
    let work = workdir();
    let mut config = fakes.config();
    config.agents.claude.resume =
        format!("{} --open {{session_id}} --verbose", fakes.path("claude"));
    let file = state(
        work.path(),
        terminal(None, &[], work.path(), claude(Some("s-1"))),
    );
    let daemon = restore(&file, config).await;
    assert_eq!(
        fakes.recorded("claude").await,
        [real(work.path()).as_str(), "--open", "s-1", "--verbose"]
    );
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn with_resume_agents_off_a_plain_shell_starts() {
    let fakes = Fakes::new();
    let work = workdir();
    let mut config = fakes.config();
    config.resume_agents = false;
    let file = state(
        work.path(),
        terminal(None, &[], work.path(), claude(Some("abc"))),
    );
    let daemon = restore(&file, config).await;
    assert_eq!(
        fakes.recorded("shell").await,
        [real(work.path()).as_str(), "-i"]
    );
    assert!(!fakes.ran("claude"));
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shell_terminal_without_an_agent_starts_a_plain_shell() {
    let fakes = Fakes::new();
    let work = workdir();
    let file = state(work.path(), terminal(None, &[], work.path(), None));
    let daemon = restore(&file, fakes.config()).await;
    assert_eq!(
        fakes.recorded("shell").await,
        [real(work.path()).as_str(), "-i"]
    );
    assert!(!fakes.ran("claude"));
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_explicit_agent_command_is_replaced_by_the_resume_command() {
    let fakes = Fakes::new();
    let work = workdir();
    // Launched as `<claude> --model x`: the program is the agent itself.
    let file = state(
        work.path(),
        terminal(
            Some(&fakes.path("claude")),
            &["--model", "x"],
            work.path(),
            claude(Some("xyz")),
        ),
    );
    let daemon = restore(&file, fakes.config()).await;
    assert_eq!(
        fakes.recorded("claude").await,
        [real(work.path()).as_str(), "--resume", "xyz"]
    );
    // No shell was involved.
    assert!(!fakes.ran("shell"));
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_explicit_program_that_is_not_the_agent_starts_as_saved() {
    let fakes = Fakes::new();
    let work = workdir();
    // The program is the fake shell, not an agent, whatever the saved agent says.
    let file = state(
        work.path(),
        terminal(
            Some(&fakes.path("shell")),
            &["-x"],
            work.path(),
            claude(Some("abc")),
        ),
    );
    let daemon = restore(&file, fakes.config()).await;
    assert_eq!(
        fakes.recorded("shell").await,
        [real(work.path()).as_str(), "-x"]
    );
    assert!(!fakes.ran("claude"));
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_state_file_without_the_agent_field_still_loads() {
    let fakes = Fakes::new();
    let work = workdir();
    let mut value =
        serde_json::to_value(state(work.path(), terminal(None, &[], work.path(), None))).unwrap();
    // A file written before the field existed.
    value["sessions"][0]["projects"][0]["panels"][0]["terminals"][0]
        .as_object_mut()
        .unwrap()
        .remove("agent")
        .unwrap();
    let home = TestDaemon::new_home();
    seed(home.path(), &serde_json::to_vec(&value).unwrap());
    let daemon = TestDaemon::start_on_config(home, fakes.config(), Options::default()).await;
    assert_eq!(
        fakes.recorded("shell").await,
        [real(work.path()).as_str(), "-i"]
    );
    daemon.stop().await.unwrap();
}

fn saved_agent(paths: &DaemonPaths) -> Option<Option<AgentResume>> {
    let file: StateFile =
        serde_json::from_slice(&fs::read(paths.state_dir.join("state.json")).ok()?).ok()?;
    let t = file
        .sessions
        .first()?
        .projects
        .first()?
        .panels
        .first()?
        .terminals
        .first()?
        .clone();
    Some(t.agent)
}

async fn wait_saved_agent(paths: &DaemonPaths, want: Option<AgentResume>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if saved_agent(paths) == Some(want.clone()) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "not saved: want {want:?}, saved {:?}",
            saved_agent(paths)
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn hook(
    control: &mut Client,
    terminal: TerminalId,
    status: Option<deck_core::agents::AgentStatus>,
    id: Option<&str>,
) {
    let r = control
        .request(Request::AgentEvent {
            terminal,
            kind: AgentKind::Claude,
            status,
            session_id: id.map(Into::into),
            prompt: None,
            raw: None,
        })
        .await;
    assert!(matches!(r, Response::Ok), "{r:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_agent_is_saved_and_survives_the_shutdown_and_a_late_session_end() {
    use deck_core::agents::AgentStatus;
    // The foreground process is an agent, so the hook state stays.
    let options = Options {
        agent_poll: Duration::from_millis(50),
        foreground_query: ForegroundQuery::new(|_| Some(vec!["claude".to_string()])),
        ..Options::default()
    };
    let daemon = TestDaemon::start_with(support::config(), options).await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let terminal = client.terminal();
    let mut control = Client::control(&daemon.paths.socket).await;

    // Found by its process: the kind is saved, with no session id.
    wait_saved_agent(&daemon.paths, claude(None)).await;
    // A hook brings the session id, a later one replaces it: each change is saved.
    hook(&mut control, terminal, Some(AgentStatus::Idle), Some("abc")).await;
    wait_saved_agent(&daemon.paths, claude(Some("abc"))).await;
    hook(
        &mut control,
        terminal,
        Some(AgentStatus::Working),
        Some("def"),
    )
    .await;
    wait_saved_agent(&daemon.paths, claude(Some("def"))).await;

    let paths = daemon.paths.clone();
    let stopping = tokio::spawn(daemon.stop_keep());
    // Once the clients are told, the daemon is shutting down: the agent's own SessionEnd
    // must not clear what resume needs. The daemon may already have closed the connection;
    // either way the saved agent stays. (The task absorbs the panic of a broken pipe.)
    client.wait(|c| c.bye.is_some()).await;
    let _ = tokio::spawn(async move {
        hook(&mut control, terminal, Some(AgentStatus::None), None).await;
    })
    .await;
    let _home = stopping.await.unwrap();
    assert_eq!(saved_agent(&paths), Some(claude(Some("def"))));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cleared_agent_is_saved_as_none() {
    use deck_core::agents::AgentStatus;
    let options = Options {
        agent_poll: Duration::from_millis(50),
        foreground_query: ForegroundQuery::new(|_| Some(vec!["claude".to_string()])),
        ..Options::default()
    };
    let daemon = TestDaemon::start_with(support::config(), options).await;
    let client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let terminal = client.terminal();
    let mut control = Client::control(&daemon.paths.socket).await;
    hook(&mut control, terminal, Some(AgentStatus::Idle), Some("abc")).await;
    wait_saved_agent(&daemon.paths, claude(Some("abc"))).await;
    hook(&mut control, terminal, Some(AgentStatus::None), None).await;
    // The process detection finds the agent again on its next tick; either way the file
    // follows the model, and never keeps a stale session id.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if matches!(saved_agent(&daemon.paths), Some(a) if a != claude(Some("abc"))) {
            break;
        }
        assert!(Instant::now() < deadline, "the old session id stayed");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_right_after_a_restore_keeps_the_agent_and_its_session() {
    let fakes = Fakes::new();
    let work = workdir();
    let file = state(
        work.path(),
        terminal(None, &[], work.path(), claude(Some("abc"))),
    );
    let home = TestDaemon::new_home();
    let paths = seed(home.path(), &serde_json::to_vec_pretty(&file).unwrap());
    let daemon = TestDaemon::start_on_config(home, fakes.config(), Options::default()).await;
    // Well under the 1 s detection tick: the saved agent is in the model from the start.
    let _home = daemon.stop_keep().await;
    assert_eq!(saved_agent(&paths), Some(claude(Some("abc"))));
}
