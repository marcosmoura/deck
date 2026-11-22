//! The daemon: socket server, sessions, PTYs, terminal emulation and frames. Persistence
//! and agent status come with later specs.

mod clock;
mod conn;
mod emu;
mod env;
mod error;
mod queue;
mod server;
mod startup;
mod terminal;

use std::fs;
use std::future::Future;
use std::io;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use deck_core::config::Config;
use deck_core::protocol::PROTOCOL_VERSION;
use tokio::net::UnixListener;

pub use clock::{Clock, Drain, Fire, ManualClock, Poke, SystemClock, Throttle};
pub use error::DaemonError;
pub use startup::{DaemonPaths, check_runtime_dir, effective_uid};

/// Locks a mutex whose holders never leave data half-written, so poisoning is ignored.
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The environment variable that makes the daemon report another protocol version, so
/// tests can run a daemon "of another version".
pub const TEST_PROTOCOL_VERSION_VAR: &str = "DECK_TEST_PROTOCOL_VERSION";

/// Knobs for tests; the defaults are what `deck daemon` uses.
#[derive(Debug, Clone)]
pub struct Options {
    pub clock: Arc<dyn Clock>,
    pub protocol_version: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            clock: Arc::new(SystemClock::new()),
            protocol_version: PROTOCOL_VERSION,
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
        on_locked: H,
    ) -> Result<(), DaemonError>
    where
        F: Future<Output = ()>,
        H: FnOnce() -> io::Result<()>,
    {
        Self::run_inner(paths, config, shutdown, Options::from_env(), on_locked).await
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

        let (shared, mut shutdown_rx) = server::Shared::new(
            paths.clone(),
            config,
            options.clock,
            options.protocol_version,
        );
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
                () = &mut shutdown => {
                    tracing::info!("shutting down");
                    shared.request_shutdown();
                    break;
                }
                _ = shutdown_rx.changed() => break,
            }
        }
        drop(listener);
        shared.finish_shutdown().await;
        if let Err(e) = fs::remove_file(&paths.socket) {
            tracing::warn!("cannot remove the socket: {e}");
        }
        drop(locks);
        Ok(())
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
