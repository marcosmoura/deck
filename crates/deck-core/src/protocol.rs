//! Messages between the daemon and its clients, and their framing: a `u32` little-endian
//! length, then a `postcard` payload.

use std::ffi::OsString;
use std::path::PathBuf;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::action::{Action, ActionError};
use crate::agent_sessions::AgentSession;
use crate::agents::{AgentKind, AgentStatus};
use crate::config::Config;
use crate::keys::KeySpec;
use crate::model::{LaunchSpec, PanelId, SessionId, SessionView, TerminalId};

pub const PROTOCOL_VERSION: u32 = 22;

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
        /// The outer terminal's cell size in pixels, `None` when unknown.
        cell: Option<CellPx>,
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
        cell: Option<CellPx>,
    },
    /// Whether the outer terminal has focus. A client that never reported counts as focused.
    Focus(bool),
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
        /// The outer terminal's cell size in pixels, `None` when unknown.
        cell: Option<CellPx>,
        /// Whether the client can show images.
        images: bool,
        /// The outer terminal's name from its XTVERSION answer (printable, at most 64
        /// characters), `None` when it gave none.
        outer_name: Option<String>,
    },
    Control,
}

/// The outer terminal's cell size in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellPx {
    pub width: u16,
    pub height: u16,
}

/// The screen a kitty image belongs to; each has its own image store and ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ImageScreen {
    Primary,
    Alternate,
}

/// The layout of an image's pixels: 3 or 4 bytes per pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageFormat {
    Rgb,
    Rgba,
}

/// A rectangle of an image, in image pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Where a placement is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlacementKind {
    /// The top-left cell relative to the viewport; negative when the image starts above or
    /// left of it.
    Placed { col: i32, row: i32 },
    /// A virtual placement (`U=1`), shown only where the program writes placeholder cells.
    Virtual,
}

/// One kitty image placement touching a terminal's viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placement {
    pub screen: ImageScreen,
    /// The program's image id.
    pub image: u32,
    /// The image's generation; it changes when the image is transmitted again.
    pub generation: u64,
    pub kind: PlacementKind,
    /// The cells the placement covers.
    pub cols: u16,
    pub rows: u16,
    pub source: SourceRect,
    /// The rendered size in pixels.
    pub pixels: (u32, u32),
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
    /// One session's tree (by name), or every session's; answered by `Response::Tree`.
    Tree {
        session: Option<String>,
    },
    /// Adds a project with one panel to a session; answered by `Response::Created` with the
    /// project's id.
    NewProject {
        session: SessionId,
        name: String,
        #[serde(with = "crate::wire::path")]
        root: PathBuf,
        focus: bool,
    },
    /// Adds a panel with one terminal to a project of the session; answered by
    /// `Response::Created` with the panel's id.
    NewPanel {
        session: SessionId,
        project: Target,
        name: Option<String>,
        launch: Option<LaunchSpec>,
        focus: bool,
    },
    /// Adds a terminal to a panel; answered by `Response::Created` with the terminal's id.
    NewTerminal {
        panel: PanelId,
        launch: Option<LaunchSpec>,
        keep: bool,
        focus: bool,
    },
    /// Focuses a terminal (and its panel and project) or a panel, by its daemon-wide id.
    Focus {
        id: u64,
    },
    /// Closes a terminal, a panel or a project, by its daemon-wide id.
    Close {
        id: u64,
    },
    RenamePanel {
        panel: PanelId,
        name: String,
    },
    /// Gives a panel its live name back (`deck rename-panel --auto`).
    AutoNamePanel {
        panel: PanelId,
    },
    /// Types bytes into a terminal; with `paste` they are wrapped in bracketed-paste markers
    /// when the terminal asked for them. Answered by `Ok`.
    Send {
        terminal: TerminalId,
        bytes: Vec<u8>,
        paste: bool,
    },
    /// Types keys into a terminal, encoded for the terminal's current modes. Answered by `Ok`.
    SendKeys {
        terminal: TerminalId,
        keys: Vec<KeySpec>,
    },
    /// Reads a terminal's screen as text (`lines`: only the last ones; `history`: with the
    /// scrollback; `ansi`: with colours and attributes); answered by `Response::Captured`.
    Capture {
        terminal: TerminalId,
        lines: Option<u32>,
        history: bool,
        ansi: bool,
    },
    /// Answers, after a while, with `Response::Waited`; `timeout_ms` is enforced by the
    /// daemon.
    Wait {
        terminal: TerminalId,
        until: WaitFor,
        timeout_ms: Option<u64>,
    },
    /// An agent's state, from its hooks (or a test): sets the terminal's agent state with
    /// `source: Hooks`. `status: None` leaves the status as it is; `raw` is an agent's own
    /// event, still to be interpreted (OpenCode). Answered by `Ok`, or an error for a
    /// terminal that does not exist.
    AgentEvent {
        terminal: TerminalId,
        kind: AgentKind,
        status: Option<AgentStatus>,
        session_id: Option<String>,
        prompt: Option<String>,
        raw: Option<String>,
    },
    /// Lists an agent's recent sessions, newest first, at most `limit` (clamped to 1 to 100).
    /// Needs an attached session. Answered by `Response::AgentSessions`, which is not in
    /// order with the other answers: the client routes it by `kind`.
    ListAgentSessions {
        kind: AgentKind,
        limit: u16,
    },
    /// What the daemon uses and what it holds; answered by `Response::Stats`.
    Stats,
}

