//! The handoff file: everything the new image needs that is not in a descriptor.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use deck_core::model::{PanelId, Session, SessionId, TerminalId};
use deck_core::protocol::{CellPx, NoticeLevel};
use serde::{Deserialize, Serialize};

use super::{HANDOFF_VERSION, b64};

/// What the old daemon hands to the new one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffFile {
    pub handoff_version: u32,
    /// The protocol version the old daemon spoke.
    pub protocol_version: u32,
    /// The old daemon's pid (the new image has the same one).
    pub pid: u32,
    /// How many handoffs came before this one.
    pub generation: u64,
    /// The old daemon's clock when the file was made, in nanoseconds; the new one goes on
    /// from there, so times kept in the model stay meaningful.
    pub clock_nanos: u64,
    pub next_id: u64,
    pub tick: u64,
    pub sessions: Vec<SessionRec>,
    pub notices: Vec<NoticeRec>,
    /// The exit codes `wait --exit` can still ask for, with their age in nanoseconds.
    pub exits: Vec<ExitRec>,
    /// Termination signals that came during the handoff, by name.
    pub signals: Vec<String>,
    pub terminals: Vec<TermRec>,
    pub listener_fd: i32,
    pub lock_fd: i32,
    pub state_lock_fd: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRec {
    pub model: Session,
    #[serde(with = "deck_core::wire::env")]
    pub env: Vec<(OsString, OsString)>,
    pub viewport: Option<(u16, u16)>,
    pub cell: Option<CellPx>,
    pub last_attach: u64,
    pub unseen: Vec<PanelId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoticeRec {
    pub level: NoticeLevel,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitRec {
    pub terminal: TerminalId,
    pub code: i32,
    pub age_nanos: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermRec {
    pub id: TerminalId,
    pub session: SessionId,
    pub program: String,
    /// `None` for a kept terminal whose program ended.
    pub pid: Option<u32>,
    /// The number of the PTY master's descriptor; `None` with no pid.
    pub fd: Option<i32>,
    pub cols: u16,
    pub rows: u16,
    pub cell: Option<CellPx>,
    pub reports_cwd: bool,
    /// An exit status the old daemon had already reaped.
    pub reaped: Option<i32>,
    /// The exit code of a terminal that ended and was read to the end.
    pub exited: Option<i32>,
    pub started_nanos: u64,
    pub last_data_nanos: u64,
    #[serde(with = "b64::bytes")]
    pub snapshot: Vec<u8>,
    /// What the reader held back when it paused.
    #[serde(with = "b64::bytes")]
    pub held: Vec<u8>,
}

/// Writes `file` to an unlinked 0600 file in `dir` and leaves it positioned at the start,
/// ready to be read through the same descriptor.
pub fn write(dir: &Path, file: &HandoffFile) -> io::Result<File> {
    let path = dir.join(format!("deck.handoff.{}.tmp", std::process::id()));
    match fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let mut out = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    // Unlinked at once: the descriptor is all that is left of it.
    fs::remove_file(&path)?;
    let bytes = serde_json::to_vec(file).map_err(io::Error::other)?;
    out.write_all(&bytes)?;
    out.flush()?;
    out.seek(SeekFrom::Start(0))?;
    Ok(out)
}

/// Reads and checks a handoff file, from its start.
pub fn read(mut from: File) -> Result<HandoffFile, String> {
    from.seek(SeekFrom::Start(0))
        .map_err(|e| format!("cannot rewind the handoff file: {e}"))?;
    let mut bytes = Vec::new();
    from.read_to_end(&mut bytes)
        .map_err(|e| format!("cannot read the handoff file: {e}"))?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("the handoff file is not JSON: {e}"))?;
    let version = value
        .get("handoff_version")
        .and_then(serde_json::Value::as_u64)
        .ok_or("the handoff file has no version")?;
    if version == 0 || version > u64::from(HANDOFF_VERSION) {
        return Err(format!(
            "the handoff file has version {version}; this deck reads 1 to {HANDOFF_VERSION}"
        ));
    }
    serde_json::from_value(value).map_err(|e| format!("the handoff file is not valid: {e}"))
}

#[cfg(test)]
mod tests {
    use deck_core::model::{
        LaunchSpec, NameSource, Panel, PanelId, Project, ProjectId, Session, SessionId, Terminal,
    };

    use super::*;

    pub(crate) fn sample() -> HandoffFile {
        let terminal = Terminal {
            id: TerminalId(4),
            launch: LaunchSpec {
                program: Some("sh".into()),
                args: vec!["-c".into(), "echo".into()],
                cwd: "/tmp".into(),
                env: vec![(OsString::from("A"), OsString::from("b"))],
            },
            program: "sh".into(),
            title: Some("a title".into()),
            cwd: Some("/var".into()),
            exit_status: Some(3),
            keep: true,
            agent: None,
        };
        let model = Session {
            id: SessionId(1),
            name: "main".into(),
            active_project: Some(ProjectId(2)),
            projects: vec![Project {
                id: ProjectId(2),
                name: "p".into(),
                root: "/tmp".into(),
                collapsed: false,
                active_panel: Some(PanelId(3)),
                panels: vec![Panel {
                    id: PanelId(3),
                    name: "1".into(),
                    name_source: NameSource::Default,
                    focused_terminal: Some(TerminalId(4)),
                    ratios: deck_core::layout::Ratios::default_for(1),
                    terminals: vec![terminal],
                }],
            }],
        };
        HandoffFile {
            handoff_version: HANDOFF_VERSION,
            protocol_version: 7,
            pid: 99,
            generation: 2,
            clock_nanos: 123_456_789,
            next_id: 10,
            tick: 5,
            sessions: vec![SessionRec {
                model,
                env: vec![(OsString::from("HOME"), OsString::from("/home/x"))],
                viewport: Some((100, 30)),
                cell: Some(CellPx {
                    width: 8,
                    height: 16,
                }),
                last_attach: 4,
                unseen: vec![PanelId(3)],
            }],
            notices: vec![NoticeRec {
                level: NoticeLevel::Warning,
                text: "careful".into(),
            }],
            exits: vec![ExitRec {
                terminal: TerminalId(8),
                code: 143,
                age_nanos: 5_000,
            }],
            signals: vec!["SIGTERM".into()],
            terminals: vec![
                TermRec {
                    id: TerminalId(4),
                    session: SessionId(1),
                    program: "sh".into(),
                    pid: Some(555),
                    fd: Some(9),
                    cols: 80,
                    rows: 24,
                    cell: None,
                    reports_cwd: true,
                    reaped: Some(7),
                    exited: None,
                    started_nanos: 11,
                    last_data_nanos: 22,
                    snapshot: b"\x1b[1mhello\x1b[0m\r\n\xff\x00".to_vec(),
                    held: b"\x1b[3".to_vec(),
                },
                TermRec {
                    id: TerminalId(5),
                    session: SessionId(1),
                    program: "sh".into(),
                    pid: None,
                    fd: None,
                    cols: 10,
                    rows: 5,
                    cell: Some(CellPx {
                        width: 9,
                        height: 18,
                    }),
                    reports_cwd: false,
                    reaped: None,
                    exited: Some(0),
                    started_nanos: 0,
                    last_data_nanos: 0,
                    snapshot: Vec::new(),
                    held: Vec::new(),
                },
            ],
            listener_fd: 5,
            lock_fd: 6,
            state_lock_fd: 7,
        }
    }

    #[test]
    fn every_field_survives_a_round_trip_through_an_unlinked_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = sample();
        let handle = write(dir.path(), &file).unwrap();
        // Nothing is left on disk.
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
        assert_eq!(read(handle).unwrap(), file);
    }

    #[test]
    fn the_file_is_private() {
        use std::os::unix::fs::MetadataExt;
        let dir = tempfile::tempdir().unwrap();
        let handle = write(dir.path(), &sample()).unwrap();
        assert_eq!(handle.metadata().unwrap().mode() & 0o777, 0o600);
    }

    #[test]
    fn a_newer_or_missing_version_and_junk_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut newer = serde_json::to_value(sample()).unwrap();
        newer["handoff_version"] = (HANDOFF_VERSION + 1).into();
        let path = dir.path().join("f");
        for (text, why) in [
            (newer.to_string(), "version"),
            ("{}".to_string(), "no version"),
            ("nonsense".to_string(), "not JSON"),
            (r#"{"handoff_version":1}"#.to_string(), "not valid"),
        ] {
            fs::write(&path, text).unwrap();
            let err = read(File::open(&path).unwrap()).unwrap_err();
            assert!(err.contains(why), "{err}");
        }
    }
}
