//! The daemon: socket server, sessions, PTYs, terminal emulation, frames, persistence and
//! the status of the coding agents running in terminals.

mod agents;
mod clock;
mod config_file;
mod conn;
mod emu;
mod env;
mod error;
mod images;
mod persist;
mod platform;
mod queue;
mod server;
mod startup;
mod terminal;

use std::fs;
use std::future::Future;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use deck_core::config::Config;
use deck_core::model::TerminalId;
use deck_core::protocol::{NoticeLevel, PROTOCOL_VERSION};
use tokio::net::UnixListener;

use config_file::{Change, ConfigFile};

pub use clock::{Clock, Drain, Fire, ManualClock, Poke, SystemClock, Throttle};
pub use config_file::Notice;
pub use error::DaemonError;
pub use persist::saved_session_count;
pub use platform::process_cwd;
pub use startup::{DaemonPaths, check_runtime_dir, effective_uid};

/// Locks a mutex whose holders never leave data half-written, so poisoning is ignored.
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The environment variable that makes the daemon report another protocol version, so
/// tests can run a daemon "of another version".
pub const TEST_PROTOCOL_VERSION_VAR: &str = "DECK_TEST_PROTOCOL_VERSION";

/// The hidden variable that sets how often the config file is looked at, in milliseconds.
pub const TEST_CONFIG_POLL_VAR: &str = "DECK_TEST_CONFIG_POLL_MS";

/// The hidden variable that sets how often terminals' directories are asked of the
/// operating system, in milliseconds.
pub const TEST_CWD_POLL_VAR: &str = "DECK_TEST_CWD_POLL_MS";

/// How often the directories of the terminals that do not report them are looked up.
const CWD_POLL: Duration = Duration::from_secs(5);

/// How the daemon asks for the directory of a process: the operating system, unless a test
/// puts something else in its place.
#[derive(Clone)]
pub struct CwdQuery(Arc<dyn Fn(u32) -> Option<PathBuf> + Send + Sync>);

impl CwdQuery {
    pub fn new(query: impl Fn(u32) -> Option<PathBuf> + Send + Sync + 'static) -> Self {
        Self(Arc::new(query))
    }

    pub(crate) fn ask(&self, pid: u32) -> Option<PathBuf> {
        (self.0)(pid)
    }
}

impl Default for CwdQuery {
    fn default() -> Self {
        Self::new(process_cwd)
    }
}

impl std::fmt::Debug for CwdQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CwdQuery").finish_non_exhaustive()
    }
}

/// The hidden variable that sets how often terminals' foreground processes are looked at
/// for coding agents, in milliseconds.
pub const TEST_AGENT_POLL_VAR: &str = "DECK_TEST_AGENT_POLL_MS";

/// Shortens the interval of the stats log line, in milliseconds.
pub const TEST_STATS_LOG_VAR: &str = "DECK_TEST_STATS_LOG_MS";

/// The hidden variable that makes the daemon leak one file descriptor for every terminal it
/// starts (set it to `1`), so the stress suite can check that it catches a leak.
pub const TEST_LEAK_FD_VAR: &str = "DECK_TEST_LEAK_FD";

/// How often the foreground process of every terminal is looked at.
const AGENT_POLL: Duration = Duration::from_secs(1);

/// How the daemon asks for the argv of a terminal's foreground process: the operating
/// system, unless a test puts something else in its place.
#[derive(Clone, Default)]
pub struct ForegroundQuery(Option<Arc<ArgvFn>>);

type ArgvFn = dyn Fn(TerminalId) -> Option<Vec<String>> + Send + Sync;

impl ForegroundQuery {
    pub fn new(query: impl Fn(TerminalId) -> Option<Vec<String>> + Send + Sync + 'static) -> Self {
        Self(Some(Arc::new(query)))
    }

    pub(crate) fn ask(&self, terminal: &terminal::TermHandle) -> Option<Vec<String>> {
        match &self.0 {
            Some(query) => query(terminal.id),
            None => terminal.foreground().map(|(_, argv)| argv),
        }
    }
}

impl std::fmt::Debug for ForegroundQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForegroundQuery").finish_non_exhaustive()
    }
}

/// What the agent-session listings read (`Request::ListAgentSessions`), for tests that check
/// the bounds: nothing else reads these counters.
#[derive(Debug, Default)]
pub struct ListingStats {
    /// Session files opened.
    pub files_opened: AtomicUsize,
    /// Bytes read from them.
    pub bytes_read: AtomicUsize,
    /// The most OpenCode listing children that ran at the same time.
    pub max_children: AtomicUsize,
    pub(crate) running: AtomicUsize,
}

/// How often the stats line is logged.
const STATS_LOG: Duration = Duration::from_secs(600);

