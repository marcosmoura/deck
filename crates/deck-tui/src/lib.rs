//! The attach client: screen mirror, rendering, input handling and key bindings.

mod agent;
#[cfg(test)]
mod agent_tests;
mod app;
mod backend;
mod bar;
#[cfg(test)]
mod bar_tests;
mod caps;
#[cfg(test)]
mod caps_tests;
mod colors;
mod context;
#[cfg(test)]
mod context_tests;
mod convert;
mod copy;
#[cfg(test)]
mod copy_tests;
mod diacritics;
#[cfg(test)]
mod dialog_tests;
mod draw;
mod graphics;
#[cfg(test)]
mod graphics_tests;
mod grid;
#[cfg(test)]
mod grid_tests;
#[cfg(test)]
mod icons_tests;
mod input;
#[cfg(test)]
mod kitty_tests;
mod menu;
#[cfg(test)]
mod menu_tests;
mod mirror;
mod modal;
mod picker;
mod placed;
#[cfg(test)]
mod placed_tests;
mod reader;
#[cfg(test)]
mod reload_tests;
mod resize;
#[cfg(test)]
mod resize_tests;
mod restore;
#[cfg(test)]
mod restore_tests;
mod search;
#[cfg(test)]
mod search_tests;
mod select;
#[cfg(test)]
mod select_tests;
mod sidebar;
#[cfg(test)]
mod sidebar_bar_tests;
#[cfg(test)]
mod sidebar_drag_tests;
#[cfg(test)]
mod sidebar_reorder_tests;
#[cfg(test)]
mod sidebar_terminal_tests;
#[cfg(test)]
mod sidebar_tests;
mod term;
#[cfg(test)]
mod theme_tests;
mod tier;

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use deck_core::config::Config;
use deck_core::protocol::{
    ByeReason, CellPx, ClientMsg, FrameReader, OuterColors, PROTOCOL_VERSION, Request, Response,
    Role, ServerMsg, encode,
};
use ratatui::Terminal;
use ratatui::crossterm::event as ct;
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal as ct_terminal;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;
use tokio::time::sleep_until;

use crate::app::{App, Out};
pub use crate::reader::{StdinState, WaitStdin};

/// The redraw timer: at most one draw per tick, and only when something changed.
const REDRAW: Duration = Duration::from_millis(16);
/// How often the input thread looks at its stop flag.
const INPUT_POLL: Duration = Duration::from_millis(50);
/// How long the main loop waits for the input thread on the way out.
const READER_JOIN: Duration = Duration::from_millis(200);
/// How long the writer may take to flush the last messages (`Detach`) on the way out.
const FLUSH_FOR: Duration = Duration::from_millis(500);

/// Reads the outer terminal's input for at most the given time. `Some(bytes)` (empty after
/// a timeout) or `None` when it is closed. It is used once, to read the answers to the
/// colour query before the event reader starts.
pub type ReadStdin = Box<dyn FnMut(Duration) -> Option<Vec<u8>> + Send>;

/// What the client needs from `deck`.
pub struct Options {
    pub socket: PathBuf,
    /// The session to attach to; `None` is the most recent one.
    pub session: Option<String>,
    /// Create the session when it does not exist (`deck` without a name).
    pub create: bool,
    pub read_stdin: ReadStdin,
    /// Waits for input without reading it and tells when the terminal is gone.
    pub wait_stdin: WaitStdin,
}

impl std::fmt::Debug for Options {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Options")
            .field("socket", &self.socket)
            .field("session", &self.session)
            .finish_non_exhaustive()
    }
}

/// Why the client ended; `deck` prints the message and picks the exit code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exit {
    Detached(String),
    SessionEnded(String),
    DaemonStopped,
    /// The connection to the daemon broke.
    Lost,
    /// The daemon speaks another protocol version.
    Mismatch(u32),
    /// A `Bye { Error }` text or a setup failure.
    Error(String),
    /// `SIGTERM` or `SIGHUP`.
    Signal,
    /// The daemon hands itself over to the binary at `exe`; [`run`] executes
    /// `<exe> attach <session>` once the terminal is restored and never returns this.
    Upgrading {
        exe: PathBuf,
        session: Option<String>,
    },
    /// The daemon was upgraded and the client could not execute the new binary.
    Upgraded,
}

