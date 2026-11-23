//! `deck daemon` (hidden): runs the daemon in the foreground.

use std::fs::{self, DirBuilder};
use std::io::{self, Write};
use std::os::unix::fs::DirBuilderExt;
use std::path::Path;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use deck_core::config::Config;
use deck_core::paths;
use deck_daemon::{Daemon, DaemonError, DaemonPaths, Options};
use tokio::signal::unix::{SignalKind, signal};

use crate::platform;

/// A log over this size is rotated when the daemon starts.
const LOG_LIMIT: u64 = 10 * 1024 * 1024;
const PARENT_CHECK: Duration = Duration::from_secs(1);
/// Hidden: shut down when this process no longer exists, so a killed test leaks no daemon.
const PARENT_PID_VAR: &str = "DECK_TEST_PARENT_PID";

pub fn run() -> anyhow::Result<ExitCode> {
    let env = |name: &str| std::env::var(name).ok();
    let daemon_paths = DaemonPaths::from_env(&env, platform::effective_uid())?;

    // Until the daemon holds its lock, stderr stays where the launcher put it: a second
    // daemon must not rotate or write the running daemon's log.
    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .init();

    let original_stderr = Arc::new(Mutex::new(None));
    let outcome = serve(&env, daemon_paths, &original_stderr);
    match outcome {
        Ok(()) => Ok(ExitCode::SUCCESS),
        Err(message) => {
            // After the lock, stderr is the log; what the user must read (the reason this
            // daemon stopped) goes to the original stderr.
            let original = original_stderr
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
            match original {
                Some(mut terminal) => {
                    tracing::error!("{message}");
                    let _ = writeln!(terminal, "{message}");
                }
                None => eprintln!("{message}"),
            }
            Ok(ExitCode::FAILURE)
        }
    }
}

/// Runs the daemon; the error is the line to show the user. The original stderr is stored
/// in `original_stderr` once the log took its place.
fn serve(
    env: &dyn Fn(&str) -> Option<String>,
    daemon_paths: DaemonPaths,
    original_stderr: &Arc<Mutex<Option<fs::File>>>,
) -> Result<(), String> {
    // The daemon reads the file itself, at start and on every change.
    let config_path = paths::config_file(env).map_err(|e| format!("deck daemon: {e}"))?;
    let mut options = Options::from_env();
    options.config_path = Some(config_path);
    let parent = std::env::var(PARENT_PID_VAR)
        .ok()
        .and_then(|v| v.parse::<i32>().ok());

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("deck daemon: cannot start the async runtime: {e}"))?;
    let log_paths = daemon_paths.clone();
    let result = runtime.block_on(async {
        // Installed before start-up, so an early signal still ends in a clean shutdown.
        let mut term = signal(SignalKind::terminate())?;
        let mut int = signal(SignalKind::interrupt())?;
        let mut hup = signal(SignalKind::hangup())?;
        let shutdown = async move {
            tokio::select! {
                _ = term.recv() => tracing::info!("shutting down: SIGTERM"),
                _ = int.recv() => tracing::info!("shutting down: SIGINT"),
                _ = hup.recv() => tracing::info!("shutting down: SIGHUP"),
                () = parent_gone(parent) => tracing::info!(
                    "shutting down: parent process {} exited",
                    parent.unwrap_or_default()
                ),
            }
        };
        let on_locked = || {
            let original = redirect_to_log(&log_paths)?;
            *original_stderr.lock().unwrap_or_else(|e| e.into_inner()) = Some(original);
            Ok(())
        };
        Ok::<_, io::Error>(
            Daemon::run_locked(
                daemon_paths,
                Config::default(),
                shutdown,
                options,
                on_locked,
            )
            .await,
        )
    });
    match result {
        Err(e) => Err(format!("deck daemon: cannot install signal handlers: {e}")),
        Ok(Ok(())) => Ok(()),
        Ok(Err(DaemonError::AlreadyRunning)) => Err("deck daemon is already running".into()),
        Ok(Err(e)) => Err(format!("deck daemon: {e}")),
    }
}

/// Rotates the log, opens it for append and points stderr at it; returns the original
/// stderr.
fn redirect_to_log(paths: &DaemonPaths) -> io::Result<fs::File> {
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&paths.state_dir)?;
    rotate_log(&paths.log_file)?;
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&paths.log_file)?;
    platform::redirect_stderr(&log)
}

/// Completes once the process `pid` is gone; never when there is none to watch.
async fn parent_gone(pid: Option<i32>) {
    let Some(pid) = pid else {
        std::future::pending::<()>().await;
        return;
    };
    loop {
        tokio::time::sleep(PARENT_CHECK).await;
        if !platform::process_exists(pid) {
            return;
        }
    }
}

/// Moves a log over [`LOG_LIMIT`] to `<log>.1`, replacing an older one.
fn rotate_log(log: &Path) -> io::Result<()> {
    let size = match fs::metadata(log) {
        Ok(meta) => meta.len(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if size <= LOG_LIMIT {
        return Ok(());
    }
    let mut rotated = log.as_os_str().to_owned();
    rotated.push(".1");
    fs::rename(log, rotated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_replaces_the_old_copy_only_over_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("daemon.log");
        let old = dir.path().join("daemon.log.1");
        rotate_log(&log).unwrap();

        fs::write(&log, b"small").unwrap();
        rotate_log(&log).unwrap();
        assert!(log.exists() && !old.exists());

        fs::write(&old, b"older").unwrap();
        fs::File::create(&log)
            .unwrap()
            .set_len(LOG_LIMIT + 1)
            .unwrap();
        rotate_log(&log).unwrap();
        assert!(!log.exists());
        assert_eq!(fs::metadata(&old).unwrap().len(), LOG_LIMIT + 1);
    }
}
