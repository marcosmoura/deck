//! The commands that talk to the daemon: start-up, `ls`, `kill-server`, `_probe` and the
//! guards of attaching.

use std::fs::{self, File, TryLockError};
use std::io::{self, IsTerminal, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow, bail};
use deck_core::protocol::{
    ByeReason, ClientMsg, FrameReader, PROTOCOL_VERSION, Request, Response, Role, ServerMsg,
    SessionSummary, encode,
};
use deck_daemon::{DaemonPaths, check_runtime_dir, saved_session_count};

use crate::platform;

/// How long the launcher waits for a daemon it started to accept connections.
const START_TIMEOUT: Duration = Duration::from_secs(3);
/// How long `kill-server` waits for the daemon to answer before it sends `SIGTERM`.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(1);
/// How long `kill-server` waits for the lock to be released.
const STOP_TIMEOUT: Duration = Duration::from_secs(5);
/// Read timeout of commands that expect an answer from a healthy daemon.
pub(crate) const REPLY_TIMEOUT: Duration = Duration::from_secs(5);
/// Most daemons one launcher starts while it waits for a socket.
const START_ATTEMPTS: u32 = 20;
const RESTART_BACKOFF: Duration = Duration::from_millis(100);
const POLL: Duration = Duration::from_millis(20);

/// The terminal size `_probe` attaches with; it never shows a screen.
const PROBE_SIZE: (u16, u16) = (80, 24);

fn daemon_paths() -> anyhow::Result<DaemonPaths> {
    let env = |name: &str| std::env::var(name).ok();
    Ok(DaemonPaths::from_env(&env, platform::effective_uid())?)
}

/// The paths, after the runtime directory check every connecting command makes.
pub(crate) fn checked_paths() -> anyhow::Result<DaemonPaths> {
    let paths = daemon_paths()?;
    check_runtime_dir(&paths.runtime_dir, platform::effective_uid())?;
    Ok(paths)
}

/// The refusals that come before anything else when attaching.
pub fn attach_guards() -> anyhow::Result<()> {
    if std::env::var_os("DECK").is_some() {
        bail!("already inside deck (unset DECK to nest)");
    }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("attach needs a terminal (stdin and stdout must be a TTY)");
    }
    Ok(())
}

/// `deck` and `deck attach`: the guards, the start-up, then the attach client.
///
/// Without a name the client attaches to the most recent session or creates `main`; with a
/// name the session must exist, unless `create` asks for it to be created.
pub fn attach(session: Option<String>, create: bool) -> anyhow::Result<ExitCode> {
    attach_guards()?;
    let paths = checked_paths()?;
    ensure_daemon(&paths)?;
    let create = create || session.is_none();
    run_client(paths, session, create)
}

/// Runs the attach client on `session`.
fn run_client(
    paths: DaemonPaths,
    session: Option<String>,
    create: bool,
) -> anyhow::Result<ExitCode> {
    let exit = deck_tui::run(deck_tui::Options {
        socket: paths.socket,
        create,
        session,
        read_stdin: Box::new(platform::read_stdin),
    });
    finish(exit)
}

/// `deck new`: creates a session, then attaches to it unless `detached`.
pub fn new_session(
    name: Option<String>,
    dir: Option<PathBuf>,
    detached: bool,
) -> anyhow::Result<ExitCode> {
    if !detached {
        attach_guards()?;
    }
    let cwd = std::env::current_dir().context("cannot read the current directory")?;
    let dir = match dir {
        Some(dir) => std::path::absolute(&dir)
            .with_context(|| format!("cannot resolve {}", dir.display()))?,
        None => cwd,
    };
    let paths = checked_paths()?;
    ensure_daemon(&paths)?;
    let mut conn = Conn::open(&paths.socket, REPLY_TIMEOUT).context("cannot connect")?;
    let created = match conn.control(Request::NewSession { name, cwd: dir }) {
        Outcome::Response(Response::Session { name, .. }) => name,
        Outcome::Response(Response::Error(text)) => bail!("{text}"),
        Outcome::Response(_) => bail!("unexpected answer from the daemon"),
        Outcome::Mismatch(daemon) => bail!("{}", mismatch_text(daemon)),
        Outcome::Other(e) => return Err(e),
    };
    drop(conn);
    if detached {
        println!("{created}");
        return Ok(ExitCode::SUCCESS);
    }
    run_client(paths, Some(created), false)
}

