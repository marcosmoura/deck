//! The live upgrade: the daemon re-executes itself as a new binary, keeping its pid, its
//! PTYs, its socket and its locks, and the new image adopts what the old one hands over.
//!
//! This module has the files in the runtime directory (the marker, the request and the
//! result), the control the embedder's signal handlers talk to, and the handoff file.

mod attempt;
pub(crate) mod b64;
pub(crate) mod file;

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

pub(crate) use attempt::{Inherited, attempt};

use crate::lock;

/// The version of the handoff file this build writes; it reads every version from 1 up to
/// it.
pub const HANDOFF_VERSION: u32 = 1;

/// The hidden variable that makes the old image's `execve` fail, so tests can check the
/// rollback.
pub const TEST_EXEC_FAIL_VAR: &str = "DECK_TEST_EXEC_FAIL";

/// The hidden variable that makes the old image wait this many milliseconds once frozen,
/// so tests can act while a handoff runs.
pub const TEST_HANDOFF_DELAY_VAR: &str = "DECK_TEST_HANDOFF_DELAY_MS";

/// The hidden variable that makes the old image wait this many milliseconds after the marker
/// says `upgrading` and before it freezes.
pub const TEST_PREFREEZE_DELAY_VAR: &str = "DECK_TEST_PREFREEZE_DELAY_MS";

/// Test seams of the old image.
#[derive(Debug, Clone, Copy, Default)]
pub struct HandoffTest {
    /// `execve` fails instead of running.
    pub exec_fail: bool,
    /// How long to wait once frozen.
    pub frozen_delay: std::time::Duration,
    /// How long to wait before freezing.
    pub prefreeze_delay: std::time::Duration,
}

/// How long the new binary's `daemon --handoff-check` may take.
pub(crate) const PREFLIGHT_LIMIT: std::time::Duration = std::time::Duration::from_secs(10);

/// What `<runtime>/deck.handoff` says: that this daemon can hand itself over, and where it
/// is in doing so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    pub handoff: u32,
    pub pid: u32,
    /// Grows by one with each adoption.
    pub generation: u64,
    pub terminals: usize,
    /// A handoff is under way.
    pub upgrading: bool,
}

impl Marker {
    pub fn render(&self) -> String {
        let mut text = format!(
            "handoff {}\npid {}\ngeneration {}\nterminals {}\n",
            self.handoff, self.pid, self.generation, self.terminals
        );
        if self.upgrading {
            text.push_str("upgrading\n");
        }
        text
    }

    /// `None` when a line is missing or does not parse.
    pub fn parse(text: &str) -> Option<Self> {
        let (mut handoff, mut pid, mut generation, mut terminals) = (None, None, None, None);
        let mut upgrading = false;
        for line in text.lines() {
            let (key, value) = line.split_once(' ').unwrap_or((line, ""));
            match key {
                "handoff" => handoff = value.trim().parse().ok(),
                "pid" => pid = value.trim().parse().ok(),
                "generation" => generation = value.trim().parse().ok(),
                "terminals" => terminals = value.trim().parse().ok(),
                "upgrading" => upgrading = true,
                _ => {}
            }
        }
        Some(Self {
            handoff: handoff?,
            pid: pid?,
            generation: generation?,
            terminals: terminals?,
            upgrading,
        })
    }

    /// Reads the marker at `path`; `None` when it is missing or unreadable.
    pub fn read(path: &Path) -> Option<Self> {
        Self::parse(&fs::read_to_string(path).ok()?)
    }

    /// Writes the marker at `path` atomically (a temp name, then a rename), mode 0600.
    pub fn write(&self, path: &Path) -> io::Result<()> {
        write_private(path, self.render().as_bytes())
    }
}

/// Writes `bytes` to `path` atomically with mode 0600.
pub fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    match fs::remove_file(&tmp) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let mut out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    out.write_all(bytes)?;
    out.flush()?;
    drop(out);
    fs::rename(&tmp, path)
}

/// Writes `deck.upgrade.result`: one line, `refused: <reason>` or `failed: <reason>`.
pub(crate) fn write_result(path: &Path, line: &str) {
    if let Err(e) = write_private(path, format!("{line}\n").as_bytes()) {
        tracing::warn!("cannot write {}: {e}", path.display());
    }
}

