//! The client's state and its reaction to input and server messages. Pure: it returns what
//! to do (`Out`) and never touches the terminal or the socket.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use deck_core::action::Action as Act;
use deck_core::config::Config;
use deck_core::keys::{KeyCode, KeyEvent, Modifiers, encode};
use deck_core::layout;
use deck_core::model::TerminalId;
use deck_core::mouse::{self, MouseButton, MouseEvent, MouseKind};
use deck_core::protocol::{
    ByeReason, ClientMsg, CursorShape, Modes, NoticeLevel, ScrollTo, ServerMsg,
};
use deck_core::theme::Theme;
use ratatui::crossterm::event as ct;

use crate::Exit;
use crate::grid::Grid;
use crate::input::{Action, Bindings, convert, event_label, keys_from_bytes};
use crate::mirror::Mirror;
use crate::modal::{Modal, PromptKind, Retry};
use crate::picker::{Picker, Req};
use crate::resize::Drag;
use crate::sidebar::Sel;

/// Below this many columns or rows the client shows only a message.
pub const MIN_SIZE: (u16, u16) = (20, 5);
/// How long a flash message replaces the hints.
pub const FLASH_FOR: Duration = Duration::from_secs(2);
/// How long an error from the daemon stays.
pub const ERROR_FLASH_FOR: Duration = Duration::from_secs(4);

pub fn too_small((cols, rows): (u16, u16)) -> bool {
    cols < MIN_SIZE.0 || rows < MIN_SIZE.1
}

/// How long a notice stays in the status bar, by level.
pub fn notice_span(level: NoticeLevel) -> Duration {
    Duration::from_secs(match level {
        NoticeLevel::Info => 2,
        NoticeLevel::Warning => 4,
        NoticeLevel::Error => 6,
    })
}

/// Lines the wheel scrolls per notch.
const WHEEL_LINES: i32 = 3;

/// What the event loop must do after the state changed.
#[derive(Debug, PartialEq, Eq)]
pub enum Out {
    Send(ClientMsg),
    /// Bytes for the outer terminal (an OSC 52 copy).
    Clipboard(Vec<u8>),
    Exit(Exit),
    /// Attach this client to another session.
    Switch(String),
}

#[derive(Debug)]
pub struct App {
    pub mirror: Mirror,
    pub bindings: Bindings,
    /// The colours, border style and glyphs everything is drawn with.
    pub theme: Theme,
    /// The config in force: the daemon's, from `Welcome` and `Config`.
    pub config: Config,
    /// The outer terminal does truecolor; if not, RGB colours are drawn as palette colours.
    pub truecolor: bool,
    /// The outer terminal's size in columns and rows.
    pub size: (u16, u16),
    pub armed: bool,
    pub flash: Option<(String, Instant)>,
    /// The flash is an error, drawn in the error colour.
    pub flash_error: bool,
    /// The level of the notice the flash shows; `None` for a message of the client's own.
    pub flash_level: Option<NoticeLevel>,
    /// Notices that arrived while one was showing, oldest first.
    notices: VecDeque<(NoticeLevel, String)>,
    /// The terminal scroll mode was entered for.
    pub scroll_mode: Option<TerminalId>,
    /// A scroll of ours is on its way that brings the terminal to the bottom, which must
    /// not end scroll mode.
    bottom_expected: bool,
    /// Typed ahead while the colours were queried, or before the daemon's config arrived;
    /// replayed by [`App::start`].
    typeahead: Vec<KeyEvent>,
    /// False until the daemon's config is in (see [`App::hold_until_configured`]).
    configured: bool,
    /// Keys and pastes for a terminal that is not there yet, sent when it is.
    queued: Vec<Queued>,
    pub(crate) dirty: bool,
    /// The prompt, confirmation or help overlay that has the keyboard.
    pub modal: Option<Modal>,
    /// What to reopen for each action sent whose answer is still to come, oldest first.
    /// The second field: a refusal is not shown.
    pub(crate) pending: VecDeque<(Option<Retry>, bool)>,
    /// The session picker, open over everything but the prompts it starts.
    pub picker: Option<Picker>,
    /// What each request sent whose answer is still to come was for, oldest first.
    pub(crate) requests: VecDeque<Req>,
    /// A click focused a terminal; the rest of that click (drag, release) is not forwarded.
    swallow_release: bool,
    /// Until when the resize keys act without the prefix.
    pub(crate) resize_until: Option<Instant>,
    /// The border being dragged with the mouse.
    pub(crate) drag: Option<Drag>,
    /// The user hid the sidebar (it also hides when the window is narrow).
    pub sidebar_hidden: bool,
    pub sidebar_width: u16,
    /// The sidebar has the keyboard.
    pub sidebar_focus: bool,
    pub selection: Option<Sel>,
    /// The first sidebar row shown.
    pub sidebar_offset: usize,
    /// Where the client was started, for new projects' directories.
    pub(crate) cwd: PathBuf,
    pub(crate) home: Option<PathBuf>,
}

/// A mouse cell on a terminal of the grid.
struct Hit {
    /// The terminal's index in the grid.
    i: usize,
    grid: Grid,
    /// The 0-based cell of its screen, `None` on its border.
    cell: Option<(u16, u16)>,
}

/// Something the user sent to the focused terminal, encoded when it is sent.
#[derive(Debug)]
enum Queued {
    Key(KeyEvent),
    Paste(String),
}

impl App {
    pub fn new(config: &Config, size: (u16, u16), typeahead: &[u8]) -> Self {
        Self {
            mirror: Mirror::default(),
            bindings: Bindings::from_config(config),
            theme: config.theme.clone(),
            config: config.clone(),
            truecolor: true,
            size,
            armed: false,
            flash: None,
            flash_error: false,
            flash_level: None,
            notices: VecDeque::new(),
            scroll_mode: None,
            bottom_expected: false,
            typeahead: keys_from_bytes(typeahead),
            configured: true,
            queued: Vec::new(),
            dirty: true,
            modal: None,
            pending: VecDeque::new(),
            picker: None,
            requests: VecDeque::new(),
            swallow_release: false,
            resize_until: None,
            drag: None,
            sidebar_hidden: !config.sidebar,
            sidebar_width: config.sidebar_width,
            sidebar_focus: false,
            selection: None,
            sidebar_offset: 0,
            cwd: PathBuf::from("/"),
            home: None,
        }
    }

    /// The directory the client was started in and the home directory.
    pub fn with_dirs(mut self, cwd: PathBuf, home: Option<PathBuf>) -> Self {
        self.cwd = cwd;
        self.home = home;
        self
    }

    /// Whether the sidebar is on screen: not hidden by the user, and the panel area keeps
    /// its minimum width.
    pub fn sidebar_visible(&self) -> bool {
        self.sidebar_cols() > 0
    }

    /// Columns the sidebar and its separator take.
    pub fn sidebar_cols(&self) -> u16 {
        layout::sidebar_cols(self.size.0, self.sidebar_width, !self.sidebar_hidden)
    }

    /// The panel area, in columns and rows: what the daemon sizes terminals for.
    pub fn viewport(&self) -> (u16, u16) {
        layout::panel_area(self.size, &self.config, !self.sidebar_hidden)
    }

