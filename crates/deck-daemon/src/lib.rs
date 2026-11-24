//! The daemon: socket server, sessions, PTYs, terminal emulation, frames, persistence and
//! the status of the coding agents running in terminals.

mod agents;
mod clock;
mod config_file;
mod conn;
mod emu;
mod env;
mod error;
mod handoff;
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
pub use handoff::{
    Adopt, EarlySignals, HANDOFF_VERSION, HandoffTest, Marker, TEST_EXEC_FAIL_VAR,
    TEST_HANDOFF_DELAY_VAR, TEST_PREFREEZE_DELAY_VAR, Upgrade, catch_early_signals, early_signals,
    prepare_adoption, write_private,
};
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
    /// The control the embedder's signal handlers use for live upgrades. When set, the
    /// daemon keeps `deck.handoff` and upgrades itself when asked.
    pub upgrade: Option<Upgrade>,
    /// What a new image found of the old one's handoff (`deck daemon --adopt <fd>`); see
    /// [`prepare_adoption`].
    pub adopt: Option<Adopt>,
    /// Test seams of a handoff.
    pub handoff_test: HandoffTest,
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
            upgrade: None,
            adopt: None,
            handoff_test: HandoffTest::default(),
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
        if env(TEST_EXEC_FAIL_VAR).is_some_and(|v| v == "1") {
            options.handoff_test.exec_fail = true;
        }
        if let Some(ms) = env(TEST_HANDOFF_DELAY_VAR).and_then(|v| v.parse::<u64>().ok()) {
            options.handoff_test.frozen_delay = Duration::from_millis(ms);
        }
        if let Some(ms) = env(TEST_PREFREEZE_DELAY_VAR).and_then(|v| v.parse::<u64>().ok()) {
            options.handoff_test.prefreeze_delay = Duration::from_millis(ms);
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
        mut options: Options,
        on_locked: H,
    ) -> Result<(), DaemonError>
    where
        F: Future<Output = ()>,
        H: FnOnce() -> io::Result<()>,
    {
        let uid = effective_uid()
            .await
            .map_err(|e| DaemonError::io("cannot read the user id", e))?;

        // A new image has read what the old one handed over (`prepare_adoption`). When that
        // failed, it starts over from `state.json` like a daemon that was never upgraded.
        let mut adoption = None;
        let mut adopt_error = None;
        match options.adopt.take() {
            Some(Adopt::Handed(file)) => adoption = Some(*file),
            Some(Adopt::Failed(why)) => {
                // Logged by the notice below, once the log file is stderr.
                adopt_error = Some(why);
            }
            None => {}
        }

        let (locks, listener, loaded) = match &adoption {
            Some(file) => {
                let locks = startup::Locks::inherited(
                    fs::File::from(platform::own_fd(file.lock_fd)),
                    fs::File::from(platform::own_fd(file.state_lock_fd)),
                );
                let std_listener =
                    std::os::unix::net::UnixListener::from(platform::own_fd(file.listener_fd));
                std_listener
                    .set_nonblocking(true)
                    .map_err(|e| DaemonError::io("cannot use the inherited socket", e))?;
                let listener = UnixListener::from_std(std_listener)
                    .map_err(|e| DaemonError::io("cannot use the inherited socket", e))?;
                (locks, listener, persist::Loaded::Missing)
            }
            None => {
                let locks = startup::prepare(&paths, uid)?;
                on_locked().map_err(|e| DaemonError::io("cannot prepare the daemon log", e))?;

                // A save that did not finish leaves a temp file; the state itself is read
                // before anything is bound, so a file this daemon cannot use stops it early.
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
                let listener = UnixListener::bind(&paths.socket).map_err(|e| {
                    DaemonError::io(format!("cannot bind {}", paths.socket.display()), e)
                })?;
                (locks, listener, loaded)
            }
        };
        tracing::info!(socket = %paths.socket.display(), pid = std::process::id(), "deck daemon listening");

        let (config, notices, watched) = match options.config_path {
            Some(path) => {
                let (file, started) = ConfigFile::open(path);
                (started.config, started.notices, Some(file))
            }
            None => (config, Vec::new(), None),
        };
        let clock: Arc<dyn Clock> = match &adoption {
            Some(file) => Arc::new(SystemClock::starting_at(Duration::from_nanos(
                file.clock_nanos,
            ))),
            None => options.clock,
        };
        let (shared, mut shutdown_rx) = server::Shared::new(
            paths.clone(),
            config,
            notices,
            clock,
            options.protocol_version,
            server::Probes {
                cwd: options.cwd_query,
                foreground: options.foreground_query,
                listing: options.listing,
            },
            options.waiters,
        );
        if let Some(why) = adopt_error {
            shared.notify(
                NoticeLevel::Error,
                format!(
                    "the upgrade could not hand the terminals over ({why}); see {}",
                    paths.log_file.display()
                ),
            );
        }
        let mut generation = 0;
        let mut recorded = Vec::new();
        let launches = if let Some(file) = adoption {
            generation = file.generation + 1;
            recorded = file.signals.clone();
            let (old_pid, old_protocol) = (file.pid, file.protocol_version);
            let adopting = Arc::clone(&shared);
            let adopted = tokio::task::spawn_blocking(move || {
                let mut file = file;
                sweep_children(&mut file);
                adopting.adopt(file)
            })
            .await
            .map_err(|e| DaemonError::io("cannot adopt the terminals", io::Error::other(e)))?;
            let rt = tokio::runtime::Handle::current();
            for terminal in &adopted.live {
                terminal.nudge_pty(&rt);
            }
            // The same count the marker gets: every terminal of the model, the ones whose
            // program already ended included.
            tracing::info!(
                "adopted {} terminals from pid {old_pid} (protocol {old_protocol} \u{2192} {}), {} running",
                shared.terminal_count(),
                options.protocol_version,
                adopted.live.len()
            );
            if adopted.dropped > 0 {
                tracing::warn!("{} terminals could not be adopted", adopted.dropped);
            }
            let saving = Arc::clone(&shared);
            let _ = tokio::task::spawn_blocking(move || saving.save_now()).await;
            adopted.launches
        } else {
            match loaded {
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
            }
        };
        let upgrade = options.upgrade.clone();
        let inherited = {
            let (lock, state_lock) = locks.raw_fds();
            handoff::Inherited {
                listener: std::os::fd::AsRawFd::as_raw_fd(&listener),
                lock,
                state_lock,
            }
        };
        let write_marker = || {
            if upgrade.is_some() {
                let marker = Marker {
                    handoff: HANDOFF_VERSION,
                    pid: std::process::id(),
                    generation,
                    terminals: shared.terminal_count(),
                    upgrading: false,
                };
                if let Err(e) = marker.write(&paths.marker_file()) {
                    tracing::warn!("cannot write {}: {e}", paths.marker_file().display());
                }
            }
        };
        write_marker();
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
        // A termination signal that came during the handoff ends the new image at once.
        let stop_now = recorded.first().is_some_and(|first| {
            tracing::info!("shutting down: {first}");
            shared.request_shutdown();
            true
        });
        tokio::pin!(shutdown);
        let mut shutdown_done = false;
        if !stop_now {
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
                    () = &mut shutdown, if !shutdown_done => {
                        shared.request_shutdown();
                        break;
                    }
                    _ = shutdown_rx.changed() => break,
                    () = upgrade_requested(upgrade.as_ref()) => {
                        let Some(control) = upgrade.as_ref() else { continue };
                        // The embedder's signal task lives in `shutdown`: it is polled on,
                        // so a termination signal is recorded and not lost.
                        let handoff = handoff::attempt(
                        &shared,
                        control,
                        &paths,
                        uid,
                        generation,
                        inherited,
                        options.handoff_test,
                    );
                        tokio::pin!(handoff);
                        loop {
                            tokio::select! {
                                () = &mut handoff => break,
                                () = &mut shutdown, if !shutdown_done => {
                                    shutdown_done = true;
                                    control.record("the embedder asked to stop");
                                }
                            }
                        }
                        let signals = control.end();
                        if let Some(first) = signals.first() {
                            if shutdown_done {
                                tracing::info!("shutting down: {first}");
                            } else {
                                tracing::info!("shutting down: {first} (during the upgrade)");
                            }
                            shared.request_shutdown();
                            break;
                        }
                        if shutdown_done {
                            shared.request_shutdown();
                            break;
                        }
                    }
                }
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
        if upgrade.is_some() {
            let _ = fs::remove_file(paths.marker_file());
        }
        drop(locks);
        Ok(())
    }
}