/// Runs the client until it ends; the outer terminal is restored when this returns.
pub fn run(options: Options) -> Exit {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => return Exit::Error(format!("cannot start the runtime: {e}")),
    };
    let exit = runtime.block_on(run_async(options));
    // The runtime and its handlers go before the process is replaced.
    drop(runtime);
    reattach(exit, exec_attach)
}

/// A client told that the daemon is upgrading comes back to its session by executing the new
/// binary (the terminal is restored by now); `exec` replaces the process and returns only
/// the error when it cannot.
fn reattach(exit: Exit, exec: impl FnOnce(&Path, Option<&str>) -> io::Error) -> Exit {
    match exit {
        Exit::Upgrading { exe, session } => {
            let _ = exec(&exe, session.as_deref());
            Exit::Upgraded
        }
        other => other,
    }
}

/// `<exe> attach <session>` (plain `<exe>` before the client had a session) in place of
/// this process, with its tty and environment.
fn exec_attach(exe: &Path, session: Option<&str>) -> io::Error {
    use std::os::unix::process::CommandExt;
    let mut command = std::process::Command::new(exe);
    if let Some(session) = session {
        command.arg("attach").arg(session);
    }
    command.exec()
}

/// Why `attach_to` did not get a session.
#[derive(Debug, PartialEq, Eq)]
enum Refused {
    Text(String),
    /// The daemon is handing itself over to this binary: come back to the session there.
    Upgrading {
        exe: PathBuf,
        session: String,
    },
}

impl Refused {
    fn text(self) -> String {
        match self {
            Self::Text(text) => text,
            Self::Upgrading { .. } => "the deck daemon is upgrading".into(),
        }
    }

    fn exit(self) -> Exit {
        match self {
            Self::Text(text) => Exit::Error(text),
            Self::Upgrading { exe, session } => Exit::Upgrading {
                exe,
                session: Some(session),
            },
        }
    }
}

/// What a `Bye` answers to an attach to `name`.
fn refusal(reason: ByeReason, name: &str) -> Refused {
    match reason {
        ByeReason::Error(text) => Refused::Text(text),
        ByeReason::Upgrading { exe } => Refused::Upgrading {
            exe,
            session: name.to_string(),
        },
        _ => Refused::Text(format!("cannot attach to {name}")),
    }
}

enum Server {
    Msg(Box<ServerMsg>),
    Closed,
}

