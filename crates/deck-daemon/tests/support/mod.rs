//! Test support: an in-process daemon on a temp `DECK_HOME` and a small protocol client.

#![allow(dead_code)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "shared integration-test helpers fail the test on unexpected I/O or protocol state"
)]

pub mod sys;

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use deck_core::action::{Action, ActionError};
use deck_core::config::Config;
use deck_core::model::{PanelId, ProjectId, SessionView, TerminalId};
use deck_core::protocol::{
    ByeReason, ClientMsg, Frame, FrameReader, NoticeLevel, OuterColors, PROTOCOL_VERSION, Request,
    Response, Role, ServerMsg, encode,
};
use deck_daemon::{Clock, Daemon, DaemonError, DaemonPaths, Options, SystemClock};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

pub const TIMEOUT: Duration = Duration::from_secs(5);

pub fn config() -> Config {
    Config {
        shell: Some("/bin/sh".into()),
        shell_args: Vec::new(),
        // Without the sidebar the panel area is the window minus the status-bar row.
        sidebar: false,
        ..Config::default()
    }
}

pub fn paths_in(home: &Path) -> DaemonPaths {
    DaemonPaths::from_env(
        &|k| (k == "DECK_HOME").then(|| home.to_string_lossy().into_owned()),
        0,
    )
    .expect("paths")
}

pub struct TestDaemon {
    pub home: TempDir,
    pub paths: DaemonPaths,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<Result<(), DaemonError>>>,
}

impl TestDaemon {
    pub async fn start() -> Self {
        Self::start_with(config(), Options::default()).await
    }

    /// A daemon that reads and watches `<home>/config.toml` (written with `text` first, when
    /// given), looking at it every `poll`.
    pub async fn start_watching(text: Option<&str>, poll: Duration) -> Self {
        let home = tempfile::Builder::new()
            .prefix("dk")
            .tempdir_in(short_tmp())
            .unwrap();
        if let Some(text) = text {
            std::fs::write(home.path().join("config.toml"), text).unwrap();
        }
        let options = Options {
            config_path: Some(home.path().join("config.toml")),
            config_poll: poll,
            ..Options::default()
        };
        Self::start_in(home, Config::default(), options).await
    }

    /// A temp home that no daemon runs on yet.
    pub fn new_home() -> TempDir {
        tempfile::Builder::new()
            .prefix("dk")
            .tempdir_in(short_tmp())
            .unwrap()
    }

    /// A daemon on an existing home (its state is read).
    pub async fn start_on(home: TempDir) -> Self {
        Self::start_in(home, config(), Options::default()).await
    }

    /// A daemon on an existing home with the given options.
    pub async fn start_on_with(home: TempDir, options: Options) -> Self {
        Self::start_in(home, config(), options).await
    }

    /// A daemon on an existing home with the given config and options.
    pub async fn start_on_config(home: TempDir, config: Config, options: Options) -> Self {
        Self::start_in(home, config, options).await
    }

    /// Stops the daemon like `SIGTERM` does and returns its home.
    pub async fn stop_keep(mut self) -> TempDir {
        self.stop_inner().await.expect("a clean stop");
        self.home
    }

    pub fn config_file(&self) -> PathBuf {
        self.home.path().join("config.toml")
    }

    pub async fn start_with(config: Config, options: Options) -> Self {
        let home = tempfile::Builder::new()
            .prefix("dk")
            .tempdir_in(short_tmp())
            .unwrap();
        Self::start_in(home, config, options).await
    }

    async fn start_in(home: TempDir, config: Config, options: Options) -> Self {
        let paths = paths_in(home.path());
        let (tx, rx) = oneshot::channel::<()>();
        let task = tokio::spawn({
            let paths = paths.clone();
            async move {
                Daemon::run_with(
                    paths,
                    config,
                    async {
                        let _ = rx.await;
                    },
                    options,
                )
                .await
            }
        });
        let daemon = Self {
            home,
            paths,
            shutdown: Some(tx),
            task: Some(task),
        };
        daemon.wait_for_socket().await;
        daemon
    }