/// Sends a request to a daemon that must already run: `kill-session` and `rename-session`
/// never start one.
fn session_request(request: Request) -> anyhow::Result<ExitCode> {
    let paths = checked_paths()?;
    let Ok(mut conn) = Conn::open(&paths.socket, REPLY_TIMEOUT) else {
        eprintln!("no deck daemon running");
        return Ok(ExitCode::FAILURE);
    };
    match conn.control(request) {
        Outcome::Response(Response::Ok) => Ok(ExitCode::SUCCESS),
        Outcome::Response(Response::Error(text)) => bail!("{text}"),
        Outcome::Response(_) => bail!("unexpected answer from the daemon"),
        Outcome::Mismatch(daemon) => bail!("{}", mismatch_text(daemon)),
        Outcome::Other(e) => Err(e),
    }
}

/// `deck kill-session`.
pub fn kill_session(name: String) -> anyhow::Result<ExitCode> {
    session_request(Request::KillSession { name })
}

/// `deck rename-session`.
pub fn rename_session(old: String, new: String) -> anyhow::Result<ExitCode> {
    session_request(Request::RenameSession { from: old, to: new })
}

/// Prints the message that ends an attach, now that the terminal is restored, and picks
/// the exit code.
fn finish(exit: deck_tui::Exit) -> anyhow::Result<ExitCode> {
    use deck_tui::Exit;
    match exit {
        Exit::Detached(name) => println!("[detached from {name}]"),
        Exit::SessionEnded(name) => println!("[session {name} ended]"),
        Exit::DaemonStopped => println!("[deck daemon stopped]"),
        Exit::Lost => {
            eprintln!("[lost connection to the deck daemon]");
            return Ok(ExitCode::FAILURE);
        }
        Exit::Mismatch(daemon) => bail!("{}", mismatch_text(daemon)),
        Exit::Error(text) => bail!("{text}"),
        Exit::Signal => return Ok(ExitCode::FAILURE),
    }
    Ok(ExitCode::SUCCESS)
}

/// A connection to the daemon.
pub(crate) struct Conn {
    stream: UnixStream,
    reader: FrameReader,
    /// The `Hello` of a control connection was sent.
    said_hello: bool,
}

/// What the daemon answered to a control request.
pub(crate) enum Outcome {
    Response(Response),
    Mismatch(u32),
    /// Anything else: a timeout, a closed connection, another `Bye`.
    Other(anyhow::Error),
}

impl Conn {
    pub(crate) fn open(socket: &Path, timeout: Duration) -> io::Result<Self> {
        let stream = UnixStream::connect(socket)?;
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        Ok(Self {
            stream,
            reader: FrameReader::new(),
            said_hello: false,
        })
    }

    /// Lets reads wait without limit (for the answer to a `wait`).
    pub(crate) fn no_read_timeout(&self) -> io::Result<()> {
        self.stream.set_read_timeout(None)
    }

    fn send(&mut self, msg: &ClientMsg) -> anyhow::Result<()> {
        self.stream.write_all(&encode(msg)?)?;
        Ok(())
    }

    fn hello(&mut self, role: Role) -> anyhow::Result<()> {
        self.send(&ClientMsg::Hello {
            version: PROTOCOL_VERSION,
            role,
            // A directory that was removed: `/`, as the attach client does.
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
            env: std::env::vars_os().collect(),
            colors: None,
        })
    }

