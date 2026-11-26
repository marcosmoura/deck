//! The end-to-end harness: the real `deck` (or any program) in a PTY, mirrored in an
//! alacritty_terminal emulator, on a temp `DECK_HOME` that is cleaned up on drop.

#![allow(dead_code)]
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "shared integration-test helpers require a working PTY and screen"
)]

pub mod compat;
pub mod record;
pub mod sys;

use std::ffi::OsString;
use std::fs::{self, File, TryLockError};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
pub use alacritty_terminal::vte::ansi::Color;
use alacritty_terminal::vte::ansi::{Processor, Rgb};
use portable_pty::{Child, CommandBuilder, ExitStatus, MasterPty, PtySize, native_pty_system};

/// The default theme's `accent`: the focused border.
pub const ACCENT: Color = Color::Indexed(4);

/// Marks a config key to be left out.
const UNSET: &str = "\0unset";

pub const TIMEOUT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(10);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
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

/// A temp `DECK_HOME` with its config and the environment every process of a test gets.
/// Dropping it stops the daemon with `deck kill-server`.
#[derive(Debug)]
pub struct Home {
    dir: Arc<tempfile::TempDir>,
    env: Vec<(OsString, OsString)>,
    /// False for an [`Home::alias`], which leaves the daemon running when it is dropped.
    owner: bool,
}

impl Home {
    pub fn new() -> Self {
        Self::with(&[], &[])
    }

    /// `config` overrides or adds top-level config keys (the value is a TOML literal);
    /// `env` sets a variable, or removes it with `None`.
    pub fn with(config: &[(&str, &str)], env: &[(&str, Option<&str>)]) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("dk")
            .tempdir_in(short_tmp())
            .expect("temp DECK_HOME");
        let mut keys: Vec<(String, String)> = vec![
            ("shell".into(), "\"/bin/sh\"".into()),
            ("shell_args".into(), "[]".into()),
            // The sidebar is off unless a test turns it on (`config_unset("sidebar")` brings
            // back the product default), so the panel area keeps the whole window.
            ("sidebar".into(), "false".into()),
            // Tests never show a real desktop notification: the terminal's OSC 9 unless a test
            // asks for deck's own, with a fake notifier on its `PATH`.
            ("notify".into(), "\"terminal\"".into()),
        ];
        for (key, value) in config {
            if *value == UNSET {
                keys.retain(|(k, _)| k != key);
                continue;
            }
            match keys.iter_mut().find(|(k, _)| k == key) {
                Some(slot) => slot.1 = (*value).into(),
                None => keys.push(((*key).into(), (*value).into())),
            }
        }
        let text: String = keys.iter().map(|(k, v)| format!("{k} = {v}\n")).collect();
        fs::write(dir.path().join("config.toml"), text).expect("config.toml");