    /// Takes in the daemon's config, changing only what differs from the one in force (a
    /// sidebar the user hid stays hidden unless `sidebar` itself changed). An armed prefix
    /// and an open prompt are kept.
    pub(crate) fn apply_config(&mut self, config: Config) -> Vec<Out> {
        if config == self.config {
            return Vec::new();
        }
        let before = self.viewport();
        let old = std::mem::replace(&mut self.config, config);
        let new = &self.config;
        self.dirty = true;
        if new.theme != old.theme {
            self.theme = new.theme.clone();
        }
        if new.prefix != old.prefix || new.keys != old.keys {
            self.bindings = Bindings::from_config(new);
        }
        let mut sized = false;
        if new.sidebar != old.sidebar {
            self.sidebar_hidden = !new.sidebar;
            sized = true;
        }
        if new.sidebar_width != old.sidebar_width {
            self.sidebar_width = new.sidebar_width;
            sized = true;
        }
        if !sized {
            return Vec::new();
        }
        if !self.sidebar_visible() {
            self.sidebar_focus = false;
        }
        self.ensure_visible();
        let (cols, rows) = self.viewport();
        if too_small(self.size) || (cols, rows) == before {
            return Vec::new();
        }
        // Not a claim on the terminals' size: the client the user last typed in keeps it.
        vec![Out::Send(ClientMsg::ReloadViewport { cols, rows })]
    }

    /// A message from the daemon for the status bar: at once when nothing of the daemon's
    /// is showing, else after the ones before it.
    fn push_notice(&mut self, level: NoticeLevel, text: String, now: Instant) {
        if self.flash.is_some() && self.flash_level.is_some() {
            self.notices.push_back((level, text));
        } else {
            self.show_notice(level, text, now);
        }
    }

    fn show_notice(&mut self, level: NoticeLevel, text: String, now: Instant) {
        self.flash = Some((text, now + notice_span(level)));
        self.flash_error = level == NoticeLevel::Error;
        self.flash_level = Some(level);
        self.dirty = true;
    }

    fn viewport_msg(&self) -> Out {
        let (cols, rows) = self.viewport();
        Out::Send(ClientMsg::Viewport { cols, rows })
    }

    /// A prompt, a confirmation or an overlay has the keyboard.
    pub fn overlay_open(&self) -> bool {
        self.modal.is_some() || self.picker.is_some()
    }

    /// Forgets everything that belongs to the session the client was attached to, for the
    /// next one. What is not specific to a session (the sidebar's visibility and width, the
    /// flash) stays.
    pub(crate) fn reset_session(&mut self) {
        self.mirror = Mirror::default();
        self.modal = None;
        self.picker = None;
        self.pending.clear();
        self.requests.clear();
        self.armed = false;
        self.scroll_mode = None;
        self.bottom_expected = false;
        self.swallow_release = false;
        self.resize_until = None;
        self.drag = None;
        self.queued.clear();
        self.sidebar_focus = false;
        self.selection = None;
        self.sidebar_offset = 0;
        self.dirty = true;
    }

    /// Shows a message in the status bar for a while.
    pub(crate) fn set_flash(&mut self, text: String, now: Instant, error: bool) {
        let span = if error { ERROR_FLASH_FOR } else { FLASH_FOR };
        self.flash = Some((text, now + span));
        self.flash_error = error;
        self.flash_level = None;
        self.dirty = true;
    }

    /// There is a session but no panel to show.
    pub fn panel_area_empty(&self) -> bool {
        self.mirror.view().is_some() && self.mirror.focused().is_none()
    }

    /// Runs what was typed ahead through the key path, as if it came just now.
    pub fn start(&mut self, now: Instant) -> Vec<Out> {
        let keys = std::mem::take(&mut self.typeahead);
        keys.into_iter().flat_map(|k| self.on_key(k, now)).collect()
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty && self.configured
    }

    /// Keeps keys and drawing back until the first `Welcome` brings the daemon's config:
    /// before it the bindings and the theme are not the user's.
    pub fn hold_until_configured(&mut self) {
        self.configured = false;
    }

    /// When the flash message ends.
    pub fn flash_deadline(&self) -> Option<Instant> {
        self.flash.as_ref().map(|(_, until)| *until)
    }

    /// The next moment `tick` has something to do: a flash ends or the resize repeat does.
    pub fn wake_deadline(&self) -> Option<Instant> {
        [self.flash_deadline(), self.resize_until]
            .into_iter()
            .flatten()
            .min()
    }

    pub fn take_dirty(&mut self) -> bool {
        self.configured && std::mem::take(&mut self.dirty)
    }

    /// Drops an expired flash message.
    pub fn tick(&mut self, now: Instant) {
        if self.resize_until.is_some_and(|until| now >= until) {
            self.resize_until = None;
            self.dirty = true;
        }
        if self.flash.as_ref().is_some_and(|(_, until)| now >= *until) {
            self.flash = None;
            self.flash_error = false;
            self.flash_level = None;
            self.dirty = true;
            if let Some((level, text)) = self.notices.pop_front() {
                self.show_notice(level, text, now);
            }
        }
    }

    /// The cursor shape the outer terminal should show, when a terminal's cursor is visible.
    pub fn cursor_shape(&self) -> Option<CursorShape> {
        let t = self.mirror.terminal(self.mirror.focused_id()?)?;
        t.cursor.visible.then_some(t.cursor.shape)
    }

    pub fn on_event(&mut self, event: ct::Event, now: Instant) -> Vec<Out> {
        match event {
            ct::Event::Key(key) if !self.configured => {
                self.typeahead.extend(convert(key));
                Vec::new()
            }
            ct::Event::Key(key) => convert(key).map_or_else(Vec::new, |k| self.on_key(k, now)),
            ct::Event::Paste(text) => self.on_paste(text),
            ct::Event::Resize(cols, rows) => {
                self.size = (cols, rows);
                self.dirty = true;
                if !self.sidebar_visible() {
                    self.sidebar_focus = false;
                }
                self.ensure_visible();
                if too_small(self.size) {
                    return Vec::new();
                }
                vec![self.viewport_msg()]
            }
            ct::Event::Mouse(mouse) => self.on_mouse(mouse, now),
            ct::Event::FocusGained => self.on_focus(true),
            ct::Event::FocusLost => self.on_focus(false),
        }
    }

    /// The terminal under a cell of the outer window, its index in the grid, and the
    /// 0-based cell of its screen when the cell is on it (not on a border).
    fn terminal_at(&self, column: u16, row: u16) -> Option<Hit> {
        let (panel_cols, panel_rows) = (self.size.0, self.size.1.saturating_sub(1));
        let x = column.checked_sub(self.sidebar_cols())?;
        if too_small(self.size) || column >= panel_cols || row >= panel_rows {
            return None;
        }
        let grid = Grid::of(self)?;
        let i = grid.hit(x, row)?;
        let inner = grid.inner(i);
        let on_screen = x >= inner.x
            && row >= inner.y
            && x - inner.x < inner.width
            && row - inner.y < inner.height;
        let cell = on_screen.then(|| (x - inner.x, row - inner.y));
        Some(Hit { i, grid, cell })
    }

