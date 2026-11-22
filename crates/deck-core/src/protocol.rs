//! Messages between the daemon and its clients, and their framing: a `u32` little-endian
//! length, then a `postcard` payload.

use std::ffi::OsString;
use std::path::PathBuf;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::action::{Action, ActionError};
use crate::config::Config;
use crate::model::{SessionId, SessionView, TerminalId};

pub const PROTOCOL_VERSION: u32 = 3;

/// Largest accepted payload.
pub const MAX_FRAME_LEN: usize = 16 * 1024 * 1024;

/// `Hello` must stay the first variant with `version` its first field, so mismatched
/// versions can always read each other's version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientMsg {
    Hello {
        version: u32,
        role: Role,
        #[serde(with = "crate::wire::path")]
        cwd: PathBuf,
        #[serde(with = "crate::wire::env")]
        env: Vec<(OsString, OsString)>,
        colors: Option<OuterColors>,
    },
    Viewport {
        cols: u16,
        rows: u16,
    },
    Input {
        terminal: TerminalId,
        bytes: Vec<u8>,
    },
    Detach,
    Request(Request),
    /// Scrolls a terminal's display through its history.
    Scroll {
        terminal: TerminalId,
        to: ScrollTo,
    },
    /// An action on the attached session, answered by `ServerMsg::ActionResult` in order.
    Action(Action),
    /// Like `Viewport`, but sent because a config reload moved the sidebar: it records the
    /// size without making this client the one the terminals follow.
    ReloadViewport {
        cols: u16,
        rows: u16,
    },
}

/// Where to scroll a terminal's display. Positive counts go back into the history (up),
/// negative ones towards the live screen (down).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScrollTo {
    Lines(i32),
    Pages(i32),
    Top,
    Bottom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    Attach {
        session: Option<String>,
        create: bool,
        cols: u16,
        rows: u16,
    },
    Control,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    ListSessions,
    KillServer,
    /// Creates a session with a project for `cwd`; answered by `Response::Session`.
    NewSession {
        name: Option<String>,
        #[serde(with = "crate::wire::path")]
        cwd: PathBuf,
    },
    KillSession {
        name: String,
    },
    RenameSession {
        from: String,
        to: String,
    },
}

/// The outer terminal's colours as the client found them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OuterColors {
    pub fg: (u8, u8, u8),
    pub bg: (u8, u8, u8),
}

/// `Bye` keeps its variant index in every version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServerMsg {
    Welcome {
        session: SessionId,
        /// The daemon's config: the only one a client uses.
        config: Box<Config>,
    },
    SessionView(SessionView),
    Frame(Frame),
    /// A program's OSC 52 copy, for the client to write to the outer terminal.
    Clipboard {
        selection: String,
        text: String,
    },
    Bye {
        reason: ByeReason,
    },
    Response(Response),
    /// The outcome of the client's `Action`s, one per action, in order.
    ActionResult(Result<(), ActionError>),
    /// The daemon reloaded its config.
    Config(Box<Config>),
    /// A message for the user, shown in the status bar.
    Notice {
        level: NoticeLevel,
        text: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoticeLevel {
    Info,
    Warning,
    Error,
}

/// `VersionMismatch` keeps its shape in every version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ByeReason {
    VersionMismatch { daemon: u32 },
    SessionEnded,
    ServerShutdown,
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Response {
    Sessions(Vec<SessionSummary>),
    Ok,
    Error(String),
    /// The session `Request::NewSession` created.
    Session {
        id: SessionId,
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: SessionId,
    pub name: String,
    pub attached_clients: u32,
    pub terminals: u32,
    pub projects: u32,
    /// Higher is more recently attached; 0 for a session nobody attached yet.
    pub last_attached: u64,
}

/// An update of one terminal's screen. `lines` holds the changed rows, or every row when
/// `full`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    pub terminal: TerminalId,
    pub size: (u16, u16),
    pub cursor: Cursor,
    pub modes: Modes,
    pub display_offset: u32,
    pub history: u32,
    pub lines: Vec<(u16, Line)>,
    pub full: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    pub runs: Vec<Run>,
}

/// Text starting at emulator column `col`. A wide character spans two columns and its
/// spacer cell contributes no text; combining characters follow their base character.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub col: u16,
    pub text: String,
    pub style: Style,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    pub flags: u16,
}