        // The environment of this process, minus `ENV` and every `DECK*`.
        let mut vars: Vec<(OsString, OsString)> = std::env::vars_os()
            .filter(|(k, _)| k != "ENV" && !k.to_string_lossy().starts_with("DECK"))
            .collect();
        let mut set = |key: &str, value: Option<&str>| {
            vars.retain(|(k, _)| k != key);
            if let Some(value) = value {
                vars.push((key.into(), value.into()));
            }
        };
        set("PS1", Some("$ "));
        set("COLORTERM", Some("truecolor"));
        set("TERM", Some("xterm-256color"));
        // The terminal of the developer running the tests must not change the results (the
        // client draws logos as images in Ghostty and kitty); a test opts in with `.env(...)`.
        for key in [
            "TERM_PROGRAM",
            "KITTY_WINDOW_ID",
            "GHOSTTY_RESOURCES_DIR",
            "WEZTERM_EXECUTABLE",
            "TMUX",
            "ZELLIJ",
            "DECK",
        ] {
            set(key, None);
        }
        // The fonts installed on a developer's machine must not change the results either: the
        // client skips its font checks (`defaults`, `fc-list`).
        set("DECK_TEST_NO_FONT_CHECK", Some("1"));
        set("DECK_HOME", Some(&dir.path().to_string_lossy()));
        set(
            "DECK_TEST_PARENT_PID",
            Some(&std::process::id().to_string()),
        );
        for (key, value) in env {
            set(key, *value);
        }
        Self {
            dir: Arc::new(dir),
            env: vars,
            owner: true,
        }
    }

    /// The same directory and environment, as a second handle: dropping it stops nothing
    /// (a harness that is dropped between runs on one daemon and one state).
    pub fn alias(&self) -> Self {
        Self {
            dir: Arc::clone(&self.dir),
            env: self.env.clone(),
            owner: false,
        }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub fn socket(&self) -> PathBuf {
        self.path().join("run").join("deck.sock")
    }

    pub fn lock_file(&self) -> PathBuf {
        self.path().join("run").join("deck.lock")
    }

    pub fn log_file(&self) -> PathBuf {
        self.path().join("state").join("daemon.log")
    }

    /// True while a daemon holds `deck.lock`. It takes the lock itself for an instant to
    /// find out, so do not call it while a daemon is starting.
    pub fn daemon_running(&self) -> bool {
        lock_held(&self.lock_file())
    }

    /// The pid the daemon wrote into `deck.lock`.
    pub fn daemon_pid(&self) -> Option<i32> {
        fs::read_to_string(self.lock_file())
            .ok()?
            .trim()
            .parse()
            .ok()
    }

    /// The environment of the processes of this home.
    pub fn env(&self) -> &[(OsString, OsString)] {
        &self.env
    }

    /// `deck <args>`, with the home's environment and stdin on `/dev/null`.
    pub fn deck_command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_deck"));
        cmd.args(args)
            .env_clear()
            .envs(self.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null());
        cmd
    }

    /// Runs `deck <args>` to the end.
    pub fn run_deck(&self, args: &[&str]) -> Output {
        self.deck_command(args).output().expect("run deck")
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        // Without its `DECK_HOME`, `kill-server` would reach the user's own daemon.
        if !self.owner || !self.env.iter().any(|(k, _)| k == "DECK_HOME") {
            return;
        }
        let _ = self.deck_command(&["kill-server"]).output();
    }
}

/// True while some process holds the lock file.
pub fn lock_held(path: &Path) -> bool {
    let Ok(file) = File::open(path) else {
        return false;
    };
    matches!(file.try_lock(), Err(TryLockError::WouldBlock))
}

pub fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

pub fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Waits until `done` holds; panics with `what` after `limit`.
pub fn wait_until(what: &str, limit: Duration, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + limit;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(POLL);
    }
}

/// Whether the terminal is in canonical (cooked) mode or not (raw).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TtyMode {
    Cooked,
    Raw,
}

/// One cell of the mirrored screen.
#[derive(Debug, Clone, PartialEq)]
pub struct CellView {
    pub ch: char,
    /// The combining characters after `ch` (a placeholder's diacritics).
    pub zerowidth: Vec<char>,
    pub fg: Color,
    pub bg: Color,
    pub flags: Flags,
    /// The colour of the underline (SGR 58), if one was set.
    pub underline_color: Option<Color>,
}

/// The kind of underline of a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Underline {
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

impl CellView {
    /// The base character and its combining marks.
    pub fn grapheme(&self) -> String {
        std::iter::once(self.ch)
            .chain(self.zerowidth.iter().copied())
            .collect()
    }

    /// The kind of underline, if the cell has one.
    pub fn underline_kind(&self) -> Option<Underline> {
        [
            (Flags::UNDERLINE, Underline::Single),
            (Flags::DOUBLE_UNDERLINE, Underline::Double),
            (Flags::UNDERCURL, Underline::Curly),
            (Flags::DOTTED_UNDERLINE, Underline::Dotted),
            (Flags::DASHED_UNDERLINE, Underline::Dashed),
        ]
        .into_iter()
        .find(|(flag, _)| self.flags.contains(*flag))
        .map(|(_, kind)| kind)
    }

    pub fn bold(&self) -> bool {
        self.flags.contains(Flags::BOLD)
    }

    pub fn italic(&self) -> bool {
        self.flags.contains(Flags::ITALIC)
    }

    pub fn underline(&self) -> bool {
        self.flags.intersects(Flags::ALL_UNDERLINES)
    }

    pub fn inverse(&self) -> bool {
        self.flags.contains(Flags::INVERSE)
    }
}

#[derive(Clone, Default)]
struct Listener(Arc<Mutex<Vec<Event>>>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        lock(&self.0).push(event);
    }
}

struct Size {
    cols: usize,
    rows: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

struct Mirror {
    term: Term<Listener>,
    raw: Vec<u8>,
    eof: bool,
}

type Writer = Arc<Mutex<Box<dyn Write + Send>>>;

/// The screen of the harness's terminal, as a watch's test sees it.
pub struct Screen<'a> {
    term: &'a Term<Listener>,
}