    fn on_mouse(&mut self, event: ct::MouseEvent, now: Instant) -> Vec<Out> {
        // A release ends the swallowed click wherever it lands.
        let swallowed = self.swallow_release;
        if matches!(event.kind, ct::MouseEventKind::Up(_)) {
            self.swallow_release = false;
        }
        if self.overlay_open() || too_small(self.size) {
            self.drag = None;
            return Vec::new();
        }
        // A border drag is ours from its press to its release, never a program's.
        if let Some(outs) = self.drag_mouse(event, now) {
            return outs;
        }
        if event.column < self.sidebar_cols() {
            self.dirty = true;
            return self.sidebar_mouse(event);
        }
        let Some(Hit { i, grid, cell }) = self.terminal_at(event.column, event.row) else {
            return Vec::new();
        };
        let Some(&terminal) = grid.ids.get(i) else {
            return Vec::new();
        };
        let wheel = matches!(
            event.kind,
            ct::MouseEventKind::ScrollUp | ct::MouseEventKind::ScrollDown
        );
        // The rest of a click that focused a terminal never reaches a program.
        if swallowed && !wheel {
            return Vec::new();
        }
        if grid.focused != Some(i) && !wheel {
            // A click in an unfocused terminal focuses it and is not forwarded.
            if matches!(event.kind, ct::MouseEventKind::Down(_)) {
                self.swallow_release = true;
                return vec![self.act(Act::FocusTerminal { terminal }, None)];
            }
            return Vec::new();
        }
        let Some(term) = self.mirror.terminal(terminal) else {
            return Vec::new();
        };
        let Some((col, row)) = cell.filter(|(c, r)| *c < term.size.0 && *r < term.size.1) else {
            return Vec::new();
        };
        let modes = term.modes;
        let history = term.history;
        let button = |b: ct::MouseButton| match b {
            ct::MouseButton::Left => MouseButton::Left,
            ct::MouseButton::Right => MouseButton::Right,
            ct::MouseButton::Middle => MouseButton::Middle,
        };
        let kind = match event.kind {
            ct::MouseEventKind::ScrollUp | ct::MouseEventKind::ScrollDown => {
                let up = event.kind == ct::MouseEventKind::ScrollUp;
                return self.on_wheel(terminal, up, (col, row), event.modifiers, history, modes);
            }
            // Clicks, releases and motion belong to the program, never to scroll mode.
            _ if self.scroll_mode.is_some() => return Vec::new(),
            ct::MouseEventKind::Down(b) => MouseKind::Press(button(b)),
            ct::MouseEventKind::Up(b) => MouseKind::Release(button(b)),
            ct::MouseEventKind::Drag(b) => MouseKind::Drag(button(b)),
            ct::MouseEventKind::Moved => MouseKind::Move,
            ct::MouseEventKind::ScrollLeft | ct::MouseEventKind::ScrollRight => {
                return Vec::new();
            }
        };
        let event = MouseEvent {
            kind,
            col,
            row,
            mods: key_mods(event.modifiers),
        };
        mouse::encode(&event, modes)
            .map(|bytes| Out::Send(ClientMsg::Input { terminal, bytes }))
            .into_iter()
            .collect()
    }

    /// The wheel: the program's if it asked for the mouse, else the cursor keys on the
    /// alternate screen, else the history.
    fn on_wheel(
        &mut self,
        terminal: TerminalId,
        up: bool,
        (col, row): (u16, u16),
        mods: ct::KeyModifiers,
        history: u32,
        modes: Modes,
    ) -> Vec<Out> {
        let input = |bytes: Vec<u8>| vec![Out::Send(ClientMsg::Input { terminal, bytes })];
        if self.scroll_mode != Some(terminal) {
            if mouse::reporting(modes) {
                let kind = if up {
                    MouseKind::WheelUp
                } else {
                    MouseKind::WheelDown
                };
                let event = MouseEvent {
                    kind,
                    col,
                    row,
                    mods: key_mods(mods),
                };
                return mouse::encode(&event, modes).map_or_else(Vec::new, input);
            }
            if modes.has(Modes::ALTERNATE_SCREEN) {
                if !modes.has(Modes::ALTERNATE_SCROLL) {
                    return Vec::new();
                }
                let code = if up { KeyCode::Up } else { KeyCode::Down };
                let press = encode(&KeyEvent::new(code, Modifiers::NONE), modes);
                return input(press.repeat(3));
            }
        }
        if history == 0 {
            return Vec::new();
        }
        let lines = if up { WHEEL_LINES } else { -WHEEL_LINES };
        vec![self.scroll(terminal, ScrollTo::Lines(lines))]
    }

    /// Focus-in and focus-out of the outer terminal, for programs that asked for them.
    fn on_focus(&mut self, gained: bool) -> Vec<Out> {
        self.end_repeat();
        if self.overlay_open() {
            return Vec::new();
        }
        let Some(terminal) = self.ready() else {
            return Vec::new();
        };
        let reports = self
            .mirror
            .terminal(terminal)
            .is_some_and(|t| t.modes.has(Modes::FOCUS_REPORTING));
        if !reports {
            return Vec::new();
        }
        let bytes: &[u8] = if gained { b"\x1b[I" } else { b"\x1b[O" };
        vec![Out::Send(ClientMsg::Input {
            terminal,
            bytes: bytes.to_vec(),
        })]
    }

    /// A scroll request; remembers when it is meant to land on the live screen.
    fn scroll(&mut self, terminal: TerminalId, to: ScrollTo) -> Out {
        let (offset, rows) = self.mirror.terminal(terminal).map_or((0, 1), |t| {
            (i64::from(t.display_offset), i64::from(t.size.1))
        });
        let reaches_bottom = match to {
            ScrollTo::Bottom => true,
            ScrollTo::Lines(n) => n < 0 && offset <= -i64::from(n),
            ScrollTo::Pages(n) => n < 0 && offset <= -i64::from(n) * rows,
            ScrollTo::Top => false,
        };
        if self.scroll_mode.is_some() && reaches_bottom {
            self.bottom_expected = true;
        }
        Out::Send(ClientMsg::Scroll { terminal, to })
    }

    /// Leaves scroll mode and returns the terminal to the bottom.
    pub(crate) fn leave_scroll(&mut self) -> Vec<Out> {
        self.bottom_expected = false;
        match self.scroll_mode.take() {
            Some(terminal) => vec![Out::Send(ClientMsg::Scroll {
                terminal,
                to: ScrollTo::Bottom,
            })],
            None => Vec::new(),
        }
    }

    /// A key typed in scroll mode: it scrolls, or is dropped; it never reaches the terminal.
    fn scroll_key(&mut self, terminal: TerminalId, key: KeyEvent) -> Vec<Out> {
        let (ctrl, alt) = (key.mods.ctrl, key.mods.alt);
        if alt {
            return Vec::new();
        }
        let rows = self
            .mirror
            .terminal(terminal)
            .map_or(1, |t| i32::from(t.size.1));
        let half = (rows / 2).max(1);
        let to = match key.code {
            KeyCode::Up => ScrollTo::Lines(1),
            KeyCode::Char('k') if !ctrl => ScrollTo::Lines(1),
            KeyCode::Down => ScrollTo::Lines(-1),
            KeyCode::Char('j') if !ctrl => ScrollTo::Lines(-1),
            KeyCode::PageUp => ScrollTo::Pages(1),
            KeyCode::Char('b') if ctrl => ScrollTo::Pages(1),
            KeyCode::PageDown => ScrollTo::Pages(-1),
            KeyCode::Char('f') if ctrl => ScrollTo::Pages(-1),
            KeyCode::Char('u') if ctrl => ScrollTo::Lines(half),
            KeyCode::Char('d') if ctrl => ScrollTo::Lines(-half),
            KeyCode::Home => ScrollTo::Top,
            KeyCode::Char('g') if !ctrl => ScrollTo::Top,
            KeyCode::End => ScrollTo::Bottom,
            KeyCode::Char('G') if !ctrl => ScrollTo::Bottom,
            KeyCode::Esc => return self.leave_scroll(),
            KeyCode::Char('q') if !ctrl => return self.leave_scroll(),
            _ => return Vec::new(),
        };
        vec![self.scroll(terminal, to)]
    }

