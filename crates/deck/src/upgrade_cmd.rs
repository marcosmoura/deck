//! `deck upgrade`: hands the running daemon over to this binary, and the waiting that other
//! commands do while a handoff runs.

use std::fs;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use deck_core::protocol::{PROTOCOL_VERSION, Request};
use deck_daemon::{DaemonPaths, Marker, write_private};

use crate::control::{Conn, Outcome, REPLY_TIMEOUT, checked_paths, control_socket};
use crate::{control, platform};

/// How long a client waits for a handoff to end, and `deck upgrade` for its outcome.
const HANDOFF_LIMIT: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(20);

const NOT_UPGRADABLE: &str =
    "the running daemon cannot be upgraded in place; run \"deck kill-server\" once";

/// The paths of the daemon the control commands talk to: inside deck, the one of
/// `DECK_SOCKET`.
pub(crate) fn runtime_paths(paths: &DaemonPaths) -> DaemonPaths {
    let socket = control_socket(paths);
    match (socket.parent(), paths.lock.file_name()) {
        (Some(dir), Some(lock)) if dir != paths.runtime_dir => DaemonPaths {
            runtime_dir: dir.to_path_buf(),
            lock: dir.join(lock),
            socket,
            ..paths.clone()
        },
        _ => paths.clone(),
    }
}

/// The marker of a daemon that is alive: the file names the pid that holds the lock.
fn live_marker(paths: &DaemonPaths) -> Option<Marker> {
    let marker = Marker::read(&paths.marker_file())?;
    let pid: u32 = fs::read_to_string(&paths.lock).ok()?.trim().parse().ok()?;
    (marker.pid == pid && control::lock_held(&paths.lock)).then_some(marker)
}

/// Whether the daemon of `paths` is handing itself over right now.
pub(crate) fn handing_over(paths: &DaemonPaths) -> bool {
    live_marker(paths).is_some_and(|m| m.upgrading)
}

/// Waits for a handoff to end, up to 30 s; true when there was one.
pub(crate) fn wait_out_handoff(paths: &DaemonPaths) -> anyhow::Result<bool> {
    if !handing_over(paths) {
        return Ok(false);
    }
    let deadline = Instant::now() + HANDOFF_LIMIT;
    while handing_over(paths) {
        if Instant::now() >= deadline {
            bail!(
                "the deck daemon is still upgrading; see {}",
                paths.log_file.display()
            );
        }
        std::thread::sleep(POLL);
    }
    Ok(true)
}

/// Whether a daemon that speaks `daemon` can be handed over to this deck by `deck upgrade`.
pub(crate) fn can_upgrade(paths: &DaemonPaths, daemon: u32) -> bool {
    daemon < PROTOCOL_VERSION && runtime_paths(paths).marker_file().exists()
}

/// The protocol version the daemon speaks, when it answers.
pub(crate) fn daemon_version(paths: &DaemonPaths) -> Option<u32> {
    let mut conn = Conn::open(&control_socket(paths), REPLY_TIMEOUT).ok()?;
    match conn.control(Request::ListSessions) {
        Outcome::Response(_) => Some(PROTOCOL_VERSION),
        Outcome::Mismatch(daemon) => Some(daemon),
        Outcome::Other(_) => None,
    }
}

/// What came of asking the daemon to upgrade.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Done {
    NoDaemon,
    NotUpgradable,
    Upgraded {
        from: Option<u32>,
        terminals: usize,
    },
    /// The daemon said no (or failed): why.
    Refused(String),
    TimedOut,
}