impl Screen<'_> {
    /// The character at `row`, `col`.
    pub fn char_at(&self, row: usize, col: usize) -> char {
        let line = Line(i32::try_from(row).unwrap_or(i32::MAX));
        self.term.grid()[line][Column(col)].c
    }

    /// One row of the screen as text, trailing spaces removed.
    pub fn row_text(&self, row: usize) -> String {
        let line = Line(i32::try_from(row).unwrap_or(i32::MAX));
        let grid = self.term.grid();
        let text: String = (0..self.term.columns())
            .map(|col| &grid[line][Column(col)])
            .filter(|cell| !cell.flags.contains(Flags::WIDE_CHAR_SPACER))
            .map(|cell| cell.c)
            .collect();
        text.trim_end().to_string()
    }

    /// The cursor's row.
    pub fn cursor_row(&self) -> usize {
        usize::try_from(self.term.grid().cursor.point.line.0).unwrap_or(0)
    }
}

/// When a watch fired, both instants taken in the PTY reader thread right after `read`
/// returned, so the polling period of the test thread is not in them.
#[derive(Debug, Clone, Copy)]
pub struct Seen {
    /// The first bytes that arrived after the watch was armed, whatever they were.
    pub first_output: Instant,
    /// The bytes after which the watched condition held.
    pub shown: Instant,
}

struct Watch {
    when: Box<dyn Fn(&Screen<'_>) -> bool + Send>,
    first_output: Option<Instant>,
    tx: Sender<Seen>,
}

/// Per switch to the alternate screen: when it arrived and when a `$` first showed after it.
type ScreenEntries = Vec<(Instant, Option<Instant>)>;

type WatchSlot = Arc<Mutex<Option<Watch>>>;

/// The receiving end of [`Harness::watch`] and [`Harness::watch_cell`].
pub struct CellWatch(Receiver<Seen>);

impl CellWatch {
    /// Waits up to `limit` for the condition; `None` when it did not hold in time.
    pub fn wait(&self, limit: Duration) -> Option<Seen> {
        self.0.recv_timeout(limit).ok()
    }
}

/// How the harness's terminal answers a program's OSC 10/11 colour queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorAnswers {
    /// Light grey on black.
    Default,
    /// This foreground and background, as 8-bit channels.
    Fixed((u8, u8, u8), (u8, u8, u8)),
    /// No answer to the colour queries (the device attributes are answered).
    Silent,
    /// No answer to anything: the colour queries and the device attributes.
    Mute,
}

/// Builds a [`Harness`] with extra config keys and environment variables.
#[derive(Debug)]
pub struct Builder {
    size: (u16, u16),
    config: Vec<(String, String)>,
    env: Vec<(String, Option<String>)>,
    colors: ColorAnswers,
    cell_px: (u16, u16),
    xtversion: Option<String>,
    kitty_keyboard: bool,
}

impl Builder {
    /// Sets a top-level config key; the value is a TOML literal, e.g. `"\"zsh\""`.
    pub fn config(mut self, key: &str, value: &str) -> Self {
        self.config.push((key.into(), value.into()));
        self
    }

    /// Leaves a config key out, so the program's default applies (the harness writes
    /// `shell_args = []` otherwise).
    pub fn config_unset(self, key: &str) -> Self {
        self.config(key, UNSET)
    }

    pub fn env(mut self, key: &str, value: &str) -> Self {
        self.env.push((key.into(), Some(value.into())));
        self
    }

    pub fn env_remove(mut self, key: &str) -> Self {
        self.env.push((key.into(), None));
        self
    }

    /// Makes the terminal answer colour queries with these colours.
    pub fn colors(mut self, fg: (u8, u8, u8), bg: (u8, u8, u8)) -> Self {
        self.colors = ColorAnswers::Fixed(fg, bg);
        self
    }

    /// Gives the PTY a pixel size: every cell is `width` by `height` pixels (0 by default,
    /// like a terminal that does not report one).
    pub fn cell_px(mut self, width: u16, height: u16) -> Self {
        self.cell_px = (width, height);
        self
    }

    /// Makes the outer terminal one that speaks the kitty graphics protocol, in the way
    /// the client detects: Ghostty's `TERM`, 24-bit colour and a cell of 8×16 pixels unless
    /// [`Builder::cell_px`] says otherwise.
    pub fn kitty(mut self) -> Self {
        self.env.push(("TERM".into(), Some("xterm-ghostty".into())));
        if self.cell_px == (0, 0) {
            self.cell_px = (8, 16);
        }
        self
    }

