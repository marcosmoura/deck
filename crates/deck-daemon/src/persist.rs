//! `state.json` on disk: atomic writes, and reading it back at start.

use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use deck_core::state::{STATE_VERSION, StateFile};

use crate::error::DaemonError;
use crate::startup::DaemonPaths;

/// What reading `state.json` found.
#[derive(Debug)]
pub(crate) enum Loaded {
    /// No file.
    Missing,
    /// A file, validated; the lines say what had to change.
    State(Box<StateFile>, Vec<String>),
    /// An unreadable file, moved aside; the line says why.
    Bad(String),
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// The leftover of a save that did not finish.
pub(crate) fn tmp_file(state: &Path) -> PathBuf {
    sibling(state, ".tmp")
}

/// Writes the file whole and atomically: a temp file in the same directory, fsynced,
/// renamed over `state.json`, then the directory fsynced.
pub(crate) fn write(state: &Path, file: &StateFile) -> io::Result<()> {
    let dir = state.parent().unwrap_or_else(|| Path::new("."));
    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let tmp = tmp_file(state);
    match fs::remove_file(&tmp) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let bytes = serde_json::to_vec_pretty(file).map_err(io::Error::other)?;
    let mut out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    out.write_all(&bytes)?;
    out.sync_all()?;
    drop(out);
    fs::rename(&tmp, state)?;
    File::open(dir)?.sync_all()
}

/// Deletes a leftover temp file.
pub(crate) fn remove_tmp(state: &Path) {
    if let Err(e) = fs::remove_file(tmp_file(state))
        && e.kind() != io::ErrorKind::NotFound
    {
        tracing::warn!("cannot remove the leftover state.json.tmp: {e}");
    }
}

/// What is wrong with a JSON text, without its contents: serde's own message quotes the
/// value it stopped at, which can be an environment value, and this text goes to the log.
pub(crate) fn json_error(e: &serde_json::Error) -> String {
    use serde_json::error::Category;

    let kind = match e.classify() {
        Category::Io => "read error",
        Category::Syntax => "syntax error",
        Category::Data => "unexpected value",
        Category::Eof => "unexpected end",
    };
    if e.line() == 0 {
        kind.to_string()
    } else {
        format!("{kind} at line {}, column {}", e.line(), e.column())
    }
}

fn newer_version(value: &serde_json::Value) -> Option<u32> {
    let version = value.get("version")?.as_u64()?;
    let version = u32::try_from(version).unwrap_or(u32::MAX);
    (version > STATE_VERSION).then_some(version)
}

/// Reads and validates `state.json`. A file from a newer deck is an error and stays as it
/// is; one that does not parse is renamed to `state.json.bad-<unix seconds>`.
pub(crate) fn load(paths: &DaemonPaths) -> Result<Loaded, DaemonError> {
    let path = paths.state_file();
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Loaded::Missing),
        Err(e) => {
            return Err(DaemonError::io(
                format!("cannot read {}", path.display()),
                e,
            ));
        }
    };
    let parsed = serde_json::from_slice::<serde_json::Value>(&bytes)
        .map_err(|e| json_error(&e))
        .and_then(|value| match newer_version(&value) {
            Some(version) => Ok(Err(version)),
            None => serde_json::from_value::<StateFile>(value)
                .map(Ok)
                .map_err(|e| {
                    // An error of a parsed value has no position: the text gives it.
                    let e = serde_json::from_slice::<StateFile>(&bytes)
                        .err()
                        .unwrap_or(e);
                    json_error(&e)
                }),
        });
    match parsed {
        Ok(Err(version)) => Err(DaemonError::NewerState(version)),
        Ok(Ok(mut file)) => {
            let notes = file.validate();
            Ok(Loaded::State(Box::new(file), notes))
        }
        Err(reason) => {
            let secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let aside = sibling(&path, &format!(".bad-{secs}"));
            // Left in place, the first save would overwrite it: refuse to start instead.
            fs::rename(&path, &aside).map_err(|e| {
                DaemonError::io(
                    format!(
                        "{} could not be read ({reason}) and could not be moved aside",
                        path.display()
                    ),
                    e,
                )
            })?;
            Ok(Loaded::Bad(format!(
                "state.json could not be read ({reason}); moved to {}",
                aside.display()
            )))
        }
    }
}