/// Asks the daemon to hand itself over to this binary and waits for the outcome.
pub(crate) fn perform(paths: &DaemonPaths) -> anyhow::Result<Done> {
    if !control::lock_held(&paths.lock) {
        return Ok(Done::NoDaemon);
    }
    let pid = control::read_pid(&paths.lock).context("cannot read the daemon's pid")?;
    let live = |m: &Marker| i64::from(m.pid) == i64::from(pid);
    if !Marker::read(&paths.marker_file()).is_some_and(|m| live(&m)) {
        return Ok(Done::NotUpgradable);
    }
    wait_out_handoff(paths)?;
    let Some(before) = Marker::read(&paths.marker_file()).filter(live) else {
        return Ok(Done::NotUpgradable);
    };
    let from = daemon_version(paths);

    let result = paths.upgrade_result_file();
    // Another client may have asked first: its handoff is the one to wait for, and a
    // second signal could start another in the new image.
    let joined = Marker::read(&paths.marker_file())
        .is_some_and(|now| now.generation > before.generation || now.upgrading);
    if !joined {
        match fs::remove_file(&result) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => bail!("cannot delete {}: {e}", result.display()),
        }
        let exe = std::env::current_exe()
            .and_then(fs::canonicalize)
            .context("cannot find the deck executable")?;
        write_private(
            &paths.upgrade_request_file(),
            format!("{}\n", exe.display()).as_bytes(),
        )
        .with_context(|| format!("cannot write {}", paths.upgrade_request_file().display()))?;
        platform::request_upgrade(pid).context("cannot signal the daemon")?;
    }

    let deadline = Instant::now() + HANDOFF_LIMIT;
    loop {
        if let Some(now) = Marker::read(&paths.marker_file())
            && now.generation > before.generation
            && !now.upgrading
        {
            return Ok(Done::Upgraded {
                from,
                terminals: now.terminals,
            });
        }
        if let Ok(text) = fs::read_to_string(&result)
            && !text.trim().is_empty()
        {
            return Ok(Done::Refused(reason(&text)));
        }
        if Instant::now() >= deadline {
            return Ok(Done::TimedOut);
        }
        std::thread::sleep(POLL);
    }
}

/// A result line (`refused: <why>`, `failed: <why>`) as a sentence.
fn reason(line: &str) -> String {
    let line = line.trim();
    match line.split_once(": ") {
        Some(("refused", why)) => format!("upgrade refused: {why}"),
        Some(("failed", why)) => format!("upgrade failed: {why}"),
        _ => line.to_string(),
    }
}

fn kept(terminals: usize) -> String {
    format!(
        "{terminals} {} kept",
        if terminals == 1 {
            "terminal"
        } else {
            "terminals"
        }
    )
}

/// The line `deck upgrade` prints on success.
fn success_line(from: Option<u32>, terminals: usize) -> String {
    let kept = kept(terminals);
    match from {
        Some(from) if from != PROTOCOL_VERSION => {
            format!("deck daemon upgraded (protocol {from} → {PROTOCOL_VERSION}, {kept})")
        }
        Some(_) => format!("deck daemon restarted in place (protocol {PROTOCOL_VERSION}, {kept})"),
        None => format!("deck daemon upgraded (protocol {PROTOCOL_VERSION}, {kept})"),
    }
}

/// `deck upgrade`.
pub fn run() -> anyhow::Result<ExitCode> {
    let paths = runtime_paths(&checked_paths()?);
    match perform(&paths)? {
        Done::NoDaemon => {
            println!("no deck daemon running");
            Ok(ExitCode::SUCCESS)
        }
        Done::NotUpgradable => {
            eprintln!("{NOT_UPGRADABLE}");
            Ok(ExitCode::FAILURE)
        }
        Done::Upgraded { from, terminals } => {
            println!("{}", success_line(from, terminals));
            Ok(ExitCode::SUCCESS)
        }
        Done::Refused(why) => {
            eprintln!("{why}");
            Ok(ExitCode::FAILURE)
        }
        Done::TimedOut => {
            eprintln!(
                "the upgrade did not finish; see {}",
                paths.log_file.display()
            );
            Ok(ExitCode::FAILURE)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_success_line_names_the_protocols_and_the_terminals() {
        assert_eq!(
            success_line(Some(PROTOCOL_VERSION - 1), 7),
            format!(
                "deck daemon upgraded (protocol {} → {PROTOCOL_VERSION}, 7 terminals kept)",
                PROTOCOL_VERSION - 1
            )
        );
        assert_eq!(
            success_line(Some(PROTOCOL_VERSION), 1),
            format!(
                "deck daemon restarted in place (protocol {PROTOCOL_VERSION}, 1 terminal kept)"
            )
        );
    }

    #[test]
    fn a_result_line_reads_as_a_sentence() {
        assert_eq!(reason("refused: no good\n"), "upgrade refused: no good");
        assert_eq!(reason("failed: boom"), "upgrade failed: boom");
    }
}
