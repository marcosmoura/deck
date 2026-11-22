//! The attach client: screen mirror, rendering, input handling and key bindings.

mod app;
mod colors;
mod draw;
mod input;
mod mirror;
mod term;

use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use deck_core::config::Config;
use deck_core::protocol::{ClientMsg, FrameReader, PROTOCOL_VERSION, Role, ServerMsg, encode};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event as ct;
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal as ct_terminal;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;
use tokio::time::sleep_until;

use crate::app::{App, Out, panel_area};

/// The redraw timer: at most one draw per tick, and only when something changed.
const REDRAW: Duration = Duration::from_millis(16);
/// How often the input thread looks at its stop flag.
const INPUT_POLL: Duration = Duration::from_millis(50);
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
    pub config: Config,
    pub read_stdin: ReadStdin,
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
    runtime.block_on(run_async(options))
}

enum Server {
    Msg(Box<ServerMsg>),
    Closed,
}

async fn run_async(mut options: Options) -> Exit {
    // Handlers first, so a signal during set-up still restores the terminal.
    let (Ok(mut sigterm), Ok(mut sighup)) = (
        signal(SignalKind::terminate()),
        signal(SignalKind::hangup()),
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
    let program = program_name(&options.config);
    let mut app = App::new(&options.config, size, program, &probe.leftover);

    let stream = match UnixStream::connect(&options.socket).await {
        Ok(stream) => stream,
        Err(e) => {
            drop(guard);
            return Exit::Error(format!("cannot connect to the deck daemon: {e}"));
        }
    };
    let (mut read_half, mut write_half) = stream.into_split();

    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<ClientMsg>();
    let hello = {
        let (cols, rows) = panel_area(size);
        ClientMsg::Hello {
            version: PROTOCOL_VERSION,
            role: Role::Attach {
                session: options.session.clone(),
                create: options.create,
                cols: cols.max(app::MIN_SIZE.0),
                rows: rows.max(app::MIN_SIZE.1 - 1),
            },
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
            env: std::env::vars_os().collect(),
            colors: probe.colors,
        }
    };
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

    let (server_tx, mut server_rx) = mpsc::unbounded_channel::<Server>();
    tokio::spawn(async move {
        let mut reader = FrameReader::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match read_half.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => reader.push(&buf[..n]),
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

    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<ct::Event>();
    let stop = Arc::new(AtomicBool::new(false));
    let reader = {
        let stop = Arc::clone(&stop);
        let mut late = colors::LateAnswers::new(probe.late, Instant::now());
        std::thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                let events = match ct::poll(INPUT_POLL) {
                    Ok(true) => match ct::read() {
                        Ok(event) => late.feed(event, Instant::now()),
                        Err(_) => break,
                    },
                    Ok(false) => late.flush(Instant::now()),
                    Err(_) => break,
                };
                if events.into_iter().any(|e| input_tx.send(e).is_err()) {
                    break;
                }
            }
        })
    };

    let mut terminal = match Terminal::new(CrosstermBackend::new(io::stdout())) {
        Ok(t) => t,
        Err(e) => return Exit::Error(format!("cannot start the screen: {e}")),
    };
    let panic_after_first_draw = std::env::var_os("DECK_TEST_PANIC").is_some_and(|v| v == "1");
    let mut shape = None;
    let mut drawn_once = false;

    let mut last_draw = Instant::now() - REDRAW;
    let mut early = app.start(Instant::now());

    let exit = 'run: loop {
        // No timer runs while the screen is idle: one is armed for the next draw (at most
        // every 16 ms) or for the end of a flash message.
        let wake = if app.is_dirty() {
            Some(last_draw + REDRAW)
        } else {
            app.flash_deadline()
        };
        let outs = if early.is_empty() {
            tokio::select! {
            event = input_rx.recv() => match event {
                Some(event) => app.on_event(event, Instant::now()),
                None => break Exit::Error("lost the terminal input".into()),
            },
            msg = server_rx.recv() => match msg {
                Some(Server::Msg(msg)) => app.on_server(*msg),
                Some(Server::Closed) | None => break Exit::Lost,
            },
            () = sleep_until(wake.unwrap_or_else(Instant::now).into()), if wake.is_some() => {
                let now = Instant::now();
                app.tick(now);
                if app.take_dirty() {
                    last_draw = now;
                    if let Err(e) = terminal.draw(|f| draw::draw(f, &app)) {
                        break Exit::Error(format!("cannot draw: {e}"));
                    }
                    let wanted = app.cursor_shape();
                    if wanted != shape {
                        if let Some(s) = wanted {
                            let _ = execute!(io::stdout(), draw::cursor_style(s));
                        }
                        shape = wanted;
                    }
                    if panic_after_first_draw && !drawn_once {
                        panic!("DECK_TEST_PANIC");
                    }
                    drawn_once = true;
                }
                continue;
            },
            _ = sigterm.recv() => break Exit::Signal,
            _ = sighup.recv() => break Exit::Signal,
            }
        } else {
            std::mem::take(&mut early)
        };
        for out in outs {
            match out {
                Out::Send(msg) => {
                    let _ = out_tx.send(msg);
                }
                Out::Clipboard(bytes) => {
                    let mut stdout = io::stdout();
                    let _ = stdout.write_all(&bytes);
                    let _ = stdout.flush();
                }
                Out::Exit(exit) => break 'run exit,
            }
        }
    };

    stop.store(true, Ordering::Release);
    let _ = reader.join();
    drop(out_tx);
    let _ = tokio::time::timeout(FLUSH_FOR, writer).await;
    drop(terminal);
    drop(guard);
    exit
}

/// The name of the program a new terminal runs: the shell's basename.
fn program_name(config: &Config) -> String {
    let shell = config
        .shell
        .clone()
        .or_else(|| std::env::var("SHELL").ok())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/bin/sh".to_string());
    shell
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("sh")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_program_is_the_shells_basename() {
        let config = Config::parse("shell = \"/usr/local/bin/fish\"")
            .unwrap()
            .config;
        assert_eq!(program_name(&config), "fish");
    }
}