    async fn wait_for_socket(&self) {
        let deadline = Instant::now() + TIMEOUT;
        while UnixStream::connect(&self.paths.socket).await.is_err() {
            assert!(Instant::now() < deadline, "daemon did not start");
            tokio::task::yield_now().await;
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    pub async fn connect(&self) -> Client {
        Client::connect(&self.paths.socket).await
    }

    /// Stops the daemon through its shutdown future and returns its result.
    pub async fn stop(mut self) -> Result<(), DaemonError> {
        self.stop_inner().await
    }

    async fn stop_inner(&mut self) -> Result<(), DaemonError> {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        let task = self.task.take().expect("daemon task");
        tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .expect("daemon stopped in time")
            .expect("daemon task did not panic")
    }

    /// Waits for a daemon that stops by itself, and returns its home.
    pub async fn wait_exit_keep(mut self) -> TempDir {
        let task = self.task.take().expect("daemon task");
        tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .expect("daemon stopped in time")
            .expect("daemon task did not panic")
            .expect("a clean stop");
        self.home
    }

    /// Waits for a daemon that stops by itself.
    pub async fn wait_exit(mut self) -> Result<(), DaemonError> {
        let task = self.task.take().expect("daemon task");
        tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .expect("daemon stopped in time")
            .expect("daemon task did not panic")
    }
}

/// A short base directory: socket paths must stay under 100 bytes.
fn short_tmp() -> PathBuf {
    let tmp = PathBuf::from("/tmp");
    if tmp.is_dir() {
        tmp
    } else {
        std::env::temp_dir()
    }
}

pub fn hello_env() -> Vec<(OsString, OsString)> {
    let mut env = vec![("PS1".into(), "$ ".into())];
    if let Some(path) = std::env::var_os("PATH") {
        env.push(("PATH".into(), path));
    }
    env
}

pub struct Attach {
    pub session: Option<String>,
    pub create: bool,
    /// The outer window; the daemon takes the status-bar row off for the panel area.
    pub cols: u16,
    pub rows: u16,
    pub cwd: PathBuf,
    pub env: Vec<(OsString, OsString)>,
    pub colors: Option<OuterColors>,
}

impl Default for Attach {
    fn default() -> Self {
        Self {
            session: None,
            create: true,
            cols: 80,
            rows: 25,
            cwd: std::env::temp_dir(),
            env: hello_env(),
            colors: None,
        }
    }
}

pub fn hello(
    role: Role,
    cwd: PathBuf,
    env: Vec<(OsString, OsString)>,
    colors: Option<OuterColors>,
) -> ClientMsg {
    ClientMsg::Hello {
        version: PROTOCOL_VERSION,
        role,
        cwd,
        env,
        colors,
    }
}

pub struct Client {
    stream: UnixStream,
    reader: FrameReader,
    /// Everything received so far.
    pub log: Vec<ServerMsg>,
    /// The screen of each terminal, row by row, as the frames built it.
    pub screens: HashMap<TerminalId, Vec<String>>,
    pub view: Option<SessionView>,
    pub bye: Option<ByeReason>,
    pub eof: bool,
}

impl Client {
    pub async fn connect(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).await.expect("connect");
        Self {
            stream,
            reader: FrameReader::new(),
            log: Vec::new(),
            screens: HashMap::new(),
            view: None,
            bye: None,
            eof: false,
        }
    }

    pub async fn send(&mut self, msg: &ClientMsg) {
        let bytes = encode(msg).unwrap();
        self.stream.write_all(&bytes).await.expect("send");
    }

    pub async fn send_raw(&mut self, bytes: &[u8]) {
        self.stream.write_all(bytes).await.expect("send");
    }

    /// Connects, attaches and waits for the first frame of the terminal.
    pub async fn attach(socket: &Path, a: Attach) -> Self {
        let mut c = Self::connect(socket).await;
        c.send(&hello(
            Role::Attach {
                session: a.session,
                create: a.create,
                cols: a.cols,
                rows: a.rows,
            },
            a.cwd,
            a.env,
            a.colors,
        ))
        .await;
        c.wait(|c| c.view.is_some() && !c.screens.is_empty()).await;
        c
    }

    pub async fn control(socket: &Path) -> Self {
        let mut c = Self::connect(socket).await;
        c.send(&hello(
            Role::Control,
            std::env::temp_dir(),
            Vec::new(),
            None,
        ))
        .await;
        c
    }

    /// The first terminal of the session view.
    pub fn terminal(&self) -> TerminalId {
        self.view.as_ref().expect("a session view").projects[0].panels[0].terminals[0].id
    }

    /// The focused terminal of the active panel.
    pub fn active_terminal(&self) -> TerminalId {
        let view = self.view.as_ref().expect("a session view");
        let project = view
            .projects
            .iter()
            .find(|p| Some(p.id) == view.active_project)
            .expect("an active project");
        let panel = project
            .panels
            .iter()
            .find(|p| Some(p.id) == view.active_panel)
            .expect("an active panel");
        panel.focused_terminal.expect("a focused terminal")
    }

    /// The projects as (name, panel names), in order.
    pub fn tree(&self) -> Vec<(String, Vec<String>)> {
        self.view
            .as_ref()
            .expect("a session view")
            .projects
            .iter()
            .map(|p| {
                (
                    p.name.clone(),
                    p.panels.iter().map(|x| x.name.clone()).collect(),
                )
            })
            .collect()
    }

    pub fn project_id(&self, name: &str) -> ProjectId {
        let view = self.view.as_ref().expect("a session view");
        view.projects
            .iter()
            .find(|p| p.name == name)
            .unwrap_or_else(|| panic!("no project {name}"))
            .id
    }

    pub fn panel_id(&self, project: &str, panel: &str) -> PanelId {
        let view = self.view.as_ref().expect("a session view");
        view.projects
            .iter()
            .find(|p| p.name == project)
            .and_then(|p| p.panels.iter().find(|x| x.name == panel))
            .unwrap_or_else(|| panic!("no panel {project}/{panel}"))
            .id
    }

    /// The panel at `position` (from 1) of a project: new panels are named after their
    /// shell, so tests find them by place.
    pub fn panel_at(&self, project: &str, position: usize) -> PanelId {
        let view = self.view.as_ref().expect("a session view");
        view.projects
            .iter()
            .find(|p| p.name == project)
            .and_then(|p| p.panels.get(position - 1))
            .unwrap_or_else(|| panic!("no panel {project}/{position}"))
            .id
    }

    /// Sends an action and waits for its result.
    pub async fn action(&mut self, action: Action) -> Result<(), ActionError> {
        self.send(&ClientMsg::Action(action)).await;
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let left = deadline
                .checked_duration_since(Instant::now())
                .expect("an action result in time");
            match self.recv(left).await {
                Some(ServerMsg::ActionResult(r)) => return r,
                Some(_) => {}
                None => panic!("no action result"),
            }
        }
    }