/// What a `Request::Wait` waits for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WaitFor {
    /// The process exits.
    Exit,
    /// A line of the visible screen contains the text.
    Text(String),
    /// The terminal printed nothing for `quiet_ms`.
    Idle { quiet_ms: u64 },
}

/// How a `Request::Wait` ended well.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Waited {
    /// The process exited with this code (a death by signal `n` is `128 + n`).
    Exit(i32),
    /// The first visible line that contains the text.
    Text(String),
    Idle,
    TimedOut,
}

/// A terminal's screen as text, from `Request::Capture`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Captured {
    pub terminal: TerminalId,
    /// The terminal's size in columns and rows.
    pub size: (u16, u16),
    /// The cursor's row and column on the visible screen.
    pub cursor: (u16, u16),
    pub alternate_screen: bool,
    /// How many earlier lines the size cap left out (0 when none).
    pub truncated: u32,
    /// One entry per line, trailing spaces trimmed.
    pub lines: Vec<String>,
}

/// A project named by id or by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    Id(u64),
    Name(String),
}

impl Target {
    /// All digits is an id, anything else a name.
    pub fn parse(text: &str) -> Self {
        match text.parse::<u64>() {
            Ok(id) if text.bytes().all(|b| b.is_ascii_digit()) => Self::Id(id),
            _ => Self::Name(text.to_string()),
        }
    }
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
    /// A message for the user, shown in the floating bar.
    Notice {
        level: NoticeLevel,
        text: String,
    },
    /// An agent needs the user or finished its turn while nobody looked: for the client to
    /// notify through the outer terminal.
    AgentAlert {
        kind: AgentKind,
        /// `Waiting` or `Idle`.
        status: AgentStatus,
        project: String,
        panel: String,
    },
    /// One piece of an image's pixels, for a client that attached with `images`. The pieces of
    /// one image come in order, `data` at most [`MAX_IMAGE_PIECE`] bytes at byte `offset` of
    /// `total`; they precede the first frame that lists the image.
    Image {
        terminal: TerminalId,
        screen: ImageScreen,
        image: u32,
        generation: u64,
        width: u32,
        height: u32,
        format: ImageFormat,
        offset: u32,
        total: u32,
        data: Vec<u8>,
    },
    /// The client can forget an image it was sent.
    ImageGone {
        terminal: TerminalId,
        screen: ImageScreen,
        image: u32,
        generation: u64,
    },
}

/// The most pixel bytes of one `ServerMsg::Image`.
pub const MAX_IMAGE_PIECE: usize = 4 << 20;

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
    /// The id of what a `New*` request created.
    Created {
        id: u64,
    },
    Tree(Vec<SessionView>),
    Captured(Captured),
    Waited(Waited),
    /// The answer to `Request::ListAgentSessions`, sent when the listing is done and so out of
    /// order with the answers to other requests. `error` says why `sessions` may be empty.
    AgentSessions {
        kind: AgentKind,
        sessions: Vec<AgentSession>,
        error: Option<String>,
    },
    /// The answer to `Request::Stats`.
    Stats(Stats),
}