/// Completes when an upgrade is asked for; never when there is no control.
async fn upgrade_requested(upgrade: Option<&Upgrade>) {
    match upgrade {
        Some(upgrade) => upgrade.requested().await,
        None => std::future::pending().await,
    }
}

/// Reaps the children that ended while nobody waited for them. A status for a terminal that
/// is adopted goes into its record, as if the old image had reaped it; the others are
/// children of the old image that nothing adopts (a process that was being discarded, a
/// listing helper).
fn sweep_children(file: &mut handoff::file::HandoffFile) {
    for (pid, status) in platform::reap_children() {
        match file
            .terminals
            .iter_mut()
            .find(|t| t.pid == Some(pid) && t.reaped.is_none())
        {
            Some(terminal) => {
                tracing::info!("reaped pid {pid} (status {status}): it ended during the handoff");
                terminal.reaped = Some(status);
            }
            None => tracing::info!("reaped pid {pid} (status {status}), a child nothing adopted"),
        }
    }
}

/// Looks at the config file every `poll` and hands what changed to the clients.
async fn watch_config(shared: Arc<server::Shared>, mut file: ConfigFile, poll: Duration) {
    let mut ticks = tokio::time::interval(poll);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticks.tick().await;
    loop {
        ticks.tick().await;
        // The new image reads the file again; a handoff does not look at it.
        if shared.st_is_upgrading() {
            continue;
        }
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
