//! The daemon's shared state: sessions, clients and the rules that tie them together.
//!
//! Lock order: `state`, then a terminal's `emu`. Nothing waits while holding either.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use alacritty_terminal::vte::ansi::{NamedColor, Rgb};
use deck_core::config::Config;
use deck_core::model::{
    LaunchSpec, NameSource, Panel, PanelId, Project, ProjectId, Session, SessionId, Terminal,
    TerminalId,
};
use deck_core::protocol::{ByeReason, Frame, OuterColors, ScrollTo, ServerMsg, SessionSummary};
use tokio::runtime::Handle;
use tokio::sync::{Notify, watch};

use crate::clock::{Clock, Fire, Poke, Throttle};
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

/// What an attaching client asks for.
#[derive(Debug, Clone)]
pub struct AttachRequest {
    pub session: Option<String>,
    pub create: bool,
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
            .terminals
            .values()
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
    let at_home = env::lookup(env, "HOME").is_some_and(|h| Path::new(h) == cwd);
    if at_home {
        return "~".into();
    }
    cwd.file_name()
        .map_or_else(|| "root".into(), |n| n.to_string_lossy().into_owned())
}

pub struct Shared {
    pub paths: DaemonPaths,
    pub config: Config,
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
        clock: Arc<dyn Clock>,
        version: u32,
    ) -> (Arc<Self>, watch::Receiver<bool>) {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let shared = Arc::new(Self {
            paths,
            config,
            clock,
            version,
            handle: Handle::current(),
            state: Mutex::new(State::default()),
            shutdown_tx,
            writers: AtomicUsize::new(0),
            writers_done: Notify::new(),
            attaching: tokio::sync::Mutex::new(()),
        });
        (shared, shutdown_rx)
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
        let plan = {
            let mut st = self.st();
            if st.shutting_down {
                return Err("the daemon is shutting down".into());
            }
            let plan = self.plan_attach(&mut st, a)?;
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
                let sid = self.add_session(&mut st, new, parts, a);
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
        client.session = Some(sid);
        client.viewport = Some((a.cols, a.rows));
        client.active = active;
        let queue = Arc::clone(&client.queue);

        queue.push_msg(ServerMsg::Welcome { session: sid });
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
    fn plan_attach(&self, st: &mut State, a: &AttachRequest) -> Result<Plan, String> {
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
            Some(name) if a.create => name.clone(),
            Some(name) => return Err(format!("no session named {name}")),
        };
        Ok(Plan::Create(self.new_session(st, name, a)))
    }

    fn new_session(&self, st: &mut State, name: String, a: &AttachRequest) -> NewSession {
        let sid = SessionId(st.alloc());
        let pid = ProjectId(st.alloc());
        let panel = PanelId(st.alloc());
        let tid = TerminalId(st.alloc());
        let inner = clamp_size(a.cols.saturating_sub(2), a.rows.saturating_sub(2));

        let program = self
            .config
            .shell
            .clone()
            .or_else(|| {
                env::lookup(&a.env, "SHELL")
                    .and_then(|v| v.to_str())
                    .map(str::to_string)
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "/bin/sh".to_string());
        let req = SpawnRequest {
            program,
            args: self.config.shell_args.clone(),
            cwd: a.cwd.clone(),
            env: env::terminal_env(
                &a.env,
                &TerminalIds {
                    socket: &self.paths.socket,
                    session: sid,
                    panel,
                    terminal: tid,
                },
            ),
            cols: inner.0,
            rows: inner.1,
        };
        NewSession {
            sid,
            pid,
            panel,
            tid,
            name,
            req,
        }
    }

    /// Adds a planned session around its started first process.
    fn add_session(
        self: &Arc<Self>,
        st: &mut State,
        new: NewSession,
        parts: terminal::Parts,
        a: &AttachRequest,
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
            viewport: Some((a.cols, a.rows)),
            title_throttle: Throttle::new(TITLE_VIEW_INTERVAL),
            last_attach: 0,
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
        if !matches!(bytes.as_slice(), b"\x1b[I" | b"\x1b[O")
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
        if let Some(frame) = frame {
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
        let inner = clamp_size(vp.0.saturating_sub(2), vp.1.saturating_sub(2));
        for t in sess.terminals.values() {
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
        if st.clients_of(h.session).next().is_none() {
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
        if st.session(sid).is_none() {
            return;
        }
        // The last output reaches the clients before the terminal goes.
        if let Some(frame) = h.with_emu(|e| e.damage_frame(h.id)) {
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
        sess.terminals.remove(&h.id);
        for panel in sess
            .model
            .projects
            .iter_mut()
            .flat_map(|p| p.panels.iter_mut())
        {
            panel.terminals.retain(|t| t.id != h.id);
            if panel.focused_terminal == Some(h.id) {
                panel.focused_terminal = panel.terminals.first().map(|t| t.id);
            }
        }
        if !sess.terminals.is_empty() {
            st.push_view(sid);
            return;
        }

        // Until DECK-02: a session without terminals ends.
        let reason = if quick {
            ByeReason::Error(format!(
                "terminal exited at once (status {status}); see {}",
                self.paths.log_file.display()
            ))
        } else {
            ByeReason::SessionEnded
        };
        for client in st.clients.values_mut().filter(|c| c.session == Some(sid)) {
            client.queue.push_msg(ServerMsg::Bye {
                reason: reason.clone(),
            });
            client.session = None;
        }
        st.sessions.retain(|s| s.model.id != sid);
        // A session being created counts: its attach would otherwise fail on the shutdown.
        let none_left = st.sessions.is_empty() && st.creating == 0;
        drop(st);
        if none_left {
            self.request_shutdown();
        }
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

        let handles: Vec<Arc<TermHandle>> = self
            .st()
            .sessions
            .iter()
            .flat_map(|s| s.terminals.values().cloned())
            .collect();
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
            let _ =
                tokio::task::spawn_blocking(move || stubborn.iter().for_each(|h| h.kill())).await;
        }
    }
}