/// The daemon's own figures, from `Request::Stats`. A figure the daemon could not read is
/// `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stats {
    pub pid: u32,
    pub uptime_secs: u64,
    pub version: String,
    pub protocol: u32,
    /// Resident memory in bytes.
    pub rss_bytes: Option<u64>,
    pub fds: Option<u32>,
    pub threads: Option<u32>,
    /// CPU time of the daemon process, in milliseconds.
    pub cpu_user_ms: Option<u64>,
    pub cpu_system_ms: Option<u64>,
    pub sessions: u32,
    /// Terminals whose program still runs.
    pub live: u32,
    /// Terminals whose program ended and that are kept on screen.
    pub exited: u32,
    /// TUI clients attached to a session; control connections are not counted.
    pub clients: u32,
    /// One entry per terminal, live or kept; empty when the log line asks.
    pub per_terminal: Vec<TerminalStats>,
}

/// One terminal in [`Stats`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalStats {
    pub id: TerminalId,
    pub session: String,
    /// Lines of history the display can reach.
    pub history: u32,
    /// Columns and rows.
    pub size: (u16, u16),
    /// Bytes read from its PTY since it started.
    pub bytes_read: u64,
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
    /// The terminal's image placements, always the complete list; empty for a client that did
    /// not attach with `images`.
    pub images: Vec<Placement>,
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
    /// The program never chose a shape: the outer terminal's own shape.
    Default,
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
    use crate::agents::AgentSource;
    use crate::agents::AgentState;
    use crate::model::{NameSource, PanelId, PanelView, ProjectId, ProjectView, TerminalView};
    use std::os::unix::ffi::OsStringExt;

    fn view() -> SessionView {
        SessionView {
            id: SessionId(1),
            name: "main".into(),
            active_project: Some(ProjectId(2)),
            active_panel: Some(PanelId(3)),
            layout_viewport: Some((100, 30)),
            attached_clients: 2,
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
                    ratios: crate::layout::Ratios::default_for(1),
                    unseen: true,
                    terminals: vec![TerminalView {
                        id: TerminalId(4),
                        program: "sh".into(),
                        title: Some("t".into()),
                        cwd: Some("/x".into()),
                        exit_status: Some(1),
                        pid: Some(77),
                        keep: true,
                        agent: Some(AgentState {
                            kind: AgentKind::Codex,
                            status: AgentStatus::Waiting,
                            source: AgentSource::Hooks,
                            session_id: Some("s1".into()),
                            first_prompt: Some("fix".into()),
                            since: std::time::Duration::from_millis(1500),
                        }),
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
            images: vec![placement(ImageScreen::Primary, 7)],
        }
    }

    fn placement(screen: ImageScreen, image: u32) -> Placement {
        Placement {
            screen,
            image,
            generation: 9,
            kind: PlacementKind::Placed { col: -1, row: 2 },
            cols: 4,
            rows: 3,
            source: SourceRect {
                x: 0,
                y: 1,
                width: 20,
                height: 10,
            },
            pixels: (32, 48),
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
                    cell: Some(CellPx {
                        width: 9,
                        height: 18,
                    }),
                    images: true,
                    outer_name: Some("ghostty 1.2.0".into()),
                },
            ),
            hello(
                Some(colors),
                Role::Attach {
                    session: Some("s".into()),
                    create: false,
                    cols: 1,
                    rows: 2,
                    cell: None,
                    images: false,
                    outer_name: None,
                },
            ),
            ClientMsg::Viewport {
                cols: 120,
                rows: 40,
                cell: Some(CellPx {
                    width: 7,
                    height: 15,
                }),
            },
            ClientMsg::Viewport {
                cols: 120,
                rows: 40,
                cell: None,
            },
            ClientMsg::Input {
                terminal: TerminalId(9),
                bytes: vec![0, 1, 0x1b, 255],
            },
            ClientMsg::ReloadViewport {
                cols: 71,
                rows: 23,
                cell: Some(CellPx {
                    width: 8,
                    height: 16,
                }),
            },
            ClientMsg::ReloadViewport {
                cols: 71,
                rows: 23,
                cell: None,
            },
            ClientMsg::Focus(false),
            ClientMsg::Focus(true),
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
            ClientMsg::Request(Request::Tree { session: None }),
            ClientMsg::Request(Request::Tree {
                session: Some("s".into()),
            }),
            ClientMsg::Request(Request::NewProject {
                session: SessionId(1),
                name: "api".into(),
                root: "/tmp".into(),
                focus: true,
            }),
            ClientMsg::Request(Request::NewPanel {
                session: SessionId(1),
                project: Target::Name("api".into()),
                name: Some("tests".into()),
                launch: Some(LaunchSpec {
                    program: Some("sh".into()),
                    args: vec!["-c".into(), "true".into()],
                    cwd: PathBuf::new(),
                    env: vec![("K".into(), "V".into())],
                }),
                focus: false,
            }),
            ClientMsg::Request(Request::NewPanel {
                session: SessionId(1),
                project: Target::Id(4),
                name: None,
                launch: None,
                focus: true,
            }),
            ClientMsg::Request(Request::NewTerminal {
                panel: PanelId(3),
                launch: None,
                keep: true,
                focus: false,
            }),
            ClientMsg::Request(Request::Focus { id: 5 }),
            ClientMsg::Request(Request::Close { id: 6 }),
            ClientMsg::Request(Request::RenamePanel {
                panel: PanelId(3),
                name: "build".into(),
            }),
            ClientMsg::Request(Request::AutoNamePanel { panel: PanelId(3) }),
            ClientMsg::Request(Request::Send {
                terminal: TerminalId(4),
                bytes: b"echo \xe2\x9c\x93\r".to_vec(),
                paste: true,
            }),
            ClientMsg::Request(Request::SendKeys {
                terminal: TerminalId(4),
                keys: vec![
                    KeySpec::parse("C-c").unwrap(),
                    KeySpec::parse("Up").unwrap(),
                    KeySpec::parse("M-x").unwrap(),
                ],
            }),
            ClientMsg::Request(Request::Capture {
                terminal: TerminalId(4),
                lines: Some(30),
                history: true,
                ansi: false,
            }),
            ClientMsg::Request(Request::Capture {
                terminal: TerminalId(4),
                lines: None,
                history: false,
                ansi: true,
            }),
            ClientMsg::Request(Request::Wait {
                terminal: TerminalId(4),
                until: WaitFor::Exit,
                timeout_ms: None,
            }),
            ClientMsg::Request(Request::Wait {
                terminal: TerminalId(4),
                until: WaitFor::Text("ready ✓".into()),
                timeout_ms: Some(5000),
            }),
            ClientMsg::Request(Request::Wait {
                terminal: TerminalId(4),
                until: WaitFor::Idle { quiet_ms: 500 },
                timeout_ms: Some(0),
            }),
            ClientMsg::Request(Request::AgentEvent {
                terminal: TerminalId(4),
                kind: AgentKind::OpenCode,
                status: Some(AgentStatus::Waiting),
                session_id: Some("ses_1".into()),
                prompt: Some("fix the bug".into()),
                raw: Some("{\"type\":\"x\"}".into()),
            }),
            ClientMsg::Request(Request::AgentEvent {
                terminal: TerminalId(4),
                kind: AgentKind::Claude,
                status: None,
                session_id: None,
                prompt: None,
                raw: None,
            }),
            ClientMsg::Request(Request::ListAgentSessions {
                kind: AgentKind::OpenCode,
                limit: 50,
            }),
            ClientMsg::Request(Request::Stats),
            ClientMsg::Action(Action::KillSession),
            ClientMsg::Action(Action::NewPanelWith {
                project: crate::model::ProjectId(2),
                launch: crate::action::PanelLaunch::Resume {
                    kind: AgentKind::Codex,
                    session_id: "0b2c3d4e-2222".into(),
                    cwd: PathBuf::from(OsString::from_vec(b"/w/\xe9".to_vec())),
                },
            }),
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
            ServerMsg::AgentAlert {
                kind: AgentKind::Claude,
                status: AgentStatus::Waiting,
                project: "p ✓".into(),
                panel: "e".into(),
            },
            ServerMsg::SessionView(view()),
            ServerMsg::Frame(frame()),
            ServerMsg::Image {
                terminal: TerminalId(4),
                screen: ImageScreen::Alternate,
                image: 7,
                generation: 3,
                width: 20,
                height: 10,
                format: ImageFormat::Rgba,
                offset: 4,
                total: 800,
                data: vec![0, 1, 2, 255],
            },
            ServerMsg::ImageGone {
                terminal: TerminalId(4),
                screen: ImageScreen::Primary,
                image: 7,
                generation: 3,
            },
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
            ServerMsg::Response(Response::Created { id: 12 }),
            ServerMsg::Response(Response::Tree(vec![view()])),
            ServerMsg::Response(Response::Captured(Captured {
                terminal: TerminalId(4),
                size: (80, 24),
                cursor: (3, 7),
                alternate_screen: true,
                truncated: 12,
                lines: vec!["日本 🙂 e\u{301}".into(), String::new(), "$".into()],
            })),
            ServerMsg::Response(Response::Waited(Waited::Exit(-1))),
            ServerMsg::Response(Response::Waited(Waited::Exit(143))),
            ServerMsg::Response(Response::Waited(Waited::Text("ready".into()))),
            ServerMsg::Response(Response::Waited(Waited::Idle)),
            ServerMsg::Response(Response::Waited(Waited::TimedOut)),
            ServerMsg::Response(Response::AgentSessions {
                kind: AgentKind::Claude,
                sessions: vec![AgentSession {
                    kind: AgentKind::Claude,
                    id: "0a1b2c3d-1111-4222-8333-444455556666".into(),
                    title: "Fix ✓".into(),
                    cwd: PathBuf::from(OsString::from_vec(b"/w/\xff".to_vec())),
                    updated: 1_766_000_000,
                }],
                error: None,
            }),
            ServerMsg::Response(Response::AgentSessions {
                kind: AgentKind::OpenCode,
                sessions: Vec::new(),
                error: Some("timed out".into()),
            }),
            ServerMsg::Response(Response::Stats(Stats {
                pid: 4312,
                uptime_secs: 273_600,
                version: "0.4.0".into(),
                protocol: PROTOCOL_VERSION,
                rss_bytes: Some(88_276_787),
                fds: Some(61),
                threads: None,
                cpu_user_ms: Some(312_400),
                cpu_system_ms: Some(41_000),
                sessions: 2,
                live: 13,
                exited: 1,
                clients: 1,
                per_terminal: vec![TerminalStats {
                    id: TerminalId(4),
                    session: "main".into(),
                    history: 10_000,
                    size: (120, 40),
                    bytes_read: 50_432_000,
                }],
            })),
            ServerMsg::ActionResult(Err(ActionError::InvalidSessionId)),
        ]
    }

    #[test]
    fn a_target_is_an_id_only_when_it_is_all_digits() {
        assert_eq!(Target::parse("12"), Target::Id(12));
        assert_eq!(Target::parse("api"), Target::Name("api".into()));
        assert_eq!(Target::parse("+3"), Target::Name("+3".into()));
        assert_eq!(Target::parse("12a"), Target::Name("12a".into()));
        assert_eq!(Target::parse(""), Target::Name(String::new()));
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
    fn placements_round_trip() {
        let base = Placement {
            screen: ImageScreen::Alternate,
            image: 7,
            generation: u64::MAX,
            kind: PlacementKind::Placed { col: -3, row: 5 },
            cols: 65535,
            rows: 2,
            source: SourceRect {
                x: 1,
                y: 2,
                width: 30,
                height: 40,
            },
            pixels: (270, 36),
        };
        let virt = Placement {
            screen: ImageScreen::Primary,
            kind: PlacementKind::Virtual,
            ..base
        };
        for p in [base, virt] {
            assert_eq!(decode::<Placement>(&encode_ok(&p)).unwrap(), p);
        }
        for f in [ImageFormat::Rgb, ImageFormat::Rgba] {
            assert_eq!(decode::<ImageFormat>(&encode_ok(&f)).unwrap(), f);
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