async fn run_async(mut options: Options) -> Exit {
    // Handlers first, so a signal during set-up still restores the terminal.
    let (Ok(mut sigterm), Ok(mut sighup), Ok(mut sigwinch)) = (
        signal(SignalKind::terminate()),
        signal(SignalKind::hangup()),
        signal(SignalKind::window_change()),
    ) else {
        return Exit::Error("cannot install the signal handlers".into());
    };

    // The colour probe blocks this thread for up to 200 ms on purpose: no task exists yet,
    // so nothing else can be waiting on the runtime, and the answers must be read before
    // the event reader starts.
    let (guard, probe) = match term::Guard::enter(&mut options.read_stdin) {
        Ok(entered) => entered,
        Err(e) => {
            term::restore();
            return Exit::Error(format!("cannot set up the terminal: {e}"));
        }
    };
    let size = ct_terminal::size().unwrap_or((80, 24));
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let home = std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from);
    let mut app = App::new(&Config::default(), size, &probe.leftover).with_dirs(cwd.clone(), home);
    app.cell = measure_cell();
    app.truecolor = convert::truecolor(std::env::var("COLORTERM").ok().as_deref());
    app.outer_kitty = probe.kitty_keyboard;
    let env: std::collections::HashMap<String, String> = std::env::vars().collect();
    let plan = caps::plan(
        probe.xtversion.as_deref(),
        probe.graphics,
        &env,
        app.truecolor,
    );
    app.tier = plan.tier;
    // The font check runs on a thread; its answer upgrades the tier once, in the loop below.
    let mut font_rx = plan.check.map(|check| {
        let (tx, rx) = mpsc::unbounded_channel::<bool>();
        std::thread::spawn(move || {
            let _ = tx.send(check.run());
        });
        rx
    });
    if app.tier == tier::Tier::Image {
        app.images = Some(graphics::Images::new(
            probe.fg,
            probe.colors.map(|c| c.bg),
            probe.palette,
        ));
    }
    app.hold_until_configured();

    let ctx = Ctx {
        socket: options.socket.clone(),
        cwd,
        colors: probe.colors,
        outer_name: probe.xtversion.clone(),
    };
    let hello = ctx.hello(&app, options.session.clone(), options.create);
    let mut link = match Link::open(&ctx.socket, hello).await {
        Ok(link) => link,
        Err(e) => {
            drop(guard);
            return Exit::Error(format!("cannot connect to the deck daemon: {e}"));
        }
    };

    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<ct::Event>();
    let stop = Arc::new(AtomicBool::new(false));
    // The thread says it is done by dropping `done_tx`; the exit never waits for it for long.
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    // The main loop wakes the reader on `SIGWINCH`; see `reader::run`.
    let (wake_rx, wake_tx) = match std::os::unix::net::UnixStream::pair() {
        Ok(pair) => pair,
        Err(e) => return Exit::Error(format!("cannot start the input thread: {e}")),
    };
    let _ = wake_rx.set_nonblocking(true);
    let _ = wake_tx.set_nonblocking(true);
    let reader = {
        let stop = Arc::clone(&stop);
        let late = colors::LateAnswers::new(probe.late, Instant::now());
        let wait = options.wait_stdin;
        let stuck = std::env::var_os("DECK_TEST_STUCK_READER").is_some_and(|v| v == "1");
        std::thread::spawn(move || {
            let _done = done_tx;
            if stuck {
                loop {
                    std::thread::park();
                }
            }
            reader::run(wait, &wake_rx, late, &stop, &input_tx);
        })
    };

    let mut terminal = match Terminal::new(backend::DeckBackend::new(io::stdout())) {
        Ok(t) => t,
        Err(e) => return Exit::Error(format!("cannot start the screen: {e}")),
    };
    let ignore_sighup = std::env::var_os("DECK_TEST_IGNORE_SIGHUP").is_some_and(|v| v == "1");
    let panic_after_first_draw = std::env::var_os("DECK_TEST_PANIC").is_some_and(|v| v == "1");
    let mut shape = None;
    let mut drawn_once = false;
    // The kitty keyboard flags pushed on the outer terminal.
    let mut keyboard = 0u8;

    let mut last_draw = Instant::now() - REDRAW;
    // What was typed ahead runs once the daemon's config has arrived (`Welcome`).
    let mut early: Vec<Out> = Vec::new();

    let exit = 'run: loop {
        // The focused terminal or an overlay may have changed what the keyboard should send;
        // checked on every pass, whichever branch ended the last one.
        let wanted = app.outer_keyboard();
        if wanted != keyboard {
            term::set_keyboard(wanted);
            keyboard = wanted;
        }
        // No timer runs while the screen is idle: one is armed for the next draw (at most
        // every 16 ms) or for the end of a flash message.
        let wake = if app.is_dirty() {
            Some(last_draw + REDRAW)
        } else {
            app.wake_deadline()
        };
        let outs = if early.is_empty() {
            tokio::select! {
            event = input_rx.recv() => match event {
                Some(event) => {
                    if matches!(event, ct::Event::Resize(..)) {
                        app.cell = measure_cell();
                    }
                    app.on_event(event, Instant::now())
                }
                None => break Exit::Error("lost the terminal input".into()),
            },
            msg = link.server_rx.recv() => match msg {
                Some(Server::Msg(msg)) => app.on_server(*msg),
                Some(Server::Closed) | None => break Exit::Lost,
            },
            () = sleep_until(wake.unwrap_or_else(Instant::now).into()), if wake.is_some() => {
                let now = Instant::now();
                let outs = app.tick(now);
                if app.take_dirty() {
                    last_draw = now;
                    if let Err(e) = app.draw_frame(&mut terminal) {
                        break Exit::Error(format!("cannot draw: {e}"));
                    }
                    // What no placement used goes away.
                    if let Err(e) = app.finish_images(terminal.backend_mut()) {
                        break Exit::Error(format!("cannot draw: {e}"));
                    }
                    let wanted = app.cursor_shape();
                    if wanted != shape {
                        if let Some((s, blink)) = wanted {
                            let _ = execute!(io::stdout(), draw::cursor_style(s, blink));
                        }
                        shape = wanted;
                    }
                    #[expect(clippy::panic, reason = "the attach test verifies terminal restoration after a panic")]
                    if panic_after_first_draw && !drawn_once {
                        panic!("DECK_TEST_PANIC");
                    }
                    drawn_once = true;
                }
                outs
            },
            found = recv_font(&mut font_rx), if font_rx.is_some() => {
                font_rx = None;
                app.on_font_answer(found.unwrap_or(false));
                Vec::new()
            },
            // crossterm drops a SIGWINCH that becomes ready together with the TTY, so the
            // size is also read here; a size the app already has is not a resize.
            _ = sigwinch.recv() => {
                let _ = (&wake_tx).write(&[0]);
                match ct_terminal::size() {
                Ok((cols, rows)) if (cols, rows) != app.size => {
                    app.cell = measure_cell();
                    app.on_event(ct::Event::Resize(cols, rows), Instant::now())
                }
                // A font zoom changes the cell size and not the columns and rows.
                _ => app.set_cell(measure_cell()),
                }
            },
            _ = sigterm.recv() => break Exit::Signal,
            _ = sighup.recv(), if !ignore_sighup => break Exit::Signal,
            }
        } else {
            std::mem::take(&mut early)
        };
        for out in outs {
            match out {
                Out::Send(msg) => {
                    let _ = link.out_tx.send(msg);
                }
                Out::Switch(name) => match switch(&ctx, &mut app, &mut link, name).await {
                    Ok(()) => {}
                    Err(exit) => break 'run exit,
                },
                // The session was killed: go on in the one attached most recently, if any.
                Out::Exit(Exit::SessionEnded(ended)) => {
                    match follow(&ctx, &mut app, &mut link, &ended).await {
                        Ok(true) => {}
                        Ok(false) => break 'run Exit::SessionEnded(ended),
                        Err(exit) => break 'run exit,
                    }
                }
                Out::Clipboard(bytes) | Out::Notify(bytes) => {
                    let mut stdout = io::stdout();
                    let _ = stdout.write_all(&bytes);
                    let _ = stdout.flush();
                }
                Out::Exit(exit) => break 'run exit,
            }
        }
    };

    stop.store(true, Ordering::Release);
    // A reader that does not finish (a hung-up terminal can trap it inside crossterm) is left
    // to end with the process.
    if done_rx.recv_timeout(READER_JOIN) != Err(std::sync::mpsc::RecvTimeoutError::Timeout) {
        let _ = reader.join();
    }
    link.close().await;
    // The images go before the alternate screen does.
    let _ = app.delete_images(terminal.backend_mut());
    drop(terminal);
    drop(guard);
    exit
}

