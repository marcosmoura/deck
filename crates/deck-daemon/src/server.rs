//! The daemon's shared state: sessions, clients and the rules that tie them together.
//!
//! Lock order: `state`, then a terminal's `emu`. Nothing waits while holding either.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use deck_core::action::{Action, ActionError, NameKind, default_project_name, validate_name};
use deck_core::config::Config;
use deck_core::layout::Ratios;
use deck_core::layout::{self, Rect, panel_area};
use deck_core::model::{
    LaunchSpec, NameSource, Panel, PanelId, Project, ProjectId, Session, SessionId, SessionView,
    Terminal, TerminalId,
};
use deck_core::naming;
use deck_core::protocol::{
    ByeReason, CellPx, Frame, NoticeLevel, OuterColors, ScrollTo, ServerMsg, SessionSummary,
};
use tokio::runtime::Handle;
use tokio::sync::{Notify, watch};

use crate::clock::{Clock, Fire, Poke, Throttle};
use crate::config_file::Notice;
use crate::emu::{Emu, EmuEvent, clamp_size};
use crate::env::{self, Env, TerminalIds};
use crate::images::{self, Fetcher, Shot, Tracker};
use crate::lock;
use crate::queue::Queue;
use crate::startup::DaemonPaths;
use crate::terminal::{self, SpawnRequest, TermHandle};
use crate::{CwdQuery, ForegroundQuery};

mod agents;
mod control;
mod cwd;
mod handoff;
mod launch;
mod listing;
mod restore;
pub(crate) mod stats;
mod terminal_io;
mod waiting;

pub type ClientId = u64;

/// Title-only `SessionView`s are sent at most this often per session.
const TITLE_VIEW_INTERVAL: Duration = Duration::from_millis(100);
/// A terminal that fails within this long of starting is reported as a start failure.
const QUICK_EXIT: Duration = Duration::from_secs(1);
pub(crate) const FALLBACK_COLORS: OuterColors = OuterColors {
    fg: (192, 192, 192),
    bg: (0, 0, 0),
};
const HANG_UP_GRACE: Duration = Duration::from_secs(2);
const WRITER_FLUSH: Duration = Duration::from_secs(1);
/// What is answered to a request that arrives during a live upgrade.
pub(crate) const UPGRADING: &str = "the daemon is upgrading";
/// The panel area of a session no client has sized yet.
const DEFAULT_VIEWPORT: (u16, u16) = (80, 24);

/// Notices kept for the next client while none is attached.
const MAX_PENDING_NOTICES: usize = 20;

/// Where a terminal lives, for its environment.
#[derive(Debug, Clone, Copy)]
struct Place {
    session: SessionId,
    panel: PanelId,
    terminal: TerminalId,
}

/// The ids a `New*` request created.
#[derive(Debug, Clone, Copy)]
struct Made {
    project: ProjectId,
    panel: PanelId,
}

/// The directory a launch spec names, if it names one (an empty one means the default).
fn launch_dir(launch: Option<&LaunchSpec>) -> Option<PathBuf> {
    launch
        .map(|l| &l.cwd)
        .filter(|c| !c.as_os_str().is_empty())
        .cloned()
}

/// A new terminal's model: the launch spec it was asked with, and the directory it started in.
fn new_terminal(
    id: TerminalId,
    launch: Option<&LaunchSpec>,
    req: &SpawnRequest,
    keep: bool,
) -> Terminal {
    let program = launch.and_then(|l| l.program.clone());
    Terminal {
        id,
        launch: LaunchSpec {
            args: if program.is_some() {
                launch.map(|l| l.args.clone()).unwrap_or_default()
            } else {
                Vec::new()
            },
            program,
            cwd: req.cwd.clone(),
            env: launch.map(|l| l.env.clone()).unwrap_or_default(),
        },
        program: program_name(&req.program),
        title: None,
        cwd: None,
        exit_status: None,
        keep,
        agent: None,
    }
}

/// What an attaching client asks for.
#[derive(Debug, Clone)]
pub struct AttachRequest {
    pub session: Option<String>,
    pub create: bool,
    /// The size of the client's outer window.
    pub cols: u16,
    pub rows: u16,
    /// The client's cell size in pixels, when known.
    pub cell: Option<CellPx>,
    /// The client can show images.
    pub images: bool,
    /// The outer terminal's name, which programs' XTVERSION queries are answered with.
    pub outer_name: Option<String>,
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
    /// The cell size in pixels the terminals are sized with, of the client they follow.
    cell: Option<CellPx>,
    title_throttle: Throttle,
    /// When a client last attached; the highest tick is the most recently attached.
    last_attach: u64,
    /// The terminal the session was created with: if it fails at once, the session ends
    /// with that error instead of staying empty.
    startup: Option<TerminalId>,
    /// Panels whose agent needed the user or finished while nobody looked at them.
    unseen: HashSet<PanelId>,
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