    fn absorb(&mut self, msg: ServerMsg) {
        match &msg {
            ServerMsg::SessionView(v) => self.view = Some(v.clone()),
            ServerMsg::Frame(f) => self.apply(f),
            ServerMsg::Bye { reason } => self.bye = Some(reason.clone()),
            _ => {}
        }
        self.log.push(msg);
    }

    fn apply(&mut self, f: &Frame) {
        let rows = self.screens.entry(f.terminal).or_default();
        if f.full || rows.len() != usize::from(f.size.1) {
            *rows = vec![String::new(); usize::from(f.size.1)];
        }
        for (row, line) in &f.lines {
            let mut text = String::new();
            for run in &line.runs {
                while text.chars().count() < usize::from(run.col) {
                    text.push(' ');
                }
                text.push_str(&run.text);
            }
            if let Some(slot) = rows.get_mut(usize::from(*row)) {
                *slot = text;
            }
        }
    }

    /// Reads one message, `None` on timeout or at the end of the stream.
    pub async fn recv(&mut self, wait: Duration) -> Option<ServerMsg> {
        let deadline = Instant::now() + wait;
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            if let Some(msg) = self
                .reader
                .next_message::<ServerMsg>()
                .expect("valid frame")
            {
                self.absorb(msg.clone());
                return Some(msg);
            }
            if self.eof {
                return None;
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            match tokio::time::timeout(left, self.stream.read(&mut buf)).await {
                Ok(Ok(0) | Err(_)) => {
                    self.eof = true;
                }
                Ok(Ok(n)) => self.reader.push(&buf[..n]),
                Err(_) => return None,
            }
        }
    }