    /// Makes the terminal answer XTVERSION (`CSI > 0 q`) with this name, like Ghostty
    /// (`ghostty 1.2.0`); by default it does not answer.
    pub fn xtversion(mut self, name: &str) -> Self {
        self.xtversion = Some(name.into());
        self
    }

    /// Makes the outer terminal one that speaks the kitty keyboard protocol: it answers
    /// the client's `CSI ? u` and keeps the flag stack the client pushes (see
    /// [`Harness::keyboard_flags`]). Off by default, like a terminal without it.
    pub fn kitty_keyboard(mut self) -> Self {
        self.kitty_keyboard = true;
        self
    }

    /// Makes the terminal answer nothing: no colours, no device attributes.
    pub fn mute(mut self) -> Self {
        self.colors = ColorAnswers::Mute;
        self
    }

    /// Makes the terminal ignore colour queries.
    pub fn silent_colors(mut self) -> Self {
        self.colors = ColorAnswers::Silent;
        self
    }

    pub fn deck(self) -> Harness {
        self.spawn(&[env!("CARGO_BIN_EXE_deck")])
    }

    /// The built `deck` on an existing home (its config and environment stay; this
    /// builder's are ignored).
    pub fn deck_in(self, home: Home) -> Harness {
        Harness::start(
            self.size,
            &[env!("CARGO_BIN_EXE_deck")],
            home,
            self.colors,
            self.cell_px,
            self.xtversion,
            self.kitty_keyboard,
        )
    }

    pub fn spawn(self, argv: &[&str]) -> Harness {
        let config: Vec<(&str, &str)> = self
            .config
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let env: Vec<(&str, Option<&str>)> = self
            .env
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_deref()))
            .collect();
        Harness::start(
            self.size,
            argv,
            Home::with(&config, &env),
            self.colors,
            self.cell_px,
            self.xtversion,
            self.kitty_keyboard,
        )
    }
}

/// A program in a PTY of a fixed size, with its output mirrored in an emulator.
pub struct Harness {
    /// `None` after [`Harness::close_master`].
    master: Option<Box<dyn MasterPty + Send>>,
    child: Box<dyn Child + Send + Sync>,
    writer: Writer,
    /// Stops the PTY reader thread; see [`Harness::close_master`].
    stop_reader: Arc<AtomicBool>,
    reader: Option<JoinHandle<()>>,
    mirror: Arc<Mutex<Mirror>>,
    watch: WatchSlot,
    /// When the program was spawned, and for each switch to the alternate screen the
    /// instant it arrived and the instant a `$` first showed on the screen after it (both
    /// taken in the reader thread).
    spawned: Instant,
    marks: Arc<Mutex<ScreenEntries>>,
    /// Bytes the program wrote so far (all of them, whether or not `raw` keeps them).
    written: Arc<AtomicUsize>,
    size: (u16, u16),
    cell_px: (u16, u16),
    // Last: the home stops the daemon after the child is gone.
    home: Home,
}

impl std::fmt::Debug for Harness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Harness")
            .field("size", &self.size)
            .field("home", &self.home)
            .finish_non_exhaustive()
    }
}

impl Harness {
    pub fn builder(size: (u16, u16)) -> Builder {
        Builder {
            size,
            config: Vec::new(),
            env: Vec::new(),
            colors: ColorAnswers::Default,
            cell_px: (0, 0),
            xtversion: None,
            kitty_keyboard: false,
        }
    }

    /// The built `deck`, in a PTY of `size` (columns, rows).
    pub fn deck(size: (u16, u16)) -> Self {
        Self::builder(size).deck()
    }

    /// Any program; `argv[0]` is the program.
    pub fn spawn(size: (u16, u16), argv: &[&str]) -> Self {
        Self::builder(size).spawn(argv)
    }