/// Reads `deck.upgrade` and deletes it: the absolute path of the binary to hand over to,
/// when it is acceptable.
pub(crate) fn take_request(path: &Path, uid: u32) -> Result<PathBuf, String> {
    let text = fs::read_to_string(path);
    // Whatever it held, it is used up.
    let _ = fs::remove_file(path);
    let text = text.map_err(|e| format!("no upgrade request ({e})"))?;
    check_executable(Path::new(text.trim_end_matches(['\n', '\r'])), uid)
}

/// An absolute path to a regular, executable file owned by `uid` or by root.
pub(crate) fn check_executable(exe: &Path, uid: u32) -> Result<PathBuf, String> {
    if !exe.is_absolute() {
        return Err(format!("{} is not an absolute path", exe.display()));
    }
    let meta = fs::metadata(exe).map_err(|e| format!("cannot inspect {}: {e}", exe.display()))?;
    if !meta.is_file() {
        return Err(format!("{} is not a regular file", exe.display()));
    }
    if meta.permissions().mode() & 0o111 == 0 {
        return Err(format!("{} is not executable", exe.display()));
    }
    if meta.uid() != uid && meta.uid() != 0 {
        return Err(format!(
            "{} is owned by uid {}, not {uid} or root",
            exe.display(),
            meta.uid()
        ));
    }
    Ok(exe.to_path_buf())
}

/// What the embedder's signal handlers say to the daemon about the upgrade, and the other
/// way round. Cheap to clone; all clones are the same control.
#[derive(Debug, Clone, Default)]
pub struct Upgrade(Arc<UpgradeInner>);

#[derive(Debug, Default)]
struct UpgradeInner {
    requested: Notify,
    /// `Some` while a handoff runs: the termination signals recorded so far.
    recorded: Mutex<Option<Vec<String>>>,
}

impl Upgrade {
    pub fn new() -> Self {
        Self::default()
    }

    /// `SIGUSR2` came. Dropped while a handoff runs: its client times out.
    pub fn request(&self) {
        if lock(&self.0.recorded).is_none() {
            self.0.requested.notify_one();
        }
    }

    /// A termination signal (`why` names it) came. When this returns true a handoff is
    /// under way and the daemon recorded it: the caller must not act on it, the daemon
    /// does after the handoff. False: act now.
    pub fn hold(&self, why: &str) -> bool {
        match lock(&self.0.recorded).as_mut() {
            Some(recorded) => {
                recorded.push(why.to_string());
                true
            }
            None => false,
        }
    }

    pub(crate) async fn requested(&self) {
        self.0.requested.notified().await;
    }

    pub(crate) fn begin(&self) {
        *lock(&self.0.recorded) = Some(Vec::new());
    }

    /// The handoff is over without an exec: what was recorded, to act on.
    pub(crate) fn end(&self) -> Vec<String> {
        lock(&self.0.recorded).take().unwrap_or_default()
    }

    pub(crate) fn recorded(&self) -> Vec<String> {
        lock(&self.0.recorded).clone().unwrap_or_default()
    }

    pub(crate) fn record(&self, why: &str) {
        if let Some(recorded) = lock(&self.0.recorded).as_mut() {
            recorded.push(why.to_string());
        }
    }
}

/// What `deck daemon --adopt <fd>` found when it looked at what the old image left.
#[derive(Debug, Clone)]
pub enum Adopt {
    /// A good handoff file; the descriptors it names are inherited and checked.
    Handed(Box<file::HandoffFile>),
    /// Unusable: why. Everything inherited is closed and the daemon starts over from
    /// `state.json`.
    Failed(String),
}