/// How often the config file is looked at.
const CONFIG_POLL: Duration = Duration::from_secs(1);

/// Knobs for tests and where the config lives; the defaults are what `deck daemon` uses.
#[derive(Debug, Clone)]
pub struct Options {
    pub clock: Arc<dyn Clock>,
    pub protocol_version: u32,
    /// The config file. When set, the daemon reads it at start (the `config` it is given
    /// is then ignored) and watches it; when not, the `config` it is given stays.
    pub config_path: Option<PathBuf>,
    pub config_poll: Duration,
    /// How often the directories of terminals that never reported one are looked up.
    pub cwd_poll: Duration,
    pub cwd_query: CwdQuery,
    /// How often the foreground process of every terminal is looked at for an agent.
    pub agent_poll: Duration,
    pub foreground_query: ForegroundQuery,
    /// How often the stats line is logged.
    pub stats_log: Duration,
    /// Counts the pending `wait` requests, for tests that check none is left behind.
    pub waiters: Arc<AtomicUsize>,
    /// Counts what the agent-session listings read, for tests.
    pub listing: Arc<ListingStats>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            clock: Arc::new(SystemClock::new()),
            protocol_version: PROTOCOL_VERSION,
            config_path: None,
            config_poll: CONFIG_POLL,
            cwd_poll: CWD_POLL,
            cwd_query: CwdQuery::default(),
            agent_poll: AGENT_POLL,
            foreground_query: ForegroundQuery::default(),
            stats_log: STATS_LOG,
            waiters: Arc::default(),
            listing: Arc::default(),
        }
    }
}

impl Options {
    /// The defaults, with the protocol version of [`TEST_PROTOCOL_VERSION_VAR`] when set.
    pub fn from_env() -> Self {
        Self::from_lookup(&|name| std::env::var(name).ok())
    }

    /// Like [`Options::from_env`] with another source of variables.
    pub fn from_lookup(env: &dyn Fn(&str) -> Option<String>) -> Self {
        let mut options = Self::default();
        if let Some(v) = env(TEST_PROTOCOL_VERSION_VAR).and_then(|v| v.parse().ok()) {
            options.protocol_version = v;
        }
        if let Some(ms) = env(TEST_CONFIG_POLL_VAR).and_then(|v| v.parse::<u64>().ok()) {
            options.config_poll = Duration::from_millis(ms.max(10));
        }
        if let Some(ms) = env(TEST_CWD_POLL_VAR).and_then(|v| v.parse::<u64>().ok()) {
            options.cwd_poll = Duration::from_millis(ms.max(10));
        }
        if let Some(ms) = env(TEST_AGENT_POLL_VAR).and_then(|v| v.parse::<u64>().ok()) {
            options.agent_poll = Duration::from_millis(ms.max(10));
        }
        if let Some(ms) = env(TEST_STATS_LOG_VAR).and_then(|v| v.parse::<u64>().ok()) {
            options.stats_log = Duration::from_millis(ms.max(10));
        }
        options
    }
}

#[derive(Debug)]
pub struct Daemon;

impl Daemon {
    /// Runs the daemon until `shutdown` completes, a client asks for `KillServer`, or the
    /// last session ends.
    pub async fn run<F>(paths: DaemonPaths, config: Config, shutdown: F) -> Result<(), DaemonError>
    where
        F: Future<Output = ()>,
    {
        Self::run_with(paths, config, shutdown, Options::from_env()).await
    }

    /// Like [`Daemon::run`], and calls `on_locked` right after the daemon took its locks
    /// (before it touches the socket), so work that only the one running daemon may do,
    /// like rotating its log, happens there. An error from it stops the daemon.
    pub async fn run_locked<F, H>(
        paths: DaemonPaths,
        config: Config,
        shutdown: F,
        options: Options,
        on_locked: H,
    ) -> Result<(), DaemonError>
    where
        F: Future<Output = ()>,
        H: FnOnce() -> io::Result<()>,
    {
        Self::run_inner(paths, config, shutdown, options, on_locked).await
    }

    pub async fn run_with<F>(
        paths: DaemonPaths,
        config: Config,
        shutdown: F,
        options: Options,
    ) -> Result<(), DaemonError>
    where
        F: Future<Output = ()>,
    {
        Self::run_inner(paths, config, shutdown, options, || Ok(())).await
    }

