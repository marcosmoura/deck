//! Start-up: the runtime and state directories and the two locks.

use std::fs::{self, DirBuilder, File, Permissions, TryLockError};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use deck_core::paths::{self, EnvLookup, PathError};

use crate::error::DaemonError;

/// Where the daemon keeps its files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonPaths {
    pub runtime_dir: PathBuf,
    pub socket: PathBuf,
    pub lock: PathBuf,
    pub state_dir: PathBuf,
    pub state_lock: PathBuf,
    pub log_file: PathBuf,
}

impl DaemonPaths {
    /// The saved state.
    pub fn state_file(&self) -> PathBuf {
        self.state_dir.join("state.json")
    }

    /// The marker of a daemon that can hand itself over to a new binary.
    pub fn marker_file(&self) -> PathBuf {
        self.runtime_dir.join("deck.handoff")
    }

    /// Where a client leaves the path of the binary to hand over to.
    pub fn upgrade_request_file(&self) -> PathBuf {
        self.runtime_dir.join("deck.upgrade")
    }

    /// What the daemon says about an upgrade it refused or could not do.
    pub fn upgrade_result_file(&self) -> PathBuf {
        self.runtime_dir.join("deck.upgrade.result")
    }

    pub fn from_env(env: EnvLookup<'_>, uid: u32) -> Result<Self, PathError> {
        Ok(Self {
            runtime_dir: paths::runtime_dir(env, uid),
            socket: paths::socket_path(env, uid)?,
            lock: paths::runtime_lock_path(env, uid),
            state_dir: paths::state_dir(env)?,
            state_lock: paths::state_lock_path(env)?,
            log_file: paths::log_file(env)?,
        })
    }
}

/// The effective uid of this process, from the credentials of a socket pair (the safe
/// way to read it without `libc`).
pub async fn effective_uid() -> io::Result<u32> {
    let (a, _b) = tokio::net::UnixStream::pair()?;
    Ok(a.peer_cred()?.uid())
}

/// The two held locks; dropping them releases both.
#[derive(Debug)]
pub(crate) struct Locks {
    runtime: File,
    state: File,
}

impl Locks {
    /// The locks a new image inherited, from their descriptors.
    pub(crate) fn inherited(runtime: File, state: File) -> Self {
        Self { runtime, state }
    }

    /// The descriptors of the two lock files.
    pub(crate) fn raw_fds(&self) -> (std::os::fd::RawFd, std::os::fd::RawFd) {
        use std::os::fd::AsRawFd;
        (self.runtime.as_raw_fd(), self.state.as_raw_fd())
    }
}

/// The check [`prepare`] makes on the runtime directory, without creating it: a missing
/// directory is fine (there is no daemon), an unsafe one is an error. For clients.
pub fn check_runtime_dir(path: &Path, uid: u32) -> Result<(), DaemonError> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        _ => prepare_runtime_dir(path, uid),
    }
}

pub(crate) fn prepare(paths: &DaemonPaths, uid: u32) -> Result<Locks, DaemonError> {
    prepare_runtime_dir(&paths.runtime_dir, uid)?;
    let runtime = lock_file(&paths.lock, || DaemonError::AlreadyRunning)?;
    write_pid(&runtime, &paths.lock)?;

    private_dir(&paths.state_dir)
        .map_err(|e| DaemonError::io(format!("cannot create {}", paths.state_dir.display()), e))?;
    let state = lock_file(&paths.state_lock, || {
        DaemonError::StateInUse(paths.state_dir.clone())
    })?;
    Ok(Locks { runtime, state })
}

/// Makes what a daemon took over from an older image private, as a fresh start does: an
/// older deck left the state directory `0755`, and its log and its locks `0644`. A mode that
/// cannot be changed is logged and does not stop the daemon, which holds the sessions by now.
pub(crate) fn tighten(paths: &DaemonPaths, locks: &Locks) {
    let results = [
        (
            &paths.state_dir,
            fs::set_permissions(&paths.state_dir, Permissions::from_mode(0o700)),
        ),
        (
            &paths.lock,
            locks.runtime.set_permissions(Permissions::from_mode(0o600)),
        ),
        (
            &paths.state_lock,
            locks.state.set_permissions(Permissions::from_mode(0o600)),
        ),
        (
            &paths.log_file,
            fs::set_permissions(&paths.log_file, Permissions::from_mode(0o600)),
        ),
    ];
    for (path, result) in results {
        match result {
            Ok(()) => {}
            // A daemon started by hand has no log file.
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!("cannot make {} private: {e}", path.display()),
        }
    }
}