    fn start(
        size: (u16, u16),
        argv: &[&str],
        home: Home,
        answers: ColorAnswers,
        cell_px: (u16, u16),
        xtversion: Option<String>,
        kitty_keyboard: bool,
    ) -> Self {
        // The system has a limited number of PTYs; a heavily parallel run can find them all
        // taken for a moment.
        let mut attempts = 0;
        let pty = loop {
            match native_pty_system().openpty(pty_size(size, cell_px)) {
                Ok(pty) => break pty,
                Err(e) if attempts >= 50 => panic!("open a PTY: {e}"),
                Err(_) => {
                    attempts += 1;
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        };
        let mut cmd = CommandBuilder::new(argv[0]);
        cmd.args(&argv[1..]);
        cmd.env_clear();
        for (k, v) in home.env() {
            cmd.env(k, v);
        }
        cmd.cwd(home.path());
        let spawned = Instant::now();
        let child = pty.slave.spawn_command(cmd).expect("spawn the program");
        drop(pty.slave);

        // A terminal that names itself and has the Ghostty `TERM` also answers the graphics
        // query `OK`, as Ghostty does.
        let graphics_ok = xtversion.is_some()
            && home
                .env()
                .iter()
                .any(|(k, v)| k == "TERM" && v.to_string_lossy().contains("ghostty"));
        let events = Listener::default();
        let term = Term::new(
            Config {
                kitty_keyboard,
                ..Config::default()
            },
            &Size {
                cols: usize::from(size.0),
                rows: usize::from(size.1),
            },
            events.clone(),
        );
        let mirror = Arc::new(Mutex::new(Mirror {
            term,
            raw: Vec::new(),
            eof: false,
        }));
        let writer: Writer = Arc::new(Mutex::new(pty.master.take_writer().expect("PTY writer")));
        let mut reader = pty.master.try_clone_reader().expect("PTY reader");
        let master_fd = pty.master.as_raw_fd().expect("the PTY's descriptor");
        let stop_reader = Arc::new(AtomicBool::new(false));
        let watch: WatchSlot = Arc::new(Mutex::new(None));
        let marks = Arc::new(Mutex::new(Vec::new()));
        let written = Arc::new(AtomicUsize::new(0));
        let reader_thread = {
            let (mirror, writer) = (Arc::clone(&mirror), Arc::clone(&writer));
            let (watch, written) = (Arc::clone(&watch), Arc::clone(&written));
            let marks = Arc::clone(&marks);
            let stop = Arc::clone(&stop_reader);
            std::thread::spawn(move || {
                let mut parser: Processor = Processor::new();
                let mut buf = [0u8; 16 * 1024];
                loop {
                    // The master stays open until this thread is joined, so its descriptor
                    // number is safe to poll; a stop flag is looked at every 50 ms.
                    if stop.load(Ordering::Acquire) {
                        return;
                    }
                    if !sys::wait_readable(master_fd, 50) {
                        continue;
                    }
                    let n = match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    // Taken before anything else is done with the bytes.
                    let arrived = Instant::now();
                    written.fetch_add(n, Ordering::Relaxed);
                    {
                        let mut m = lock(&mirror);
                        m.raw.extend_from_slice(&buf[..n]);
                        parser.advance(&mut m.term, &buf[..n]);
                        {
                            let mut marks = lock(&marks);
                            if buf[..n].windows(8).any(|w| w == b"\x1b[?1049h") {
                                marks.push((arrived, None));
                            }
                            if let Some(last) = marks.last_mut()
                                && last.1.is_none()
                                && (0..m.term.screen_lines())
                                    .any(|r| Screen { term: &m.term }.row_text(r).contains('$'))
                            {
                                last.1 = Some(arrived);
                            }
                        }
                        let mut slot = lock(&watch);
                        if let Some(w) = slot.as_mut() {
                            let first_output = *w.first_output.get_or_insert(arrived);
                            if (w.when)(&Screen { term: &m.term }) {
                                let _ = w.tx.send(Seen {
                                    first_output,
                                    shown: arrived,
                                });
                                *slot = None;
                            }
                        }
                    }
                    // Answer the queries the program makes of its terminal.
                    let replies: Vec<String> = lock(&events.0)
                        .drain(..)
                        .filter_map(|event| match event {
                            Event::PtyWrite(text) => {
                                (answers != ColorAnswers::Mute).then_some(text)
                            }
                            Event::ColorRequest(index, format) => {
                                let (fg, bg) = match answers {
                                    ColorAnswers::Silent | ColorAnswers::Mute => return None,
                                    ColorAnswers::Default => ((0xcc, 0xcc, 0xcc), (0, 0, 0)),
                                    ColorAnswers::Fixed(fg, bg) => (fg, bg),
                                };
                                let (r, g, b) = if index == 256 { fg } else { bg };
                                Some(format(Rgb { r, g, b }))
                            }
                            _ => None,
                        })
                        .collect();
                    let mut replies = replies;
                    if let Some(name) = &xtversion
                        && buf[..n]
                            .windows(4)
                            .any(|w| w == b"\x1b[>q" || w == b"\x1b[>0")
                    {
                        // A real terminal answers in order: XTVERSION before the DA1 that
                        // follows it in the client's query.
                        let at = replies
                            .iter()
                            .position(|r| r.starts_with("\x1b[?"))
                            .unwrap_or(replies.len());
                        replies.insert(at, format!("\x1bP>|{name}\x1b\\"));
                    }
                    if graphics_ok && buf[..n].windows(7).any(|w| w == b"\x1b_Gi=31") {
                        let at = replies
                            .iter()
                            .position(|r| r.starts_with("\x1b[?"))
                            .unwrap_or(replies.len());
                        replies.insert(at, "\x1b_Gi=31;OK\x1b\\".into());
                    }
                    for reply in replies {
                        let _ = lock(&writer).write_all(reply.as_bytes());
                    }
                }
                lock(&mirror).eof = true;
            })
        };
        Self {
            master: Some(pty.master),
            child,
            writer,
            stop_reader,
            reader: Some(reader_thread),
            mirror,
            watch,
            spawned,
            marks,
            written,
            size,
            cell_px,
            home,
        }
    }

    /// When the program was spawned.
    pub fn spawned_at(&self) -> Instant {
        self.spawned
    }

    /// Every switch to the alternate screen so far: when it arrived, and when a `$` first
    /// showed on the screen after it.
    pub fn screen_entries(&self) -> Vec<(Instant, Option<Instant>)> {
        lock(&self.marks).clone()
    }

    pub fn home(&self) -> &Home {
        &self.home
    }

    /// The pid of the program in the PTY.
    pub fn pid(&self) -> u32 {
        self.child.process_id().expect("the program's pid")
    }

    /// Sends `SIGTERM` to the program.
    pub fn terminate(&self) {
        sys::terminate(i32::try_from(self.pid()).expect("a pid"));
    }

    pub fn send_keys(&mut self, keys: &str) {
        self.send_bytes(keys.as_bytes());
    }

    pub fn send_bytes(&mut self, bytes: &[u8]) {
        let mut writer = lock(&self.writer);
        writer.write_all(bytes).expect("write to the PTY");
        writer.flush().expect("flush the PTY");
    }

    /// Writes `bytes` on a thread of its own; `false` when the write has not finished after
    /// `limit` (deck stopped reading its input), so the test fails instead of hanging.
    pub fn send_bytes_within(&mut self, bytes: &[u8], limit: Duration) -> bool {
        let writer = Arc::clone(&self.writer);
        let bytes = bytes.to_vec();
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut writer = lock(&writer);
            let written = writer.write_all(&bytes).and_then(|()| writer.flush());
            let _ = done.send(written.is_ok());
        });
        finished.recv_timeout(limit).unwrap_or(false)
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.size = (cols, rows);
        // The mirror stays locked until the PTY is resized too, so the reader never applies
        // output for the new size to the old grid.
        let mut mirror = lock(&self.mirror);
        mirror.term.resize(Size {
            cols: usize::from(cols),
            rows: usize::from(rows),
        });
        self.master
            .as_ref()
            .expect("the PTY is open")
            .resize(pty_size(self.size, self.cell_px))
            .expect("resize");
    }

