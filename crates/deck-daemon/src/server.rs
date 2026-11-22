//! The daemon's shared state: sessions, clients and the rules that tie them together.
//!
//! Lock order: `state`, then a terminal's `emu`. Nothing waits while holding either.

use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use alacritty_terminal::vte::ansi::{NamedColor, Rgb};
use deck_core::action::{Action, ActionError, NameKind, default_project_name, validate_name};
use deck_core::config::Config;
use deck_core::layout::panel_area;
use deck_core::model::{
    LaunchSpec, NameSource, Panel, PanelId, Project, ProjectId, Session, SessionId, Terminal,
    TerminalId,
};
use deck_core::protocol::{
    ByeReason, Frame, NoticeLevel, OuterColors, ScrollTo, ServerMsg, SessionSummary,
};
use tokio::runtime::Handle;
use tokio::sync::{Notify, watch};

use crate::clock::{Clock, Fire, Poke, Throttle};
use crate::config_file::Notice;
use crate::emu::{EmuEvent, clamp_size, default_palette};
use crate::env::{self, Env, TerminalIds};
use crate::lock;
use crate::queue::Queue;
use crate::startup::DaemonPaths;
use crate::terminal::{self, SpawnRequest, TermHandle};

pub type ClientId = u64;

/// Title-only `SessionView`s are sent at most this often per session.
const TITLE_VIEW_INTERVAL: Duration = Duration::from_millis(100);
/// A terminal that fails within this long of starting is reported as a start failure.
const QUICK_EXIT: Duration = Duration::from_secs(1);
const FALLBACK_COLORS: OuterColors = OuterColors {
    fg: (192, 192, 192),
    bg: (0, 0, 0),
};
const HANG_UP_GRACE: Duration = Duration::from_secs(2);
const WRITER_FLUSH: Duration = Duration::from_secs(1);
/// The panel area of a session no client has sized yet.
const DEFAULT_VIEWPORT: (u16, u16) = (80, 24);

/// Notices kept for the next client while none is attached.
const MAX_PENDING_NOTICES: usize = 20;

/// What an attaching client asks for.
#[derive(Debug, Clone)]
pub struct AttachRequest {
    pub session: Option<String>,
    pub create: bool,
    /// The size of the client's outer window.
    pub cols: u16,
    pub rows: u16,
    pub cwd: PathBuf,
    pub env: Env,
}

struct SessionRt {
    model: Session,
    /// The environment of the client that created the session, refreshed on attach.
    env: Env,
    terminals: HashMap<TerminalId, Arc<TermHandle>>,
    /// The panel area the terminals are sized for.
    viewport: Option<(u16, u16)>,
    title_throttle: Throttle,
    /// When a client last attached; the highest tick is the most recently attached.
    last_attach: u64,
    /// The terminal the session was created with: if it fails at once, the session ends
    /// with that error instead of staying empty.
    startup: Option<TerminalId>,
}