/// Reads and checks the handoff file at descriptor `fd`: the file itself, that the two lock
/// descriptors are the lock files on disk, that the listener listens and that every
/// terminal's descriptor is open. Synchronous, and meant to run first thing in a new image,
/// before the async runtime, tracing or any signal setup opens a descriptor of its own: on
/// a failure it closes every inherited descriptor above 2, so the daemon can lock again.
pub fn prepare_adoption(fd: std::os::fd::RawFd, paths: &crate::DaemonPaths) -> Adopt {
    match read_adoption(fd, paths) {
        Ok(file) => Adopt::Handed(Box::new(file)),
        Err(why) => {
            crate::platform::close_inherited(&[]);
            Adopt::Failed(why)
        }
    }
}

fn read_adoption(
    fd: std::os::fd::RawFd,
    paths: &crate::DaemonPaths,
) -> Result<file::HandoffFile, String> {
    use crate::platform;
    platform::set_cloexec(fd, true).map_err(|e| format!("cannot use descriptor {fd}: {e}"))?;
    let owned = platform::own_fd(fd).map_err(|e| format!("cannot use descriptor {fd}: {e}"))?;
    let file = file::read(fs::File::from(owned))?;
    // Each descriptor gets one owner (`platform::own_fd`): none may be named twice.
    let mut seen = std::collections::HashSet::new();
    let twice = [file.lock_fd, file.state_lock_fd, file.listener_fd]
        .into_iter()
        .chain(file.terminals.iter().filter_map(|t| t.fd))
        .find(|fd| !seen.insert(*fd));
    if let Some(fd) = twice {
        return Err(format!("the handoff names descriptor {fd} twice"));
    }
    for (what, fd, path) in [
        ("the daemon lock", file.lock_fd, &paths.lock),
        ("the state lock", file.state_lock_fd, &paths.state_lock),
    ] {
        platform::set_cloexec(fd, true).map_err(|e| format!("{what} was not inherited: {e}"))?;
        let held = platform::fd_identity(fd).map_err(|e| format!("{what}: {e}"))?;
        let there = fs::metadata(path)
            .map(|m| (m.dev(), m.ino()))
            .map_err(|e| format!("cannot inspect {}: {e}", path.display()))?;
        if held != there {
            return Err(format!("{what} is not the file at {}", path.display()));
        }
    }
    platform::set_cloexec(file.listener_fd, true)
        .map_err(|e| format!("the socket was not inherited: {e}"))?;
    if !platform::is_listening_socket(file.listener_fd) {
        return Err("the inherited socket does not listen".into());
    }
    for fd in file.terminals.iter().filter_map(|t| t.fd) {
        platform::set_cloexec(fd, true)
            .map_err(|e| format!("terminal descriptor {fd} was not inherited: {e}"))?;
    }
    Ok(file)
}

/// What a new image's early signal handlers caught before the real ones were in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EarlySignals {
    /// `SIGUSR2`: an upgrade was asked for.
    pub upgrade: bool,
    /// The name of a termination signal.
    pub terminate: Option<&'static str>,
}

/// For a new image, as its first act: catches the four handoff signals (the old image left
/// them blocked), then unblocks them. Call [`early_signals`] once the real handlers are in
/// place.
pub fn catch_early_signals() -> io::Result<()> {
    crate::platform::catch_early_signals()
}