    async fn run_inner<F, H>(
        paths: DaemonPaths,
        config: Config,
        shutdown: F,
        options: Options,
        on_locked: H,
    ) -> Result<(), DaemonError>
    where
        F: Future<Output = ()>,
        H: FnOnce() -> io::Result<()>,
    {
        let uid = effective_uid()
            .await
            .map_err(|e| DaemonError::io("cannot read the user id", e))?;
        let locks = startup::prepare(&paths, uid)?;
        on_locked().map_err(|e| DaemonError::io("cannot prepare the daemon log", e))?;

        // A save that did not finish leaves a temp file; the state itself is read before
        // anything is bound, so a file this daemon cannot use stops it early.
        persist::remove_tmp(&paths.state_file());
        let loaded = persist::load(&paths)?;

        // Only the daemon removes the socket, and only under the lock.
        match fs::remove_file(&paths.socket) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(DaemonError::io(
                    format!("cannot remove {}", paths.socket.display()),
                    e,
                ));
            }
        }
        let listener = UnixListener::bind(&paths.socket)
            .map_err(|e| DaemonError::io(format!("cannot bind {}", paths.socket.display()), e))?;
        tracing::info!(socket = %paths.socket.display(), pid = std::process::id(), "deck daemon listening");

        let (config, notices, watched) = match options.config_path {
            Some(path) => {
                let (file, started) = ConfigFile::open(path);
                (started.config, started.notices, Some(file))
            }
            None => (config, Vec::new(), None),
        };
        let (shared, mut shutdown_rx) = server::Shared::new(
            paths.clone(),
            config,
            notices,
            options.clock,
            options.protocol_version,
            server::Probes {
                cwd: options.cwd_query,
                foreground: options.foreground_query,
                listing: options.listing,
            },
            options.waiters,
        );
        let launches = match loaded {
            persist::Loaded::Missing => Vec::new(),
            persist::Loaded::State(file, notes) => {
                for note in notes {
                    tracing::warn!("state.json: {note}");
                }
                shared.restore(*file)
            }
            persist::Loaded::Bad(why) => {
                tracing::error!("{why}");
                shared.notify(
                    NoticeLevel::Error,
                    format!(
                        "previous state could not be read; see {}",
                        paths.log_file.display()
                    ),
                );
                Vec::new()
            }
        };
        // Clients can connect while the terminals start.
        let launching = (!launches.is_empty())
            .then(|| tokio::spawn(Arc::clone(&shared).launch_restored(launches)));
        let watcher = watched
            .map(|file| tokio::spawn(watch_config(Arc::clone(&shared), file, options.config_poll)));
        let tracker = tokio::spawn(Arc::clone(&shared).track_cwds(options.cwd_poll));
        let agents = tokio::spawn(Arc::clone(&shared).track_agents(options.agent_poll));
        let stats_log = tokio::spawn(server::stats::log_every(
            Arc::clone(&shared),
            options.stats_log,
        ));
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                accepted = listener.accept() => match accepted {
                    Ok((stream, _)) => {
                        tokio::spawn(conn::handle(Arc::clone(&shared), stream, uid));
                    }
                    Err(e) => {
                        tracing::warn!("cannot accept a connection: {e}");
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                },
                // The embedder (`deck daemon`) logs why it asked.
                () = &mut shutdown => {
                    shared.request_shutdown();
                    break;
                }
                _ = shutdown_rx.changed() => break,
            }
        }
        drop(listener);
        if let Some(watcher) = watcher {
            watcher.abort();
        }
        tracker.abort();
        agents.abort();
        stats_log.abort();
        // One last line while the terminals are still there.
        shared.log_stats().await;
        shared.finish_shutdown().await;
        // It stops at the shutdown flag; an open that was in flight is discarded and reaped.
        if let Some(launching) = launching {
            let _ = launching.await;
        }
        if let Err(e) = fs::remove_file(&paths.socket) {
            tracing::warn!("cannot remove the socket: {e}");
        }
        drop(locks);
        Ok(())
    }
}

/// Looks at the config file every `poll` and hands what changed to the clients.
async fn watch_config(shared: Arc<server::Shared>, mut file: ConfigFile, poll: Duration) {
    let mut ticks = tokio::time::interval(poll);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticks.tick().await;
    loop {
        ticks.tick().await;
        // Reading the file may block (a slow disk): not on an async task.
        let polled = tokio::task::spawn_blocking(move || {
            let change = file.poll();
            (file, change)
        })
        .await;
        let Ok((back, change)) = polled else { return };
        file = back;
        match change {
            Some(Change::Reload { config, notices }) => shared.reload(*config, notices),
            Some(Change::Rejected(notice)) => shared.notify(notice.level, notice.text),
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_test_protocol_version_comes_from_the_environment() {
        let set = |name: &str| (name == TEST_PROTOCOL_VERSION_VAR).then(|| "999".to_string());
        assert_eq!(Options::from_lookup(&set).protocol_version, 999);
        let junk = |_: &str| Some("x".to_string());
        assert_eq!(
            Options::from_lookup(&junk).protocol_version,
            PROTOCOL_VERSION
        );
        assert_eq!(
            Options::from_lookup(&|_| None).protocol_version,
            PROTOCOL_VERSION
        );
    }
}