/// The outer terminal's cell size in pixels, from its pixel size (`TIOCGWINSZ`); `None` when
/// it does not report one.
fn measure_cell() -> Option<CellPx> {
    cell_of(&ct_terminal::window_size().ok()?)
}

/// The cell size of a window: its pixel width over its columns, its pixel height over its
/// rows; `None` when any of them is 0.
fn cell_of(window: &ct_terminal::WindowSize) -> Option<CellPx> {
    let size = |pixels: u16, cells: u16| pixels.checked_div(cells).filter(|n| *n > 0);
    Some(CellPx {
        width: size(window.width, window.columns)?,
        height: size(window.height, window.rows)?,
    })
}

#[cfg(test)]
mod cell_tests {
    use super::*;

    fn window(columns: u16, rows: u16, width: u16, height: u16) -> ct_terminal::WindowSize {
        ct_terminal::WindowSize {
            columns,
            rows,
            width,
            height,
        }
    }

    #[test]
    fn the_cell_size_is_the_pixel_size_over_the_cells() {
        assert_eq!(
            cell_of(&window(80, 24, 720, 432)),
            Some(CellPx {
                width: 9,
                height: 18
            })
        );
    }

    #[test]
    fn a_window_without_pixels_has_no_cell_size() {
        assert_eq!(cell_of(&window(80, 24, 0, 0)), None);
        assert_eq!(cell_of(&window(80, 24, 720, 0)), None);
        assert_eq!(cell_of(&window(0, 0, 720, 432)), None);
        assert_eq!(cell_of(&window(80, 24, 40, 20)), None);
    }