    /// The next message; `None` when the daemon closed the connection.
    fn recv(&mut self) -> anyhow::Result<Option<ServerMsg>> {
        let mut buf = [0u8; 16 * 1024];
        loop {
            if let Some(msg) = self.reader.next_message::<ServerMsg>()? {
                return Ok(Some(msg));
            }
            match self.stream.read(&mut buf) {
                Ok(0) => return Ok(None),
                Ok(n) => {
                    let bytes = buf.get(..n).context("read exceeds the buffer length")?;
                    self.reader.push(bytes);
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    bail!("the daemon did not answer");
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Says hello as a control client (once per connection), sends `request` and waits for
    /// the answer.
    pub(crate) fn control(&mut self, request: Request) -> Outcome {
        let hello = if self.said_hello {
            Ok(())
        } else {
            self.said_hello = true;
            self.hello(Role::Control)
        };
        let sent = hello.and_then(|()| self.send(&ClientMsg::Request(request)));
        if let Err(e) = sent {
            return Outcome::Other(e);
        }
        loop {
            match self.recv() {
                Ok(Some(ServerMsg::Response(r))) => return Outcome::Response(r),
                Ok(Some(ServerMsg::Bye {
                    reason: ByeReason::VersionMismatch { daemon },
                })) => return Outcome::Mismatch(daemon),
                Ok(Some(ServerMsg::Bye { reason })) => {
                    return Outcome::Other(anyhow!(bye_text(&reason)));
                }
                Ok(Some(_)) => {}
                Ok(None) => return Outcome::Other(anyhow!("the daemon closed the connection")),
                Err(e) => return Outcome::Other(e),
            }
        }
    }
}

pub(crate) fn bye_text(reason: &ByeReason) -> String {
    match reason {
        ByeReason::Error(text) => text.clone(),
        ByeReason::SessionEnded => "the session ended".into(),
        ByeReason::ServerShutdown => "deck daemon stopped".into(),
        ByeReason::VersionMismatch { daemon } => mismatch_text(*daemon),
    }
}

pub(crate) fn mismatch_text(daemon: u32) -> String {
    format!(
        "the daemon speaks protocol {daemon}, this deck speaks {PROTOCOL_VERSION}; \
         run \"deck kill-server\" to restart it; sessions are restored when the daemon saved them"
    )
}

/// What the daemon logs when `state.json` comes from a newer deck.
const NEWER_STATE: &str = "state.json was written by a newer deck";

/// The line a daemon logged after byte `from` of its log that says `state.json` is newer.
fn newer_state_line(log: &Path, from: u64) -> Option<String> {
    let bytes = fs::read(log).ok()?;
    // A log that is now shorter than before was rotated: read it from the start.
    let from = usize::try_from(from)
        .ok()
        .filter(|from| *from <= bytes.len())
        .unwrap_or(0);
    let fresh = bytes.get(from..)?;
    String::from_utf8_lossy(fresh)
        .lines()
        .rev()
        .find_map(|line| line.split_once(NEWER_STATE))
        .map(|(_, rest)| format!("{NEWER_STATE}{rest}"))
}

/// Whether a daemon answers on the socket. A daemon that is shutting down does not: it says
/// `Bye`, or the connection breaks when it stops listening.
fn daemon_answers(paths: &DaemonPaths) -> bool {
    // Within the start-up deadline, so a daemon that never answers cannot stall it.
    let Ok(mut conn) = Conn::open(&paths.socket, START_TIMEOUT) else {
        return false;
    };
    // A daemon of another version answers too; the command that follows says so.
    matches!(
        conn.control(Request::ListSessions),
        Outcome::Response(_) | Outcome::Mismatch(_)
    )
}

/// Connects to the daemon, starting it when none answers on the socket.
///
/// A daemon this launcher spawns can exit at once: it lost the race for the lock to another
/// starter, or an old daemon still holds the lock while it shuts down. So the launcher keeps
/// polling the socket until the deadline and, once its daemon is gone, starts another after
/// a short back-off (a few times at most).
fn ensure_daemon(paths: &DaemonPaths) -> anyhow::Result<()> {
    if daemon_answers(paths) {
        return Ok(());
    }
    let exe = std::env::current_exe().context("cannot find the deck executable")?;
    let deadline = Instant::now() + START_TIMEOUT;
    let log_start = fs::metadata(&paths.log_file).map_or(0, |m| m.len());
    let mut attempts = 0;
    let mut exited: Option<Arc<AtomicBool>> = None;
    loop {
        if daemon_answers(paths) {
            return Ok(());
        }
        let gone = exited.as_ref().is_none_or(|e| e.load(Ordering::Acquire));
        // Our daemon is gone: it may have refused a state file written by a newer deck.
        if gone
            && attempts > 0
            && let Some(line) = newer_state_line(&paths.log_file, log_start)
        {
            bail!("{line}");
        }
        if Instant::now() >= deadline {
            bail!(
                "could not start the daemon; see {}",
                paths.log_file.display()
            );
        }
        if gone && attempts < START_ATTEMPTS {
            if attempts > 0 {
                std::thread::sleep(RESTART_BACKOFF);
                if daemon_answers(paths) {
                    return Ok(());
                }
            }
            attempts += 1;
            exited = Some(
                platform::detach_spawn(Command::new(&exe).arg("daemon"))
                    .context("cannot start the daemon")?,
            );
        }
        std::thread::sleep(POLL);
    }
}

/// `deck _probe`: starts the daemon if needed, attaches, prints the session name and the
/// protocol version, detaches.
pub fn probe(session: Option<String>) -> anyhow::Result<ExitCode> {
    let paths = checked_paths()?;
    ensure_daemon(&paths)?;
    let mut conn = Conn::open(&paths.socket, REPLY_TIMEOUT).context("cannot connect")?;
    conn.hello(Role::Attach {
        session,
        create: true,
        cols: PROBE_SIZE.0,
        rows: PROBE_SIZE.1,
    })?;
    let (mut welcomed, mut name) = (false, None);
    while !(welcomed && name.is_some()) {
        match conn.recv()? {
            Some(ServerMsg::Welcome { .. }) => welcomed = true,
            Some(ServerMsg::SessionView(view)) => name = Some(view.name),
            Some(ServerMsg::Bye { reason }) => bail!("{}", bye_text(&reason)),
            Some(_) => {}
            None => bail!("the daemon closed the connection"),
        }
    }
    conn.send(&ClientMsg::Detach)?;
    println!("{}", name.unwrap_or_default());
    println!("protocol {PROTOCOL_VERSION}");
    Ok(ExitCode::SUCCESS)
}

/// The daemon's socket for the commands that run inside deck too: `DECK_SOCKET` when set,
/// else the path of the environment (`DECK_HOME` or the XDG directories).
pub(crate) fn control_socket(paths: &DaemonPaths) -> PathBuf {
    std::env::var_os("DECK_SOCKET")
        .filter(|s| !s.is_empty())
        .map_or_else(|| paths.socket.clone(), PathBuf::from)
}

/// `deck ls`, of the session `only` when given.
pub fn ls(only: Option<&str>) -> anyhow::Result<ExitCode> {
    let paths = checked_paths()?;
    let Ok(mut conn) = Conn::open(&control_socket(&paths), REPLY_TIMEOUT) else {
        match saved_session_count(&paths) {
            0 => println!("no sessions"),
            n => println!(
                "no daemon running; {n} saved {} will be restored by \"deck\"",
                if n == 1 { "session" } else { "sessions" }
            ),
        }
        return Ok(ExitCode::SUCCESS);
    };
    match conn.control(Request::ListSessions) {
        Outcome::Response(Response::Sessions(mut sessions)) => {
            if let Some(name) = only {
                sessions.retain(|s| s.name == name);
            }
            if sessions.is_empty() {
                println!("no sessions");
            } else {
                print!("{}", session_lines(&sessions));
            }
        }
        Outcome::Response(Response::Error(text)) => bail!("{text}"),
        Outcome::Response(_) => bail!("unexpected answer from the daemon"),
        Outcome::Mismatch(daemon) => bail!("{}", mismatch_text(daemon)),
        Outcome::Other(e) => return Err(e),
    }
    Ok(ExitCode::SUCCESS)
}

/// One line per session, in columns.
fn session_lines(sessions: &[SessionSummary]) -> String {
    let count = |n: u32, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let rows: Vec<(&str, String, String, bool)> = sessions
        .iter()
        .map(|s| {
            (
                s.name.as_str(),
                count(s.projects, "space", "spaces"),
                count(s.terminals, "terminal", "terminals"),
                s.attached_clients > 0,
            )
        })
        .collect();
    let width = rows.iter().map(|r| r.0.len()).max().unwrap_or(0);
    let pw = rows.iter().map(|r| r.1.len()).max().unwrap_or(0);
    let mut out = String::new();
    for (name, projects, terminals, attached) in rows {
        let attached = if attached { "  (attached)" } else { "" };
        out.push_str(&format!(
            "{name:<width$}  {projects:<pw$}  {terminals}{attached}\n"
        ));
    }
    out
}

/// True while a daemon holds `deck.lock`.
fn lock_held(lock: &Path) -> bool {
    let Ok(file) = File::open(lock) else {
        return false;
    };
    matches!(file.try_lock(), Err(TryLockError::WouldBlock))
}

/// Waits until the lock is released; false when it is still held at the deadline.
fn wait_unlocked(lock: &Path, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    while lock_held(lock) {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(POLL);
    }
    true
}

/// The pid in the lock file; the daemon writes it just after taking the lock, so retry
/// briefly. Never a pid that `kill` would treat as a group (0 or below).
fn read_pid(lock: &Path) -> Option<i32> {
    let deadline = Instant::now() + ANSWER_TIMEOUT;
    loop {
        let pid = fs::read_to_string(lock)
            .ok()
            .and_then(|text| text.trim().parse::<i32>().ok())
            .filter(|pid| *pid > 0);
        if pid.is_some() || Instant::now() >= deadline {
            return pid;
        }
        std::thread::sleep(POLL);
    }
}

/// Deletes the saved state and a leftover temp file.
fn forget_state(paths: &DaemonPaths) -> anyhow::Result<()> {
    let state = paths.state_file();
    let mut tmp = state.clone().into_os_string();
    tmp.push(".tmp");
    for file in [state, PathBuf::from(tmp)] {
        match fs::remove_file(&file) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => bail!("cannot delete {}: {e}", file.display()),
        }
    }
    Ok(())
}

/// `deck kill-server`; with `forget`, the saved state goes too.
pub fn kill_server(forget: bool) -> anyhow::Result<ExitCode> {
    let paths = checked_paths()?;
    if !lock_held(&paths.lock) {
        if forget {
            forget_state(&paths)?;
            println!("state cleared");
        } else {
            println!("no deck daemon running");
        }
        return Ok(ExitCode::SUCCESS);
    }
    // The daemon may still be binding its socket: retry until the answer timeout.
    let deadline = Instant::now() + ANSWER_TIMEOUT;
    let outcome = loop {
        match Conn::open(&paths.socket, ANSWER_TIMEOUT) {
            Ok(mut conn) => break conn.control(Request::KillServer),
            Err(e) if Instant::now() >= deadline => break Outcome::Other(e.into()),
            Err(_) => std::thread::sleep(POLL),
        }
    };
    match outcome {
        Outcome::Response(Response::Ok) => {}
        Outcome::Response(Response::Error(text)) => bail!("{text}"),
        // The daemon cannot be asked: it takes the clean shutdown path on `SIGTERM`.
        Outcome::Response(_) | Outcome::Mismatch(_) | Outcome::Other(_) => {
            if lock_held(&paths.lock) {
                let pid = read_pid(&paths.lock)
                    .context("cannot read the daemon's pid from the lock file")?;
                if let Err(e) = platform::terminate(pid)
                    && e.raw_os_error() != Some(libc::ESRCH)
                {
                    bail!("cannot stop the daemon (pid {pid}): {e}");
                }
            }
        }
    }
    if !wait_unlocked(&paths.lock, STOP_TIMEOUT) {
        bail!("the daemon did not stop; see {}", paths.log_file.display());
    }
    if forget {
        forget_state(&paths)?;
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use deck_core::model::SessionId;

    #[test]
    fn sessions_are_listed_in_columns() {
        let summary = |name: &str, attached, projects, terminals| SessionSummary {
            id: SessionId(1),
            name: name.into(),
            attached_clients: attached,
            terminals,
            projects,
            last_attached: 0,
        };
        let text = session_lines(&[summary("main", 1, 1, 1), summary("work-long", 0, 2, 3)]);
        assert_eq!(
            text,
            "main       1 space   1 terminal  (attached)\nwork-long  2 spaces  3 terminals\n"
        );
    }

    #[test]
    fn a_missing_or_free_lock_is_not_held() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("deck.lock");
        assert!(!lock_held(&lock));
        let file = File::create(&lock).unwrap();
        assert!(!lock_held(&lock));
        file.lock().unwrap();
        assert!(lock_held(&lock));
    }
}
