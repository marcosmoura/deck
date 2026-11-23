//! The end-to-end harness: the real `deck` (or any program) in a PTY, mirrored in an
//! alacritty_terminal emulator, on a temp `DECK_HOME` that is cleaned up on drop.

#![allow(dead_code)]
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "shared integration-test helpers require a working PTY and screen"
)]

pub mod sys;

use std::ffi::OsString;
use std::fs::{self, File, TryLockError};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::{Color, Processor, Rgb};
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
    dir: tempfile::TempDir,
    env: Vec<(OsString, OsString)>,
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
        for key in ["TERM_PROGRAM", "KITTY_WINDOW_ID", "TMUX", "ZELLIJ", "DECK"] {
            set(key, None);
        }
        set("DECK_HOME", Some(&dir.path().to_string_lossy()));
        set(
            "DECK_TEST_PARENT_PID",
            Some(&std::process::id().to_string()),
        );
        for (key, value) in env {
            set(key, *value);
        }
        Self { dir, env: vars }
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
        if !self.env.iter().any(|(k, _)| k == "DECK_HOME") {
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
    pub fg: Color,
    pub bg: Color,
    pub flags: Flags,
}

impl CellView {
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

/// How the harness's terminal answers a program's OSC 10/11 colour queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorAnswers {
    /// Light grey on black.
    Default,
    /// This foreground and background, as 8-bit channels.
    Fixed((u8, u8, u8), (u8, u8, u8)),
    /// No answer at all, like a terminal that does not know the query.
    Silent,
}

/// Builds a [`Harness`] with extra config keys and environment variables.
#[derive(Debug)]
pub struct Builder {
    size: (u16, u16),
    config: Vec<(String, String)>,
    env: Vec<(String, Option<String>)>,
    colors: ColorAnswers,
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

    /// Makes the terminal ignore colour queries.
    pub fn silent_colors(mut self) -> Self {
        self.colors = ColorAnswers::Silent;
        self
    }

    pub fn deck(self) -> Harness {
        self.spawn(&[env!("CARGO_BIN_EXE_deck")])
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
        Harness::start(self.size, argv, Home::with(&config, &env), self.colors)
    }
}

/// A program in a PTY of a fixed size, with its output mirrored in an emulator.
pub struct Harness {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    writer: Writer,
    mirror: Arc<Mutex<Mirror>>,
    size: (u16, u16),
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

    fn start(size: (u16, u16), argv: &[&str], home: Home, answers: ColorAnswers) -> Self {
        let pty = native_pty_system()
            .openpty(pty_size(size))
            .expect("open a PTY");
        let mut cmd = CommandBuilder::new(argv[0]);
        cmd.args(&argv[1..]);
        cmd.env_clear();
        for (k, v) in home.env() {
            cmd.env(k, v);
        }
        cmd.cwd(home.path());
        let child = pty.slave.spawn_command(cmd).expect("spawn the program");
        drop(pty.slave);

        let events = Listener::default();
        let term = Term::new(
            Config::default(),
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
        {
            let (mirror, writer) = (Arc::clone(&mirror), Arc::clone(&writer));
            std::thread::spawn(move || {
                let mut parser: Processor = Processor::new();
                let mut buf = [0u8; 16 * 1024];
                loop {
                    let n = match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    {
                        let mut m = lock(&mirror);
                        m.raw.extend_from_slice(&buf[..n]);
                        parser.advance(&mut m.term, &buf[..n]);
                    }
                    // Answer the queries the program makes of its terminal.
                    let replies: Vec<String> = lock(&events.0)
                        .drain(..)
                        .filter_map(|event| match event {
                            Event::PtyWrite(text) => Some(text),
                            Event::ColorRequest(index, format) => {
                                let (fg, bg) = match answers {
                                    ColorAnswers::Silent => return None,
                                    ColorAnswers::Default => ((0xcc, 0xcc, 0xcc), (0, 0, 0)),
                                    ColorAnswers::Fixed(fg, bg) => (fg, bg),
                                };
                                let (r, g, b) = if index == 256 { fg } else { bg };
                                Some(format(Rgb { r, g, b }))
                            }
                            _ => None,
                        })
                        .collect();
                    for reply in replies {
                        let _ = lock(&writer).write_all(reply.as_bytes());
                    }
                }
                lock(&mirror).eof = true;
            });
        }
        Self {
            master: pty.master,
            child,
            writer,
            mirror,
            size,
            home,
        }
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

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.size = (cols, rows);
        // The mirror stays locked until the PTY is resized too, so the reader never applies
        // output for the new size to the old grid.
        let mut mirror = lock(&self.mirror);
        mirror.term.resize(Size {
            cols: usize::from(cols),
            rows: usize::from(rows),
        });
        self.master.resize(pty_size(self.size)).expect("resize");
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
    /// skips a command line that echoes the text.
    pub fn wait_for_row_start(&self, prefix: &str, limit: Duration) {
        let deadline = Instant::now() + limit;
        while !self.screen_text().lines().any(|r| r.starts_with(prefix)) {
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
            fg: cell.fg,
            bg: cell.bg,
            flags: cell.flags,
        }
    }

    /// Every byte the program wrote so far.
    pub fn raw_output(&self) -> Vec<u8> {
        lock(&self.mirror).raw.clone()
    }

    /// Cooked while the terminal is in canonical mode, raw otherwise.
    pub fn tty_mode(&self) -> TtyMode {
        let fd = self.master.as_raw_fd().expect("the PTY's descriptor");
        if sys::canonical(fd).expect("tcgetattr") {
            TtyMode::Cooked
        } else {
            TtyMode::Raw
        }
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
        fs::write(&path, self.screen_text() + "\n").expect("capture file");
        path
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn pty_size((cols, rows): (u16, u16)) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}