    #[test]
    fn a_cell_size_change_alone_sends_a_viewport() {
        let mut app = App::new(&Config::default(), (100, 30), &[]);
        let cell = Some(CellPx {
            width: 9,
            height: 18,
        });
        let (cols, rows) = app.viewport();
        assert_eq!(
            app.set_cell(cell),
            [app::Out::Send(ClientMsg::Viewport { cols, rows, cell })]
        );
        assert!(app.set_cell(cell).is_empty());
        assert_eq!(
            app.set_cell(None),
            [app::Out::Send(ClientMsg::Viewport {
                cols,
                rows,
                cell: None
            })]
        );
    }
}

/// The font check's answer; the channel closes after it, so it is not waited on again.
async fn recv_font(rx: &mut Option<mpsc::UnboundedReceiver<bool>>) -> Option<bool> {
    match rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

/// What a connection's `Hello` needs besides the session.
struct Ctx {
    socket: PathBuf,
    cwd: PathBuf,
    colors: Option<OuterColors>,
    /// The outer terminal's XTVERSION name.
    outer_name: Option<String>,
}

impl Ctx {
    fn hello(&self, app: &App, session: Option<String>, create: bool) -> ClientMsg {
        let (cols, rows) = app.size;
        ClientMsg::Hello {
            version: PROTOCOL_VERSION,
            role: Role::Attach {
                session,
                create,
                cols: cols.max(app::MIN_SIZE.0),
                rows: rows.max(app::MIN_SIZE.1),
                cell: app.cell,
                images: app.program_images(),
                outer_name: self.outer_name.clone(),
            },
            cwd: self.cwd.clone(),
            env: std::env::vars_os().collect(),
            colors: self.colors,
        }
    }
}

/// One connection to the daemon: what goes out and what comes in.
struct Link {
    out_tx: mpsc::UnboundedSender<ClientMsg>,
    writer: tokio::task::JoinHandle<()>,
    server_rx: mpsc::UnboundedReceiver<Server>,
}

impl Link {
    /// Connects and sends `hello`.
    async fn open(socket: &Path, hello: ClientMsg) -> io::Result<Self> {
        let stream = UnixStream::connect(socket).await?;
        let (mut read_half, mut write_half) = stream.into_split();

        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<ClientMsg>();
        let _ = out_tx.send(hello);
        let writer = tokio::spawn(async move {
            while let Some(msg) = out_rx.recv().await {
                let Ok(bytes) = encode(&msg) else { continue };
                if write_half.write_all(&bytes).await.is_err() {
                    break;
                }
            }
            let _ = write_half.shutdown().await;
        });

        let (server_tx, server_rx) = mpsc::unbounded_channel::<Server>();
        tokio::spawn(async move {
            let mut reader = FrameReader::new();
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match read_half.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let Some(bytes) = buf.get(..n) else {
                            break;
                        };
                        reader.push(bytes);
                    }
                }
                loop {
                    match reader.next_message::<ServerMsg>() {
                        Ok(Some(msg)) => {
                            if server_tx.send(Server::Msg(Box::new(msg))).is_err() {
                                return;
                            }
                        }
                        Ok(None) => break,
                        Err(_) => {
                            let _ = server_tx.send(Server::Closed);
                            return;
                        }
                    }
                }
            }
            let _ = server_tx.send(Server::Closed);
        });
        Ok(Self {
            out_tx,
            writer,
            server_rx,
        })
    }

    /// Lets the writer flush what is queued (`Detach`) and ends the connection.
    async fn close(self) {
        drop(self.out_tx);
        let _ = tokio::time::timeout(FLUSH_FOR, self.writer).await;
    }
}

/// How long a new connection may take to deliver its first view.
const ATTACH_FOR: Duration = Duration::from_secs(5);