    /// Ends scroll mode when its terminal is no longer the focused one.
    fn check_scroll_target(&mut self) {
        if self
            .scroll_mode
            .is_some_and(|t| self.mirror.focused_id() != Some(t))
        {
            self.scroll_mode = None;
            self.bottom_expected = false;
        }
    }

    fn on_key(&mut self, key: KeyEvent, now: Instant) -> Vec<Out> {
        self.dirty = true;
        if self.modal.is_some() {
            return self.modal_key(key);
        }
        if self.picker.is_some() {
            return self.picker_key(key);
        }
        if !self.armed {
            if let Some(until) = self.resize_until.take() {
                // The resize keys act without the prefix for a while; any other key ends it.
                let action = (now < until)
                    .then(|| self.bindings.lookup(&key))
                    .flatten()
                    .filter(|a| matches!(a, Action::Resize(_) | Action::ResetLayout));
                if let Some(action) = action {
                    return self.run_action(action, now);
                }
            }
            if self.bindings.is_prefix(&key) {
                self.armed = true;
                return Vec::new();
            }
            if let Some(terminal) = self.scroll_mode {
                return self.scroll_key(terminal, key);
            }
            if self.sidebar_focus {
                return self.sidebar_key(key);
            }
            return self.send_or_queue(Queued::Key(key));
        }
        self.armed = false;
        if self.bindings.is_prefix(&key) {
            // The key goes to the terminal, which returns to the bottom.
            self.scroll_mode = None;
            self.bottom_expected = false;
            return self.send_or_queue(Queued::Key(key));
        }
        if key.code == KeyCode::Esc && !key.mods.ctrl && !key.mods.alt {
            return Vec::new();
        }
        match self.bindings.lookup(&key) {
            Some(action) => {
                let outs = self.run_action(action, now);
                self.ensure_visible();
                if self.overlay_open() {
                    self.drag = None;
                }
                outs
            }
            None => {
                let text = format!("no binding for {}", event_label(&key));
                self.set_flash(text, now, false);
                Vec::new()
            }
        }
    }

    /// What a binding does after the prefix.
    fn run_action(&mut self, action: Action, now: Instant) -> Vec<Out> {
        let active_project = self.mirror.active_project().map(|p| p.id);
        let active_panel = self.mirror.active_panel().map(|p| p.id);
        match action {
            Action::Detach => {
                let mut outs = self.leave_scroll();
                outs.push(Out::Send(ClientMsg::Detach));
                outs.push(Out::Exit(Exit::Detached(self.session_name())));
                outs
            }
            Action::Sessions => self.open_picker(),
            Action::RenameSession => {
                let name = self.session_name();
                let kind = PromptKind::RenameSession {
                    from: name.clone(),
                    current: true,
                };
                self.open_prompt(kind, &name, None);
                Vec::new()
            }
            Action::Scroll => {
                if self.scroll_mode.is_none() {
                    self.scroll_mode = self.ready();
                }
                Vec::new()
            }
            Action::NewTerminal => {
                if active_panel.is_some() {
                    vec![self.act(Act::NewTerminal { panel: None }, None)]
                } else {
                    self.set_flash("no panel to add a terminal to".into(), now, false);
                    Vec::new()
                }
            }
            Action::CloseTerminal => self.confirm_close_terminal(now),
            Action::FocusTerminal(dir) => {
                if active_panel.is_some() {
                    vec![self.act(Act::FocusDirection { dir }, None)]
                } else {
                    Vec::new()
                }
            }
            Action::Resize(dir) => self.resize_key(dir, now),
            Action::ResetLayout => self.reset_layout(),
            Action::NewPanel => match active_project {
                Some(project) => vec![self.act(
                    Act::NewPanel {
                        project,
                        name: None,
                    },
                    None,
                )],
                None => {
                    self.set_flash("no project to add a panel to".into(), now, false);
                    Vec::new()
                }
            },
            Action::NextPanel => vec![self.act(Act::CyclePanel { delta: 1 }, None)],
            Action::PrevPanel => vec![self.act(Act::CyclePanel { delta: -1 }, None)],
            Action::GotoPanel(n) => {
                let panel = self
                    .mirror
                    .active_project()
                    .and_then(|p| p.panels.get(n - 1))
                    .map(|p| p.id);
                panel.map_or_else(Vec::new, |panel| {
                    vec![self.act(Act::FocusPanel { panel }, None)]
                })
            }
            Action::RenamePanel => {
                if let Some(panel) = active_panel {
                    self.open_rename_panel(panel);
                }
                Vec::new()
            }
            Action::ClosePanel => {
                if let Some(panel) = active_panel {
                    self.confirm_close_panel(panel);
                }
                Vec::new()
            }
            Action::NewProject => {
                self.open_new_project();
                Vec::new()
            }
            Action::RenameProject => {
                if let Some(project) = active_project {
                    self.open_rename_project(project);
                }
                Vec::new()
            }
            Action::CloseProject => {
                if let Some(project) = active_project {
                    self.confirm_close_project(project);
                }
                Vec::new()
            }
            Action::ToggleSidebar => {
                self.sidebar_hidden = !self.sidebar_hidden;
                if self.sidebar_hidden {
                    self.sidebar_focus = false;
                }
                if !self.sidebar_hidden && !self.sidebar_visible() {
                    self.set_flash("sidebar hidden (window too narrow)".into(), now, false);
                }
                if too_small(self.size) {
                    return Vec::new();
                }
                vec![self.viewport_msg()]
            }
            Action::FocusSidebar => {
                if self.sidebar_visible() {
                    self.enter_sidebar()
                } else {
                    Vec::new()
                }
            }
            Action::Help => {
                self.modal = Some(Modal::Help);
                Vec::new()
            }
        }
    }

    fn on_paste(&mut self, text: String) -> Vec<Out> {
        self.dirty = true;
        self.end_repeat();
        if self.modal.is_some() {
            self.modal_paste(&text);
            return Vec::new();
        }
        if let Some(picker) = &mut self.picker {
            picker.paste(&text);
            return Vec::new();
        }
        // Scroll mode and the sidebar keep keys and pastes from the terminal.
        if self.scroll_mode.is_some() || self.sidebar_focus {
            return Vec::new();
        }
        self.send_or_queue(Queued::Paste(text))
    }

    /// Ends the resize repeat.
    fn end_repeat(&mut self) {
        if self.resize_until.take().is_some() {
            self.dirty = true;
        }
    }

    /// The focused terminal, once its screen has arrived.
    fn ready(&self) -> Option<TerminalId> {
        let id = self.mirror.focused_id()?;
        self.mirror.terminal(id).map(|_| id)
    }