/// How many sessions `state.json` holds; 0 when it is missing or unreadable. For `deck ls`
/// without a daemon.
pub fn saved_session_count(paths: &DaemonPaths) -> usize {
    let Ok(bytes) = fs::read(paths.state_file()) else {
        return 0;
    };
    serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|v| v.get("sessions")?.as_array().map(Vec::len))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    fn paths(dir: &Path) -> DaemonPaths {
        DaemonPaths::from_env(
            &|k| (k == "DECK_HOME").then(|| dir.to_string_lossy().into_owned()),
            0,
        )
        .unwrap()
    }

    fn empty() -> StateFile {
        StateFile::new(5, vec![], [])
    }

    #[test]
    fn a_write_is_private_complete_and_leaves_no_temp_file() {
        let home = tempfile::tempdir().unwrap();
        let paths = paths(home.path());
        write(&paths.state_file(), &empty()).unwrap();
        assert_eq!(
            fs::metadata(&paths.state_dir).unwrap().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(paths.state_file()).unwrap().mode() & 0o777,
            0o600
        );
        assert!(!tmp_file(&paths.state_file()).exists());
        let Loaded::State(file, notes) = load(&paths).unwrap() else {
            panic!("a state")
        };
        assert_eq!((file.next_id, notes.len()), (5, 0));
        // Writing again replaces it.
        write(&paths.state_file(), &empty()).unwrap();
    }

    #[test]
    fn a_reader_never_sees_a_half_written_file() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let home = tempfile::tempdir().unwrap();
        let paths = paths(home.path());
        write(&paths.state_file(), &empty()).unwrap();
        let done = Arc::new(AtomicBool::new(false));
        let reader = {
            let (path, done) = (paths.state_file(), Arc::clone(&done));
            std::thread::spawn(move || {
                let mut reads = 0u32;
                while !done.load(Ordering::Acquire) {
                    let bytes = fs::read(&path).expect("the file always exists");
                    serde_json::from_slice::<StateFile>(&bytes).expect("a whole file");
                    reads += 1;
                }
                reads
            })
        };
        // A bigger file than the test default makes a torn write more likely.
        let mut big = empty();
        big.last_attached = (0..2000).map(deck_core::model::SessionId).collect();
        for _ in 0..100 {
            write(&paths.state_file(), &big).unwrap();
        }
        done.store(true, Ordering::Release);
        assert!(reader.join().unwrap() > 0);
    }

    #[test]
    fn a_missing_file_is_nothing_and_junk_is_moved_aside() {
        let home = tempfile::tempdir().unwrap();
        let paths = paths(home.path());
        assert!(matches!(load(&paths).unwrap(), Loaded::Missing));
        fs::create_dir_all(&paths.state_dir).unwrap();
        fs::write(paths.state_file(), b"{ nope").unwrap();
        let Loaded::Bad(why) = load(&paths).unwrap() else {
            panic!("bad")
        };
        assert!(why.contains("state.json.bad-"), "{why}");
        assert!(!paths.state_file().exists());
        let names: Vec<String> = fs::read_dir(&paths.state_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.iter().any(|n| n.starts_with("state.json.bad-")));
    }

    /// Privacy review, P-3: serde's message would quote the value into `daemon.log`.
    #[test]
    fn a_bad_file_is_reported_without_its_contents() {
        let home = tempfile::tempdir().unwrap();
        let paths = paths(home.path());
        fs::create_dir_all(&paths.state_dir).unwrap();
        // A whole state whose saved `--env` was edited by hand into the wrong shape.
        let model = crate::handoff::file::tests::sample()
            .sessions
            .remove(0)
            .model;
        let mut value = serde_json::to_value(StateFile::new(5, vec![], [&model])).unwrap();
        value["sessions"][0]["projects"][0]["panels"][0]["terminals"][0]["env"] =
            serde_json::json!(["API_TOKEN=handedit-sk-12345"]);
        let text = serde_json::to_string_pretty(&value).unwrap();
        fs::write(paths.state_file(), &text).unwrap();
        let Loaded::Bad(why) = load(&paths).unwrap() else {
            panic!("bad")
        };
        assert!(!why.contains("handedit"), "{why}");
        assert!(!why.contains("API_TOKEN"), "{why}");
        let line = 1 + text.lines().position(|l| l.contains("handedit")).unwrap();
        assert!(
            why.contains(&format!("unexpected value at line {line}, column ")),
            "{why}"
        );

        // Text that is not JSON at all: where it stops, not what it holds.
        fs::write(paths.state_file(), b"{\"token\": handedit-sk-12345}").unwrap();
        let Loaded::Bad(why) = load(&paths).unwrap() else {
            panic!("bad")
        };
        assert!(!why.contains("handedit"), "{why}");
        assert!(why.contains("syntax error at line 1, column "), "{why}");
    }

    #[test]
    fn a_bad_file_that_cannot_be_moved_aside_stops_the_daemon() {
        use std::os::unix::fs::PermissionsExt;

        let home = tempfile::tempdir().unwrap();
        let paths = paths(home.path());
        fs::create_dir_all(&paths.state_dir).unwrap();
        fs::write(paths.state_file(), b"junk").unwrap();
        fs::set_permissions(&paths.state_dir, fs::Permissions::from_mode(0o500)).unwrap();
        let result = load(&paths);
        fs::set_permissions(&paths.state_dir, fs::Permissions::from_mode(0o700)).unwrap();
        let err = result.unwrap_err();
        assert!(err.to_string().contains("state.json"), "{err}");
        assert_eq!(fs::read(paths.state_file()).unwrap(), b"junk");
    }

    #[test]
    fn a_newer_file_is_refused_and_left_alone() {
        let home = tempfile::tempdir().unwrap();
        let paths = paths(home.path());
        fs::create_dir_all(&paths.state_dir).unwrap();
        let text = br#"{"version": 2, "something": "new"}"#;
        fs::write(paths.state_file(), text).unwrap();
        let err = load(&paths).unwrap_err();
        assert!(matches!(err, DaemonError::NewerState(2)), "{err}");
        assert_eq!(fs::read(paths.state_file()).unwrap(), text);
    }

    #[test]
    fn saved_sessions_are_counted_without_reading_them_all() {
        let home = tempfile::tempdir().unwrap();
        let paths = paths(home.path());
        assert_eq!(saved_session_count(&paths), 0);
        fs::create_dir_all(&paths.state_dir).unwrap();
        fs::write(paths.state_file(), br#"{"sessions": [{}, {}]}"#).unwrap();
        assert_eq!(saved_session_count(&paths), 2);
        fs::write(paths.state_file(), b"junk").unwrap();
        assert_eq!(saved_session_count(&paths), 0);
    }
}