    /// Waits until `text` is on the screen; panics with the screen after `limit`.
    pub fn wait_for_text(&self, text: &str, limit: Duration) {
        let deadline = Instant::now() + limit;
        while !self.screen_text().contains(text) {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {text:?}; the screen is:\n{}",
                self.screen_text()
            );
            std::thread::sleep(POLL);
        }
    }

    /// Waits until a row of the screen starts with `prefix`; unlike `wait_for_text`, this
    /// skips a command line that echoes the text. A prompt before the text is allowed: a
    /// line typed before the shell printed its prompt has its output on the prompt's row,
    /// which starts wherever the last command left the cursor.
    pub fn wait_for_row_start(&self, prefix: &str, limit: Duration) {
        let deadline = Instant::now() + limit;
        let starts = |r: &str| {
            let after_prompt = r.trim_start().strip_prefix("$ ");
            r.starts_with(prefix) || after_prompt.is_some_and(|rest| rest.starts_with(prefix))
        };
        while !self.screen_text().lines().any(starts) {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for a row starting with {prefix:?}; the screen is:\n{}",
                self.screen_text()
            );
            std::thread::sleep(POLL);
        }
    }

    /// Waits for the program to exit.
    pub fn wait_for_exit(&mut self, limit: Duration) -> ExitStatus {
        let deadline = Instant::now() + limit;
        loop {
            if let Some(status) = self.child.try_wait().expect("wait for the program") {
                self.drain();
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "the program did not exit; the screen is:\n{}",
                self.screen_text()
            );
            std::thread::sleep(POLL);
        }
    }

    /// After the program exited: its last output may still be in the PTY. Waits for the
    /// end of the output, or until nothing came for 100 ms (another process can hold the
    /// PTY open), at most 2 s.
    fn drain(&self) {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut len = lock(&self.mirror).raw.len();
        let mut quiet_since = Instant::now();
        while Instant::now() < deadline {
            let m = lock(&self.mirror);
            if m.eof {
                return;
            }
            if m.raw.len() != len {
                len = m.raw.len();
                quiet_since = Instant::now();
            } else if quiet_since.elapsed() >= Duration::from_millis(100) {
                return;
            }
            drop(m);
            std::thread::sleep(POLL);
        }
    }

    /// The screen, one line per row, trailing spaces removed.
    pub fn screen_text(&self) -> String {
        let mirror = lock(&self.mirror);
        let grid = mirror.term.grid();
        let mut lines = Vec::new();
        for row in 0..mirror.term.screen_lines() {
            let mut line = String::new();
            for col in 0..mirror.term.columns() {
                let cell = &grid[Line(i32::try_from(row).unwrap_or(i32::MAX))][Column(col)];
                if !cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    line.push(cell.c);
                }
            }
            lines.push(line.trim_end().to_string());
        }
        lines.join("\n")
    }

    /// The scrollback and the screen, one line per row, trailing spaces removed, and the
    /// empty rows at the end dropped: what `deck capture --history` prints for the other side.
    pub fn history_text(&self) -> String {
        let mirror = lock(&self.mirror);
        let grid = mirror.term.grid();
        let top = grid.topmost_line().0;
        let bottom = i32::try_from(mirror.term.screen_lines()).unwrap_or(i32::MAX);
        let mut lines = Vec::new();
        for row in top..bottom {
            let mut line = String::new();
            for col in 0..mirror.term.columns() {
                let cell = &grid[Line(row)][Column(col)];
                if !cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    line.push(cell.c);
                    line.extend(cell.zerowidth().into_iter().flatten());
                }
            }
            lines.push(line.trim_end().to_string());
        }
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        lines.join("\n")
    }

    /// Whether the sidebar row `row` has the selection band (reversed with the default
    /// colours): the active panel's row outside sidebar focus.
    pub fn is_active_row(&self, row: usize) -> bool {
        self.cell(row, 0).inverse()
    }

    /// Waits for the band on a row that starts with `prefix`.
    pub fn wait_for_active_row(&self, prefix: &str, limit: Duration) {
        let deadline = Instant::now() + limit;
        loop {
            let screen = self.screen_text();
            if screen
                .lines()
                .enumerate()
                .any(|(y, r)| r.starts_with(prefix) && self.is_active_row(y))
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for the band on a row starting with {prefix:?}; the screen is:\n{screen}"
            );
            std::thread::sleep(POLL);
        }
    }

    pub fn cell(&self, row: usize, col: usize) -> CellView {
        let mirror = lock(&self.mirror);
        let cell = &mirror.term.grid()[Line(i32::try_from(row).expect("row"))][Column(col)];
        CellView {
            ch: cell.c,
            zerowidth: cell.zerowidth().map(<[char]>::to_vec).unwrap_or_default(),
            fg: cell.fg,
            bg: cell.bg,
            flags: cell.flags,
            underline_color: cell.underline_color(),
        }
    }

    /// Arms a watch that fires on the first bytes after which `when` holds for the screen.
    /// The instants are taken in the PTY reader thread (see [`Seen`]); arm it before the
    /// keys that should cause the change. One watch at a time: a new one replaces the old.
    pub fn watch(&self, when: impl Fn(&Screen<'_>) -> bool + Send + 'static) -> CellWatch {
        let (tx, rx) = channel();
        *lock(&self.watch) = Some(Watch {
            when: Box::new(when),
            first_output: None,
            tx,
        });
        CellWatch(rx)
    }

    /// [`Harness::watch`] for the cell at `row`, `col` to show `ch`.
    pub fn watch_cell(&self, row: usize, col: usize, ch: char) -> CellWatch {
        self.watch(move |screen| screen.char_at(row, col) == ch)
    }

    /// How many bytes the program wrote to its terminal so far.
    pub fn output_bytes(&self) -> usize {
        self.written.load(Ordering::Relaxed)
    }

    /// The screen's size in columns and rows.
    pub fn size(&self) -> (u16, u16) {
        self.size
    }

    /// The cursor's row and column, and whether it is shown.
    pub fn cursor(&self) -> (usize, usize, bool) {
        let mirror = lock(&self.mirror);
        let point = mirror.term.grid().cursor.point;
        let row = usize::try_from(point.line.0).unwrap_or(0);
        let shown = mirror.term.mode().contains(TermMode::SHOW_CURSOR);
        (row, point.column.0, shown)
    }

    /// The cursor's shape and whether it blinks, as text (`"Beam blinking"`).
    pub fn cursor_style(&self) -> String {
        let style = lock(&self.mirror).term.cursor_style();
        format!(
            "{:?} {}",
            style.shape,
            if style.blinking { "blinking" } else { "steady" }
        )
    }

    /// The kitty keyboard flags in force on the screen now (1 disambiguate, 2 event types,
    /// 4 alternate keys, 8 all keys, 16 text), as the client pushed them; 0 without
    /// [`Builder::kitty_keyboard`].
    pub fn keyboard_flags(&self) -> u8 {
        let mode = *lock(&self.mirror).term.mode();
        [
            (TermMode::DISAMBIGUATE_ESC_CODES, 1),
            (TermMode::REPORT_EVENT_TYPES, 2),
            (TermMode::REPORT_ALTERNATE_KEYS, 4),
            (TermMode::REPORT_ALL_KEYS_AS_ESC, 8),
            (TermMode::REPORT_ASSOCIATED_TEXT, 16),
        ]
        .iter()
        .filter(|(flag, _)| mode.contains(*flag))
        .fold(0, |acc, (_, bit)| acc | bit)
    }

    /// Every byte the program wrote so far.
    pub fn raw_output(&self) -> Vec<u8> {
        lock(&self.mirror).raw.clone()
    }

    /// Cooked while the terminal is in canonical mode, raw otherwise.
    pub fn tty_mode(&self) -> TtyMode {
        let fd = self
            .master
            .as_ref()
            .expect("the PTY is open")
            .as_raw_fd()
            .expect("the PTY's descriptor");
        if sys::canonical(fd).expect("tcgetattr") {
            TtyMode::Cooked
        } else {
            TtyMode::Raw
        }
    }

    /// Closes the PTY master, as closing a terminal's tab does: the program's terminal hangs
    /// up. The PTY reader thread (which holds a clone of the master and the writer) is
    /// stopped and joined first, and then the harness's master and writer go. The child
    /// handle stays, for `pid` and `wait_for_exit`.
    pub fn close_master(&mut self) {
        self.stop_reader.store(true, Ordering::Release);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        *lock(&self.writer) = Box::new(std::io::sink());
        self.master = None;
    }

    /// Runs `deck <args>` on this harness's `DECK_HOME`.
    pub fn run_deck(&self, args: &[&str]) -> Output {
        self.home.run_deck(args)
    }

    /// Writes `screen_text()` to `target/e2e-captures/<test>/<name>.txt` and returns the path.
    pub fn capture(&self, name: &str) -> PathBuf {
        let test = std::thread::current()
            .name()
            .filter(|n| *n != "main")
            .map_or_else(
                || {
                    std::env::current_exe()
                        .ok()
                        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
                        .unwrap_or_else(|| "test".into())
                },
                str::to_string,
            )
            .replace("::", "__");
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join("e2e-captures")
            .join(test);
        fs::create_dir_all(&dir).expect("capture directory");
        let path = dir.join(format!("{name}.txt"));
        // Written aside then renamed: another run of the same test (a second suite in this
        // checkout) may be reading it.
        let tmp = dir.join(format!("{name}.txt.{}.tmp", std::process::id()));
        fs::write(&tmp, self.screen_text() + "\n").expect("capture file");
        fs::rename(&tmp, &path).expect("capture file");
        path
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        // The reader keeps draining the PTY: an exiting program waits for its output to be read.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn pty_size((cols, rows): (u16, u16), (cw, ch): (u16, u16)) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: cols.saturating_mul(cw),
        pixel_height: rows.saturating_mul(ch),
    }
}