impl SessionRt {
    /// The terminals of the active panel: the only ones whose frames are streamed.
    fn active_terminals(&self) -> Vec<Arc<TermHandle>> {
        let Some(active) = self.model.active_panel() else {
            return Vec::new();
        };
        self.model
            .projects
            .iter()
            .flat_map(|p| &p.panels)
            .find(|p| p.id == active)
            .map(|p| {
                p.terminals
                    .iter()
                    .filter_map(|t| self.terminals.get(&t.id).cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn is_active(&self, terminal: TerminalId) -> bool {
        self.active_terminals().iter().any(|t| t.id == terminal)
    }
}

/// Where a new panel goes.
enum PanelTarget {
    Project {
        project: ProjectId,
        name: Option<String>,
    },
    NewProject {
        name: String,
        root: PathBuf,
    },
}

/// What an attach does once the state lock is released.
enum Plan {
    Join(SessionId),
    Create(NewSession),
}

/// A session to create; its ids are allocated, its first process not started yet.
struct NewSession {
    sid: SessionId,
    pid: ProjectId,
    panel: PanelId,
    tid: TerminalId,
    name: String,
    req: SpawnRequest,
}

struct Client {
    queue: Arc<Queue>,
    session: Option<SessionId>,
    viewport: Option<(u16, u16)>,
    colors: Option<OuterColors>,
    /// Higher is more recent.
    active: u64,
}

#[derive(Default)]
struct State {
    next_id: u64,
    next_client: u64,
    tick: u64,
    sessions: Vec<SessionRt>,
    clients: HashMap<ClientId, Client>,
    /// Sessions being created: their first process is starting, outside the lock.
    creating: usize,
    shutting_down: bool,
    /// Terminals that left the model and are being hung up; shutdown finishes them.
    hanging: Vec<Arc<TermHandle>>,
    /// Notices no attached client has seen, oldest first.
    notices: VecDeque<(NoticeLevel, String)>,
}

impl State {
    fn alloc(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    fn touch(&mut self) -> u64 {
        self.tick += 1;
        self.tick
    }

    fn session(&self, id: SessionId) -> Option<&SessionRt> {
        self.sessions.iter().find(|s| s.model.id == id)
    }

    fn session_mut(&mut self, id: SessionId) -> Option<&mut SessionRt> {
        self.sessions.iter_mut().find(|s| s.model.id == id)
    }

    fn unique_name(&self, base: &str) -> String {
        let taken = |n: &str| self.sessions.iter().any(|s| s.model.name == n);
        if !taken(base) {
            return base.to_string();
        }
        (2..)
            .map(|i| format!("{base}-{i}"))
            .find(|n| !taken(n))
            .expect("an unbounded range has a free name")
    }

    fn clients_of(&self, sid: SessionId) -> impl Iterator<Item = (&ClientId, &Client)> {
        self.clients
            .iter()
            .filter(move |(_, c)| c.session == Some(sid))
    }

    /// The client of a session that was active last.
    fn active_client(&self, sid: SessionId) -> Option<&Client> {
        self.clients_of(sid)
            .map(|(_, c)| c)
            .max_by_key(|c| c.active)
    }

    fn push_view(&self, sid: SessionId) {
        let Some(sess) = self.session(sid) else {
            return;
        };
        let view = sess.model.view(sess.viewport);
        for (_, c) in self.clients_of(sid) {
            c.queue.push_view(view.clone());
        }
    }

    fn full_frames(&self, sid: SessionId) -> Vec<Frame> {
        let Some(sess) = self.session(sid) else {
            return Vec::new();
        };
        let mut frames: Vec<Frame> = sess
            .active_terminals()
            .iter()
            .filter_map(|t| t.with_emu(|e| e.full_frame(t.id)))
            .collect();
        frames.sort_by_key(|f| f.terminal);
        frames
    }

    fn push_frame(&self, sid: SessionId, frame: &Frame) {
        for (_, c) in self.clients_of(sid) {
            c.queue.push_frame(frame.clone());
        }
    }
}

fn terminal_model(session: &mut Session, id: TerminalId) -> Option<&mut Terminal> {
    session
        .projects
        .iter_mut()
        .flat_map(|p| p.panels.iter_mut())
        .flat_map(|p| p.terminals.iter_mut())
        .find(|t| t.id == id)
}

fn project_name(cwd: &Path, env: &Env) -> String {
    default_project_name(cwd, env::lookup(env, "HOME").map(Path::new))
}

/// A directory a client named: absolute (clients resolve `~` and relative paths) and
/// existing.
fn check_dir(dir: &Path) -> Result<(), ActionError> {
    if !dir.is_absolute() {
        return Err(ActionError::NotAbsolute(dir.display().to_string()));
    }
    if !dir.is_dir() {
        return Err(ActionError::NotADirectory(dir.display().to_string()));
    }
    Ok(())
}

fn program_name(program: &str) -> String {
    Path::new(program)
        .file_name()
        .map_or_else(|| program.to_string(), |n| n.to_string_lossy().into_owned())
}

/// The size of a panel's terminal: the whole panel area (a single terminal has no border).
fn inner_size(viewport: (u16, u16)) -> (u16, u16) {
    clamp_size(viewport.0, viewport.1)
}

pub struct Shared {
    pub paths: DaemonPaths,
    config: Mutex<Arc<Config>>,
    pub clock: Arc<dyn Clock>,
    pub version: u32,
    pub handle: Handle,
    state: Mutex<State>,
    shutdown_tx: watch::Sender<bool>,
    writers: AtomicUsize,
    writers_done: Notify,
    attaching: tokio::sync::Mutex<()>,
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared").finish_non_exhaustive()
    }
}

/// Counts a connection's writer task until it ends or is aborted.
#[derive(Debug)]
pub struct WriterGuard(Arc<Shared>);

impl Drop for WriterGuard {
    fn drop(&mut self) {
        self.0.writers.fetch_sub(1, Ordering::AcqRel);
        self.0.writers_done.notify_one();
    }
}

impl Shared {
    pub fn new(
        paths: DaemonPaths,
        config: Config,
        notices: Vec<Notice>,
        clock: Arc<dyn Clock>,
        version: u32,
    ) -> (Arc<Self>, watch::Receiver<bool>) {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let shared = Arc::new(Self {
            paths,
            config: Mutex::new(Arc::new(config)),
            clock,
            version,
            handle: Handle::current(),
            state: Mutex::new(State::default()),
            shutdown_tx,
            writers: AtomicUsize::new(0),
            writers_done: Notify::new(),
            attaching: tokio::sync::Mutex::new(()),
        });
        for notice in notices {
            shared.notify(notice.level, notice.text);
        }
        (shared, shutdown_rx)
    }

    /// The config in force.
    pub fn config(&self) -> Arc<Config> {
        Arc::clone(&lock(&self.config))
    }

    /// Replaces the config, tells the attached clients, then sends `notices`.
    pub fn reload(&self, config: Config, notices: Vec<Notice>) {
        let config = Arc::new(config);
        *lock(&self.config) = Arc::clone(&config);
        for client in self.st().clients.values().filter(|c| c.session.is_some()) {
            client
                .queue
                .push_msg(ServerMsg::Config(Box::new(Config::clone(&config))));
        }
        for notice in notices {
            self.notify(notice.level, notice.text);
        }
    }

    /// Shows a message to the user: to every attached client, or to the next one that
    /// attaches when there is none (the last 20 are kept). Always in the log.
    pub fn notify(&self, level: NoticeLevel, text: String) {
        match level {
            NoticeLevel::Info => tracing::info!("{text}"),
            NoticeLevel::Warning => tracing::warn!("{text}"),
            NoticeLevel::Error => tracing::error!("{text}"),
        }
        let mut st = self.st();
        let mut delivered = false;
        for client in st.clients.values().filter(|c| c.session.is_some()) {
            client.queue.push_msg(ServerMsg::Notice {
                level,
                text: text.clone(),
            });
            delivered = true;
        }
        if !delivered {
            // The same message again replaces the earlier one: the newest is kept.
            st.notices.retain(|(l, t)| !(*l == level && *t == text));
            st.notices.push_back((level, text));
            while st.notices.len() > MAX_PENDING_NOTICES {
                st.notices.pop_front();
            }
        }
    }

    fn st(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }

    pub fn writer_guard(self: &Arc<Self>) -> WriterGuard {
        self.writers.fetch_add(1, Ordering::AcqRel);
        WriterGuard(Arc::clone(self))
    }

    // Clients.

    /// Registers a connection; `None` while the daemon is shutting down.
    pub fn add_client(&self, colors: Option<OuterColors>) -> Option<(ClientId, Arc<Queue>)> {
        let mut st = self.st();
        if st.shutting_down {
            return None;
        }
        st.next_client += 1;
        let id = st.next_client;
        let active = st.touch();
        let queue = Arc::new(Queue::new());
        st.clients.insert(
            id,
            Client {
                queue: Arc::clone(&queue),
                session: None,
                viewport: None,
                colors,
                active,
            },
        );
        Some((id, queue))
    }

    /// Removes a client; its session and processes keep running.
    pub fn remove_client(&self, id: ClientId) {
        if let Some(client) = self.st().clients.remove(&id) {
            client.queue.close();
        }
    }

    pub async fn attach(self: &Arc<Self>, cid: ClientId, a: &AttachRequest) -> Result<(), String> {
        // One attach at a time, so a session name that is free while a PTY opens (outside
        // the state lock) is still free when the session is added.
        let _attaching = self.attaching.lock().await;
        let config = self.config();
        let viewport = panel_area((a.cols, a.rows), &config, config.sidebar);
        let plan = {
            let mut st = self.st();
            if st.shutting_down {
                return Err("the daemon is shutting down".into());
            }
            let plan = self.plan_attach(&mut st, a, viewport)?;
            if matches!(plan, Plan::Create(_)) {
                st.creating += 1;
            }
            plan
        };
        let (sid, mut st) = match plan {
            Plan::Join(sid) => (sid, self.st()),
            Plan::Create(new) => {
                let req = new.req.clone();
                let opened = tokio::task::spawn_blocking(move || terminal::open(&req))
                    .await
                    .map_err(|e| format!("could not start {}: {e}", new.req.program))
                    .and_then(|opened| opened);
                let mut st = self.st();
                st.creating -= 1;
                let parts = opened?;
                if st.shutting_down {
                    parts.discard();
                    return Err("the daemon is shutting down".into());
                }
                let sid = self.add_session(&mut st, new, parts, a, viewport);
                (sid, st)
            }
        };

        let active = st.touch();
        if let Some(sess) = st.session_mut(sid) {
            env::refresh(&mut sess.env, &a.env);
            sess.last_attach = active;
        }
        let Some(client) = st.clients.get_mut(&cid) else {
            return Err("connection closed".into());
        };
        // The config as it is now (a reload may have come while the session was created):
        // the viewport and the `Welcome` both come from this one read.
        let config = self.config();
        client.session = Some(sid);
        client.viewport = Some(panel_area((a.cols, a.rows), &config, config.sidebar));
        client.active = active;
        let queue = Arc::clone(&client.queue);

        queue.push_msg(ServerMsg::Welcome {
            session: sid,
            config: Box::new(Config::clone(&config)),
        });
        for (level, text) in std::mem::take(&mut st.notices) {
            queue.push_msg(ServerMsg::Notice { level, text });
        }
        if let Some(sess) = st.session(sid) {
            queue.push_view(sess.model.view(sess.viewport));
        }
        for frame in st.full_frames(sid) {
            queue.push_frame(frame);
        }
        // The attach size is a viewport claim, like a `Viewport` message.
        self.follow_viewport(&mut st, cid);
        Ok(())
    }

    /// The session an attach joins, or the one it creates.
    fn plan_attach(
        &self,
        st: &mut State,
        a: &AttachRequest,
        viewport: (u16, u16),
    ) -> Result<Plan, String> {
        let existing = match &a.session {
            None => st
                .sessions
                .iter()
                .max_by_key(|s| s.last_attach)
                .map(|s| s.model.id),
            Some(name) => st
                .sessions
                .iter()
                .find(|s| s.model.name == *name)
                .map(|s| s.model.id),
        };
        if let Some(sid) = existing {
            return Ok(Plan::Join(sid));
        }
        let name = match &a.session {
            None => st.unique_name("main"),
            Some(name) if a.create => {
                validate_name(name, NameKind::Session).map_err(|e| e.to_string())?
            }
            Some(name) => return Err(format!("no session named {name}")),
        };
        Ok(Plan::Create(self.new_session_plan(st, name, a, viewport)))
    }

    fn new_session_plan(
        &self,
        st: &mut State,
        name: String,
        a: &AttachRequest,
        viewport: (u16, u16),
    ) -> NewSession {
        let sid = SessionId(st.alloc());
        let pid = ProjectId(st.alloc());
        let panel = PanelId(st.alloc());
        let tid = TerminalId(st.alloc());
        let req = self.spawn_request(&a.env, &a.cwd, viewport, sid, panel, tid);
        NewSession {
            sid,
            pid,
            panel,
            tid,
            name,
            req,
        }
    }

    /// The process of a new terminal: the shell, in `cwd`, sized for `viewport`.
    fn spawn_request(
        &self,
        session_env: &Env,
        cwd: &Path,
        viewport: (u16, u16),
        session: SessionId,
        panel: PanelId,
        terminal: TerminalId,
    ) -> SpawnRequest {
        let inner = inner_size(viewport);
        let config = self.config();
        let program = config
            .shell
            .clone()
            .or_else(|| {
                env::lookup(session_env, "SHELL")
                    .and_then(|v| v.to_str())
                    .map(str::to_string)
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "/bin/sh".to_string());
        SpawnRequest {
            program,
            args: config.shell_args.clone(),
            cwd: cwd.to_path_buf(),
            env: env::terminal_env(
                session_env,
                &TerminalIds {
                    socket: &self.paths.socket,
                    session,
                    panel,
                    terminal,
                },
            ),
            cols: inner.0,
            rows: inner.1,
        }
    }

    /// Adds a planned session around its started first process.
    fn add_session(
        self: &Arc<Self>,
        st: &mut State,
        new: NewSession,
        parts: terminal::Parts,
        a: &AttachRequest,
        viewport: (u16, u16),
    ) -> SessionId {
        let NewSession {
            sid,
            pid,
            panel,
            tid,
            name,
            req,
        } = new;
        let handle = terminal::start(self, tid, sid, &req, parts);

        let model = Session {
            id: sid,
            name,
            active_project: Some(pid),
            projects: vec![Project {
                id: pid,
                name: project_name(&a.cwd, &a.env),
                root: a.cwd.clone(),
                collapsed: false,
                active_panel: Some(panel),
                panels: vec![Panel {
                    id: panel,
                    name: "1".into(),
                    name_source: NameSource::Default,
                    focused_terminal: Some(tid),
                    terminals: vec![Terminal {
                        id: tid,
                        launch: LaunchSpec {
                            program: None,
                            args: Vec::new(),
                            cwd: a.cwd.clone(),
                            env: Vec::<(OsString, OsString)>::new(),
                        },
                        program: program_name(&req.program),
                        title: None,
                        cwd: None,
                        exit_status: None,
                    }],
                }],
            }],
        };
        st.sessions.push(SessionRt {
            model,
            env: a.env.clone(),
            terminals: HashMap::from([(tid, handle)]),
            viewport: Some(viewport),
            title_throttle: Throttle::new(TITLE_VIEW_INTERVAL),
            last_attach: 0,
            startup: Some(tid),
        });
        sid
    }

    // Requests.

    pub fn list_sessions(&self) -> Vec<SessionSummary> {
        let st = self.st();
        let mut out: Vec<SessionSummary> = st
            .sessions
            .iter()
            .map(|s| SessionSummary {
                id: s.model.id,
                name: s.model.name.clone(),
                attached_clients: u32::try_from(st.clients_of(s.model.id).count())
                    .unwrap_or(u32::MAX),
                terminals: u32::try_from(s.terminals.len()).unwrap_or(u32::MAX),
                projects: u32::try_from(s.model.projects.len()).unwrap_or(u32::MAX),
                last_attached: s.last_attach,
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    pub fn input(&self, cid: ClientId, terminal: TerminalId, bytes: Vec<u8>) {
        let mut st = self.st();
        let Some(sid) = st.clients.get(&cid).and_then(|c| c.session) else {
            tracing::debug!("input from a client without a session ignored");
            return;
        };
        let handle = st
            .session(sid)
            .and_then(|s| s.terminals.get(&terminal))
            .cloned();
        let Some(handle) = handle else {
            tracing::debug!(%terminal, "input for a terminal outside the client's session ignored");
            return;
        };
        let tick = st.touch();
        if let Some(c) = st.clients.get_mut(&cid) {
            c.active = tick;
        }
        self.follow_viewport(&mut st, cid);
        // Typing returns the terminal to the live screen first; focus reports do not.
        let id = handle.id;
        let active = st.session(sid).is_some_and(|s| s.is_active(id));
        if active
            && !matches!(bytes.as_slice(), b"\x1b[I" | b"\x1b[O")
            && let Some(frame) = handle
                .with_emu(|e| e.scroll_to_bottom().then(|| e.full_frame(id)))
                .flatten()
        {
            st.push_frame(sid, &frame);
        }
        handle.write(bytes);
    }

    /// Scrolls a terminal of the client's session and sends its screen in full.
    pub fn scroll(&self, cid: ClientId, terminal: TerminalId, to: ScrollTo) {
        let st = self.st();
        let Some(sid) = st.clients.get(&cid).and_then(|c| c.session) else {
            return;
        };
        let Some(handle) = st.session(sid).and_then(|s| s.terminals.get(&terminal)) else {
            tracing::debug!(%terminal, "scroll of a terminal outside the client's session ignored");
            return;
        };
        let frame = handle.with_emu(|e| {
            e.scroll(to);
            e.full_frame(terminal)
        });
        if let Some(frame) = frame
            && st.session(sid).is_some_and(|s| s.is_active(terminal))
        {
            st.push_frame(sid, &frame);
        }
    }

    pub fn viewport(&self, cid: ClientId, cols: u16, rows: u16) {
        let mut st = self.st();
        let tick = st.touch();
        let Some(client) = st.clients.get_mut(&cid) else {
            return;
        };
        client.viewport = Some((cols, rows));
        client.active = tick;
        self.follow_viewport(&mut st, cid);
    }

    /// A viewport change that came from a config reload: the client's size is recorded, and
    /// the terminals follow it only when that client is the one that was active last.
    pub fn reload_viewport(&self, cid: ClientId, cols: u16, rows: u16) {
        let mut st = self.st();
        let Some(client) = st.clients.get_mut(&cid) else {
            return;
        };
        client.viewport = Some((cols, rows));
        let Some(sid) = client.session else {
            return;
        };
        let newest = st
            .clients_of(sid)
            .max_by_key(|(_, c)| c.active)
            .map(|(id, _)| *id);
        if newest == Some(cid) {
            self.follow_viewport(&mut st, cid);
        }
    }

    /// Sizes the session's terminals for the client that was active last.
    fn follow_viewport(&self, st: &mut State, cid: ClientId) {
        let Some(client) = st.clients.get(&cid) else {
            return;
        };
        let (Some(sid), Some(vp)) = (client.session, client.viewport) else {
            return;
        };
        let Some(sess) = st.session(sid) else {
            return;
        };
        if sess.viewport == Some(vp) {
            return;
        }
        let inner = inner_size(vp);
        for t in sess.active_terminals() {
            if t.with_emu(|e| e.size()) != Some(inner) {
                t.resize_pty(inner.0, inner.1);
            }
        }
        if let Some(sess) = st.session_mut(sid) {
            sess.viewport = Some(vp);
        }
        st.push_view(sid);
        for frame in st.full_frames(sid) {
            st.push_frame(sid, &frame);
        }
    }

    /// After the writer dropped frames: the current view and every screen in full.
    pub fn resync(&self, cid: ClientId) {
        let st = self.st();
        let Some(client) = st.clients.get(&cid) else {
            return;
        };
        let Some(sid) = client.session else {
            return;
        };
        if let Some(sess) = st.session(sid) {
            client.queue.push_view(sess.model.view(sess.viewport));
        }
        for frame in st.full_frames(sid) {
            client.queue.push_frame(frame);
        }
    }

    // Terminal callbacks.

    /// Program output: into the emulator, then whatever the emulator asks for.
    pub fn output(self: &Arc<Self>, h: &Arc<TermHandle>, bytes: &[u8]) {
        let events = h.with_emu(|e| e.feed(bytes)).unwrap_or_default();
        h.dirty.notify_one();
        for event in events {
            self.emu_event(h, event);
        }
    }

    fn emu_event(self: &Arc<Self>, h: &Arc<TermHandle>, event: EmuEvent) {
        match event {
            EmuEvent::Title(title) => self.title(h, title),
            EmuEvent::Reply(text) => h.write(text.into_bytes()),
            EmuEvent::Color {
                index,
                palette,
                format,
            } => {
                let colors = self
                    .st()
                    .active_client(h.session)
                    .and_then(|c| c.colors)
                    .unwrap_or(FALLBACK_COLORS);
                let rgb = if index == NamedColor::Background as usize {
                    colors.bg
                } else if index < 256 {
                    let Rgb { r, g, b } = palette.unwrap_or_else(|| default_palette(index));
                    (r, g, b)
                } else {
                    colors.fg
                };
                let rgb = Rgb {
                    r: rgb.0,
                    g: rgb.1,
                    b: rgb.2,
                };
                h.write(format(rgb).into_bytes());
            }
            EmuEvent::ClipboardStore { selection, text } => {
                let st = self.st();
                if let Some(client) = st.active_client(h.session) {
                    client.queue.push_msg(ServerMsg::Clipboard {
                        selection: selection.into(),
                        text,
                    });
                }
            }
            EmuEvent::ClipboardLoad => {
                tracing::warn!(terminal = %h.id, "denied an OSC 52 clipboard read");
            }
        }
    }

    fn title(self: &Arc<Self>, h: &Arc<TermHandle>, title: Option<String>) {
        let mut st = self.st();
        let now = self.clock.now();
        let sid = h.session;
        let Some(sess) = st.session_mut(sid) else {
            return;
        };
        let Some(model) = terminal_model(&mut sess.model, h.id) else {
            return;
        };
        if model.title == title {
            return;
        }
        model.title = title;
        match sess.title_throttle.poke(now) {
            Poke::Now => st.push_view(sid),
            Poke::Wait(wait) => {
                let shared = Arc::clone(self);
                self.handle.spawn(async move {
                    let mut wait = wait;
                    loop {
                        tokio::time::sleep(wait).await;
                        let mut st = shared.st();
                        let now = shared.clock.now();
                        let fire = st
                            .session_mut(sid)
                            .map_or(Fire::Idle, |s| s.title_throttle.fire(now));
                        match fire {
                            Fire::Due => {
                                st.push_view(sid);
                                return;
                            }
                            // The injected clock decides when the interval has passed.
                            Fire::Wait(d) => wait = d.max(Duration::from_millis(5)),
                            Fire::Idle => return,
                        }
                    }
                });
            }
            Poke::Covered => {}
        }
    }

    /// Turns the emulator's news into a frame for the session's clients.
    pub fn pump(&self, h: &Arc<TermHandle>) {
        let st = self.st();
        let streamed = st.session(h.session).is_some_and(|s| s.is_active(h.id));
        if !streamed || st.clients_of(h.session).next().is_none() {
            h.with_emu(|e| e.reset_damage());
            return;
        }
        if let Some(frame) = h.with_emu(|e| e.damage_frame(h.id)) {
            st.push_frame(h.session, &frame);
        }
    }

    /// A terminal's process exited and its output was read.
    pub fn terminal_gone(&self, h: &Arc<TermHandle>, status: u32) {
        let mut st = self.st();
        if st.shutting_down {
            return;
        }
        h.close();
        let sid = h.session;
        // Closed by an action, or its session was killed: already out of the model.
        if !st
            .session(sid)
            .is_some_and(|s| s.terminals.contains_key(&h.id))
        {
            h.release();
            return;
        }
        // The last output reaches the clients before the terminal goes.
        let streamed = st.session(sid).is_some_and(|s| s.is_active(h.id));
        if streamed && let Some(frame) = h.with_emu(|e| e.damage_frame(h.id)) {
            st.push_frame(sid, &frame);
        }
        // Whatever a background job still prints from here on is discarded.
        h.release();

        let now = self.clock.now();
        let quick = status != 0 && now.saturating_sub(h.started) < QUICK_EXIT;
        if quick {
            tracing::error!(
                terminal = %h.id,
                program = %h.program,
                status,
                "terminal exited at once"
            );
        }
        let Some(sess) = st.session_mut(sid) else {
            return;
        };
        let before = sess.model.active_panel();
        sess.terminals.remove(&h.id);
        let panel = sess
            .model
            .projects
            .iter_mut()
            .flat_map(|p| p.panels.iter_mut())
            .find(|p| p.terminals.iter().any(|t| t.id == h.id));
        let mut emptied = None;
        if let Some(panel) = panel {
            panel.terminals.retain(|t| t.id != h.id);
            if panel.focused_terminal == Some(h.id) {
                panel.focused_terminal = panel.terminals.first().map(|t| t.id);
            }
            if panel.terminals.is_empty() {
                emptied = Some(panel.id);
            }
        }
        if let Some(panel) = emptied {
            sess.model.remove_panel(panel);
        }

        // A session whose first process fails at once ends with that error, rather than
        // staying empty behind a broken shell.
        if quick && sess.startup == Some(h.id) && sess.terminals.is_empty() {
            let reason = ByeReason::Error(format!(
                "terminal exited at once (status {status}); see {}",
                self.paths.log_file.display()
            ));
            self.end_session(&mut st, sid, reason);
            let none_left = st.sessions.is_empty() && st.creating == 0;
            drop(st);
            if none_left {
                self.request_shutdown();
            }
            return;
        }
        self.settle(&mut st, sid, before);
    }

    /// After a change to a session: its terminals match the viewport, every client has the
    /// new view, and a new active panel is shown in full.
    fn settle(&self, st: &mut State, sid: SessionId, before: Option<PanelId>) {
        let Some(sess) = st.session(sid) else {
            return;
        };
        let inner = inner_size(sess.viewport.unwrap_or(DEFAULT_VIEWPORT));
        let moved = sess.model.active_panel() != before;
        if moved {
            for t in sess.active_terminals() {
                if t.with_emu(|e| e.size()) != Some(inner) {
                    t.resize_pty(inner.0, inner.1);
                }
            }
        }
        st.push_view(sid);
        if moved {
            for frame in st.full_frames(sid) {
                st.push_frame(sid, &frame);
            }
        }
    }

    /// Sends `Bye` to the session's clients and removes the session. Its terminals are not
    /// touched.
    fn end_session(&self, st: &mut State, sid: SessionId, reason: ByeReason) {
        for client in st.clients.values_mut().filter(|c| c.session == Some(sid)) {
            client.queue.push_msg(ServerMsg::Bye {
                reason: reason.clone(),
            });
            client.session = None;
        }
        st.sessions.retain(|s| s.model.id != sid);
    }

    // Actions.

    /// Runs an action of an attached client on its session.
    pub async fn action(
        self: &Arc<Self>,
        cid: ClientId,
        action: Action,
    ) -> Result<(), ActionError> {
        let sid = self
            .st()
            .clients
            .get(&cid)
            .and_then(|c| c.session)
            .ok_or(ActionError::NotAttached)?;
        match action {
            Action::NewProject { name, root } => {
                let name = validate_name(&name, NameKind::Project)?;
                check_dir(&root)?;
                self.add_panel(sid, PanelTarget::NewProject { name, root })
                    .await
            }
            Action::NewPanel { project, name } => {
                let name = name
                    .map(|n| validate_name(&n, NameKind::Panel))
                    .transpose()?;
                self.add_panel(sid, PanelTarget::Project { project, name })
                    .await
            }
            Action::KillSession => {
                let queue = self.st().clients.get(&cid).map(|c| Arc::clone(&c.queue));
                let answer = || {
                    if let Some(q) = queue {
                        q.push_msg(ServerMsg::ActionResult(Ok(())));
                    }
                };
                if self.kill_session(|s| s.model.id == sid, answer)? {
                    self.request_shutdown();
                }
                Ok(())
            }
            other => self.edit(sid, other),
        }
    }

    /// The actions that need no new process.
    fn edit(self: &Arc<Self>, sid: SessionId, action: Action) -> Result<(), ActionError> {
        let mut st = self.st();
        let state = &mut *st;
        let name_taken = |state: &State, name: &str, except: SessionId| {
            state
                .sessions
                .iter()
                .any(|s| s.model.id != except && s.model.name == name)
        };
        let renamed = match &action {
            Action::RenameSession { name } => {
                let name = validate_name(name, NameKind::Session)?;
                if name_taken(state, &name, sid) {
                    return Err(ActionError::SessionExists(name));
                }
                Some(name)
            }
            _ => None,
        };
        let sess = state.session_mut(sid).ok_or(ActionError::NotAttached)?;
        let before = sess.model.active_panel();
        let mut doomed: Vec<Arc<TermHandle>> = Vec::new();
        match action {
            Action::RenameSession { .. } => {
                sess.model.name = renamed.unwrap_or_default();
            }
            Action::RenameProject { project, name } => {
                let name = validate_name(&name, NameKind::Project)?;
                if sess
                    .model
                    .projects
                    .iter()
                    .any(|p| p.id != project && p.name == name)
                {
                    return Err(ActionError::ProjectExists(name));
                }
                let p = sess
                    .model
                    .projects
                    .iter_mut()
                    .find(|p| p.id == project)
                    .ok_or(ActionError::NoSuchProject(project))?;
                p.name = name;
            }
            Action::SetProjectCollapsed { project, collapsed } => {
                sess.model
                    .projects
                    .iter_mut()
                    .find(|p| p.id == project)
                    .ok_or(ActionError::NoSuchProject(project))?
                    .collapsed = collapsed;
            }
            Action::FocusProject { project } => {
                if !sess.model.focus_project(project) {
                    return Err(ActionError::NoSuchProject(project));
                }
            }
            Action::CloseProject { project } => {
                let removed = sess
                    .model
                    .remove_project(project)
                    .ok_or(ActionError::NoSuchProject(project))?;
                for t in removed.panels.iter().flat_map(|p| &p.terminals) {
                    doomed.extend(sess.terminals.remove(&t.id));
                }
            }
            Action::RenamePanel { panel, name } => {
                let name = validate_name(&name, NameKind::Panel)?;
                let p = sess
                    .model
                    .projects
                    .iter_mut()
                    .flat_map(|p| p.panels.iter_mut())
                    .find(|p| p.id == panel)
                    .ok_or(ActionError::NoSuchPanel(panel))?;
                p.name = name;
                p.name_source = NameSource::Manual;
            }
            Action::ClosePanel { panel } => {
                let removed = sess
                    .model
                    .remove_panel(panel)
                    .ok_or(ActionError::NoSuchPanel(panel))?;
                for t in &removed.terminals {
                    doomed.extend(sess.terminals.remove(&t.id));
                }
            }
            Action::FocusPanel { panel } => {
                if !sess.model.focus_panel(panel) {
                    return Err(ActionError::NoSuchPanel(panel));
                }
            }
            Action::CyclePanel { delta } => {
                sess.model.cycle_panel(delta);
            }
            Action::NewProject { .. } | Action::NewPanel { .. } | Action::KillSession => {
                unreachable!("handled by `action`")
            }
        }
        self.settle(&mut st, sid, before);
        drop(st);
        self.hang_up(doomed);
        Ok(())
    }

    /// Adds a panel with one new shell, in an existing project or a new one.
    async fn add_panel(
        self: &Arc<Self>,
        sid: SessionId,
        target: PanelTarget,
    ) -> Result<(), ActionError> {
        let (project_id, panel_id, tid, req) = {
            let mut st = self.st();
            if st.shutting_down {
                return Err(ActionError::ShuttingDown);
            }
            let sess = st.session(sid).ok_or(ActionError::NotAttached)?;
            let (existing, root) = match &target {
                PanelTarget::Project { project, .. } => {
                    let p = sess
                        .model
                        .projects
                        .iter()
                        .find(|p| p.id == *project)
                        .ok_or(ActionError::NoSuchProject(*project))?;
                    (Some(p.id), p.root.clone())
                }
                PanelTarget::NewProject { name, root } => {
                    if sess.model.projects.iter().any(|p| p.name == *name) {
                        return Err(ActionError::ProjectExists(name.clone()));
                    }
                    (None, root.clone())
                }
            };
            let env = sess.env.clone();
            let viewport = sess.viewport.unwrap_or(DEFAULT_VIEWPORT);
            let project_id = existing.unwrap_or_else(|| ProjectId(st.alloc()));
            let panel_id = PanelId(st.alloc());
            let tid = TerminalId(st.alloc());
            let req = self.spawn_request(&env, &root, viewport, sid, panel_id, tid);
            (project_id, panel_id, tid, req)
        };

        let open_req = req.clone();
        let parts = tokio::task::spawn_blocking(move || terminal::open(&open_req))
            .await
            .map_err(|e| ActionError::Spawn(format!("could not start {}: {e}", req.program)))?
            .map_err(ActionError::Spawn)?;

        let mut st = self.st();
        // The world may have changed while the process started.
        let checked = if st.shutting_down {
            Err(ActionError::ShuttingDown)
        } else if let Some(sess) = st.session(sid) {
            match &target {
                PanelTarget::Project { project, .. } => sess
                    .model
                    .projects
                    .iter()
                    .any(|p| p.id == *project)
                    .then_some(())
                    .ok_or(ActionError::NoSuchProject(*project)),
                PanelTarget::NewProject { name, .. } => {
                    (!sess.model.projects.iter().any(|p| p.name == *name))
                        .then_some(())
                        .ok_or_else(|| ActionError::ProjectExists(name.clone()))
                }
            }
        } else {
            Err(ActionError::NotAttached)
        };
        if let Err(e) = checked {
            parts.discard();
            return Err(e);
        }
        let handle = terminal::start(self, tid, sid, &req, parts);
        let Some(sess) = st.session_mut(sid) else {
            return Err(ActionError::NotAttached);
        };
        sess.terminals.insert(tid, handle);
        let before = sess.model.active_panel();
        let root = req.cwd.clone();
        let program = program_name(&req.program);
        let (panel_name, name_source) = match &target {
            PanelTarget::Project {
                name: Some(name), ..
            } => (Some(name.clone()), NameSource::Manual),
            _ => (None, NameSource::Default),
        };
        if let PanelTarget::NewProject { name, .. } = target {
            sess.model.projects.push(Project {
                id: project_id,
                name,
                root: root.clone(),
                panels: Vec::new(),
                active_panel: None,
                collapsed: false,
            });
        }
        let Some(project) = sess.model.projects.iter_mut().find(|p| p.id == project_id) else {
            return Err(ActionError::NoSuchProject(project_id));
        };
        let name = panel_name.unwrap_or_else(|| project.next_panel_name());
        project.panels.push(Panel {
            id: panel_id,
            name,
            name_source,
            focused_terminal: Some(tid),
            terminals: vec![Terminal {
                id: tid,
                launch: LaunchSpec {
                    program: None,
                    args: Vec::new(),
                    cwd: root,
                    env: Vec::<(OsString, OsString)>::new(),
                },
                program,
                title: None,
                cwd: None,
                exit_status: None,
            }],
        });
        sess.model.focus_panel(panel_id);
        self.settle(&mut st, sid, before);
        Ok(())
    }

    /// Hangs up the processes of the session `which` picks and ends it. `answer` runs once
    /// the session is found and before its clients get `Bye`, so a requester attached to it
    /// still receives its answer. True when no session is left: the caller then shuts the
    /// daemon down.
    fn kill_session(
        self: &Arc<Self>,
        which: impl Fn(&SessionRt) -> bool,
        answer: impl FnOnce(),
    ) -> Result<bool, ActionError> {
        let mut st = self.st();
        let Some(sess) = st.sessions.iter().find(|s| which(s)) else {
            return Err(ActionError::NotAttached);
        };
        let sid = sess.model.id;
        let handles: Vec<Arc<TermHandle>> = sess.terminals.values().cloned().collect();
        answer();
        self.end_session(&mut st, sid, ByeReason::SessionEnded);
        let none_left = st.sessions.is_empty() && st.creating == 0;
        drop(st);
        self.hang_up(handles);
        Ok(none_left)
    }

    /// Hangs up terminals that left the model: `SIGHUP`, `SIGKILL` after two seconds.
    fn hang_up(self: &Arc<Self>, handles: Vec<Arc<TermHandle>>) {
        if handles.is_empty() {
            return;
        }
        self.st().hanging.extend(handles.iter().cloned());
        let shared = Arc::clone(self);
        self.handle.spawn(async move {
            hang_up_and_wait(handles.clone()).await;
            shared
                .st()
                .hanging
                .retain(|h| !handles.iter().any(|d| d.id == h.id));
        });
    }

    // Session requests.

    /// Creates a session with a project for `cwd` and its first panel.
    pub async fn new_session(
        self: &Arc<Self>,
        name: Option<String>,
        cwd: PathBuf,
        env: Env,
    ) -> Result<(SessionId, String), ActionError> {
        let name = name
            .map(|n| validate_name(&n, NameKind::Session))
            .transpose()?;
        check_dir(&cwd)?;
        let a = AttachRequest {
            session: name.clone(),
            create: true,
            cols: DEFAULT_VIEWPORT.0,
            rows: DEFAULT_VIEWPORT.1,
            cwd,
            env,
        };
        let _attaching = self.attaching.lock().await;
        let new = {
            let mut st = self.st();
            if st.shutting_down {
                return Err(ActionError::ShuttingDown);
            }
            let name = match name {
                Some(n) if st.sessions.iter().any(|s| s.model.name == n) => {
                    return Err(ActionError::SessionExists(n));
                }
                Some(n) => n,
                None => st.unique_name("main"),
            };
            st.creating += 1;
            self.new_session_plan(&mut st, name, &a, DEFAULT_VIEWPORT)
        };
        let req = new.req.clone();
        let opened = tokio::task::spawn_blocking(move || terminal::open(&req))
            .await
            .map_err(|e| ActionError::Spawn(format!("could not start {}: {e}", new.req.program)))
            .and_then(|opened| opened.map_err(ActionError::Spawn));
        let mut st = self.st();
        st.creating -= 1;
        let parts = opened?;
        if st.shutting_down {
            parts.discard();
            return Err(ActionError::ShuttingDown);
        }
        let name = new.name.clone();
        let sid = self.add_session(&mut st, new, parts, &a, DEFAULT_VIEWPORT);
        Ok((sid, name))
    }

    /// Ends the named session; true when it was the last one.
    pub fn kill_session_named(
        self: &Arc<Self>,
        name: &str,
        answer: impl FnOnce(),
    ) -> Result<bool, ActionError> {
        self.kill_session(|s| s.model.name == name, answer)
            .map_err(|_| ActionError::NoSuchSession(name.to_string()))
    }

    pub fn rename_session_named(self: &Arc<Self>, from: &str, to: &str) -> Result<(), ActionError> {
        let to = validate_name(to, NameKind::Session)?;
        let st = self.st();
        let sid = st
            .sessions
            .iter()
            .find(|s| s.model.name == from)
            .map(|s| s.model.id)
            .ok_or_else(|| ActionError::NoSuchSession(from.to_string()))?;
        drop(st);
        self.edit(sid, Action::RenameSession { name: to })
    }

    // Shutdown.

    /// Tells every client and wakes `Daemon::run`. Idempotent.
    pub fn request_shutdown(&self) {
        let mut st = self.st();
        if st.shutting_down {
            return;
        }
        st.shutting_down = true;
        for client in st.clients.values() {
            client.queue.push_msg(ServerMsg::Bye {
                reason: ByeReason::ServerShutdown,
            });
        }
        drop(st);
        let _ = self.shutdown_tx.send(true);
    }

    /// Lets the writers deliver their `Bye`, then hangs up every terminal (`SIGHUP`, then
    /// `SIGKILL` after two seconds).
    pub async fn finish_shutdown(&self) {
        let deadline = tokio::time::Instant::now() + WRITER_FLUSH;
        while self.writers.load(Ordering::Acquire) > 0 {
            if tokio::time::timeout_at(deadline, self.writers_done.notified())
                .await
                .is_err()
            {
                break;
            }
        }

        let handles: Vec<Arc<TermHandle>> = {
            let st = self.st();
            st.sessions
                .iter()
                .flat_map(|s| s.terminals.values().cloned())
                .chain(st.hanging.iter().cloned())
                .collect()
        };
        hang_up_and_wait(handles).await;
    }
}

/// `SIGHUP` to every terminal, then `SIGKILL` for those still running after two seconds.
async fn hang_up_and_wait(handles: Vec<Arc<TermHandle>>) {
    for h in &handles {
        h.close();
        h.hang_up();
    }
    let deadline = tokio::time::Instant::now() + HANG_UP_GRACE;
    while handles.iter().any(|h| !h.has_exited()) && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let stubborn: Vec<_> = handles.into_iter().filter(|h| !h.has_exited()).collect();
    if !stubborn.is_empty() {
        let _ = tokio::task::spawn_blocking(move || stubborn.iter().for_each(|h| h.kill())).await;
    }
}