    /// The active panel's model.
    fn active_panel_model(&self) -> Option<&Panel> {
        let active = self.model.active_panel()?;
        self.model
            .projects
            .iter()
            .flat_map(|p| &p.panels)
            .find(|p| p.id == active)
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
    cell: Option<CellPx>,
    colors: Option<OuterColors>,
    /// The outer terminal's name, for XTVERSION answers.
    outer_name: Option<String>,
    /// The images the client was sent.
    images: Tracker,
    /// Higher is more recent.
    active: u64,
    /// The outer terminal has focus; a client that never reported counts as focused.
    focused: bool,
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
    /// A live upgrade is under way: what changes the model or opens a process is refused.
    upgrading: bool,
    /// The handoff has taken its list of terminals: a process that was started meanwhile
    /// waits to be added until the handoff is over.
    frozen: bool,
    /// Terminals that left the model and are being hung up; shutdown finishes them.
    hanging: Vec<Arc<TermHandle>>,
    /// Notices no attached client has seen, oldest first.
    notices: VecDeque<(NoticeLevel, String)>,
    /// A save is scheduled and has not taken its snapshot yet.
    save_pending: bool,
    /// Terminals started by a restore: one that fails at once is reported.
    restored: HashSet<TerminalId>,
    /// The exit codes of terminals that ended, for `wait --exit` calls that come late.
    exits: waiting::Exits,
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
        let mut i = 2u64;
        loop {
            let name = format!("{base}-{i}");
            if !taken(&name) {
                return name;
            }
            i += 1;
        }
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

    /// The session as clients see it: the model, plus what only the daemon knows (the
    /// processes' ids and the attached clients).
    fn view_of(&self, sess: &SessionRt) -> SessionView {
        let mut view = sess
            .model
            .view(Some(sess.viewport.unwrap_or(DEFAULT_VIEWPORT)));
        view.attached_clients =
            u32::try_from(self.clients_of(sess.model.id).count()).unwrap_or(u32::MAX);
        for panel in view.projects.iter_mut().flat_map(|p| p.panels.iter_mut()) {
            panel.unseen = sess.unseen.contains(&panel.id);
        }
        for t in view
            .projects
            .iter_mut()
            .flat_map(|p| p.panels.iter_mut())
            .flat_map(|p| p.terminals.iter_mut())
        {
            // Until the shell reports a directory, the one it was started in.
            if t.cwd.is_none() {
                t.cwd = sess
                    .model
                    .projects
                    .iter()
                    .flat_map(|p| &p.panels)
                    .flat_map(|p| &p.terminals)
                    .find(|m| m.id == t.id)
                    .map(|m| m.launch.cwd.clone())
                    .filter(|c| !c.as_os_str().is_empty());
            }
            let handle = sess.terminals.get(&t.id);
            t.pid = handle
                .and_then(|h| h.pid)
                .filter(|_| t.exit_status.is_none());
            // An agent's own status wins over a command's.
            t.busy = t.agent.is_none()
                && t.exit_status.is_none()
                && handle.is_some_and(|h| lock(&h.agent_rt).busy);
        }
        view
    }

    /// Whether a client of the session has its outer terminal focused.
    fn session_focused(&self, sid: SessionId) -> bool {
        self.clients_of(sid).any(|(_, c)| c.focused)
    }

    /// Drops the unseen marks of panels that are gone, and the mark of the active panel
    /// while a focused client shows it.
    fn sync_unseen(&mut self, sid: SessionId) {
        let focused = self.session_focused(sid);
        let Some(sess) = self.session_mut(sid) else {
            return;
        };
        if sess.unseen.is_empty() {
            return;
        }
        let panels: HashSet<PanelId> = sess
            .model
            .projects
            .iter()
            .flat_map(|p| p.panels.iter().map(|panel| panel.id))
            .collect();
        sess.unseen.retain(|p| panels.contains(p));
        if focused && let Some(active) = sess.model.active_panel() {
            sess.unseen.remove(&active);
        }
    }

    fn push_view(&mut self, sid: SessionId) {
        self.sync_unseen(sid);
        let Some(sess) = self.session(sid) else {
            return;
        };
        let view = self.view_of(sess);
        for (_, c) in self.clients_of(sid) {
            c.queue.push_view(view.clone());
        }
    }

    /// The session's terminals by id, for reading their emulators later.
    fn handles(&self, sid: SessionId) -> HashMap<TerminalId, Arc<TermHandle>> {
        self.session(sid)
            .map(|s| s.terminals.clone())
            .unwrap_or_default()
    }

    /// The ids of the images of a terminal that some client of the session was sent.
    fn sent_ids(&self, sid: SessionId, terminal: TerminalId) -> Vec<u32> {
        let mut ids: Vec<u32> = self
            .clients_of(sid)
            .flat_map(|(_, c)| c.images.ids_of(terminal))
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// What `frame` makes of the emulator, read together with the storage state the clients'
    /// image tracking needs. `None` when `frame` gives none or the terminal was released.
    fn shot(
        &self,
        sid: SessionId,
        handle: &TermHandle,
        frame: impl FnOnce(&mut Emu) -> Option<Frame> + Send + 'static,
    ) -> Option<Shot> {
        let ids = self.sent_ids(sid, handle.id);
        handle
            .with_emu(move |e| frame(e).map(|f| Shot::new(e, f, &ids)))
            .flatten()
    }

    /// The active panel's terminals in full, in terminal order; with `only`, just those.
    fn full_shots(&self, sid: SessionId, only: Option<&[TerminalId]>) -> Vec<Shot> {
        let Some(sess) = self.session(sid) else {
            return Vec::new();
        };
        let mut shots: Vec<Shot> = sess
            .active_terminals()
            .iter()
            .filter(|t| only.is_none_or(|o| o.contains(&t.id)))
            .filter_map(|t| {
                let id = t.id;
                self.shot(sid, t, move |e| Some(e.full_frame(id)))
            })
            .collect();
        shots.sort_by_key(|s| s.frame.terminal);
        shots
    }

    /// Queues a frame for the clients of a session (just `only`, when given), with the image
    /// pieces and `ImageGone` messages that belong to it. Returns the clients that miss a
    /// placement because `fetch` has not copied its pixels yet.
    fn push_shot(
        &mut self,
        sid: SessionId,
        only: Option<ClientId>,
        shot: &Shot,
        fetch: &mut Fetcher,
    ) -> Vec<ClientId> {
        let mut pending = Vec::new();
        for (cid, c) in &mut self.clients {
            if c.session != Some(sid) || only.is_some_and(|o| o != *cid) {
                continue;
            }
            if images::deliver(&c.queue, &mut c.images, shot, fetch) {
                pending.push(*cid);
            }
        }
        pending
    }

    /// [`State::push_shot`] to every client, copying image pixels now (under the state lock).
    fn push_live(&mut self, sid: SessionId, shots: &[Shot]) {
        let mut fetch = Fetcher::live(self.handles(sid));
        for shot in shots {
            self.push_shot(sid, None, shot, &mut fetch);
        }
    }

    /// The terminal ended: its clients drop its images without being told.
    fn forget_images(&mut self, sid: SessionId, terminal: TerminalId) {
        for (_, c) in self
            .clients
            .iter_mut()
            .filter(|(_, c)| c.session == Some(sid))
        {
            c.images.forget_terminal(terminal);
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

fn panel_mut(session: &mut Session, id: PanelId) -> Option<&mut Panel> {
    session
        .projects
        .iter_mut()
        .flat_map(|p| p.panels.iter_mut())
        .find(|p| p.id == id)
}

fn panel_of_terminal(session: &mut Session, id: TerminalId) -> Option<&mut Panel> {
    session
        .projects
        .iter_mut()
        .flat_map(|p| p.panels.iter_mut())
        .find(|p| p.terminals.iter().any(|t| t.id == id))
}

/// The size of a panel's only terminal: the whole panel area (no border).
fn single_size(viewport: (u16, u16)) -> (u16, u16) {
    clamp_size(viewport.0, viewport.1)
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

/// The configured shell, else the session's `SHELL`, else `/bin/sh`.
fn shell_program(config: &Config, session_env: &Env) -> String {
    config
        .shell
        .clone()
        .or_else(|| {
            env::lookup(session_env, "SHELL")
                .and_then(|v| v.to_str())
                .map(str::to_string)
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/bin/sh".to_string())
}

fn program_name(program: &str) -> String {
    Path::new(program)
        .file_name()
        .map_or_else(|| program.to_string(), |n| n.to_string_lossy().into_owned())
}

pub struct Shared {
    pub paths: DaemonPaths,
    config: Mutex<Arc<Config>>,
    pub clock: Arc<dyn Clock>,
    pub version: u32,
    /// When the daemon started, for the uptime in `Request::Stats`.
    started: std::time::Instant,
    pub handle: Handle,
    state: Mutex<State>,
    /// The colours of the client each session follows, for the reader threads (which must
    /// not take the state lock for every chunk of output); see `publish_colors`.
    session_colors: Mutex<HashMap<SessionId, OuterColors>>,
    /// The name XTVERSION is answered with, per session: the followed client's, when it can
    /// show images and gave one; see `publish_colors`.
    session_names: Mutex<HashMap<SessionId, String>>,
    shutdown_tx: watch::Sender<bool>,
    /// Counts up whenever something a pending `wait` may care about changed.
    changes: watch::Sender<u64>,
    /// How many `wait` requests are pending.
    waiters: Arc<AtomicUsize>,
    writers: AtomicUsize,
    writers_done: Notify,
    /// Processes about to be started or being started: a handoff waits for them. Counted
    /// under the state lock that checks `upgrading`, so a spawn is either refused or seen.
    opening: AtomicUsize,
    attaching: tokio::sync::Mutex<()>,
    /// Serializes the writes of `state.json`; taken before `state`.
    save_lock: Mutex<()>,
    /// The machine's host name, for OSC 7.
    pub(crate) hostname: String,
    cwd_query: CwdQuery,
    foreground_query: ForegroundQuery,
    /// What the session listings read, for tests.
    listing: Arc<crate::ListingStats>,
    /// Free places for OpenCode listing children, daemon-wide.
    opencode_slots: Arc<tokio::sync::Semaphore>,
}

/// What the daemon asks the operating system about its terminals' processes.
pub struct Probes {
    pub cwd: CwdQuery,
    pub foreground: ForegroundQuery,
    pub listing: Arc<crate::ListingStats>,
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

/// One process being started; see `Shared::open_slot`.
pub(crate) struct OpenSlot(Arc<Shared>);

impl Drop for OpenSlot {
    fn drop(&mut self) {
        self.0.opening.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Shared {
    /// Counts a process about to be started. Call it while holding the state lock that just
    /// saw `upgrading` unset; drop it once the process is open.
    pub(crate) fn open_slot(self: &Arc<Self>) -> OpenSlot {
        self.opening.fetch_add(1, Ordering::AcqRel);
        OpenSlot(Arc::clone(self))
    }

    pub fn new(
        paths: DaemonPaths,
        config: Config,
        notices: Vec<Notice>,
        clock: Arc<dyn Clock>,
        version: u32,
        probes: Probes,
        waiters: Arc<AtomicUsize>,
    ) -> (Arc<Self>, watch::Receiver<bool>) {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let shared = Arc::new(Self {
            paths,
            config: Mutex::new(Arc::new(config)),
            clock,
            version,
            started: std::time::Instant::now(),
            handle: Handle::current(),
            state: Mutex::new(State::default()),
            session_colors: Mutex::new(HashMap::new()),
            session_names: Mutex::new(HashMap::new()),
            shutdown_tx,
            changes: watch::channel(0).0,
            waiters,
            writers: AtomicUsize::new(0),
            writers_done: Notify::new(),
            opening: AtomicUsize::new(0),
            attaching: tokio::sync::Mutex::new(()),
            save_lock: Mutex::new(()),
            hostname: crate::platform::hostname(),
            cwd_query: probes.cwd,
            foreground_query: probes.foreground,
            listing: probes.listing,
            opencode_slots: Arc::new(tokio::sync::Semaphore::new(listing::OPENCODE_PARALLEL)),
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
        self.show(level, text);
    }

    /// [`Shared::notify`] without the log line: for text that holds a name the user chose
    /// (or a title or prompt that became a panel name), which the caller logs by id.
    pub fn show(&self, level: NoticeLevel, text: String) {
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

    /// Shows a message to one client only, without logging it or keeping it for a later
    /// client: for text that holds a directory.
    pub fn notify_client(&self, cid: ClientId, level: NoticeLevel, text: String) {
        if let Some(client) = self.st().clients.get(&cid) {
            client.queue.push_msg(ServerMsg::Notice { level, text });
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
                cell: None,
                colors,
                outer_name: None,
                images: Tracker::default(),
                active,
                focused: true,
            },
        );
        Some((id, queue))
    }

    /// Records the colours of the client `sid` follows (the most recently active), for the
    /// emulators' colour answers.
    fn publish_colors(&self, st: &State, sid: SessionId) {
        let active = st.active_client(sid);
        let colors = active.and_then(|c| c.colors);
        let name = active
            .filter(|c| c.images.enabled)
            .and_then(|c| c.outer_name.clone());
        let mut map = lock(&self.session_colors);
        match colors {
            Some(colors) => map.insert(sid, colors),
            None => map.remove(&sid),
        };
        let mut names = lock(&self.session_names);
        match name {
            Some(name) => names.insert(sid, name),
            None => names.remove(&sid),
        };
    }

    /// Removes a client; its session and processes keep running.
    pub fn remove_client(&self, id: ClientId) {
        let mut st = self.st();
        if let Some(client) = st.clients.remove(&id) {
            client.queue.close();
            if let Some(sid) = client.session {
                self.publish_colors(&st, sid);
            }
        }
    }

    pub async fn attach(self: &Arc<Self>, cid: ClientId, a: &AttachRequest) -> Result<(), String> {
        // One attach at a time, so a session name that is free while a PTY opens (outside
        // the state lock) is still free when the session is added.
        let _attaching = self.attaching.lock().await;
        let config = self.config();
        let viewport = panel_area(
            (a.cols, a.rows),
            if config.sidebar_collapsed {
                layout::COLLAPSED_SIDEBAR_WIDTH
            } else {
                layout::clamp_sidebar_width(a.cols, config.sidebar_width)
            },
            config.sidebar,
        );
        let plan = {
            let mut st = self.st();
            if st.shutting_down {
                return Err("the daemon is shutting down".into());
            }
            if st.upgrading {
                return Err(UPGRADING.into());
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
                let mut st = self.st_after_open().await;
                st.creating -= 1;
                let parts = opened?;
                if st.shutting_down {
                    parts.discard();
                    return Err("the daemon is shutting down".into());
                }
                let sid = self.add_session(&mut st, new, parts, a, viewport)?;
                (sid, st)
            }
        };

        let active = st.touch();
        if let Some(sess) = st.session_mut(sid) {
            env::refresh(&mut sess.env, &a.env);
            sess.last_attach = active;
        }
        self.mark_dirty(&mut st);
        let Some(client) = st.clients.get_mut(&cid) else {
            return Err("connection closed".into());
        };
        // The config as it is now (a reload may have come while the session was created):
        // the viewport and the `Welcome` both come from this one read.
        let config = self.config();
        client.session = Some(sid);
        client.cell = a.cell;
        client.images.enabled = a.images;
        client.outer_name = a.outer_name.clone();
        client.viewport = Some(panel_area(
            (a.cols, a.rows),
            if config.sidebar_collapsed {
                layout::COLLAPSED_SIDEBAR_WIDTH
            } else {
                config.sidebar_width
            },
            config.sidebar,
        ));
        client.active = active;
        let queue = Arc::clone(&client.queue);
        self.publish_colors(&st, sid);

        queue.push_msg(ServerMsg::Welcome {
            session: sid,
            config: Box::new(Config::clone(&config)),
        });
        for (level, text) in std::mem::take(&mut st.notices) {
            queue.push_msg(ServerMsg::Notice { level, text });
        }
        st.sync_unseen(sid);
        if let Some(sess) = st.session(sid) {
            queue.push_view(st.view_of(sess));
        }
        let shots = st.full_shots(sid, None);
        let mut fetch = Fetcher::live(st.handles(sid));
        for shot in &shots {
            st.push_shot(sid, Some(cid), shot, &mut fetch);
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
        let req = self.spawn_request(
            &a.env,
            None,
            &a.cwd,
            single_size(viewport),
            a.cell,
            Place {
                session: sid,
                panel,
                terminal: tid,
            },
        );
        NewSession {
            sid,
            pid,
            panel,
            tid,
            name,
            req,
        }
    }

    /// The process of a new terminal: the launch spec's program or else the shell, in `cwd`,
    /// with a PTY of `inner` size.
    fn spawn_request(
        &self,
        session_env: &Env,
        launch: Option<&LaunchSpec>,
        cwd: &Path,
        inner: (u16, u16),
        cell: Option<CellPx>,
        at: Place,
    ) -> SpawnRequest {
        let config = self.config();
        let (program, args) = match launch.and_then(|l| l.program.clone()) {
            Some(program) => (program, launch.map(|l| l.args.clone()).unwrap_or_default()),
            None => (
                shell_program(&config, session_env),
                config.shell_args.clone(),
            ),
        };
        SpawnRequest {
            program,
            args,
            cwd: cwd.to_path_buf(),
            env: env::terminal_env(
                session_env,
                launch.map(|l| l.env.as_slice()).unwrap_or_default(),
                &TerminalIds {
                    socket: &self.paths.socket,
                    session: at.session,
                    panel: at.panel,
                    terminal: at.terminal,
                },
            ),
            cols: inner.0,
            rows: inner.1,
            cell,
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
    ) -> Result<SessionId, String> {
        let NewSession {
            sid,
            pid,
            panel,
            tid,
            name,
            req,
        } = new;
        let handle = terminal::start(self, tid, sid, &req, parts)?;

        let mut model = Session {
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
                    ratios: Ratios::default_for(1),
                    terminals: vec![new_terminal(tid, None, &req, false)],
                }],
            }],
        };
        naming::refresh_panel(&mut model, panel);
        st.sessions.push(SessionRt {
            model,
            env: a.env.clone(),
            terminals: HashMap::from([(tid, handle)]),
            viewport: Some(viewport),
            cell: a.cell,
            title_throttle: Throttle::new(TITLE_VIEW_INTERVAL),
            last_attach: 0,
            startup: Some(tid),
            unseen: HashSet::new(),
        });
        self.mark_dirty(st);
        Ok(sid)
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
                terminals: u32::try_from(
                    s.model
                        .projects
                        .iter()
                        .flat_map(|p| &p.panels)
                        .map(|p| p.terminals.len())
                        .sum::<usize>(),
                )
                .unwrap_or(u32::MAX),
                projects: u32::try_from(s.model.projects.len()).unwrap_or(u32::MAX),
                last_attached: s.last_attach,
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    pub fn input(self: &Arc<Self>, cid: ClientId, terminal: TerminalId, bytes: Vec<u8>) {
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
        self.publish_colors(&st, sid);
        self.follow_viewport(&mut st, cid);
        // Focus reports are not typing.
        let report = matches!(bytes.as_slice(), b"\x1b[I" | b"\x1b[O");
        if !report {
            self.agent_answered(&mut st, &handle);
        }
        // The keys go to the program first: nothing below waits for the emulator before they do.
        handle.write(bytes);
        // Typing returns the terminal to the live screen; focus reports do not.
        let id = handle.id;
        let active = st.session(sid).is_some_and(|s| s.is_active(id));
        if active
            && !report
            && handle.take_scrolled()
            && let Some(shot) = st.shot(sid, &handle, move |e| {
                e.scroll_to_bottom().then(|| e.full_frame(id))
            })
        {
            st.push_live(sid, &[shot]);
        }
    }

    /// Scrolls a terminal of the client's session and sends its screen in full.
    pub fn scroll(&self, cid: ClientId, terminal: TerminalId, to: ScrollTo) {
        let mut st = self.st();
        let Some(sid) = st.clients.get(&cid).and_then(|c| c.session) else {
            return;
        };
        let Some(handle) = st
            .session(sid)
            .and_then(|s| s.terminals.get(&terminal))
            .cloned()
        else {
            tracing::debug!(%terminal, "scroll of a terminal outside the client's session ignored");
            return;
        };
        handle.mark_scrolled(!matches!(to, ScrollTo::Bottom));
        let shot = st.shot(sid, &handle, move |e| {
            e.scroll(to);
            Some(e.full_frame(terminal))
        });
        if let Some(shot) = shot
            && st.session(sid).is_some_and(|s| s.is_active(terminal))
        {
            st.push_live(sid, &[shot]);
        }
    }

    pub fn viewport(&self, cid: ClientId, cols: u16, rows: u16, cell: Option<CellPx>) {
        let mut st = self.st();
        if st.upgrading {
            return;
        }
        let tick = st.touch();
        let Some(client) = st.clients.get_mut(&cid) else {
            return;
        };
        client.viewport = Some((cols, rows));
        client.cell = cell;
        client.active = tick;
        if let Some(sid) = client.session {
            self.publish_colors(&st, sid);
        }
        self.follow_viewport(&mut st, cid);
    }

    /// The outer terminal of a client gained or lost focus.
    pub fn focus(&self, cid: ClientId, focused: bool) {
        let mut st = self.st();
        let Some(client) = st.clients.get_mut(&cid) else {
            return;
        };
        if client.focused == focused {
            return;
        }
        client.focused = focused;
        // Focus gained: the active panel is seen now.
        if let Some(sid) = client.session {
            st.push_view(sid);
        }
    }

    /// A viewport change that came from a config reload: the client's size is recorded, and
    /// the terminals follow it only when that client is the one that was active last.
    pub fn reload_viewport(&self, cid: ClientId, cols: u16, rows: u16, cell: Option<CellPx>) {
        let mut st = self.st();
        if st.upgrading {
            return;
        }
        let Some(client) = st.clients.get_mut(&cid) else {
            return;
        };
        client.viewport = Some((cols, rows));
        client.cell = cell;
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
        let cell = client.cell;
        if sess.viewport == Some(vp) && sess.cell == cell {
            return;
        }
        if let Some(sess) = st.session_mut(sid) {
            sess.viewport = Some(vp);
            sess.cell = cell;
        }
        let changed = self.apply_sizes(st, sid);
        st.push_view(sid);
        let shots = st.full_shots(sid, Some(&changed));
        st.push_live(sid, &shots);
    }

    /// Sizes the active panel's PTYs and emulators from its layout in the session's
    /// viewport; returns the terminals whose size changed.
    fn apply_sizes(&self, st: &State, sid: SessionId) -> Vec<TerminalId> {
        let Some(sess) = st.session(sid) else {
            return Vec::new();
        };
        let Some(panel) = sess.active_panel_model() else {
            return Vec::new();
        };
        let (cols, rows) = sess.viewport.unwrap_or(DEFAULT_VIEWPORT);
        let cell = sess.cell;
        let n = panel.terminals.len();
        let rects = layout::panel_rects(Rect::new(0, 0, cols, rows), n, &panel.ratios);
        let mut changed = Vec::new();
        for (term, rect) in panel.terminals.iter().zip(rects) {
            let (c, r) = layout::inner(rect, n);
            let size = clamp_size(c, r);
            let Some(handle) = sess.terminals.get(&term.id) else {
                continue;
            };
            // The PTY too: its size can differ from the emulator's.
            let pty = Some((size, terminal::pixels_of(size.0, size.1, cell)));
            if handle.with_emu(|e| (e.size(), e.cell())) != Some((size, cell))
                || handle.pty_size() != pty
            {
                handle.resize_pty(size.0, size.1, cell);
                handle.recheck_pty(&self.handle);
                changed.push(term.id);
            }
        }
        changed
    }

    /// After the writer dropped frames: the current view and every screen in full.
    pub fn resync(&self, cid: ClientId) {
        let mut st = self.st();
        let Some(client) = st.clients.get_mut(&cid) else {
            return;
        };
        let Some(sid) = client.session else {
            return;
        };
        // The full frames below owe the client again what they hold back.
        client.images.owed.clear();
        if let Some(sess) = st.session(sid)
            && let Some(client) = st.clients.get(&cid)
        {
            client.queue.push_view(st.view_of(sess));
        }
        let shots = st.full_shots(sid, None);
        let mut fetch = Fetcher::live(st.handles(sid));
        for shot in &shots {
            st.push_shot(sid, Some(cid), shot, &mut fetch);
        }
    }

    /// After the pieces of the images a client was sent drained: the frames it was owed
    /// because an image did not fit.
    pub fn send_owed(&self, cid: ClientId) {
        let mut st = self.st();
        let Some(client) = st.clients.get_mut(&cid) else {
            return;
        };
        let Some(sid) = client.session else {
            return;
        };
        let owed: Vec<TerminalId> = std::mem::take(&mut client.images.owed)
            .into_iter()
            .collect();
        if owed.is_empty() {
            return;
        }
        // New pixels are copied with the state lock released, as `pump` does.
        let mut fetch = Fetcher::deferred(st.handles(sid));
        let mut missing = false;
        for shot in &st.full_shots(sid, Some(&owed)) {
            missing |= !st.push_shot(sid, Some(cid), shot, &mut fetch).is_empty();
        }
        if !missing {
            return;
        }
        drop(st);
        fetch.copy_wanted();
        let mut st = self.st();
        if st.clients.get(&cid).and_then(|c| c.session) != Some(sid) {
            return;
        }
        for shot in &st.full_shots(sid, Some(&owed)) {
            st.push_shot(sid, Some(cid), shot, &mut fetch);
        }
    }

    // Terminal callbacks.

    /// Program output: into the emulator, then whatever the emulator asks for.
    pub fn output(self: &Arc<Self>, h: &Arc<TermHandle>, bytes: &[u8]) {
        let bytes = bytes.to_vec();
        // The emulator answers colour queries itself, with the colours of the client the
        // session follows.
        let colors = lock(&self.session_colors)
            .get(&h.session)
            .copied()
            .unwrap_or(FALLBACK_COLORS);
        let name = lock(&self.session_names).get(&h.session).cloned();
        let events = h
            .with_emu(move |e| {
                e.set_colors(colors.fg, colors.bg);
                e.set_xtversion(name.as_deref());
                e.feed(&bytes)
            })
            .unwrap_or_default();
        h.dirty.notify_one();
        self.bump();
        for event in events {
            self.emu_event(h, event);
        }
    }

    fn emu_event(self: &Arc<Self>, h: &Arc<TermHandle>, event: EmuEvent) {
        match event {
            EmuEvent::Title(title) => self.title(h, title),
            EmuEvent::Reply(text) => h.write(text.into_bytes()),
            EmuEvent::ClipboardStore { selection, text } => {
                let st = self.st();
                if let Some(client) = st.active_client(h.session) {
                    client.queue.push_msg(ServerMsg::Clipboard {
                        selection: selection.into(),
                        text,
                    });
                }
            }
        }
    }

    fn title(self: &Arc<Self>, h: &Arc<TermHandle>, title: Option<String>) {
        let mut st = self.st();
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
        // Only an animation in front of an agent's title changed: nothing to tell.
        let animation = model.agent.is_some()
            && match (&model.title, &title) {
                (Some(old), Some(new)) => naming::title_core(old) == naming::title_core(new),
                _ => false,
            };
        model.title = title;
        // A new name travels in the same throttled view as the title.
        if !self.refresh_name_of(&mut st, sid, h.id) && animation {
            return;
        }
        self.push_view_throttled(&mut st, sid);
    }

    /// Sends the session's view for a change a program's output makes at any rate, a title
    /// or a directory: at most one per [`TITLE_VIEW_INTERVAL`], and a last one after a burst.
    fn push_view_throttled(self: &Arc<Self>, st: &mut State, sid: SessionId) {
        let now = self.clock.now();
        let Some(sess) = st.session_mut(sid) else {
            return;
        };
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

    /// Turns the emulator's news into a frame for the session's clients. Returns how long
    /// until the pass that follows the timeout of a synchronized output the program holds.
    pub fn pump(&self, h: &Arc<TermHandle>) -> Option<Duration> {
        let sid = h.session;
        let mut st = self.st();
        let streamed = st.session(sid).is_some_and(|s| s.is_active(h.id))
            && st.clients_of(sid).next().is_some();
        if !streamed {
            // Nobody sees the damage: dropping it waits for the emulator, which is not done
            // under the state lock.
            drop(st);
            h.with_emu(|e| e.reset_damage());
            // The terminal may have become streamed while the lock was free, and the reader
            // may have fed output that the reset then wiped: those clients get a full frame.
            let mut st = self.st();
            let now_streamed = st.session(sid).is_some_and(|s| s.is_active(h.id))
                && st.clients_of(sid).next().is_some();
            if now_streamed {
                let id = h.id;
                if let Some(shot) = st.shot(sid, h, move |e| Some(e.full_frame(id))) {
                    st.push_live(sid, &[shot]);
                }
            }
            return None;
        }
        // The damage is read and pushed under one hold of the lock: a frame read outside it
        // can reach the clients after a full frame (a scroll, a terminal's last output) that
        // already shows newer output.
        let now = self.clock.now();
        let ids = st.sent_ids(sid, h.id);
        let (shot, hold) = h.with_emu({
            let id = h.id;
            move |e| {
                let shot = e
                    .damage_frame(id, now)
                    .map(|frame| Shot::new(e, frame, &ids));
                (shot, e.hold_remaining(now))
            }
        })?;
        let Some(shot) = shot else {
            return hold;
        };
        // A new image is not copied under the state lock: the frame goes out without its
        // placement, and a full frame with the pixels ahead of it follows.
        let mut fetch = Fetcher::deferred(HashMap::from([(h.id, Arc::clone(h))]));
        let missing = st.push_shot(sid, None, &shot, &mut fetch);
        if missing.is_empty() {
            return hold;
        }
        drop(st);
        fetch.copy_wanted();
        let mut st = self.st();
        // The lock was released: the panel may have changed. An image placed in that window
        // is not lost: the next damage frame lists it, because the placements the emulator
        // compares in `images_state` changed, and runs this again.
        if !st.session(sid).is_some_and(|s| s.is_active(h.id)) {
            return hold;
        }
        let id = h.id;
        if let Some(shot) = st.shot(sid, h, move |e| Some(e.full_frame(id))) {
            for cid in missing {
                st.push_shot(sid, Some(cid), &shot, &mut fetch);
            }
        }
        hold
    }

    /// A terminal's process exited and its output was read.
    pub fn terminal_gone(self: &Arc<Self>, h: &Arc<TermHandle>, status: i32) {
        let mut st = self.st();
        h.finish(status);
        st.exits.record(h.id, status, self.clock.now());
        self.bump();
        if st.shutting_down {
            return;
        }
        let sid = h.session;
        let in_model = st
            .session(sid)
            .is_some_and(|s| s.terminals.contains_key(&h.id));
        let kept = in_model
            && st
                .session_mut(sid)
                .and_then(|s| terminal_model(&mut s.model, h.id))
                .is_some_and(|t| t.keep);
        if kept {
            self.keep_exited(&mut st, h, status);
            return;
        }
        h.close();
        // Closed by an action, or its session was killed: already out of the model.
        if !in_model {
            st.forget_images(sid, h.id);
            h.release();
            return;
        }
        // The last output reaches the clients before the terminal goes.
        let streamed = st.session(sid).is_some_and(|s| s.is_active(h.id));
        if streamed
            && let Some(shot) = st.shot(sid, h, {
                let id = h.id;
                move |e| Some(e.last_frame(id))
            })
        {
            st.push_live(sid, &[shot]);
        }
        st.forget_images(sid, h.id);
        // Whatever a background job still prints from here on is discarded.
        h.release();

        let now = self.clock.now();
        let quick = status != 0 && now.saturating_sub(h.started) < QUICK_EXIT;
        let was_restored = st.restored.remove(&h.id);
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
        let mut emptied = None;
        let mut renamed = None;
        if let Some(panel) = panel_of_terminal(&mut sess.model, h.id) {
            panel.remove_terminal(h.id);
            renamed = Some(panel.id);
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
            if none_left {
                Self::mark_shutting_down(&mut st);
            }
            drop(st);
            if none_left {
                tracing::info!(
                    "shutting down: the last session ended, its first terminal exited at once"
                );
                self.request_shutdown();
            }
            return;
        }
        self.settle_named(&mut st, sid, before, renamed);
        drop(st);
        if quick && was_restored {
            self.notify(
                NoticeLevel::Warning,
                format!(
                    "restored terminal {} exited at once (status {status}); removed",
                    h.program
                ),
            );
        }
    }

    /// A kept terminal's process ended: it stays, with its screen and its exit code; what is
    /// typed to it is dropped.
    fn keep_exited(self: &Arc<Self>, st: &mut State, h: &Arc<TermHandle>, status: i32) {
        let sid = h.session;
        let streamed = st.session(sid).is_some_and(|s| s.is_active(h.id));
        if streamed
            && let Some(shot) = st.shot(sid, h, {
                let id = h.id;
                move |e| Some(e.last_frame(id))
            })
        {
            st.push_live(sid, &[shot]);
        }
        h.stop_input();
        st.restored.remove(&h.id);
        let Some(sess) = st.session_mut(sid) else {
            return;
        };
        let before = sess.model.active_panel();
        if let Some(t) = terminal_model(&mut sess.model, h.id) {
            t.exit_status = Some(status);
            // The agent left with its program.
            t.agent = None;
        }
        let renamed = panel_of_terminal(&mut sess.model, h.id).map(|p| p.id);
        self.settle_named(st, sid, before, renamed);
    }

    /// After a change to a session: its terminals match the viewport, every client has the
    /// new view, and a new active panel is shown in full.
    fn settle(self: &Arc<Self>, st: &mut State, sid: SessionId, before: Option<PanelId>) {
        self.mark_dirty(st);
        self.bump();
        let Some(sess) = st.session(sid) else {
            return;
        };
        let moved = sess.model.active_panel() != before;
        let changed = self.apply_sizes(st, sid);
        st.push_view(sid);
        let shots = if moved {
            st.full_shots(sid, None)
        } else {
            st.full_shots(sid, Some(&changed))
        };
        st.push_live(sid, &shots);
    }

    /// [`Shared::settle`] after a change of a panel's terminals or of their focus: the panel
    /// may take a new name.
    fn settle_named(
        self: &Arc<Self>,
        st: &mut State,
        sid: SessionId,
        before: Option<PanelId>,
        panel: Option<PanelId>,
    ) {
        if let Some(panel) = panel {
            self.refresh_name(st, sid, panel);
        }
        self.settle(st, sid, before);
    }

    /// Recomputes the name of a panel that follows what runs in it; true when it changed
    /// (the caller sends the view).
    fn refresh_name(self: &Arc<Self>, st: &mut State, sid: SessionId, panel: PanelId) -> bool {
        let renamed = st
            .session_mut(sid)
            .is_some_and(|s| naming::refresh_panel(&mut s.model, panel));
        if renamed {
            self.mark_dirty(st);
        }
        renamed
    }

    /// [`Shared::refresh_name`] for the panel of a terminal.
    fn refresh_name_of(self: &Arc<Self>, st: &mut State, sid: SessionId, tid: TerminalId) -> bool {
        let panel = st
            .session_mut(sid)
            .and_then(|s| panel_of_terminal(&mut s.model, tid))
            .map(|p| p.id);
        panel.is_some_and(|panel| self.refresh_name(st, sid, panel))
    }

    /// Sends `Bye` to the session's clients and removes the session. Its terminals are not
    /// touched.
    fn end_session(self: &Arc<Self>, st: &mut State, sid: SessionId, reason: ByeReason) {
        for client in st.clients.values_mut().filter(|c| c.session == Some(sid)) {
            client.queue.push_msg(ServerMsg::Bye {
                reason: reason.clone(),
            });
            client.session = None;
        }
        st.sessions.retain(|s| s.model.id != sid);
        lock(&self.session_colors).remove(&sid);
        lock(&self.session_names).remove(&sid);
        self.mark_dirty(st);
        self.bump();
    }

    // Actions.

    /// Runs an action of an attached client on its session.
    pub async fn action(
        self: &Arc<Self>,
        cid: ClientId,
        action: Action,
    ) -> Result<(), ActionError> {
        if self.st().upgrading {
            return Err(ActionError::Spawn(UPGRADING.into()));
        }
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
                self.add_panel(sid, PanelTarget::NewProject { name, root }, None, true)
                    .await
                    .map(|_| ())
            }
            Action::NewPanel { project, name } => {
                let name = name
                    .map(|n| validate_name(&n, NameKind::Panel))
                    .transpose()?;
                self.add_panel(sid, PanelTarget::Project { project, name }, None, true)
                    .await
                    .map(|_| ())
            }
            Action::NewPanelWith { project, launch } => {
                self.new_panel_with(cid, sid, project, launch).await
            }
            Action::NewTerminal { panel } => self
                .add_terminal(sid, panel, None, true, false)
                .await
                .map(|_| ()),
            Action::KillSession => {
                let queue = self.st().clients.get(&cid).map(|c| Arc::clone(&c.queue));
                let answer = || {
                    if let Some(q) = queue {
                        q.push_msg(ServerMsg::ActionResult(Ok(())));
                    }
                };
                if self.kill_session(|s| s.model.id == sid, answer)? {
                    tracing::info!("shutting down: last session killed from an attached client");
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
        // The panel that may take a new name after the action: its terminals or the focused
        // one changed, or the user gave the name back.
        let mut renamed_panel = None;
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
            Action::MoveProject { project, to } => {
                let projects = &mut sess.model.projects;
                let from = projects
                    .iter()
                    .position(|p| p.id == project)
                    .ok_or(ActionError::NoSuchProject(project))?;
                let item = projects.remove(from);
                projects.insert(to.min(projects.len()), item);
            }
            Action::MovePanel { panel, to } => {
                let panels = sess
                    .model
                    .projects
                    .iter_mut()
                    .map(|p| &mut p.panels)
                    .find(|panels| panels.iter().any(|p| p.id == panel))
                    .ok_or(ActionError::NoSuchPanel(panel))?;
                let from = panels.iter().position(|p| p.id == panel).unwrap_or(0);
                let item = panels.remove(from);
                panels.insert(to.min(panels.len()), item);
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
            Action::AutoNamePanel { panel } => {
                panel_mut(&mut sess.model, panel)
                    .ok_or(ActionError::NoSuchPanel(panel))?
                    .name_source = NameSource::Auto;
                renamed_panel = Some(panel);
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
            Action::CloseTerminal { terminal } => {
                let panel = panel_of_terminal(&mut sess.model, terminal)
                    .ok_or(ActionError::NoSuchTerminal(terminal))?;
                panel.remove_terminal(terminal);
                renamed_panel = Some(panel.id);
                let emptied = panel.terminals.is_empty().then_some(panel.id);
                doomed.extend(sess.terminals.remove(&terminal));
                if let Some(panel) = emptied {
                    sess.model.remove_panel(panel);
                }
            }
            Action::FocusTerminal { terminal } => {
                let panel = panel_of_terminal(&mut sess.model, terminal)
                    .ok_or(ActionError::NoSuchTerminal(terminal))?;
                panel.focused_terminal = Some(terminal);
                renamed_panel = Some(panel.id);
            }
            Action::FocusDirection { dir } => {
                let (cols, rows) = sess.viewport.unwrap_or(DEFAULT_VIEWPORT);
                let active = sess
                    .model
                    .active_panel()
                    .ok_or(ActionError::NoActivePanel)?;
                let panel = panel_mut(&mut sess.model, active).ok_or(ActionError::NoActivePanel)?;
                let rects = layout::panel_rects(
                    Rect::new(0, 0, cols, rows),
                    panel.terminals.len(),
                    &panel.ratios,
                );
                let from = panel
                    .terminals
                    .iter()
                    .position(|t| Some(t.id) == panel.focused_terminal);
                let target = from
                    .and_then(|from| layout::neighbor(&rects, from, dir))
                    .and_then(|to| panel.terminals.get(to));
                if let Some(target) = target {
                    panel.focused_terminal = Some(target.id);
                    renamed_panel = Some(panel.id);
                }
            }
            Action::SetRatios { panel, n, ratios } => {
                let p = panel_mut(&mut sess.model, panel).ok_or(ActionError::NoSuchPanel(panel))?;
                if p.terminals.len() != n {
                    return Err(ActionError::LayoutChanged);
                }
                if !ratios.is_valid_for(n) {
                    return Err(ActionError::InvalidRatios);
                }
                p.ratios = ratios;
            }
            Action::NewProject { .. }
            | Action::NewPanel { .. }
            | Action::NewPanelWith { .. }
            | Action::NewTerminal { .. }
            | Action::KillSession => {
                unreachable!("handled by `action`")
            }
        }
        self.settle_named(&mut st, sid, before, renamed_panel);
        self.hang_up(&mut st, doomed);
        Ok(())
    }

    /// Adds a panel with one new process (a shell unless `launch` names a program), in an
    /// existing project or a new one. `focus` makes the new panel (and project) the active
    /// one; without it the session's focus stays where it was.
    async fn add_panel(
        self: &Arc<Self>,
        sid: SessionId,
        target: PanelTarget,
        launch: Option<LaunchSpec>,
        focus: bool,
    ) -> Result<Made, ActionError> {
        let (project_id, panel_id, tid, req, slot) = {
            let mut st = self.st();
            if st.shutting_down {
                return Err(ActionError::ShuttingDown);
            }
            if st.upgrading {
                return Err(ActionError::Spawn(UPGRADING.into()));
            }
            let slot = self.open_slot();
            let sess = st.session(sid).ok_or(ActionError::NotAttached)?;
            let cell = sess.cell;
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
            let cwd = launch_dir(launch.as_ref()).unwrap_or_else(|| root.clone());
            let req = self.spawn_request(
                &env,
                launch.as_ref(),
                &cwd,
                single_size(viewport),
                cell,
                Place {
                    session: sid,
                    panel: panel_id,
                    terminal: tid,
                },
            );
            (project_id, panel_id, tid, req, slot)
        };

        let open_req = req.clone();
        let parts = tokio::task::spawn_blocking(move || terminal::open(&open_req))
            .await
            .map_err(|e| ActionError::Spawn(format!("could not start {}: {e}", req.program)))?
            .map_err(ActionError::Spawn)?;
        drop(slot);

        let mut st = self.st_after_open().await;
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
        let handle = terminal::start(self, tid, sid, &req, parts).map_err(ActionError::Spawn)?;
        let Some(sess) = st.session_mut(sid) else {
            return Err(ActionError::NotAttached);
        };
        sess.terminals.insert(tid, handle);
        let before = sess.model.active_panel();
        let (panel_name, name_source) = match &target {
            PanelTarget::Project {
                name: Some(name), ..
            } => (Some(name.clone()), NameSource::Manual),
            _ => (None, NameSource::Default),
        };
        if let PanelTarget::NewProject { name, root } = target {
            sess.model.projects.push(Project {
                id: project_id,
                name,
                root,
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
            ratios: Ratios::default_for(1),
            terminals: vec![new_terminal(tid, launch.as_ref(), &req, false)],
        });
        if focus {
            sess.model.focus_panel(panel_id);
        } else {
            // Without focus the panel is still its project's active one when that has none,
            // and a session without an active project gets this one.
            if project.active_panel.is_none() {
                project.active_panel = Some(panel_id);
            }
            if sess.model.active_project.is_none() {
                sess.model.active_project = Some(project_id);
            }
        }
        self.settle_named(&mut st, sid, before, Some(panel_id));
        Ok(Made {
            project: project_id,
            panel: panel_id,
        })
    }

    /// Adds a terminal to a panel (the active one by default): a shell, or the program of
    /// `launch`. It is refused when the grid for one more terminal would leave a terminal
    /// below the minimum size. `focus` focuses the new terminal in its panel; `keep` makes it
    /// stay after its process exits.
    async fn add_terminal(
        self: &Arc<Self>,
        sid: SessionId,
        panel: Option<PanelId>,
        launch: Option<LaunchSpec>,
        focus: bool,
        keep: bool,
    ) -> Result<TerminalId, ActionError> {
        let room = |sess: &SessionRt, panel: PanelId| -> Result<(usize, (u16, u16)), ActionError> {
            let p = sess
                .model
                .projects
                .iter()
                .flat_map(|p| &p.panels)
                .find(|p| p.id == panel)
                .ok_or(ActionError::NoSuchPanel(panel))?;
            let n = p.terminals.len();
            let (cols, rows) = sess.viewport.unwrap_or(DEFAULT_VIEWPORT);
            let area = Rect::new(0, 0, cols, rows);
            if !layout::has_room_for(area, n + 1) {
                return Err(ActionError::NoRoom);
            }
            let adapted = Ratios::adapt(&p.ratios, n, n + 1);
            let rects = layout::panel_rects(area, n + 1, &adapted);
            let rect = rects.get(n).copied().ok_or(ActionError::NoRoom)?;
            let (c, r) = layout::inner(rect, n + 1);
            Ok((n, clamp_size(c, r)))
        };
        let (panel_id, tid, req, dirs, slot) = {
            let mut st = self.st();
            if st.shutting_down {
                return Err(ActionError::ShuttingDown);
            }
            if st.upgrading {
                return Err(ActionError::Spawn(UPGRADING.into()));
            }
            let slot = self.open_slot();
            let sess = st.session(sid).ok_or(ActionError::NotAttached)?;
            let cell = sess.cell;
            let panel_id = panel
                .or_else(|| sess.model.active_panel())
                .ok_or(ActionError::NoActivePanel)?;
            let (_, size) = room(sess, panel_id)?;
            let project = sess
                .model
                .projects
                .iter()
                .find(|p| p.panels.iter().any(|x| x.id == panel_id))
                .ok_or(ActionError::NoSuchPanel(panel_id))?;
            let root = project.root.clone();
            // The directory the launch spec names, else where the panel's focused terminal
            // is, else where it started, else the root.
            let focused = project
                .panels
                .iter()
                .find(|x| x.id == panel_id)
                .and_then(|p| {
                    let id = p.focused_terminal?;
                    p.terminals.iter().find(|t| t.id == id)
                });
            let dirs = [
                launch_dir(launch.as_ref()),
                focused.and_then(|t| t.cwd.clone()),
                focused.map(|t| t.launch.cwd.clone()),
                Some(root.clone()),
            ];
            let env = sess.env.clone();
            let tid = TerminalId(st.alloc());
            let req = self.spawn_request(
                &env,
                launch.as_ref(),
                &root,
                size,
                cell,
                Place {
                    session: sid,
                    panel: panel_id,
                    terminal: tid,
                },
            );
            (panel_id, tid, req, dirs, slot)
        };

        let program = req.program.clone();
        let explicit = dirs.first().is_some_and(Option::is_some);
        let (parts, req) = tokio::task::spawn_blocking(move || {
            // A named directory is used as it is (starting fails when it is no directory);
            // otherwise the first that exists, as one may be gone.
            let mut req = req;
            let dir = if explicit {
                dirs.into_iter().next().flatten()
            } else {
                restore::first_dir(dirs)
            };
            if let Some(dir) = dir {
                req.cwd = dir;
            }
            terminal::open(&req).map(|parts| (parts, req))
        })
        .await
        .map_err(|e| ActionError::Spawn(format!("could not start {program}: {e}")))?
        .map_err(ActionError::Spawn)?;
        drop(slot);

        let mut st = self.st_after_open().await;
        // The world may have changed while the process started.
        let checked = if st.shutting_down {
            Err(ActionError::ShuttingDown)
        } else {
            st.session(sid)
                .ok_or(ActionError::NotAttached)
                .and_then(|sess| room(sess, panel_id).map(|_| ()))
        };
        if let Err(e) = checked {
            parts.discard();
            return Err(e);
        }
        let handle = terminal::start(self, tid, sid, &req, parts).map_err(ActionError::Spawn)?;
        let Some(sess) = st.session_mut(sid) else {
            return Err(ActionError::NotAttached);
        };
        sess.terminals.insert(tid, handle);
        let before = sess.model.active_panel();
        let term = new_terminal(tid, launch.as_ref(), &req, keep);
        if let Some(panel) = panel_mut(&mut sess.model, panel_id) {
            let focused = panel.focused_terminal;
            panel.add_terminal(term);
            if !focus {
                panel.focused_terminal = focused;
            }
        }
        self.settle_named(&mut st, sid, before, Some(panel_id));
        let shots = st.full_shots(sid, Some(&[tid]));
        st.push_live(sid, &shots);
        Ok(tid)
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
        if none_left {
            Self::mark_shutting_down(&mut st);
        }
        self.hang_up(&mut st, handles);
        Ok(none_left)
    }

    /// Hangs up terminals that left the model: `SIGHUP`, `SIGKILL` after two seconds.
    /// Called under the lock that removed them, so a wait never finds them nowhere.
    fn hang_up(self: &Arc<Self>, st: &mut State, handles: Vec<Arc<TermHandle>>) {
        if handles.is_empty() {
            return;
        }
        // Retention runs from removal: a kept terminal closed long after it exited can
        // still be waited on for a minute.
        let now = self.clock.now();
        for h in &handles {
            if let Some(code) = h.finished() {
                st.exits.record(h.id, code, now);
            }
        }
        st.hanging.extend(handles.iter().cloned());
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
            cell: None,
            images: false,
            outer_name: None,
            cwd,
            env,
        };
        let _attaching = self.attaching.lock().await;
        let new = {
            let mut st = self.st();
            if st.shutting_down {
                return Err(ActionError::ShuttingDown);
            }
            if st.upgrading {
                return Err(ActionError::Spawn(UPGRADING.into()));
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
        let mut st = self.st_after_open().await;
        st.creating -= 1;
        let parts = opened?;
        if st.shutting_down {
            parts.discard();
            return Err(ActionError::ShuttingDown);
        }
        let name = new.name.clone();
        let sid = self
            .add_session(&mut st, new, parts, &a, DEFAULT_VIEWPORT)
            .map_err(ActionError::Spawn)?;
        Ok((sid, name))
    }

    /// Ends the named session; true when it was the last one.
    pub fn kill_session_named(
        self: &Arc<Self>,
        name: &str,
        answer: impl FnOnce(),
    ) -> Result<bool, ActionError> {
        if self.st().upgrading {
            return Err(ActionError::Spawn(UPGRADING.into()));
        }
        self.kill_session(|s| s.model.name == name, answer)
            .map_err(|_| ActionError::NoSuchSession(name.to_string()))
    }

    pub fn rename_session_named(self: &Arc<Self>, from: &str, to: &str) -> Result<(), ActionError> {
        let to = validate_name(to, NameKind::Session)?;
        let st = self.st();
        if st.upgrading {
            return Err(ActionError::Spawn(UPGRADING.into()));
        }
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
        Self::mark_shutting_down(&mut self.st());
        self.bump();
        let _ = self.shutdown_tx.send(true);
    }

    /// Sets the shutdown flag and says `Bye` to every client, once. Called under the lock
    /// that ended the last session too, so no new client gets in between.
    fn mark_shutting_down(st: &mut State) {
        if st.shutting_down {
            return;
        }
        st.shutting_down = true;
        for client in st.clients.values() {
            client.queue.push_msg(ServerMsg::Bye {
                reason: ByeReason::ServerShutdown,
            });
        }
    }

    /// Lets the writers deliver their `Bye`, then hangs up every terminal (`SIGHUP`, then
    /// `SIGKILL` after two seconds).
    pub async fn finish_shutdown(self: &Arc<Self>) {
        // The flag is set, so the model no longer changes: this is the state that stays.
        self.save_final().await;
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
