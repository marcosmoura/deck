//! `config.toml` in the daemon: the read at start and the watch for changes.
//!
//! The daemon is the only reader. Every poll compares the file's stamp (modification time,
//! change time, size and inode) with the last one seen; the path is looked at each time,
//! so an editor that saves through a rename is seen too.

use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use deck_core::config::{Config, Severity, validate};
use deck_core::protocol::NoticeLevel;

/// A message for the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub level: NoticeLevel,
    pub text: String,
}

impl Notice {
    fn new(level: NoticeLevel, text: impl Into<String>) -> Self {
        Self {
            level,
            text: text.into(),
        }
    }
}

/// What the daemon starts with.
#[derive(Debug)]
pub struct Started {
    pub config: Config,
    pub notices: Vec<Notice>,
}

/// What identifies a version of the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    dev: u64,
    ino: u64,
    size: u64,
    mtime: (i64, i64),
    ctime: (i64, i64),
}

/// `None` for a file that does not exist.
fn stat(path: &Path) -> io::Result<Option<Stamp>> {
    match fs::metadata(path) {
        Ok(m) => Ok(Some(Stamp {
            dev: m.dev(),
            ino: m.ino(),
            size: m.size(),
            mtime: (m.mtime(), m.mtime_nsec()),
            ctime: (m.ctime(), m.ctime_nsec()),
        })),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// A change of the file the daemon must act on.
#[derive(Debug)]
pub enum Change {
    /// A valid file (or none): the new config and the notices about it.
    Reload {
        config: Box<Config>,
        notices: Vec<Notice>,
    },
    /// An invalid file: the previous config stays.
    Rejected(Notice),
}

/// The file being watched.
#[derive(Debug)]
pub struct ConfigFile {
    path: PathBuf,
    name: String,
    /// The stamp of the version last handled; `None` when the file did not exist.
    last: Option<Stamp>,
    /// The text of the last version applied.
    last_text: Option<String>,
    /// An invalid version, with the error to report once it stays the same for a tick.
    failed: Option<(Option<Stamp>, Notice)>,
}

/// Validates `text`: the config and the warning notice, or the first error as a notice
/// ending in `suffix`.
fn evaluate(name: &str, text: &str, suffix: &str) -> Result<(Config, Vec<Notice>), Notice> {
    let report = validate(text);
    if let Some(error) = report
        .findings
        .iter()
        .find(|f| f.severity == Severity::Error)
    {
        return Err(Notice::new(
            NoticeLevel::Error,
            format!(
                "{name}:{}:{}: {} ({suffix})",
                error.line, error.column, error.message
            ),
        ));
    }
    let config = deck_core::config::Config::parse(text)
        .map_err(|e| Notice::new(NoticeLevel::Error, format!("{name}: {e} ({suffix})")))?
        .config;
    let warnings: Vec<_> = report.findings.iter().collect();
    let mut notices = Vec::new();
    if let Some(first) = warnings.first() {
        let more = match warnings.len() - 1 {
            0 => String::new(),
            n => format!(" (and {n} more)"),
        };
        notices.push(Notice::new(
            NoticeLevel::Warning,
            format!(
                "{name}:{}:{}: {}{more}",
                first.line, first.column, first.message
            ),
        ));
    }
    Ok((config, notices))
}

impl ConfigFile {
    /// Reads the file for the start: the defaults (and an error notice) when it is invalid.
    pub fn open(path: PathBuf) -> (Self, Started) {
        let name = path.file_name().map_or_else(
            || "config.toml".into(),
            |n| n.to_string_lossy().into_owned(),
        );
        let mut file = Self {
            path,
            name,
            last: None,
            last_text: None,
            failed: None,
        };
        // The stamp first: a change after it is seen by the first poll.
        let stamp = stat(&file.path);
        file.last = stamp.as_ref().ok().copied().flatten();
        let text = match fs::read_to_string(&file.path) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => {
                let notice = Notice::new(
                    NoticeLevel::Error,
                    format!(
                        "{}: cannot read the file: {e} (using the defaults)",
                        file.name
                    ),
                );
                return (
                    file,
                    Started {
                        config: Config::default(),
                        notices: vec![notice],
                    },
                );
            }
        };
        let started = match text {
            None => Started {
                config: Config::default(),
                notices: Vec::new(),
            },
            Some(text) => match evaluate(&file.name, &text, "using the defaults") {
                Ok((config, notices)) => {
                    file.last_text = Some(text);
                    Started { config, notices }
                }
                Err(notice) => Started {
                    config: Config::default(),
                    notices: vec![notice],
                },
            },
        };
        (file, started)
    }

    /// Looks at the file once; `Some` when there is something to apply or to report.
    pub fn poll(&mut self) -> Option<Change> {
        let now = match stat(&self.path) {
            Ok(now) => now,
            Err(e) => {
                tracing::debug!("cannot look at {}: {e}", self.path.display());
                return None;
            }
        };
        match &self.failed {
            // Still the same invalid version a tick later: it is not being written.
            Some((stamp, _)) if *stamp == now => {
                let (_, notice) = self.failed.take()?;
                self.last = now;
                self.last_text = None;
                return Some(Change::Rejected(notice));
            }
            None if now == self.last => return None,
            _ => {}
        }
        self.failed = None;

        let Some(stamp) = now else {
            self.last = None;
            self.last_text = None;
            return Some(Change::Reload {
                config: Box::default(),
                notices: vec![Notice::new(NoticeLevel::Info, "config reloaded")],
            });
        };
        let text = match fs::read_to_string(&self.path) {
            Ok(text) => text,
            // Gone since the stamp: the next poll sees it.
            Err(e) if e.kind() == io::ErrorKind::NotFound => return None,
            Err(e) => {
                let notice = Notice::new(
                    NoticeLevel::Error,
                    format!(
                        "{}: cannot read the file: {e} (kept the previous config)",
                        self.name
                    ),
                );
                self.failed = Some((now, notice));
                return None;
            }
        };
        match evaluate(&self.name, &text, "kept the previous config") {
            Ok((config, mut notices)) => {
                self.last = Some(stamp);
                if self.last_text.as_deref() == Some(text.as_str()) {
                    // Touched, not edited.
                    return None;
                }
                self.last_text = Some(text);
                notices.insert(0, Notice::new(NoticeLevel::Info, "config reloaded"));
                Some(Change::Reload {
                    config: Box::new(config),
                    notices,
                })
            }
            Err(notice) => {
                self.failed = Some((now, notice));
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(dir: &Path, text: &str) -> (ConfigFile, Started) {
        let path = dir.join("config.toml");
        fs::write(&path, text).unwrap();
        ConfigFile::open(path)
    }

    #[test]
    fn a_missing_file_is_the_defaults_without_a_notice() {
        let dir = tempfile::tempdir().unwrap();
        let (mut f, started) = ConfigFile::open(dir.path().join("config.toml"));
        assert_eq!(started.config, Config::default());
        assert!(started.notices.is_empty());
        assert!(f.poll().is_none());
    }

    #[test]
    fn an_invalid_file_at_start_is_the_defaults_with_an_error_notice() {
        let dir = tempfile::tempdir().unwrap();
        let (_, started) = file(dir.path(), "[theme]\naccent = \"purple\"\n");
        assert_eq!(started.config, Config::default());
        let [notice] = started.notices.as_slice() else {
            panic!("{:?}", started.notices)
        };
        assert_eq!(notice.level, NoticeLevel::Error);
        assert!(
            notice.text.starts_with("config.toml:2:10: ")
                && notice.text.ends_with("(using the defaults)"),
            "{}",
            notice.text
        );
    }

    #[test]
    fn an_edit_is_applied_a_touch_is_not_and_a_deletion_returns_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let (mut f, _) = file(dir.path(), "sidebar_width = 28\n");
        assert!(f.poll().is_none());

        // The same size, within the same tick: the content differs.
        fs::write(dir.path().join("config.toml"), "sidebar_width = 40\n").unwrap();
        let Some(Change::Reload { config, notices }) = f.poll() else {
            panic!("an edit is a reload")
        };
        assert_eq!(config.sidebar_width, 40);
        assert_eq!(notices, [Notice::new(NoticeLevel::Info, "config reloaded")]);
        assert!(f.poll().is_none());

        fs::write(dir.path().join("config.toml"), "sidebar_width = 40\n").unwrap();
        assert!(f.poll().is_none(), "the same text is not a change");

        fs::remove_file(dir.path().join("config.toml")).unwrap();
        let Some(Change::Reload { config, .. }) = f.poll() else {
            panic!("a deletion is a reload")
        };
        assert_eq!(*config, Config::default());
        assert!(f.poll().is_none());
    }

    #[test]
    fn a_rename_over_the_file_is_seen() {
        let dir = tempfile::tempdir().unwrap();
        let (mut f, _) = file(dir.path(), "sidebar_width = 28\n");
        let tmp = dir.path().join("config.toml.tmp");
        fs::write(&tmp, "sidebar_width = 29\n").unwrap();
        fs::rename(&tmp, dir.path().join("config.toml")).unwrap();
        let Some(Change::Reload { config, .. }) = f.poll() else {
            panic!("a rename is a reload")
        };
        assert_eq!(config.sidebar_width, 29);
    }

    #[test]
    fn an_invalid_edit_is_reported_once_it_has_settled_and_the_fix_applies() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let (mut f, _) = file(dir.path(), "sidebar_width = 28\n");

        fs::write(&path, "sidebar_width = 28\n[theme]\naccent = \"purple\"\n").unwrap();
        assert!(
            f.poll().is_none(),
            "the first look may be a half-written file"
        );
        // Written again: read again instead of reporting.
        fs::write(&path, "sidebar_width = 28\n[theme]\naccent = \"purple2\"\n").unwrap();
        assert!(f.poll().is_none());
        let Some(Change::Rejected(notice)) = f.poll() else {
            panic!("a settled invalid file is rejected")
        };
        assert_eq!(notice.level, NoticeLevel::Error);
        assert!(
            notice.text.starts_with("config.toml:3:10: ")
                && notice.text.ends_with("(kept the previous config)"),
            "{}",
            notice.text
        );
        assert!(f.poll().is_none());

        // Fixed back to the text that was in force: still announced.
        fs::write(&path, "sidebar_width = 28\n").unwrap();
        assert!(matches!(f.poll(), Some(Change::Reload { .. })));
    }

    #[test]
    fn warnings_come_as_one_notice_and_do_not_block() {
        let dir = tempfile::tempdir().unwrap();
        let (mut f, _) = file(dir.path(), "");
        fs::write(
            dir.path().join("config.toml"),
            "colour = 1\n[keys]\ndetach = \"n\"\n",
        )
        .unwrap();
        let Some(Change::Reload { notices, .. }) = f.poll() else {
            panic!("warnings do not block")
        };
        let levels: Vec<_> = notices.iter().map(|n| n.level).collect();
        assert_eq!(levels, [NoticeLevel::Info, NoticeLevel::Warning]);
        assert!(
            notices[1].text.contains("(and 1 more)"),
            "{}",
            notices[1].text
        );
    }
}