/// Attaches to `name` on a new connection and waits for its first view, which goes to `app`.
/// `Err` is why the daemon refused (the session is gone, say) or could not be reached.
async fn attach_to(ctx: &Ctx, app: &mut App, name: &str) -> Result<Link, Refused> {
    let hello = ctx.hello(app, Some(name.to_string()), false);
    let mut link = Link::open(&ctx.socket, hello)
        .await
        .map_err(|e| Refused::Text(format!("cannot connect to the deck daemon: {e}")))?;
    let wait = async {
        loop {
            match link.server_rx.recv().await {
                Some(Server::Msg(msg)) => match *msg {
                    ServerMsg::Bye { reason } => return Err(refusal(reason, name)),
                    msg => {
                        let first = matches!(msg, ServerMsg::SessionView(_));
                        for out in app.on_server(msg) {
                            match out {
                                Out::Send(msg) => {
                                    let _ = link.out_tx.send(msg);
                                }
                                Out::Clipboard(bytes) | Out::Notify(bytes) => {
                                    let mut stdout = io::stdout();
                                    let _ = stdout.write_all(&bytes);
                                    let _ = stdout.flush();
                                }
                                _ => {}
                            }
                        }
                        if first {
                            return Ok(());
                        }
                    }
                },
                Some(Server::Closed) | None => {
                    return Err(Refused::Text(
                        "lost the connection to the deck daemon".to_string(),
                    ));
                }
            }
        }
    };
    match tokio::time::timeout(ATTACH_FOR, wait).await {
        Ok(Ok(())) => Ok(link),
        Ok(Err(text)) => Err(text),
        Err(_) => Err(Refused::Text(format!(
            "the daemon did not answer for {name}"
        ))),
    }
}

/// Moves the client to another session on a new connection, without leaving the outer
/// terminal. When the daemon refuses, it goes back to the session it left and shows why;
/// `Err` is the exit when that is not possible either.
async fn switch(ctx: &Ctx, app: &mut App, link: &mut Link, name: String) -> Result<(), Exit> {
    let previous = app.mirror.session_name().map(str::to_string);
    let _ = link.out_tx.send(ClientMsg::Detach);
    app.reset_session();
    let (new, flash) = match attach_to(ctx, app, &name).await {
        Ok(new) => (new, None),
        Err(refused @ Refused::Upgrading { .. }) => return Err(refused.exit()),
        Err(refused) => {
            let text = refused.text();
            let back = match &previous {
                Some(previous) => attach_to(ctx, app, previous).await.ok(),
                None => None,
            };
            match back {
                Some(new) => (new, Some(text)),
                None => return Err(Exit::Error(text)),
            }
        }
    };
    std::mem::replace(link, new).close().await;
    if let Some(text) = flash {
        app.set_flash(text, Instant::now(), true);
    }
    Ok(())
}

/// The session `ended` was killed: goes on in the one attached most recently that the daemon
/// accepts. `Ok(false)` when none is left.
async fn follow(ctx: &Ctx, app: &mut App, link: &mut Link, ended: &str) -> Result<bool, Exit> {
    let _ = link.out_tx.send(ClientMsg::Detach);
    app.reset_session();
    let mut skip = vec![ended.to_string()];
    let mut flash = None;
    while let Some(name) = next_session(ctx, &skip).await {
        match attach_to(ctx, app, &name).await {
            Ok(new) => {
                std::mem::replace(link, new).close().await;
                if let Some(text) = flash {
                    app.set_flash(text, Instant::now(), true);
                }
                return Ok(true);
            }
            Err(refused @ Refused::Upgrading { .. }) => return Err(refused.exit()),
            Err(refused) => {
                flash = Some(refused.text());
                skip.push(name);
            }
        }
    }
    Ok(false)
}