    /// Reads until `done` holds for the client's state; panics after [`TIMEOUT`].
    pub async fn wait(&mut self, mut done: impl FnMut(&Client) -> bool) {
        let deadline = Instant::now() + TIMEOUT;
        while !done(self) {
            let left = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or_else(|| {
                    panic!(
                        "timed out; screens: {:?}, log: {}",
                        self.screens,
                        self.log.len()
                    )
                });
            if self.recv(left).await.is_none() && self.eof && !done(self) {
                panic!(
                    "connection ended; screens: {:?}, bye: {:?}",
                    self.screens, self.bye
                );
            }
        }
    }

    pub fn screen(&self) -> Vec<String> {
        self.screens.values().flatten().cloned().collect()
    }

    pub fn has_row(&self, text: &str) -> bool {
        self.screen().iter().any(|r| r.trim_end() == text)
    }

    pub fn has_text(&self, text: &str) -> bool {
        self.screen().iter().any(|r| r.contains(text))
    }

    pub async fn wait_row(&mut self, text: &str) {
        self.wait(|c| c.has_row(text)).await;
    }

    pub async fn wait_text(&mut self, text: &str) {
        self.wait(|c| c.has_text(text)).await;
    }

    pub async fn type_line(&mut self, line: &str) {
        let terminal = self.active_terminal();
        self.send(&ClientMsg::Input {
            terminal,
            bytes: format!("{line}\n").into_bytes(),
        })
        .await;
    }

    /// The decimal number after `marker` on a row that is not the echoed command line.
    pub fn number_after(&self, marker: &str) -> Option<i32> {
        self.screen().iter().find_map(|row| {
            let rest = row.trim_start().strip_prefix(marker)?;
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().ok()
        })
    }

    pub async fn request(&mut self, request: Request) -> Response {
        self.send(&ClientMsg::Request(request)).await;
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let left = deadline
                .checked_duration_since(Instant::now())
                .expect("a response in time");
            match self.recv(left).await {
                Some(ServerMsg::Response(r)) => return r,
                Some(_) => {}
                None => panic!("no response"),
            }
        }
    }

    pub fn frames(&self) -> impl Iterator<Item = &Frame> {
        self.log.iter().filter_map(|m| match m {
            ServerMsg::Frame(f) => Some(f),
            _ => None,
        })
    }

    /// The configs the daemon sent after `Welcome`, in order.
    pub fn configs(&self) -> Vec<&Config> {
        self.log
            .iter()
            .filter_map(|m| match m {
                ServerMsg::Config(c) => Some(&**c),
                _ => None,
            })
            .collect()
    }

    /// The notices received, in order.
    pub fn notices(&self) -> Vec<(NoticeLevel, &str)> {
        self.log
            .iter()
            .filter_map(|m| match m {
                ServerMsg::Notice { level, text } => Some((*level, text.as_str())),
                _ => None,
            })
            .collect()
    }

    pub fn has_notice(&self, text: &str) -> bool {
        self.notices().iter().any(|(_, t)| t.contains(text))
    }

    pub fn views(&self) -> impl Iterator<Item = &SessionView> {
        self.log.iter().filter_map(|m| match m {
            ServerMsg::SessionView(v) => Some(v),
            _ => None,
        })
    }
}

pub fn system_clock() -> Arc<dyn Clock> {
    Arc::new(SystemClock::new())
}
