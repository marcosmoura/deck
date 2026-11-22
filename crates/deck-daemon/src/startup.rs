//! Start-up: the runtime and state directories and the two locks.

use std::fs::{self, DirBuilder, File, TryLockError};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
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
    _runtime: File,
    _state: File,
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
    Ok(Locks {
        _runtime: runtime,
        _state: state,
    })
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
    match fs::symlink_metadata(path) {
        Ok(meta) => {
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
        Err(e) if e.kind() == io::ErrorKind::NotFound => private_dir(path)
            .map_err(|e| DaemonError::io(format!("cannot create {}", path.display()), e)),
        Err(e) => Err(DaemonError::io(
            format!("cannot inspect {}", path.display()),
            e,
        )),
    }
}

fn lock_file(path: &Path, busy: impl FnOnce() -> DaemonError) -> Result<File, DaemonError> {
    let file = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| DaemonError::io(format!("cannot open {}", path.display()), e))?;
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
    use std::os::unix::fs::PermissionsExt;

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