/// The session attached most recently, other than those in `skip`.
async fn next_session(ctx: &Ctx, skip: &[String]) -> Option<String> {
    let hello = ClientMsg::Hello {
        version: PROTOCOL_VERSION,
        role: Role::Control,
        cwd: ctx.cwd.clone(),
        env: std::env::vars_os().collect(),
        colors: None,
    };
    let ask = async {
        let mut link = Link::open(&ctx.socket, hello).await.ok()?;
        link.out_tx
            .send(ClientMsg::Request(Request::ListSessions))
            .ok()?;
        loop {
            match link.server_rx.recv().await? {
                Server::Msg(msg) => {
                    if let ServerMsg::Response(Response::Sessions(list)) = *msg {
                        return list
                            .into_iter()
                            .filter(|s| !skip.contains(&s.name))
                            .max_by_key(|s| s.last_attached)
                            .map(|s| s.name);
                    }
                }
                Server::Closed => return None,
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(2), ask)
        .await
        .ok()
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::{ByeReason, Exit, Path, Refused, reattach, refusal};

    #[test]
    fn an_upgrading_client_executes_the_new_binary_on_its_session() {
        let mut ran = None;
        let exit = reattach(
            Exit::Upgrading {
                exe: "/opt/deck/bin/deck".into(),
                session: Some("work".into()),
            },
            |exe: &Path, session: Option<&str>| {
                ran = Some((exe.to_path_buf(), session.map(str::to_string)));
                std::io::Error::other("no exec in a test")
            },
        );
        assert_eq!(
            ran,
            Some(("/opt/deck/bin/deck".into(), Some("work".to_string())))
        );
        // The exec failed: the client ends and says how to come back.
        assert_eq!(exit, Exit::Upgraded);
    }

    #[test]
    fn an_upgrading_client_without_a_session_executes_the_plain_binary() {
        let mut ran = None;
        reattach(
            Exit::Upgrading {
                exe: "/opt/deck/bin/deck".into(),
                session: None,
            },
            |exe: &Path, session: Option<&str>| {
                ran = Some((exe.to_path_buf(), session.map(str::to_string)));
                std::io::Error::other("no exec in a test")
            },
        );
        assert_eq!(ran, Some(("/opt/deck/bin/deck".into(), None)));
    }

    #[test]
    fn an_upgrade_during_a_switch_reattaches_to_the_target_session() {
        let upgrading = ByeReason::Upgrading {
            exe: "/opt/deck/bin/deck".into(),
        };
        let refused = refusal(upgrading, "other");
        assert_eq!(
            refused,
            Refused::Upgrading {
                exe: "/opt/deck/bin/deck".into(),
                session: "other".into()
            }
        );
        assert_eq!(
            refused.exit(),
            Exit::Upgrading {
                exe: "/opt/deck/bin/deck".into(),
                session: Some("other".into())
            }
        );
        assert_eq!(
            refusal(ByeReason::SessionEnded, "x"),
            Refused::Text("cannot attach to x".into())
        );
    }

    #[test]
    fn other_exits_do_not_execute_anything() {
        let exit = reattach(Exit::DaemonStopped, |_: &Path, _: Option<&str>| {
            unreachable!("no exec")
        });
        assert_eq!(exit, Exit::DaemonStopped);
    }

    /// A font check whose thread ended without an answer reads as "found nothing", and the
    /// loop's arm is then switched off.
    #[tokio::test]
    async fn recv_font_gives_none_for_a_dropped_sender() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<bool>();
        let mut rx = Some(rx);
        tx.send(true).unwrap();
        assert_eq!(super::recv_font(&mut rx).await, Some(true));
        drop(tx);
        let found = super::recv_font(&mut rx).await;
        assert_eq!(found, None);
        assert!(!found.unwrap_or(false));
    }

    /// The icon glyphs of both tiers: literals that belong to `deck-core`'s icon table.
    const GLYPHS: &[&str] = &[
        "▾", "▸", "⠋", "⠹", // The `unicode` icons.
        "✻", "◎", "◇", "❯", "✎", "±", "◔", "⇄", "▲", "¶", "λ",
        // The `nerd` icons: Codicons for the agents, `▣` for OpenCode, Font Awesome for the rest.
        "\u{ec82}", "\u{ec81}", "▣", "\u{f120}", "\u{f040}", "\u{f126}", "\u{f0e4}", "\u{f0ec}",
        "\u{f013}", "\u{f02d}", "\u{f121}", // The other Nerd icons and the drop target.
        "\u{eab4}", "\u{eab6}", "\u{f0da}", "➤", "\u{ee06}", "\u{ee0b}", "\u{f06a}", "\u{f058}",
        "\u{f10c}", "\u{f444}", "↻", "\u{ea60}", "\u{ea82}",
    ];

    /// Colours and theme glyphs of deck's own UI come from the theme: no `Color` constructor
    /// outside the conversion module and the backend (which writes resets) and no icon glyph outside `deck-core`.
    #[test]
    fn no_colour_or_theme_glyph_literals_in_the_drawing_code() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let constructor = ["Color", "::"].concat();
        let mut checked = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if !name.ends_with(".rs")
                || name.ends_with("_tests.rs")
                || name == "convert.rs"
                || name == "backend.rs"
            {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let code = text.split("#[cfg(test)]\nmod tests").next().unwrap();
            assert!(!code.contains(&constructor), "{name} builds a colour");
            assert!(!code.contains("Color as"), "{name} aliases Color");
            for glyph in GLYPHS.iter().copied() {
                assert!(!code.contains(glyph), "{name} has the glyph {glyph}");
            }
            checked += 1;
        }
        assert!(checked >= 8);
    }
}