    /// Sends to the focused terminal, or keeps it until there is one.
    fn send_or_queue(&mut self, item: Queued) -> Vec<Out> {
        // With no panel to type into, typing goes nowhere.
        if self.panel_area_empty() {
            return Vec::new();
        }
        if self.ready().is_none() {
            self.queued.push(item);
            return Vec::new();
        }
        self.send(&item).into_iter().collect()
    }

    fn send(&self, item: &Queued) -> Option<Out> {
        let terminal = self.ready()?;
        let modes = self.mirror.terminal(terminal)?.modes;
        let bytes = match item {
            Queued::Key(key) => encode(key, modes),
            Queued::Paste(text) => {
                let mut bytes = Vec::with_capacity(text.len() + 12);
                let bracketed = modes.has(Modes::BRACKETED_PASTE);
                if bracketed {
                    bytes.extend_from_slice(b"\x1b[200~");
                }
                bytes.extend_from_slice(text.as_bytes());
                if bracketed {
                    bytes.extend_from_slice(b"\x1b[201~");
                }
                bytes
            }
        };
        (!bytes.is_empty()).then_some(Out::Send(ClientMsg::Input { terminal, bytes }))
    }

    /// Sends what waited for a terminal.
    fn flush(&mut self) -> Vec<Out> {
        if self.ready().is_none() {
            return Vec::new();
        }
        let queued = std::mem::take(&mut self.queued);
        queued.iter().filter_map(|q| self.send(q)).collect()
    }

    fn session_name(&self) -> String {
        self.mirror.session_name().unwrap_or("?").to_string()
    }

    /// A frame that returns the scroll-mode terminal to the bottom, which we did not ask
    /// for, means another client typed into it: scroll mode ends.
    fn follow_scroll(&mut self, frame: &deck_core::protocol::Frame) {
        if self.scroll_mode != Some(frame.terminal) {
            return;
        }
        if frame.display_offset > 0 {
            // Still scrolled back: whatever was in flight did not bring it to the bottom.
            self.bottom_expected = false;
            return;
        }
        let was_back = self
            .mirror
            .terminal(frame.terminal)
            .is_some_and(|t| t.display_offset > 0);
        if self.bottom_expected {
            self.bottom_expected = false;
        } else if was_back {
            self.scroll_mode = None;
        }
    }

    pub fn on_server(&mut self, msg: ServerMsg) -> Vec<Out> {
        self.dirty = true;
        match msg {
            ServerMsg::Welcome { config, .. } => {
                let mut outs = self.apply_config(*config);
                if !self.configured {
                    self.configured = true;
                    outs.extend(self.start(Instant::now()));
                }
                outs
            }
            ServerMsg::Config(config) => self.apply_config(*config),
            ServerMsg::Notice { level, text } => {
                self.push_notice(level, text, Instant::now());
                Vec::new()
            }
            ServerMsg::Response(response) => self.response(response, Instant::now()),
            ServerMsg::ActionResult(result) => {
                self.action_result(result, Instant::now());
                self.refresh_picker()
            }
            ServerMsg::SessionView(view) => {
                let before = self
                    .mirror
                    .view()
                    .map(|v| (v.active_project, v.active_panel));
                self.mirror.set_view(view);
                self.check_scroll_target();
                self.check_modal_target(Instant::now());
                self.normalize_selection();
                // A wheel-scrolled list stays where it is unless the active panel moved.
                let after = self
                    .mirror
                    .view()
                    .map(|v| (v.active_project, v.active_panel));
                if before == after {
                    self.clamp_offset();
                } else {
                    self.ensure_visible();
                }
                if self.panel_area_empty() {
                    self.queued.clear();
                }
                self.flush()
            }
            ServerMsg::Frame(frame) => {
                self.follow_scroll(&frame);
                self.mirror.apply_frame(frame);
                self.flush()
            }
            ServerMsg::Clipboard { selection, text } => {
                let b64 = deck_core::base64::encode(text.as_bytes());
                vec![Out::Clipboard(
                    format!("\x1b]52;{selection};{b64}\x07").into_bytes(),
                )]
            }
            ServerMsg::Bye { reason } => vec![Out::Exit(match reason {
                ByeReason::SessionEnded => Exit::SessionEnded(self.session_name()),
                ByeReason::ServerShutdown => Exit::DaemonStopped,
                ByeReason::Error(text) => Exit::Error(text),
                ByeReason::VersionMismatch { daemon } => Exit::Mismatch(daemon),
            })],
        }
    }
}