impl Style {
    pub const BOLD: u16 = 1;
    pub const DIM: u16 = 1 << 1;
    pub const ITALIC: u16 = 1 << 2;
    pub const UNDERLINE: u16 = 1 << 3;
    pub const REVERSE: u16 = 1 << 4;
    pub const STRIKETHROUGH: u16 = 1 << 5;
    pub const HIDDEN: u16 = 1 << 6;

    pub fn has(self, flag: u16) -> bool {
        self.flags & flag != 0
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Color {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    pub row: u16,
    pub col: u16,
    pub visible: bool,
    pub shape: CursorShape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CursorShape {
    Block,
    Bar,
    Underline,
}

/// Terminal modes a client needs to encode input, as `u16` flags.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Modes(pub u16);

impl Modes {
    pub const APPLICATION_CURSOR: u16 = 1;
    pub const BRACKETED_PASTE: u16 = 1 << 1;
    pub const MOUSE_CLICK: u16 = 1 << 2;
    pub const MOUSE_DRAG: u16 = 1 << 3;
    pub const MOUSE_MOTION: u16 = 1 << 4;
    pub const SGR_MOUSE: u16 = 1 << 5;
    pub const FOCUS_REPORTING: u16 = 1 << 6;
    pub const ALTERNATE_SCREEN: u16 = 1 << 7;
    /// `?1007`: on the alternate screen the wheel stands for the cursor keys.
    pub const ALTERNATE_SCROLL: u16 = 1 << 8;

    pub fn has(self, flag: u16) -> bool {
        self.0 & flag != 0
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("frame of {len} bytes exceeds the {MAX_FRAME_LEN} byte limit")]
    FrameTooLarge { len: usize },
    #[error("frame is incomplete or its length does not match")]
    BadFrame,
    #[error("cannot decode message: {0}")]
    Decode(#[from] postcard::Error),
    #[error("cannot encode message: {0}")]
    Encode(postcard::Error),
}

/// Encodes a message as a whole frame: length header plus payload. Refuses a payload over
/// [`MAX_FRAME_LEN`], which the peer would reject.
pub fn encode<T: Serialize>(msg: &T) -> Result<Vec<u8>, ProtocolError> {
    let payload = postcard::to_stdvec(msg).map_err(ProtocolError::Encode)?;
    if payload.len() > MAX_FRAME_LEN {
        return Err(ProtocolError::FrameTooLarge { len: payload.len() });
    }
    let mut out = Vec::with_capacity(payload.len() + 4);
    // Cannot truncate: the length is at most MAX_FRAME_LEN.
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&payload);
    Ok(out)
}

/// Decodes one whole frame (as produced by [`encode`]).
pub fn decode<T: DeserializeOwned>(frame: &[u8]) -> Result<T, ProtocolError> {
    let (header, payload) = frame
        .split_first_chunk::<4>()
        .ok_or(ProtocolError::BadFrame)?;
    let len = u32::from_le_bytes(*header) as usize;
    if len > MAX_FRAME_LEN {
        return Err(ProtocolError::FrameTooLarge { len });
    }
    if payload.len() != len {
        return Err(ProtocolError::BadFrame);
    }
    decode_payload(payload)
}

/// Decodes a payload (a frame without its length header).
pub fn decode_payload<T: DeserializeOwned>(payload: &[u8]) -> Result<T, ProtocolError> {
    Ok(postcard::from_bytes(payload)?)
}

/// Reads the `version` of a `ClientMsg::Hello` payload and ignores the rest, so a daemon
/// can answer `VersionMismatch` to a client whose `Hello` has another shape.
pub fn hello_version(payload: &[u8]) -> Option<u32> {
    let (variant, rest) = postcard::take_from_bytes::<u32>(payload).ok()?;
    if variant != 0 {
        return None;
    }
    postcard::take_from_bytes::<u32>(rest)
        .ok()
        .map(|(version, _)| version)
}

/// Yields whole messages from partial reads.
#[derive(Debug, Default)]
pub struct FrameReader {
    buf: Vec<u8>,
}

impl FrameReader {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The next whole payload, `None` when more bytes are needed, an error for a frame over
    /// 16 MiB (judged from its header alone).
    pub fn next_payload(&mut self) -> Result<Option<Vec<u8>>, ProtocolError> {
        let Some(header) = self.buf.first_chunk::<4>() else {
            return Ok(None);
        };
        let len = u32::from_le_bytes(*header) as usize;
        if len > MAX_FRAME_LEN {
            return Err(ProtocolError::FrameTooLarge { len });
        }
        let Some(payload) = self.buf.get(4..4 + len) else {
            return Ok(None);
        };
        let payload = payload.to_vec();
        self.buf.drain(..4 + len);
        Ok(Some(payload))
    }

    /// The next whole message.
    pub fn next_message<T: DeserializeOwned>(&mut self) -> Result<Option<T>, ProtocolError> {
        self.next_payload()?.map(|p| decode_payload(&p)).transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{NameSource, PanelId, PanelView, ProjectId, ProjectView, TerminalView};
    use std::os::unix::ffi::OsStringExt;

    fn view() -> SessionView {
        SessionView {
            id: SessionId(1),
            name: "main".into(),
            active_project: Some(ProjectId(2)),
            active_panel: Some(PanelId(3)),
            layout_viewport: Some((100, 30)),
            projects: vec![ProjectView {
                id: ProjectId(2),
                name: "p".into(),
                root: "/r".into(),
                collapsed: true,
                active_panel: Some(PanelId(3)),
                panels: vec![PanelView {
                    id: PanelId(3),
                    name: "e".into(),
                    name_source: NameSource::Auto,
                    focused_terminal: Some(TerminalId(4)),
                    terminals: vec![TerminalView {
                        id: TerminalId(4),
                        program: "sh".into(),
                        title: Some("t".into()),
                        cwd: Some("/x".into()),
                        exit_status: Some(1),
                    }],
                }],
            }],
        }
    }

    fn frame() -> Frame {
        Frame {
            terminal: TerminalId(4),
            size: (80, 24),
            cursor: Cursor {
                row: 1,
                col: 2,
                visible: true,
                shape: CursorShape::Bar,
            },
            modes: Modes(Modes::APPLICATION_CURSOR | Modes::ALTERNATE_SCREEN),
            display_offset: 3,
            history: 500,
            lines: vec![(
                0,
                Line {
                    runs: vec![
                        Run {
                            col: 0,
                            text: "hi".into(),
                            style: Style {
                                fg: Color::Indexed(2),
                                bg: Color::Rgb(1, 2, 3),
                                flags: Style::BOLD | Style::UNDERLINE,
                            },
                        },
                        Run {
                            col: 5,
                            text: "日e\u{301}".into(),
                            style: Style::default(),
                        },
                    ],
                },
            )],
            full: true,
        }
    }

    fn encode_ok<T: Serialize>(msg: &T) -> Vec<u8> {
        encode(msg).unwrap()
    }

    fn hello(colors: Option<OuterColors>, role: Role) -> ClientMsg {
        ClientMsg::Hello {
            version: PROTOCOL_VERSION,
            role,
            cwd: PathBuf::from(OsString::from_vec(b"/home/\xe9".to_vec())),
            env: vec![
                ("A".into(), "b".into()),
                ("C".into(), OsString::new()),
                (
                    OsString::from_vec(b"K\xff".to_vec()),
                    OsString::from_vec(b"v\xfe".to_vec()),
                ),
            ],
            colors,
        }
    }

    fn client_msgs() -> Vec<ClientMsg> {
        let colors = OuterColors {
            fg: (1, 2, 3),
            bg: (250, 251, 252),
        };
        vec![
            hello(None, Role::Control),
            hello(
                Some(colors),
                Role::Attach {
                    session: None,
                    create: true,
                    cols: 80,
                    rows: 24,
                },
            ),
            hello(
                Some(colors),
                Role::Attach {
                    session: Some("s".into()),
                    create: false,
                    cols: 1,
                    rows: 2,
                },
            ),
            ClientMsg::Viewport {
                cols: 120,
                rows: 40,
            },
            ClientMsg::Input {
                terminal: TerminalId(9),
                bytes: vec![0, 1, 0x1b, 255],
            },
            ClientMsg::ReloadViewport { cols: 71, rows: 23 },
            ClientMsg::Detach,
            ClientMsg::Request(Request::ListSessions),
            ClientMsg::Request(Request::KillServer),
            ClientMsg::Request(Request::NewSession {
                name: Some("s".into()),
                cwd: "/x".into(),
            }),
            ClientMsg::Request(Request::NewSession {
                name: None,
                cwd: "/".into(),
            }),
            ClientMsg::Request(Request::KillSession { name: "s".into() }),
            ClientMsg::Request(Request::RenameSession {
                from: "a".into(),
                to: "b".into(),
            }),
            ClientMsg::Action(Action::KillSession),
            ClientMsg::Scroll {
                terminal: TerminalId(3),
                to: ScrollTo::Lines(-7),
            },
            ClientMsg::Scroll {
                terminal: TerminalId(3),
                to: ScrollTo::Pages(2),
            },
            ClientMsg::Scroll {
                terminal: TerminalId(3),
                to: ScrollTo::Top,
            },
            ClientMsg::Scroll {
                terminal: TerminalId(3),
                to: ScrollTo::Bottom,
            },
        ]
    }

    fn server_msgs() -> Vec<ServerMsg> {
        vec![
            ServerMsg::Welcome {
                session: SessionId(5),
                config: Box::default(),
            },
            ServerMsg::Config(Box::default()),
            ServerMsg::Notice {
                level: NoticeLevel::Info,
                text: "config reloaded".into(),
            },
            ServerMsg::Notice {
                level: NoticeLevel::Warning,
                text: "config.toml:2:10: x ✓".into(),
            },
            ServerMsg::Notice {
                level: NoticeLevel::Error,
                text: String::new(),
            },
            ServerMsg::SessionView(view()),
            ServerMsg::Frame(frame()),
            ServerMsg::Clipboard {
                selection: "c".into(),
                text: "copied ✓".into(),
            },
            ServerMsg::Bye {
                reason: ByeReason::VersionMismatch { daemon: 1 },
            },
            ServerMsg::Bye {
                reason: ByeReason::SessionEnded,
            },
            ServerMsg::Bye {
                reason: ByeReason::ServerShutdown,
            },
            ServerMsg::Bye {
                reason: ByeReason::Error("boom".into()),
            },
            ServerMsg::Response(Response::Sessions(vec![SessionSummary {
                id: SessionId(1),
                name: "main".into(),
                attached_clients: 2,
                terminals: 3,
                projects: 2,
                last_attached: 9,
            }])),
            ServerMsg::Response(Response::Session {
                id: SessionId(2),
                name: "main-2".into(),
            }),
            ServerMsg::ActionResult(Ok(())),
            ServerMsg::ActionResult(Err(ActionError::EmptyName)),
            ServerMsg::Response(Response::Ok),
            ServerMsg::Response(Response::Error("no".into())),
        ]
    }

    #[test]
    fn every_message_round_trips() {
        for msg in client_msgs() {
            assert_eq!(decode::<ClientMsg>(&encode_ok(&msg)).unwrap(), msg);
        }
        for msg in server_msgs() {
            assert_eq!(decode::<ServerMsg>(&encode_ok(&msg)).unwrap(), msg);
        }
    }

    #[test]
    fn reader_reassembles_at_every_split() {
        let msgs = server_msgs();
        let bytes: Vec<u8> = msgs.iter().flat_map(encode_ok).collect();
        for split in 0..=bytes.len() {
            let mut reader = FrameReader::new();
            let mut got = Vec::new();
            for part in [&bytes[..split], &bytes[split..]] {
                reader.push(part);
                while let Some(m) = reader.next_message::<ServerMsg>().unwrap() {
                    got.push(m);
                }
            }
            assert_eq!(got, msgs, "split at {split}");
        }
    }

    #[test]
    fn reader_reassembles_byte_by_byte() {
        let msgs = client_msgs();
        let bytes: Vec<u8> = msgs.iter().flat_map(encode_ok).collect();
        let mut reader = FrameReader::new();
        let mut got = Vec::new();
        for b in &bytes {
            reader.push(&[*b]);
            while let Some(m) = reader.next_message::<ClientMsg>().unwrap() {
                got.push(m);
            }
        }
        assert_eq!(got, msgs);
    }

    #[test]
    fn reader_rejects_oversized_frame() {
        let mut reader = FrameReader::new();
        reader.push(&((MAX_FRAME_LEN as u32) + 1).to_le_bytes());
        assert!(matches!(
            reader.next_payload(),
            Err(ProtocolError::FrameTooLarge { .. })
        ));

        let mut ok = FrameReader::new();
        ok.push(&(MAX_FRAME_LEN as u32).to_le_bytes());
        assert!(ok.next_payload().unwrap().is_none());
    }

    #[test]
    fn hello_version_survives_garbage() {
        let frame = encode_ok(&hello(None, Role::Control));
        let payload = &frame[4..];
        assert_eq!(hello_version(payload), Some(PROTOCOL_VERSION));
        // Truncated after the version.
        assert_eq!(hello_version(&payload[..2]), Some(PROTOCOL_VERSION));
        // Corrupted tail.
        let mut corrupt = payload.to_vec();
        for b in &mut corrupt[2..] {
            *b = 0xff;
        }
        assert_eq!(hello_version(&corrupt), Some(PROTOCOL_VERSION));
        assert!(decode_payload::<ClientMsg>(&corrupt).is_err());
        // Not a Hello.
        assert_eq!(hello_version(&encode_ok(&ClientMsg::Detach)[4..]), None);
        // A future version number.
        let mut future = vec![0u8];
        future.extend(postcard::to_stdvec(&7u32).unwrap());
        future.extend([0xde, 0xad]);
        assert_eq!(hello_version(&future), Some(7));
    }

    #[test]
    fn bye_and_hello_indices_are_stable() {
        assert_eq!(encode_ok(&ClientMsg::Detach)[4], 3);
        assert_eq!(encode_ok(&hello(None, Role::Control))[4], 0);
        let bye = encode_ok(&ServerMsg::Bye {
            reason: ByeReason::VersionMismatch { daemon: 1 },
        });
        assert_eq!(&bye[4..], &[4, 0, 1]);
    }

    #[test]
    fn decode_rejects_bad_frames() {
        assert!(matches!(
            decode::<ClientMsg>(&[1, 0]),
            Err(ProtocolError::BadFrame)
        ));
        let mut frame = encode_ok(&ClientMsg::Detach);
        frame.push(0);
        assert!(matches!(
            decode::<ClientMsg>(&frame),
            Err(ProtocolError::BadFrame)
        ));
    }

    #[test]
    fn non_utf8_paths_round_trip() {
        let mut v = view();
        v.projects[0].root = PathBuf::from(OsString::from_vec(b"/caf\xe9".to_vec()));
        v.projects[0].panels[0].terminals[0].cwd =
            Some(PathBuf::from(OsString::from_vec(b"/\xff\xfe".to_vec())));
        let msg = ServerMsg::SessionView(v);
        assert_eq!(decode::<ServerMsg>(&encode(&msg).unwrap()).unwrap(), msg);
        // The hello of this module carries a non-UTF-8 cwd and env.
        let hello = hello(None, Role::Control);
        assert_eq!(
            decode::<ClientMsg>(&encode(&hello).unwrap()).unwrap(),
            hello
        );
    }

    #[test]
    fn encode_refuses_oversized_payload() {
        let msg = ClientMsg::Input {
            terminal: TerminalId(1),
            bytes: vec![0; MAX_FRAME_LEN + 1],
        };
        assert!(matches!(
            encode(&msg),
            Err(ProtocolError::FrameTooLarge { .. })
        ));
    }
}