fn unsafe_dir(path: &Path, reason: impl Into<String>) -> DaemonError {
    DaemonError::UnsafeRuntimeDir {
        path: path.to_path_buf(),
        reason: reason.into(),
    }
}

fn private_dir(path: &Path) -> io::Result<()> {
    DirBuilder::new().recursive(true).mode(0o700).create(path)
}

/// `lstat` first: a missing directory is created with mode `0700`; an existing one must be
/// a directory (not a link) owned by us with exactly that mode.
fn prepare_runtime_dir(path: &Path, uid: u32) -> Result<(), DaemonError> {
    prepare_runtime_dir_with(path, uid, private_dir)
}

/// [`prepare_runtime_dir`] with the step that creates the directory. Creating succeeds too
/// when another user put a directory, or a link to one, at the path after the `lstat`, so
/// what is there afterwards is checked like a directory that existed.
fn prepare_runtime_dir_with(
    path: &Path,
    uid: u32,
    create: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<(), DaemonError> {
    let inspect = |e| DaemonError::io(format!("cannot inspect {}", path.display()), e);
    match fs::symlink_metadata(path) {
        Ok(meta) => check_runtime_dir_meta(path, uid, &meta),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            create(path)
                .map_err(|e| DaemonError::io(format!("cannot create {}", path.display()), e))?;
            let meta = fs::symlink_metadata(path).map_err(inspect)?;
            check_runtime_dir_meta(path, uid, &meta)
        }
        Err(e) => Err(inspect(e)),
    }
}

fn check_runtime_dir_meta(path: &Path, uid: u32, meta: &fs::Metadata) -> Result<(), DaemonError> {
    if !meta.file_type().is_dir() {
        let what = if meta.file_type().is_symlink() {
            "a symbolic link"
        } else {
            "not a directory"
        };
        return Err(unsafe_dir(path, what));
    }
    if meta.uid() != uid {
        return Err(unsafe_dir(
            path,
            format!("owned by uid {}, not {uid}", meta.uid()),
        ));
    }
    let mode = meta.mode() & 0o777;
    if mode != 0o700 {
        return Err(unsafe_dir(path, format!("mode is {mode:04o}, not 0700")));
    }
    Ok(())
}

/// Opens and locks a lock file that only the user can read: it is created `0600`, and one
/// that was left `0644` is tightened.
fn lock_file(path: &Path, busy: impl FnOnce() -> DaemonError) -> Result<File, DaemonError> {
    let err = |e| DaemonError::io(format!("cannot open {}", path.display()), e);
    let file = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
        .map_err(err)?;
    file.set_permissions(Permissions::from_mode(0o600))
        .map_err(err)?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => Err(busy()),
        Err(TryLockError::Error(e)) => Err(DaemonError::io(
            format!("cannot lock {}", path.display()),
            e,
        )),
    }
}