/// What [`catch_early_signals`] caught; forgets it.
pub fn early_signals() -> EarlySignals {
    let (upgrade, terminate) = crate::platform::take_early_signals();
    EarlySignals { upgrade, terminate }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marker() -> Marker {
        Marker {
            handoff: 1,
            pid: 4242,
            generation: 3,
            terminals: 12,
            upgrading: false,
        }
    }

    #[test]
    fn a_handoff_that_names_a_descriptor_twice_is_refused() {
        use std::os::fd::IntoRawFd;
        let dir = tempfile::tempdir().unwrap();
        let mut handed = file::tests::sample();
        handed.listener_fd = handed.lock_fd;
        let fd = file::write(dir.path(), &handed).unwrap().into_raw_fd();
        let paths = crate::DaemonPaths {
            runtime_dir: dir.path().into(),
            socket: dir.path().join("deck.sock"),
            lock: dir.path().join("deck.lock"),
            state_dir: dir.path().into(),
            state_lock: dir.path().join("state.lock"),
            log_file: dir.path().join("deck.log"),
        };
        let why = read_adoption(fd, &paths).unwrap_err();
        assert_eq!(
            why,
            format!("the handoff names descriptor {} twice", handed.lock_fd)
        );
    }

    #[test]
    fn the_marker_has_the_documented_lines() {
        assert_eq!(
            marker().render(),
            "handoff 1\npid 4242\ngeneration 3\nterminals 12\n"
        );
        let up = Marker {
            upgrading: true,
            ..marker()
        };
        assert_eq!(
            up.render(),
            "handoff 1\npid 4242\ngeneration 3\nterminals 12\nupgrading\n"
        );
    }

    #[test]
    fn the_marker_parses_back_and_refuses_a_partial_one() {
        for m in [
            marker(),
            Marker {
                upgrading: true,
                ..marker()
            },
        ] {
            assert_eq!(Marker::parse(&m.render()), Some(m));
        }
        assert_eq!(Marker::parse("handoff 1\npid 2\n"), None);
        assert_eq!(
            Marker::parse("handoff x\npid 2\ngeneration 0\nterminals 0\n"),
            None
        );
        assert_eq!(Marker::parse(""), None);
    }

    #[test]
    fn the_marker_is_written_privately_and_replaced_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deck.handoff");
        marker().write(&path).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        let next = Marker {
            generation: 4,
            upgrading: true,
            ..marker()
        };
        next.write(&path).unwrap();
        assert_eq!(Marker::read(&path), Some(next));
        assert!(!dir.path().join("deck.handoff.tmp").exists());
    }

    fn executable(dir: &Path, name: &str, mode: u32) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        path
    }

    #[test]
    fn a_request_is_checked_and_always_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let uid = fs::metadata(dir.path()).unwrap().uid();
        let request = dir.path().join("deck.upgrade");
        let good = executable(dir.path(), "good", 0o755);

        fs::write(&request, format!("{}\n", good.display())).unwrap();
        assert_eq!(take_request(&request, uid), Ok(good.clone()));
        assert!(!request.exists());

        // Missing.
        assert!(
            take_request(&request, uid)
                .unwrap_err()
                .contains("no upgrade request")
        );

        // Not absolute.
        fs::write(&request, "relative/deck").unwrap();
        assert!(
            take_request(&request, uid)
                .unwrap_err()
                .contains("absolute")
        );
        assert!(!request.exists());

        // Not a regular file.
        fs::write(&request, format!("{}\n", dir.path().display())).unwrap();
        assert!(
            take_request(&request, uid)
                .unwrap_err()
                .contains("regular file")
        );

        // Not executable.
        let plain = executable(dir.path(), "plain", 0o644);
        fs::write(&request, format!("{}\n", plain.display())).unwrap();
        assert!(
            take_request(&request, uid)
                .unwrap_err()
                .contains("not executable")
        );

        // Missing binary.
        fs::write(&request, "/nonexistent/deck\n").unwrap();
        assert!(
            take_request(&request, uid)
                .unwrap_err()
                .contains("cannot inspect")
        );

        // Owned by somebody else (and not root).
        assert!(
            check_executable(&good, uid + 1)
                .unwrap_err()
                .contains("owned by uid")
        );
    }

    #[test]
    fn the_control_records_signals_only_during_a_handoff() {
        let upgrade = Upgrade::new();
        assert!(!upgrade.hold("SIGTERM"));
        upgrade.begin();
        assert!(upgrade.hold("SIGTERM"));
        assert!(upgrade.hold("SIGHUP"));
        assert_eq!(upgrade.recorded(), ["SIGTERM", "SIGHUP"]);
        assert_eq!(upgrade.end(), ["SIGTERM", "SIGHUP"]);
        assert!(!upgrade.hold("SIGINT"));
        assert!(upgrade.end().is_empty());
    }

    #[tokio::test]
    async fn a_second_request_during_a_handoff_is_dropped() {
        let upgrade = Upgrade::new();
        upgrade.request();
        upgrade.requested().await;
        upgrade.begin();
        upgrade.request();
        upgrade.end();
        let waited =
            tokio::time::timeout(std::time::Duration::from_millis(100), upgrade.requested()).await;
        assert!(waited.is_err());
    }
}