fn key_mods(mods: ct::KeyModifiers) -> Modifiers {
    Modifiers {
        ctrl: mods.contains(ct::KeyModifiers::CONTROL),
        alt: mods.contains(ct::KeyModifiers::ALT),
        shift: mods.contains(ct::KeyModifiers::SHIFT),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mirror::tests::{frame, no_sidebar, view};
    use deck_core::model::TerminalId;

    fn app() -> App {
        let mut app = App::new(&no_sidebar(), (80, 24), &[]);
        app.on_server(ServerMsg::SessionView(view(None)));
        app.on_server(ServerMsg::Frame(frame(true, (78, 21), vec![])));
        app
    }

    fn key(code: ct::KeyCode, mods: ct::KeyModifiers) -> ct::Event {
        ct::Event::Key(ct::KeyEvent::new(code, mods))
    }

    fn ch(c: char) -> ct::Event {
        key(ct::KeyCode::Char(c), ct::KeyModifiers::NONE)
    }

    fn prefix() -> ct::Event {
        key(ct::KeyCode::Char(' '), ct::KeyModifiers::ALT)
    }

    fn input(bytes: &[u8]) -> Out {
        Out::Send(ClientMsg::Input {
            terminal: TerminalId(4),
            bytes: bytes.to_vec(),
        })
    }

    #[test]
    fn keys_are_encoded_and_sent_to_the_focused_terminal() {
        let mut a = app();
        let now = Instant::now();
        assert_eq!(a.on_event(ch('x'), now), [input(b"x")]);
        assert_eq!(
            a.on_event(key(ct::KeyCode::Enter, ct::KeyModifiers::NONE), now),
            [input(b"\r")]
        );
    }

    #[test]
    fn the_prefix_arms_and_d_detaches() {
        let mut a = app();
        let now = Instant::now();
        assert!(a.on_event(prefix(), now).is_empty());
        assert!(a.armed);
        let out = a.on_event(ch('d'), now);
        assert!(!a.armed);
        assert_eq!(
            out,
            [
                Out::Send(ClientMsg::Detach),
                Out::Exit(Exit::Detached("main".into()))
            ]
        );
    }

    #[test]
    fn the_prefix_twice_sends_the_key_as_pressed() {
        let mut a = app();
        let now = Instant::now();
        a.on_event(prefix(), now);
        assert_eq!(a.on_event(prefix(), now), [input(b"\x1b ")]);
        assert!(!a.armed);
        assert_eq!(a.on_event(ch('a'), now), [input(b"a")]);
    }

    #[test]
    fn an_unbound_key_flashes_for_two_seconds_and_sends_nothing() {
        let mut a = app();
        let now = Instant::now();
        a.on_event(prefix(), now);
        assert!(a.on_event(ch('z'), now).is_empty());
        assert_eq!(a.flash.as_ref().unwrap().0, "no binding for z");
        a.tick(now + Duration::from_millis(1999));
        assert!(a.flash.is_some());
        a.take_dirty();
        a.tick(now + FLASH_FOR);
        assert!(a.flash.is_none());
        assert!(a.take_dirty());
    }

    #[test]
    fn escape_disarms_without_a_message_and_scroll_enters_its_mode_silently() {
        let mut a = app();
        let now = Instant::now();
        a.on_event(prefix(), now);
        assert!(
            a.on_event(key(ct::KeyCode::Esc, ct::KeyModifiers::NONE), now)
                .is_empty()
        );
        assert!(!a.armed && a.flash.is_none());
        a.on_event(prefix(), now);
        assert!(a.on_event(ch('['), now).is_empty());
        assert!(!a.armed && a.flash.is_none());
    }

    #[test]
    fn a_configured_detach_key_works_shifted() {
        let config = Config::parse("[keys]\ndetach = \"Z\"").unwrap().config;
        let mut a = App::new(&config, (80, 24), &[]);
        a.on_server(ServerMsg::SessionView(view(None)));
        let now = Instant::now();
        a.on_event(prefix(), now);
        let out = a.on_event(key(ct::KeyCode::Char('Z'), ct::KeyModifiers::SHIFT), now);
        assert_eq!(out[0], Out::Send(ClientMsg::Detach));
    }

    #[test]
    fn keys_use_the_terminals_modes() {
        let mut a = app();
        let mut f = frame(false, (78, 21), vec![]);
        f.modes = Modes(Modes::APPLICATION_CURSOR);
        a.on_server(ServerMsg::Frame(f));
        let out = a.on_event(key(ct::KeyCode::Up, ct::KeyModifiers::NONE), Instant::now());
        assert_eq!(out, [input(b"\x1bOA")]);
    }

    #[test]
    fn paste_is_wrapped_only_when_the_terminal_asks() {
        let mut a = app();
        let now = Instant::now();
        // The test frame has bracketed paste on.
        assert_eq!(
            a.on_event(ct::Event::Paste("a\nb".into()), now),
            [input(b"\x1b[200~a\nb\x1b[201~")]
        );
        let mut f = frame(false, (78, 21), vec![]);
        f.modes = Modes::default();
        a.on_server(ServerMsg::Frame(f));
        assert_eq!(a.on_event(ct::Event::Paste("a".into()), now), [input(b"a")]);
    }

    #[test]
    fn resize_sends_the_panel_area_unless_too_small() {
        let mut a = app();
        let now = Instant::now();
        assert_eq!(
            a.on_event(ct::Event::Resize(100, 30), now),
            [Out::Send(ClientMsg::Viewport {
                cols: 100,
                rows: 29
            })]
        );
        assert!(a.on_event(ct::Event::Resize(19, 30), now).is_empty());
        assert!(a.on_event(ct::Event::Resize(80, 4), now).is_empty());
        assert!(too_small(a.size));
        assert!(!too_small((20, 5)));
        assert_eq!(
            a.on_event(ct::Event::Resize(20, 5), now),
            [Out::Send(ClientMsg::Viewport { cols: 20, rows: 4 })]
        );
    }

    #[test]
    fn server_messages_map_to_exits_and_the_clipboard() {
        let mut a = app();
        fn bye(reason: ByeReason) -> ServerMsg {
            ServerMsg::Bye { reason }
        }
        assert_eq!(
            a.on_server(bye(ByeReason::SessionEnded)),
            [Out::Exit(Exit::SessionEnded("main".into()))]
        );
        assert_eq!(
            a.on_server(bye(ByeReason::ServerShutdown)),
            [Out::Exit(Exit::DaemonStopped)]
        );
        assert_eq!(
            a.on_server(bye(ByeReason::Error("boom".into()))),
            [Out::Exit(Exit::Error("boom".into()))]
        );
        assert_eq!(
            a.on_server(bye(ByeReason::VersionMismatch { daemon: 9 })),
            [Out::Exit(Exit::Mismatch(9))]
        );
        assert_eq!(
            a.on_server(ServerMsg::Clipboard {
                selection: "c".into(),
                text: "hi".into()
            }),
            [Out::Clipboard(b"\x1b]52;c;aGk=\x07".to_vec())]
        );
    }

    #[test]
    fn keys_wait_for_a_terminal_and_are_encoded_when_it_is_there() {
        let mut a = App::new(&no_sidebar(), (80, 24), &[]);
        let now = Instant::now();
        assert!(a.on_event(ch('l'), now).is_empty());
        assert!(a.on_event(ct::Event::Paste("p".into()), now).is_empty());
        assert!(a.on_server(ServerMsg::SessionView(view(None))).is_empty());
        // The terminal's screen is what makes it ready; its modes decide the encoding.
        let out = a.on_server(ServerMsg::Frame(frame(true, (78, 21), vec![])));
        assert_eq!(out, [input(b"l"), input(b"\x1b[200~p\x1b[201~")]);
        assert!(a.on_server(ServerMsg::SessionView(view(None))).is_empty());
    }

    #[test]
    fn typeahead_goes_through_the_prefix_machine() {
        let mut a = App::new(&no_sidebar(), (80, 24), b"a\x1b d");
        a.on_server(ServerMsg::SessionView(view(None)));
        a.on_server(ServerMsg::Frame(frame(true, (78, 21), vec![])));
        let out = a.start(Instant::now());
        assert_eq!(
            out,
            [
                input(b"a"),
                Out::Send(ClientMsg::Detach),
                Out::Exit(Exit::Detached("main".into()))
            ]
        );
    }

    fn pressed(code: ct::KeyCode) -> ct::Event {
        key(code, ct::KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> ct::Event {
        key(ct::KeyCode::Char(c), ct::KeyModifiers::CONTROL)
    }

    fn scroll_msg(to: ScrollTo) -> Out {
        Out::Send(ClientMsg::Scroll {
            terminal: TerminalId(4),
            to,
        })
    }

    /// Shows the terminal with these modes, offset and history (a 78x21 screen).
    fn show(a: &mut App, modes: u16, offset: u32, history: u32) {
        let mut f = frame(true, (78, 21), vec![]);
        f.modes = Modes(modes);
        f.display_offset = offset;
        f.history = history;
        a.on_server(ServerMsg::Frame(f));
    }

    fn in_scroll_mode() -> App {
        let mut a = app();
        show(&mut a, 0, 0, 100);
        let now = Instant::now();
        a.on_event(prefix(), now);
        assert!(a.on_event(ch('['), now).is_empty());
        assert_eq!(a.scroll_mode, Some(TerminalId(4)));
        assert!(!a.armed);
        a
    }

    /// A mouse event at the 1-based cell `column`, `row` of the panel area, as a program
    /// sees it; the panel area starts at the window's top left in these tests.
    fn mouse(kind: ct::MouseEventKind, column: u16, row: u16) -> ct::Event {
        mouse0(kind, column - 1, row - 1)
    }

    /// A mouse event at a 0-based cell of the window.
    fn mouse0(kind: ct::MouseEventKind, column: u16, row: u16) -> ct::Event {
        ct::Event::Mouse(ct::MouseEvent {
            kind,
            column,
            row,
            modifiers: ct::KeyModifiers::NONE,
        })
    }

    const SGR_CLICK: u16 = Modes::MOUSE_CLICK | Modes::SGR_MOUSE;

    #[test]
    fn scroll_mode_keys_scroll_and_none_reach_the_terminal() {
        let mut a = in_scroll_mode();
        let now = Instant::now();
        let cases = [
            (pressed(ct::KeyCode::Up), ScrollTo::Lines(1)),
            (ch('k'), ScrollTo::Lines(1)),
            (pressed(ct::KeyCode::Down), ScrollTo::Lines(-1)),
            (ch('j'), ScrollTo::Lines(-1)),
            (pressed(ct::KeyCode::PageUp), ScrollTo::Pages(1)),
            (ctrl('b'), ScrollTo::Pages(1)),
            (pressed(ct::KeyCode::PageDown), ScrollTo::Pages(-1)),
            (ctrl('f'), ScrollTo::Pages(-1)),
            // The screen has 21 rows.
            (ctrl('u'), ScrollTo::Lines(10)),
            (ctrl('d'), ScrollTo::Lines(-10)),
            (ch('g'), ScrollTo::Top),
            (pressed(ct::KeyCode::Home), ScrollTo::Top),
            (
                key(ct::KeyCode::Char('G'), ct::KeyModifiers::SHIFT),
                ScrollTo::Bottom,
            ),
            (pressed(ct::KeyCode::End), ScrollTo::Bottom),
        ];
        for (event, to) in cases {
            assert_eq!(
                a.on_event(event.clone(), now),
                [scroll_msg(to)],
                "{event:?}"
            );
        }
        // Other keys and pastes are swallowed.
        for event in [ch('x'), ch('d'), pressed(ct::KeyCode::Enter), ctrl('c')] {
            assert!(a.on_event(event, now).is_empty());
        }
        assert!(a.on_event(ct::Event::Paste("p".into()), now).is_empty());
        assert_eq!(a.scroll_mode, Some(TerminalId(4)));
    }

    #[test]
    fn q_and_escape_leave_scroll_mode_and_return_to_the_bottom() {
        for leave in [ch('q'), pressed(ct::KeyCode::Esc)] {
            let mut a = in_scroll_mode();
            assert_eq!(
                a.on_event(leave, Instant::now()),
                [scroll_msg(ScrollTo::Bottom)]
            );
            assert_eq!(a.scroll_mode, None);
            // Keys reach the terminal again.
            assert_eq!(a.on_event(ch('x'), Instant::now()), [input(b"x")]);
        }
    }

    #[test]
    fn reaching_the_bottom_keeps_scroll_mode_but_another_clients_input_ends_it() {
        let mut a = in_scroll_mode();
        let now = Instant::now();
        show(&mut a, 0, 5, 100);
        assert_eq!(
            a.on_event(key(ct::KeyCode::Char('G'), ct::KeyModifiers::SHIFT), now),
            [scroll_msg(ScrollTo::Bottom)]
        );
        show(&mut a, 0, 0, 100);
        assert_eq!(a.scroll_mode, Some(TerminalId(4)));
        // Scrolling down one line from line 1 lands on the bottom too.
        show(&mut a, 0, 1, 100);
        a.on_event(ch('j'), now);
        show(&mut a, 0, 0, 100);
        assert_eq!(a.scroll_mode, Some(TerminalId(4)));
        // More output while scrolled keeps the mode.
        show(&mut a, 0, 3, 101);
        show(&mut a, 0, 3, 102);
        assert_eq!(a.scroll_mode, Some(TerminalId(4)));
        // Nobody on this client asked for the bottom: another client typed.
        show(&mut a, 0, 0, 102);
        assert_eq!(a.scroll_mode, None);
    }

    #[test]
    fn the_prefix_works_inside_scroll_mode() {
        let now = Instant::now();
        let mut a = in_scroll_mode();
        a.on_event(prefix(), now);
        assert!(a.armed);
        assert_eq!(
            a.on_event(ch('d'), now),
            [
                scroll_msg(ScrollTo::Bottom),
                Out::Send(ClientMsg::Detach),
                Out::Exit(Exit::Detached("main".into()))
            ]
        );

        let mut a = in_scroll_mode();
        a.on_event(prefix(), now);
        assert!(a.on_event(ch('z'), now).is_empty());
        assert_eq!(a.flash.as_ref().unwrap().0, "no binding for z");
        assert_eq!(a.scroll_mode, Some(TerminalId(4)));

        // The prefix twice sends the prefix key itself, which leaves the mode.
        let mut a = in_scroll_mode();
        a.on_event(prefix(), now);
        assert_eq!(a.on_event(prefix(), now), [input(b"\x1b ")]);
        assert_eq!(a.scroll_mode, None);

        // Scroll again inside scroll mode changes nothing.
        let mut a = in_scroll_mode();
        a.on_event(prefix(), now);
        assert!(a.on_event(ch('['), now).is_empty());
        assert_eq!(a.scroll_mode, Some(TerminalId(4)));
    }

    #[test]
    fn scroll_mode_ends_when_its_terminal_is_no_longer_focused() {
        let mut a = in_scroll_mode();
        let mut v = view(None);
        v.projects[0].panels[0].focused_terminal = None;
        a.on_server(ServerMsg::SessionView(v));
        assert_eq!(a.scroll_mode, None);
    }

    #[test]
    fn the_wheel_goes_to_a_program_that_asked_for_the_mouse() {
        let mut a = app();
        show(&mut a, SGR_CLICK, 0, 0);
        let now = Instant::now();
        let out = a.on_event(mouse(ct::MouseEventKind::ScrollUp, 5, 3), now);
        assert_eq!(out, [input(b"\x1b[<64;5;3M")]);
        let out = a.on_event(mouse(ct::MouseEventKind::ScrollDown, 1, 1), now);
        assert_eq!(out, [input(b"\x1b[<65;1;1M")]);
        assert_eq!(a.scroll_mode, None);
    }

    #[test]
    fn the_wheel_on_the_alternate_screen_sends_three_cursor_keys() {
        let mut a = app();
        let now = Instant::now();
        show(
            &mut a,
            Modes::ALTERNATE_SCREEN | Modes::ALTERNATE_SCROLL,
            0,
            0,
        );
        let up = mouse(ct::MouseEventKind::ScrollUp, 5, 3);
        let down = mouse(ct::MouseEventKind::ScrollDown, 5, 3);
        assert_eq!(a.on_event(up.clone(), now), [input(b"\x1b[A\x1b[A\x1b[A")]);
        assert_eq!(
            a.on_event(down.clone(), now),
            [input(b"\x1b[B\x1b[B\x1b[B")]
        );
        show(
            &mut a,
            Modes::ALTERNATE_SCREEN | Modes::ALTERNATE_SCROLL | Modes::APPLICATION_CURSOR,
            0,
            0,
        );
        assert_eq!(a.on_event(up.clone(), now), [input(b"\x1bOA\x1bOA\x1bOA")]);
        assert_eq!(a.on_event(down, now), [input(b"\x1bOB\x1bOB\x1bOB")]);
        // `?1007` off: nothing, history or not.
        show(&mut a, Modes::ALTERNATE_SCREEN, 0, 50);
        assert!(a.on_event(up, now).is_empty());
    }

    #[test]
    fn the_wheel_scrolls_the_history_three_lines_per_notch() {
        let mut a = app();
        let now = Instant::now();
        let up = mouse(ct::MouseEventKind::ScrollUp, 5, 3);
        let down = mouse(ct::MouseEventKind::ScrollDown, 5, 3);
        show(&mut a, 0, 0, 100);
        assert_eq!(
            a.on_event(up.clone(), now),
            [scroll_msg(ScrollTo::Lines(3))]
        );
        assert_eq!(a.on_event(down, now), [scroll_msg(ScrollTo::Lines(-3))]);
        // The wheel never enters scroll mode.
        assert_eq!(a.scroll_mode, None);
        // No history: nothing.
        show(&mut a, 0, 0, 0);
        assert!(a.on_event(up, now).is_empty());
    }

    #[test]
    fn the_wheel_in_scroll_mode_scrolls_even_for_a_mouse_program() {
        let mut a = in_scroll_mode();
        show(&mut a, SGR_CLICK, 0, 100);
        let out = a.on_event(mouse(ct::MouseEventKind::ScrollUp, 5, 3), Instant::now());
        assert_eq!(out, [scroll_msg(ScrollTo::Lines(3))]);
        // Clicks are not sent while scroll mode holds the terminal.
        let out = a.on_event(
            mouse(ct::MouseEventKind::Down(ct::MouseButton::Left), 5, 3),
            Instant::now(),
        );
        assert!(out.is_empty());
    }

    #[test]
    fn clicks_drags_and_motion_follow_the_terminals_mouse_modes() {
        use ct::MouseButton::Left;
        let now = Instant::now();
        let mut a = app();
        let down = mouse(ct::MouseEventKind::Down(Left), 5, 3);
        let up = mouse(ct::MouseEventKind::Up(Left), 5, 3);
        let drag = mouse(ct::MouseEventKind::Drag(Left), 6, 3);
        let moved = mouse(ct::MouseEventKind::Moved, 6, 3);

        show(&mut a, 0, 0, 0);
        assert!(a.on_event(down.clone(), now).is_empty());

        show(&mut a, SGR_CLICK, 0, 0);
        assert_eq!(a.on_event(down.clone(), now), [input(b"\x1b[<0;5;3M")]);
        assert_eq!(a.on_event(up, now), [input(b"\x1b[<0;5;3m")]);
        assert!(a.on_event(drag.clone(), now).is_empty());
        assert!(a.on_event(moved.clone(), now).is_empty());

        show(&mut a, Modes::MOUSE_DRAG | Modes::SGR_MOUSE, 0, 0);
        assert_eq!(a.on_event(drag.clone(), now), [input(b"\x1b[<32;6;3M")]);
        assert!(a.on_event(moved.clone(), now).is_empty());

        show(&mut a, Modes::MOUSE_MOTION | Modes::SGR_MOUSE, 0, 0);
        assert_eq!(a.on_event(moved, now), [input(b"\x1b[<35;6;3M")]);

        // X10 when SGR is off.
        show(&mut a, Modes::MOUSE_CLICK, 0, 0);
        assert_eq!(
            a.on_event(down, now),
            [input(&[0x1b, b'[', b'M', 32, 37, 35])]
        );
        // Modifiers: ctrl adds 16.
        let ctrl_click = ct::Event::Mouse(ct::MouseEvent {
            kind: ct::MouseEventKind::Down(Left),
            column: 4,
            row: 2,
            modifiers: ct::KeyModifiers::CONTROL,
        });
        show(&mut a, SGR_CLICK, 0, 0);
        assert_eq!(a.on_event(ctrl_click, now), [input(b"\x1b[<16;5;3M")]);
    }

    #[test]
    fn mouse_events_outside_the_inner_area_are_never_sent() {
        use ct::MouseButton::Left;
        let now = Instant::now();
        let mut a = app();
        show(&mut a, SGR_CLICK | Modes::MOUSE_MOTION, 0, 50);
        // The panel area is the window less the status bar: columns 0 to 79, rows 0 to 22; the
        // terminal fills 78x21 of it.
        for (column, row) in [(78, 3), (5, 21), (5, 22), (5, 23), (79, 3), (200, 200)] {
            for kind in [
                ct::MouseEventKind::Down(Left),
                ct::MouseEventKind::Moved,
                ct::MouseEventKind::ScrollUp,
            ] {
                assert!(
                    a.on_event(mouse0(kind, column, row), now).is_empty(),
                    "{kind:?} at {column},{row}"
                );
            }
        }
        // The corners of the terminal are in.
        assert_eq!(
            a.on_event(mouse0(ct::MouseEventKind::Down(Left), 0, 0), now),
            [input(b"\x1b[<0;1;1M")]
        );
        assert_eq!(
            a.on_event(mouse0(ct::MouseEventKind::Down(Left), 77, 20), now),
            [input(b"\x1b[<0;78;21M")]
        );

        // A terminal larger than the window (another client sized it): the status-bar row and
        // the columns past the window still send nothing.
        let mut f = frame(true, (100, 30), vec![]);
        f.modes = Modes(SGR_CLICK | Modes::MOUSE_MOTION);
        a.on_server(ServerMsg::Frame(f));
        for (column, row) in [(5, 23), (79, 23), (80, 3), (5, 24)] {
            assert!(
                a.on_event(mouse0(ct::MouseEventKind::Down(Left), column, row), now)
                    .is_empty(),
                "{column},{row}"
            );
        }
        assert_eq!(
            a.on_event(mouse0(ct::MouseEventKind::Down(Left), 79, 22), now),
            [input(b"\x1b[<0;80;23M")]
        );
    }

    #[test]
    fn focus_events_reach_a_program_that_asked_for_them() {
        let now = Instant::now();
        let mut a = app();
        show(&mut a, 0, 0, 0);
        assert!(a.on_event(ct::Event::FocusGained, now).is_empty());
        assert!(a.on_event(ct::Event::FocusLost, now).is_empty());
        show(&mut a, Modes::FOCUS_REPORTING, 0, 0);
        assert_eq!(a.on_event(ct::Event::FocusGained, now), [input(b"\x1b[I")]);
        assert_eq!(a.on_event(ct::Event::FocusLost, now), [input(b"\x1b[O")]);
    }

    #[test]
    fn focus_events_are_forwarded_in_scroll_mode_too() {
        let mut a = in_scroll_mode();
        show(&mut a, Modes::FOCUS_REPORTING, 5, 100);
        let now = Instant::now();
        assert_eq!(a.on_event(ct::Event::FocusLost, now), [input(b"\x1b[O")]);
        assert_eq!(a.scroll_mode, Some(TerminalId(4)));
    }

    #[test]
    fn a_frame_still_scrolled_back_clears_the_expected_bottom() {
        let mut a = in_scroll_mode();
        let now = Instant::now();
        show(&mut a, 0, 5, 100);
        a.on_event(key(ct::KeyCode::Char('G'), ct::KeyModifiers::SHIFT), now);
        // Output arrives before the answer to our scroll.
        show(&mut a, 0, 5, 101);
        // The bottom now comes from elsewhere.
        show(&mut a, 0, 0, 101);
        assert_eq!(a.scroll_mode, None);
    }

    #[test]
    fn the_prefix_twice_sends_the_non_breaking_space_when_that_is_what_arrived() {
        let mut a = app();
        let now = Instant::now();
        let nbsp = key(ct::KeyCode::Char('\u{a0}'), ct::KeyModifiers::NONE);
        a.on_event(nbsp.clone(), now);
        assert!(a.armed);
        assert_eq!(a.on_event(nbsp.clone(), now), [input("\u{a0}".as_bytes())]);
        a.on_event(nbsp, now);
        assert_eq!(a.on_event(ch('d'), now)[0], Out::Send(ClientMsg::Detach));
    }
}