fn write_pid(mut file: &File, path: &Path) -> Result<(), DaemonError> {
    let err = |e| DaemonError::io(format!("cannot write {}", path.display()), e);
    file.set_len(0).map_err(err)?;
    file.write_all(std::process::id().to_string().as_bytes())
        .map_err(err)?;
    file.flush().map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lock_files_are_private_even_when_they_were_not() {
        let tmp = tempfile::tempdir().unwrap();
        let mode = |path: &Path| fs::metadata(path).unwrap().mode() & 0o777;

        let new = tmp.path().join("deck.lock");
        drop(lock_file(&new, || DaemonError::AlreadyRunning).unwrap());
        assert_eq!(mode(&new), 0o600);

        // One that an earlier version created.
        let old = tmp.path().join("state.lock");
        fs::write(&old, "").unwrap();
        fs::set_permissions(&old, fs::Permissions::from_mode(0o644)).unwrap();
        drop(lock_file(&old, || DaemonError::AlreadyRunning).unwrap());
        assert_eq!(mode(&old), 0o600);
    }

    #[test]
    fn a_daemon_that_takes_over_makes_what_it_inherits_private() {
        let tmp = tempfile::tempdir().unwrap();
        let mode = |path: &Path| fs::metadata(path).unwrap().mode() & 0o777;
        let state_dir = tmp.path().join("state");
        fs::create_dir(&state_dir).unwrap();
        let paths = DaemonPaths {
            runtime_dir: tmp.path().to_path_buf(),
            socket: tmp.path().join("deck.sock"),
            lock: tmp.path().join("deck.lock"),
            state_lock: state_dir.join("state.lock"),
            log_file: state_dir.join("daemon.log"),
            state_dir,
        };
        // What an older deck left; the new image gets the locks as open descriptors.
        let files = [&paths.lock, &paths.state_lock, &paths.log_file];
        for file in files {
            fs::write(file, "").unwrap();
            fs::set_permissions(file, fs::Permissions::from_mode(0o644)).unwrap();
        }
        fs::set_permissions(&paths.state_dir, fs::Permissions::from_mode(0o755)).unwrap();
        let locks = Locks::inherited(
            File::open(&paths.lock).unwrap(),
            File::open(&paths.state_lock).unwrap(),
        );

        tighten(&paths, &locks);
        assert_eq!(mode(&paths.state_dir), 0o700);
        for file in files {
            assert_eq!(mode(file), 0o600, "{}", file.display());
        }

        // A daemon started by hand has no log: nothing to do, and no failure.
        fs::remove_file(&paths.log_file).unwrap();
        tighten(&paths, &locks);
        assert_eq!(mode(&paths.lock), 0o600);
    }

    #[tokio::test]
    async fn uid_matches_a_file_we_create() {
        let dir = tempfile::tempdir().unwrap();
        let uid = effective_uid().await.unwrap();
        assert_eq!(fs::metadata(dir.path()).unwrap().uid(), uid);
    }

    #[test]
    fn runtime_dir_is_created_private_and_checked() {
        let tmp = tempfile::tempdir().unwrap();
        let uid = fs::metadata(tmp.path()).unwrap().uid();
        let dir = tmp.path().join("a").join("run");
        prepare_runtime_dir(&dir, uid).unwrap();
        assert_eq!(fs::metadata(&dir).unwrap().mode() & 0o777, 0o700);
        prepare_runtime_dir(&dir, uid).unwrap();

        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        let err = prepare_runtime_dir(&dir, uid).unwrap_err();
        assert!(matches!(err, DaemonError::UnsafeRuntimeDir { .. }), "{err}");
        assert!(err.to_string().contains("0755"), "{err}");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();

        // Wrong owner.
        let err = prepare_runtime_dir(&dir, uid + 1).unwrap_err();
        assert!(matches!(err, DaemonError::UnsafeRuntimeDir { .. }));

        // A link to a good directory is refused too.
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        let err = prepare_runtime_dir(&link, uid).unwrap_err();
        assert!(err.to_string().contains("symbolic link"), "{err}");
    }

    #[test]
    fn a_runtime_dir_planted_while_it_was_created_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let uid = fs::metadata(tmp.path()).unwrap().uid();

        // Another user's directory: creating it "succeeds" because it is there.
        let open = tmp.path().join("open");
        let err = prepare_runtime_dir_with(&open, uid, |path| {
            fs::create_dir(path)?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o777))
        })
        .unwrap_err();
        assert!(matches!(err, DaemonError::UnsafeRuntimeDir { .. }), "{err}");
        assert!(err.to_string().contains("0777"), "{err}");

        // A link to a private directory elsewhere.
        let target = tmp.path().join("target");
        private_dir(&target).unwrap();
        let link = tmp.path().join("link");
        let err =
            prepare_runtime_dir_with(&link, uid, |path| std::os::unix::fs::symlink(&target, path))
                .unwrap_err();
        assert!(matches!(err, DaemonError::UnsafeRuntimeDir { .. }), "{err}");
        assert!(err.to_string().contains("symbolic link"), "{err}");

        // What the daemon creates itself passes.
        let own = tmp.path().join("own");
        prepare_runtime_dir_with(&own, uid, private_dir).unwrap();
    }

    #[test]
    fn locks_are_exclusive_and_keep_the_first_pid() {
        let tmp = tempfile::tempdir().unwrap();
        let uid = fs::metadata(tmp.path()).unwrap().uid();
        let base = tmp.path().join("h");
        let paths = DaemonPaths::from_env(
            &|k| (k == "DECK_HOME").then(|| base.to_string_lossy().into_owned()),
            uid,
        )
        .unwrap();
        let first = prepare(&paths, uid).unwrap();
        let pid = std::process::id().to_string();
        assert_eq!(fs::read_to_string(&paths.lock).unwrap(), pid);
        assert!(matches!(
            prepare(&paths, uid).unwrap_err(),
            DaemonError::AlreadyRunning
        ));
        assert_eq!(fs::read_to_string(&paths.lock).unwrap(), pid);

        // Same state, another runtime dir.
        let other = DaemonPaths {
            runtime_dir: tmp.path().join("run2"),
            lock: tmp.path().join("run2").join("deck.lock"),
            socket: tmp.path().join("run2").join("deck.sock"),
            ..paths.clone()
        };
        assert!(matches!(
            prepare(&other, uid).unwrap_err(),
            DaemonError::StateInUse(_)
        ));
        drop(first);
        prepare(&paths, uid).unwrap();
    }
}
