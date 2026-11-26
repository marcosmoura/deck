//! The client's state and its reaction to input and server messages. Pure: it returns what
//! to do (`Out`) and never touches the terminal or the socket.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::agent::SPIN_FPS;
use crate::graphics::{Images, SPINNER_FRAMES};
use crate::placed::Piece;
use deck_core::action::Action as Act;
use deck_core::config::{Config, NotifyMode};
use deck_core::keys::{KeyCode, KeyEvent, KeyEventKind, Modifiers, encode, encode_key};
use deck_core::layout;
use deck_core::model::{PanelId, ProjectId, TerminalId};
use deck_core::mouse::{self, MouseButton, MouseEvent, MouseKind};
use deck_core::protocol::{
    ByeReason, CellPx, ClientMsg, CursorShape, Modes, NoticeLevel, Pos, ScrollTo, ServerMsg,
};
use deck_core::theme::Theme;
use ratatui::crossterm::event as ct;

use crate::Exit;
use crate::agent;
use crate::context::ContextMenu;
use crate::copy::{CopyState, Edge};
use crate::grid::Grid;
use crate::input::{Action, Bindings, convert, event_label, keys_from_bytes, kind, outer_flags};
use crate::menu::NewMenu;
use crate::mirror::Mirror;
use crate::modal::{Modal, PromptKind, Retry};
use crate::picker::{Picker, Req};
use crate::resize::{Drag, SidebarDrag};
use crate::restore::Restore;
use crate::select::{self, MULTI_CLICK_MS, TextSel, Unit};
use crate::sidebar::{Hover, Reorder, RowDrag, Sel};
use crate::tier::Tier;

/// How long the pointer rests on a sidebar row before its cut texts are shown in full.
pub(crate) const TIP_DELAY: Duration = Duration::from_millis(750);

/// Below this many columns or rows the client shows only a message.
pub const MIN_SIZE: (u16, u16) = (20, 5);
/// How long a flash message replaces the hints.
pub const FLASH_FOR: Duration = Duration::from_secs(2);
/// How long an error from the daemon stays.
pub const ERROR_FLASH_FOR: Duration = Duration::from_secs(4);

pub fn too_small((cols, rows): (u16, u16)) -> bool {
    cols < MIN_SIZE.0 || rows < MIN_SIZE.1
}

/// How long a notice stays in the floating bar, by level.
pub fn notice_span(level: NoticeLevel) -> Duration {
    Duration::from_secs(match level {
        NoticeLevel::Info => 2,
        NoticeLevel::Warning => 4,
        NoticeLevel::Error => 6,
    })
}

/// Lines the wheel scrolls per notch.
const WHEEL_LINES: i32 = 3;
/// The flash when the bottom `+ New` button has no project to open the menu for.
const NO_PROJECT_FOR_NEW: &str = "no space";

/// What the event loop must do after the state changed.
#[derive(Debug, PartialEq, Eq)]
pub enum Out {
    Send(ClientMsg),
    /// Bytes for the outer terminal (an OSC 52 copy).
    Clipboard(Vec<u8>),
    /// deck's own desktop notification (`notify = "system"`); its fallback bytes go to the
    /// outer terminal when it cannot be shown.
    SystemNotify(crate::notify::SystemAlert),
    /// Bytes for the outer terminal that notify the user (OSC 9 or a bell).
    Notify(Vec<u8>),
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
    /// The form the icons are drawn in (see [`crate::tier`]).
    pub(crate) tier: Tier,
    /// The font check answered (it answers once; later answers change nothing).
    pub(crate) font_answered: bool,
    /// The logos as images; `None` unless the outer terminal is one that shows them (see
    /// [`crate::caps::plan`]).
    pub(crate) images: Option<Images>,
    /// The outer terminal's size in columns and rows.
    pub size: (u16, u16),
    /// The outer terminal's cell size in pixels; `None` when it does not say.
    pub cell: Option<CellPx>,
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
    /// The copy cursor of scroll mode.
    pub(crate) copy: Option<CopyState>,
    /// A selection drag that is past the screen's top or bottom and scrolls the view.
    pub(crate) edge: Option<Edge>,
    /// The search of scroll mode: its prompt, last query and highlights.
    pub(crate) search: crate::search::SearchState,
    /// Typed ahead while the colours were queried, or before the daemon's config arrived;
    /// replayed by [`App::start`].
    typeahead: Vec<KeyEvent>,
    /// False until the daemon's config is in (see [`App::hold_until_configured`]).
    configured: bool,
    /// Keys and pastes for a terminal that is not there yet, sent when it is.
    queued: Vec<Queued>,
    /// The outer terminal speaks the kitty keyboard protocol (it answered the start-up query).
    pub outer_kitty: bool,
    /// The keys whose press went to a terminal and whose release has not come: only their
    /// releases are sent on, to the terminal that got the press.
    held: Vec<(TerminalId, KeyCode)>,
    pub(crate) dirty: bool,
    /// The prompt, confirmation or help overlay that has the keyboard.
    pub modal: Option<Modal>,
    /// What to reopen for each action sent whose answer is still to come, oldest first.
    /// The second field: a refusal is not shown.
    pub(crate) pending: VecDeque<(Option<Retry>, bool)>,
    /// The session picker, open over everything but the prompts it starts.
    pub picker: Option<Picker>,
    /// The New menu, open over the sidebar.
    pub menu: Option<NewMenu>,
    /// The context menu of a right click, open over everything but the dialogs.
    pub context: Option<ContextMenu>,
    /// The Restore dialog.
    pub restore: Option<Restore>,
    /// How many answers come before the one of the launch the menu sent; its success ends
    /// the sidebar focus.
    pub(crate) launch_result: Option<usize>,
    /// What each request sent whose answer is still to come was for, oldest first.
    pub(crate) requests: VecDeque<Req>,
    /// A click focused a terminal; the rest of that click (drag, release) is not forwarded.
    pub(crate) swallow_release: bool,
    /// Until when the resize keys act without the prefix.
    pub(crate) resize_until: Option<Instant>,
    /// The border being dragged with the mouse.
    pub(crate) drag: Option<Drag>,
    /// The user hid the sidebar (it also hides when the window is narrow).
    pub sidebar_hidden: bool,
    /// The sidebar is collapsed to its narrow strip: client state, like `sidebar_hidden`.
    pub sidebar_collapsed: bool,
    /// The sidebar's width: client state, set by a drag and by a config that changes it.
    pub sidebar_width: u16,
    /// The sidebar's separator being dragged with the mouse.
    pub(crate) sidebar_drag: Option<SidebarDrag>,
    /// A left press landed on a space or panel row; its release on the same row acts.
    pub(crate) row_press: Option<Sel>,
    /// A space or panel row being dragged to another place.
    pub(crate) row_drag: Option<RowDrag>,
    /// The move just dropped, drawn until the daemon's view comes.
    /// How many answers come before the move's own.
    pub(crate) row_drop: Option<(Reorder, usize)>,
    /// The first `SessionView` of an attach has not come yet: the client compares its
    /// viewport with the daemon's then.
    check_viewport: bool,
    /// The sidebar has the keyboard.
    pub sidebar_focus: bool,
    pub selection: Option<Sel>,
    /// The first sidebar row shown.
    pub sidebar_offset: usize,
    /// What the pointer is over in the sidebar; client-local.
    pub(crate) hover: Option<Hover>,
    /// The last pointer position, to recompute the hover when rows move under it.
    pub(crate) pointer: Option<(u16, u16)>,
    /// A left press landed on a chip of the bottom bar; its release on the same chip acts.
    pub(crate) chip_down: Option<Hover>,
    /// Where the client was started, for new projects' directories.
    pub(crate) cwd: PathBuf,
    pub(crate) home: Option<PathBuf>,
    /// The spinner's tick counter, advanced on every spinner tick while a terminal spins.
    pub(crate) spin: usize,
    /// When the spinner moves next; `None` while nothing spins (no timer runs then).
    pub(crate) spin_at: Option<Instant>,
    /// When the client started: the loader's frame is picked from the time since.
    pub(crate) started: Instant,
    /// The loader image's frame (0..8) as of the last tick; every spinner shows it.
    pub(crate) spin_frame: u8,
    /// The last draw showed a spinner (true until one has drawn): while none shows, no timer
    /// runs and nothing is redrawn for it.
    pub(crate) spinner_seen: std::cell::Cell<bool>,
    /// The client can show deck's own notifications (a notifier was found at start).
    pub(crate) own_notifications: bool,
    /// The texts of the sidebar's rows as the last draw put them, for the hover tooltip.
    pub(crate) sidebar_texts: std::cell::RefCell<Vec<crate::sidebar::Drawn>>,
    /// The row the pointer rests on and when its tooltip shows (`TIP_DELAY` after it got
    /// there), and whether it shows.
    tip: Option<(Sel, Instant)>,
    tip_up: bool,
    /// The outer terminal has focus, as far as its focus events told.
    focused: bool,
    /// The first line of the help overlay shown.
    pub help_scroll: usize,
    /// A focus event came: the focus is re-reported on every attach.
    focus_seen: bool,
    /// Text selected with the mouse in a terminal; client-local.
    pub(crate) text_sel: Option<TextSel>,
    /// The last press that started a selection and how many presses in a row it ends, for
    /// double and triple clicks.
    last_click: Option<(Instant, TerminalId, (u16, u16), u8)>,
    /// The panel of the last alert written while the outer terminal was unfocused, and when:
    /// a focus gain soon after brings the user to it.
    alert_jump: Option<(PanelId, Instant)>,
}

/// How long after an alert a focus gain still counts as the click on its notification.
const ALERT_JUMP_WINDOW: Duration = Duration::from_secs(10);

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
    Key(KeyEvent, KeyEventKind),
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
            tier: Tier::Unicode,
            font_answered: false,
            images: None,
            size,
            cell: None,
            armed: false,
            flash: None,
            flash_error: false,
            flash_level: None,
            notices: VecDeque::new(),
            scroll_mode: None,
            bottom_expected: false,
            copy: None,
            edge: None,
            search: crate::search::SearchState::default(),
            typeahead: keys_from_bytes(typeahead),
            configured: true,
            queued: Vec::new(),
            outer_kitty: false,
            held: Vec::new(),
            dirty: true,
            modal: None,
            pending: VecDeque::new(),
            picker: None,
            menu: None,
            context: None,
            restore: None,
            launch_result: None,
            requests: VecDeque::new(),
            swallow_release: false,
            resize_until: None,
            drag: None,
            sidebar_hidden: !config.sidebar,
            sidebar_collapsed: config.sidebar_collapsed,
            sidebar_width: layout::clamp_sidebar_width(size.0, config.sidebar_width),
            sidebar_drag: None,
            row_press: None,
            row_drag: None,
            row_drop: None,
            check_viewport: true,
            sidebar_focus: false,
            selection: None,
            sidebar_offset: 0,
            hover: None,
            pointer: None,
            chip_down: None,
            cwd: PathBuf::from("/"),
            home: None,
            spin: 0,
            spin_at: None,
            started: Instant::now(),
            spin_frame: 0,
            spinner_seen: std::cell::Cell::new(true),
            own_notifications: false,
            sidebar_texts: std::cell::RefCell::default(),
            tip: None,
            tip_up: false,
            focused: true,
            help_scroll: 0,
            focus_seen: false,
            text_sel: None,
            last_click: None,
            alert_jump: None,
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
        layout::sidebar_cols(self.size.0, self.side_width(), !self.sidebar_hidden)
    }

    /// The panel area, in columns and rows: what the daemon sizes terminals for.
    pub fn viewport(&self) -> (u16, u16) {
        layout::panel_area(self.size, self.side_width(), !self.sidebar_hidden)
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
        let collapsed =
            (new.sidebar_collapsed != old.sidebar_collapsed).then_some(new.sidebar_collapsed);
        if new.sidebar_width != old.sidebar_width {
            self.sidebar_width = layout::clamp_sidebar_width(self.size.0, new.sidebar_width);
            sized = true;
        }
        if let Some(collapsed) = collapsed {
            // Where only the collapsed strip fits, a reload cannot expand it (as the chip and
            // the action cannot).
            let full_fits = layout::sidebar_cols(self.size.0, self.sidebar_width, true) > 0;
            let strip_fits =
                layout::sidebar_cols(self.size.0, layout::COLLAPSED_SIDEBAR_WIDTH, true) > 0;
            if !collapsed && !full_fits && strip_fits {
                self.set_flash("window too narrow to expand".into(), Instant::now(), false);
            } else {
                self.set_sidebar_collapsed(collapsed);
                sized = true;
            }
        }
        if self.config.sidebar_space_gap != old.sidebar_space_gap {
            self.ensure_visible();
            self.refresh_hover();
        }
        self.check_row_gesture();
        if !sized {
            return Vec::new();
        }
        if !self.sidebar_visible() {
            self.sidebar_focus = false;
        }
        self.ensure_visible();
        self.refresh_hover();
        let (cols, rows) = self.viewport();
        if too_small(self.size) || (cols, rows) == before {
            return Vec::new();
        }
        // Not a claim on the terminals' size: the client the user last typed in keeps it.
        vec![Out::Send(ClientMsg::ReloadViewport {
            cols,
            rows,
            cell: self.cell,
        })]
    }

    /// A message from the daemon for the floating bar: at once when nothing of the daemon's
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

    /// Ends a sidebar drag from outside the mouse: the button may still be down, and the rest
    /// of the gesture (drags, the release) is not forwarded to a program.
    fn end_sidebar_drag(&mut self) {
        if self.sidebar_drag.take().is_some() {
            self.swallow_release = true;
            self.dirty = true;
        }
    }

    pub(crate) fn viewport_msg(&self) -> Out {
        let (cols, rows) = self.viewport();
        Out::Send(ClientMsg::Viewport {
            cols,
            rows,
            cell: self.cell,
        })
    }

    /// Whether the client can show program images: the outer terminal speaks the kitty
    /// protocol, draws in 24-bit colour and has a known cell size. What `Attach` says.
    pub fn program_images(&self) -> bool {
        self.images.is_some() && self.cell.is_some()
    }

    /// The outer terminal's cell size changed (a font zoom keeps the columns and rows): the
    /// daemon is told, since it sizes the terminals' pixels with it.
    pub fn set_cell(&mut self, cell: Option<CellPx>) -> Vec<Out> {
        if self.cell == cell {
            return Vec::new();
        }
        self.cell = cell;
        if too_small(self.size) {
            return Vec::new();
        }
        vec![self.viewport_msg()]
    }

    /// A prompt, a confirmation or an overlay has the keyboard.
    pub fn overlay_open(&self) -> bool {
        self.modal.is_some()
            || self.picker.is_some()
            || self.menu.is_some()
            || self.context.is_some()
            || self.restore.is_some()
    }

    /// Forgets everything that belongs to the session the client was attached to, for the
    /// next one. What is not specific to a session (the sidebar's visibility and width, the
    /// flash) stays.
    pub(crate) fn reset_session(&mut self) {
        self.mirror = Mirror::default();
        if let Some(images) = &mut self.images {
            images.program.clear();
        }
        self.modal = None;
        self.picker = None;
        self.menu = None;
        self.context = None;
        self.restore = None;
        self.pending.clear();
        self.launch_result = None;
        self.requests.clear();
        self.armed = false;
        self.scroll_mode = None;
        self.bottom_expected = false;
        self.end_copy();
        self.swallow_release = false;
        self.resize_until = None;
        self.drag = None;
        self.sidebar_drag = None;
        self.row_press = None;
        self.row_drag = None;
        self.row_drop = None;
        self.check_viewport = true;
        self.queued.clear();
        self.held.clear();
        self.sidebar_focus = false;
        self.selection = None;
        self.sidebar_offset = 0;
        self.hover = None;
        self.chip_down = None;
        self.spin_at = None;
        self.dirty = true;
    }

    /// Shows a message in the floating bar for a while.
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

    /// The next moment `tick` has something to do: a flash ends, the resize repeat does or
    /// the spinner moves.
    pub fn wake_deadline(&self) -> Option<Instant> {
        let edge = self.edge.as_ref().map(|e| e.next);
        let tip = self.tip.filter(|_| !self.tip_up).map(|(_, at)| at);
        [
            self.flash_deadline(),
            self.resize_until,
            self.spin_at,
            edge,
            tip,
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// The font check's answer: a late `Nerd` upgrade redraws. Only the first answer counts.
    pub fn on_font_answer(&mut self, found: bool) {
        if std::mem::replace(&mut self.font_answered, true) {
            return;
        }
        let tier = self.tier.with_font(found);
        if tier != self.tier {
            self.tier = tier;
            self.dirty = true;
        }
    }

    pub fn take_dirty(&mut self) -> bool {
        self.configured && std::mem::take(&mut self.dirty)
    }

    /// Starts the spinner's timer when a terminal spins, a spinner is on screen and none runs
    /// yet.
    pub(crate) fn arm_spinner(&mut self, now: Instant) {
        if self.spin_at.is_none()
            && self.spinner_seen.get()
            && self.mirror.view().is_some_and(agent::view_spins)
        {
            self.spin_at = Some(self.next_frame_at(now));
        }
    }

    /// The loader image's frame at `now`: the tick of the clock (`SPIN_FPS` a second, since
    /// the client started) scaled to the loader's `SPINNER_FRAMES`, so it never rounds to 0
    /// if the two differ.
    fn frame_at(&self, now: Instant) -> u8 {
        let elapsed = now.saturating_duration_since(self.started).as_nanos();
        let tick = elapsed * SPIN_FPS as u128 / 1_000_000_000 % SPIN_FPS as u128;
        u8::try_from(tick * SPINNER_FRAMES as u128 / SPIN_FPS as u128).unwrap_or(0)
    }

    /// The next 1/[`SPIN_FPS`] s boundary of the time since the start, after `now`.
    fn next_frame_at(&self, now: Instant) -> Instant {
        let frames = SPIN_FPS as u128;
        let elapsed = now.saturating_duration_since(self.started).as_nanos();
        let next = elapsed * frames / 1_000_000_000 + 1;
        let nanos = (next * 1_000_000_000).div_ceil(frames);
        self.started + Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
    }

    /// The row whose hover tooltip shows: the pointer rested on it for `TIP_DELAY` and is
    /// still there.
    pub(crate) fn tip_shown(&self) -> Option<Sel> {
        let (sel, _) = self.tip.filter(|_| self.tip_up)?;
        (self.hover == Some(Hover::Row(sel))).then_some(sel)
    }

    /// Follows the pointer for the hover tooltip after an event: moving onto a row starts its
    /// delay, and anything else (another row, leaving the rows, a key, a click, the wheel)
    /// takes the tooltip down.
    fn follow_tip(&mut self, moved: bool, now: Instant) {
        // Only a row with a cut text has a tooltip: any other row starts no timer and no draw.
        let cut = |sel: Sel| {
            self.sidebar_texts
                .borrow()
                .iter()
                .any(|d| d.owner == sel && d.cut)
        };
        let row = match self.hover {
            Some(Hover::Row(sel)) if moved && cut(sel) => Some(sel),
            _ => None,
        };
        if row.is_some() && row == self.tip.map(|(sel, _)| sel) {
            return;
        }
        if std::mem::take(&mut self.tip_up) {
            self.dirty = true;
        }
        self.tip = row.map(|sel| (sel, now + TIP_DELAY));
    }

    /// Moves the spinner and drops an expired flash message.
    pub fn tick(&mut self, now: Instant) -> Vec<Out> {
        if let Some((sel, at)) = self.tip
            && !self.tip_up
            && now >= at
        {
            if self.hover == Some(Hover::Row(sel)) {
                self.tip_up = true;
                self.dirty = true;
            } else {
                self.tip = None;
            }
        }
        if self.spin_at.is_some_and(|at| now >= at) {
            self.spin = self.spin.wrapping_add(1);
            self.spin_frame = self.frame_at(now);
            self.spin_at = None;
            // Off screen it only stops: the draw that shows it again starts it.
            if self.spinner_seen.get() {
                self.arm_spinner(now);
                self.dirty = true;
            }
        }
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
        self.edge_tick(now)
    }

    /// The cursor shape, and whether it blinks, the outer terminal should show, when a
    /// terminal's cursor is visible.
    pub fn cursor_shape(&self) -> Option<(CursorShape, bool)> {
        let t = self.mirror.terminal(self.mirror.focused_id()?)?;
        t.cursor.visible.then_some((t.cursor.shape, t.cursor.blink))
    }

    pub fn on_event(&mut self, event: ct::Event, now: Instant) -> Vec<Out> {
        let moved = matches!(
            &event,
            ct::Event::Mouse(m) if m.kind == ct::MouseEventKind::Moved
        );
        let outs = self.handle_event(event, now);
        self.follow_tip(moved && !self.overlay_open(), now);
        // A prompt or an overlay that opened ends the hover.
        if self.overlay_open() {
            self.set_hover(None);
        }
        outs
    }

    fn handle_event(&mut self, event: ct::Event, now: Instant) -> Vec<Out> {
        match event {
            ct::Event::Key(key) if !self.configured => {
                if kind(&key) != KeyEventKind::Release {
                    self.typeahead.extend(convert(key));
                }
                Vec::new()
            }
            ct::Event::Key(key) => match (convert(key), kind(&key)) {
                (None, _) => Vec::new(),
                (Some(k), KeyEventKind::Release) => self.on_key_release(k),
                (Some(k), kind) => self.key(k, kind, now),
            },
            ct::Event::Paste(text) => self.on_paste(text),
            ct::Event::Resize(cols, rows) => {
                self.size = (cols, rows);
                self.dirty = true;
                self.end_sidebar_drag();
                self.sidebar_width = layout::clamp_sidebar_width(cols, self.sidebar_width);
                self.check_row_gesture();
                if !self.sidebar_visible() {
                    self.sidebar_focus = false;
                }
                self.ensure_visible();
                self.restore_fit();
                self.refresh_hover();
                if too_small(self.size) {
                    return Vec::new();
                }
                vec![self.viewport_msg()]
            }
            ct::Event::Mouse(mouse) => self.on_mouse(mouse, now),
            ct::Event::FocusGained => self.on_focus(true, now),
            ct::Event::FocusLost => self.on_focus(false, now),
        }
    }

    /// The terminal under a cell of the outer window, its index in the grid, and the
    /// 0-based cell of its screen when the cell is on it (not on a border).
    fn terminal_at(&self, column: u16, row: u16) -> Option<Hit> {
        let (panel_cols, panel_rows) = self.size;
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

    /// Whether the right button on a terminal is deck's: its program has no mouse reporting,
    /// Option/Alt is held, or scroll mode is on.
    fn deck_has_right(&self, terminal: TerminalId, mods: ct::KeyModifiers) -> bool {
        self.scroll_mode.is_some()
            || mods.contains(ct::KeyModifiers::ALT)
            || !self
                .mirror
                .terminal(terminal)
                .is_some_and(|t| mouse::reporting(t.modes))
    }

    fn on_mouse(&mut self, event: ct::MouseEvent, now: Instant) -> Vec<Out> {
        // A release ends the swallowed click wherever it lands.
        let swallowed = self.swallow_release;
        if matches!(event.kind, ct::MouseEventKind::Up(_)) {
            self.swallow_release = false;
        }
        self.pointer = Some((event.column, event.row));
        let released = matches!(event.kind, ct::MouseEventKind::Up(_));
        if let Some(outs) = self.select_mouse(event, now) {
            return outs;
        }
        // Any other press ends the selection.
        if matches!(event.kind, ct::MouseEventKind::Down(_)) && self.text_sel.take().is_some() {
            self.dirty = true;
        }
        // The press on the chip survives the pointer drifting (a drag with the left button);
        // anything else but its release ends it.
        let chip_down = match event.kind {
            ct::MouseEventKind::Up(_) => self.chip_down.take(),
            ct::MouseEventKind::Drag(ct::MouseButton::Left) => None,
            _ => {
                self.chip_down = None;
                None
            }
        };
        // The Restore dialog has the mouse before everything else.
        if self.restore.is_some() && !too_small(self.size) {
            self.drag = None;
            self.end_sidebar_drag();
            self.end_row_gesture(!released);
            return self.restore_mouse(event);
        }
        // The context menu and the New menu have the mouse before the sidebar.
        if self.context.is_some() && !too_small(self.size) {
            self.drag = None;
            self.end_sidebar_drag();
            self.end_row_gesture(!released);
            return self.context_mouse(event);
        }
        if self.menu.is_some() && !too_small(self.size) {
            self.drag = None;
            self.end_sidebar_drag();
            self.end_row_gesture(!released);
            return self.menu_mouse(event);
        }
        if self.overlay_open() || too_small(self.size) {
            self.drag = None;
            self.end_sidebar_drag();
            self.end_row_gesture(!released);
            return Vec::new();
        }
        // The sidebar's separator drag and a border drag are ours from their press to their
        // release, never a program's.
        if let Some(outs) = self.sidebar_drag_mouse(event, now) {
            return outs;
        }
        // A press on a row is the sidebar's until its release, wherever the pointer goes.
        if let Some(outs) = self.row_drag_mouse(event) {
            return outs;
        }
        // A border drag is ours from its press to its release, never a program's.
        if let Some(outs) = self.drag_mouse(event, now) {
            return outs;
        }
        if event.column < self.sidebar_cols() {
            return match event.kind {
                // Motion redraws only when the hover target changed.
                ct::MouseEventKind::Moved => {
                    let hover = self.hover_at(event.column, event.row);
                    self.set_hover(hover);
                    Vec::new()
                }
                ct::MouseEventKind::Up(ct::MouseButton::Left)
                    if chip_down.is_some()
                        && self.hover_at(event.column, event.row) == chip_down =>
                {
                    self.dirty = true;
                    self.hover = None;
                    match chip_down {
                        Some(Hover::Restore) => return self.open_restore(now),
                        Some(Hover::CollapseSidebar | Hover::ExpandSidebar) => {
                            return self.toggle_sidebar_collapsed(now);
                        }
                        Some(Hover::New) => match self.new_target() {
                            Some(project) => self.open_new_menu(project),
                            None => self.set_flash(NO_PROJECT_FOR_NEW.into(), now, false),
                        },
                        _ => self.open_new_project(),
                    }
                    Vec::new()
                }
                ct::MouseEventKind::Down(ct::MouseButton::Right) => {
                    self.dirty = true;
                    self.hover = None;
                    if let Some(Hover::Row(sel)) = self.hover_at(event.column, event.row) {
                        self.open_row_menu(sel, (event.column, event.row));
                    }
                    Vec::new()
                }
                kind => {
                    self.dirty = true;
                    let outs = self.sidebar_mouse(event);
                    // A click ends the hover; a wheel scroll recomputes it for the new rows.
                    self.hover = None;
                    if matches!(
                        kind,
                        ct::MouseEventKind::ScrollUp | ct::MouseEventKind::ScrollDown
                    ) {
                        self.refresh_hover();
                    }
                    outs
                }
            };
        }
        // Anything over the panel area ends the hover.
        self.set_hover(None);
        // The floating bar's cells belong to no program: presses, motion and the wheel there
        // reach nothing. A release goes on as it would without the bar, so a program that
        // took the press gets its button-up.
        if !matches!(event.kind, ct::MouseEventKind::Up(_))
            && self.bar_covers(event.column, event.row)
        {
            return Vec::new();
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
                let outs = vec![self.act(Act::FocusTerminal { terminal }, None)];
                // The menu opens on the terminal when deck has the right button there.
                if event.kind == ct::MouseEventKind::Down(ct::MouseButton::Right)
                    && cell.is_some()
                    && self.deck_has_right(terminal, event.modifiers)
                {
                    self.open_terminal_menu(terminal, (event.column, event.row));
                }
                return outs;
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
        if event.kind == ct::MouseEventKind::Down(ct::MouseButton::Right)
            && self.deck_has_right(terminal, event.modifiers)
        {
            self.open_terminal_menu(terminal, (event.column, event.row));
            return Vec::new();
        }
        // A left press selects text unless the program asked for the mouse; Option/Alt
        // takes it back from the program, and scroll mode never gives it one.
        if event.kind == ct::MouseEventKind::Down(ct::MouseButton::Left)
            && (self.scroll_mode.is_some()
                || !mouse::reporting(modes)
                || event.modifiers.contains(ct::KeyModifiers::ALT))
        {
            self.start_selection(terminal, (col, row), now);
            return Vec::new();
        }
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

    /// A press that starts a selection: a third press in a row on the same cell selects the
    /// line, a second the word.
    fn start_selection(&mut self, terminal: TerminalId, cell: (u16, u16), now: Instant) {
        let count = match self.last_click {
            Some((at, t, c, n))
                if t == terminal
                    && c == cell
                    && now.duration_since(at).as_millis() <= MULTI_CLICK_MS =>
            {
                n % 3 + 1
            }
            _ => 1,
        };
        self.last_click = Some((now, terminal, cell, count));
        let unit = match count {
            1 => Unit::Cell,
            2 => Unit::Word,
            _ => Unit::Line,
        };
        let Some(term) = self.mirror.terminal(terminal) else {
            return;
        };
        let at = Pos {
            row: term.top_row + u64::from(cell.1),
            col: cell.0,
        };
        let cols = select::grow(
            unit,
            term.lines.get(usize::from(cell.1)),
            term.size.0,
            cell.0,
        );
        let mut sel = TextSel::new(terminal, at, unit, cols);
        sel.dragging = true;
        self.text_sel = Some(sel);
        self.dirty = true;
    }

    /// The drag and release of a selection, ours wherever the pointer goes; the release
    /// copies the text. `None` when no selection is being dragged.
    fn select_mouse(&mut self, event: ct::MouseEvent, now: Instant) -> Option<Vec<Out>> {
        let sel = self.text_sel.as_ref().filter(|s| s.dragging)?;
        let terminal = sel.terminal;
        match event.kind {
            ct::MouseEventKind::Drag(ct::MouseButton::Left) => {
                if self.scroll_mode == Some(terminal)
                    && let Some(up) = self.edge_dir(terminal, event.row)
                {
                    let col = self
                        .clamped_cell(terminal, event.column, event.row)
                        .map_or(0, |(col, _)| col);
                    return Some(self.edge_drag(terminal, up, col, now));
                }
                self.edge = None;
                let head = self.clamped_cell(terminal, event.column, event.row);
                if let (Some((col, row)), Some(term)) = (head, self.mirror.terminal(terminal)) {
                    let head = Pos {
                        row: term.top_row + u64::from(row),
                        col,
                    };
                    let unit = self.text_sel.as_ref().map(|s| s.unit)?;
                    let cols =
                        select::grow(unit, term.lines.get(usize::from(row)), term.size.0, col);
                    if let Some(sel) = self.text_sel.as_mut()
                        && sel.head != head
                    {
                        sel.set_head(head, cols);
                        self.dirty = true;
                    }
                }
                Some(Vec::new())
            }
            ct::MouseEventKind::Up(_) => {
                self.edge = None;
                // The release is ours even when nothing ends up selected.
                let mut sel = self.text_sel.take()?;
                sel.dragging = false;
                self.dirty = true;
                if sel.range().is_none() {
                    return Some(Vec::new());
                }
                if self.scroll_mode == Some(terminal)
                    && let Some(copy) = self.copy.as_mut()
                {
                    copy.cursor = sel.head;
                }
                let outs = self.copy_mouse_selection(&sel, now);
                self.text_sel = Some(sel);
                Some(outs)
            }
            _ => None,
        }
    }

    /// Whether the pointer's window row is on or above the first row of the screen of
    /// `terminal` (`true`), or on or below its last row (`false`).
    fn edge_dir(&self, terminal: TerminalId, row: u16) -> Option<bool> {
        let grid = Grid::of(self)?;
        let i = grid.ids.iter().position(|t| *t == terminal)?;
        let inner = grid.inner(i);
        // A screen row at the window's edge counts as the edge; one with a border row
        // beyond it only past it.
        let last = inner.y + inner.height;
        if row < inner.y || (row == inner.y && inner.y == 0) {
            Some(true)
        } else if row >= last || (row + 1 == last && last >= self.size.1) {
            Some(false)
        } else {
            None
        }
    }

    /// The cell of `terminal`'s screen nearest to a cell of the window.
    fn clamped_cell(&self, terminal: TerminalId, column: u16, row: u16) -> Option<(u16, u16)> {
        let grid = Grid::of(self)?;
        let i = grid.ids.iter().position(|t| *t == terminal)?;
        let inner = grid.inner(i);
        let size = self.mirror.terminal(terminal)?.size;
        let (w, h) = (inner.width.min(size.0), inner.height.min(size.1));
        if w == 0 || h == 0 {
            return None;
        }
        let x = column.saturating_sub(self.sidebar_cols());
        Some((
            x.saturating_sub(inner.x).min(w - 1),
            row.saturating_sub(inner.y).min(h - 1),
        ))
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
    fn on_focus(&mut self, gained: bool, now: Instant) -> Vec<Out> {
        self.focused = gained;
        self.focus_seen = true;
        if !gained && self.sidebar_drag.take().is_some() {
            self.dirty = true;
        }
        if !gained {
            self.end_row_gesture(false);
        }
        if !gained {
            self.set_hover(None);
        }
        let mut outs = vec![Out::Send(ClientMsg::Focus(gained))];
        let jump = if gained { self.alert_jump(now) } else { None };
        // The focus-in goes to the terminal the user lands on.
        outs.extend(self.forward_focus(gained, jump.map(|(_, _, terminal)| terminal)));
        if let Some((project, panel, terminal)) = jump {
            outs.push(self.act(Act::FocusProject { project }, None));
            outs.push(self.act(Act::FocusPanel { panel }, None));
            outs.push(self.act(Act::FocusTerminal { terminal }, None));
        }
        outs
    }

    /// On a focus gain: the space, panel and terminal to show for a recent alert, if its panel
    /// still needs the user and no prompt or overlay is open. The alert is forgotten either way.
    fn alert_jump(&mut self, now: Instant) -> Option<(ProjectId, PanelId, TerminalId)> {
        let (panel, at) = self.alert_jump.take()?;
        if self.overlay_open() || now.saturating_duration_since(at) > ALERT_JUMP_WINDOW {
            return None;
        }
        let view = self.mirror.view()?;
        let project = view
            .projects
            .iter()
            .find(|p| p.panels.iter().any(|x| x.id == panel))?;
        let terminal = agent::panel_attention(project.panels.iter().find(|x| x.id == panel)?)?;
        Some((project.id, panel, terminal))
    }

    /// The focus report for the focused terminal, if its program asked for them.
    /// `to` is the terminal a jump lands on, which gets the report instead of the focused one.
    fn forward_focus(&mut self, gained: bool, to: Option<TerminalId>) -> Vec<Out> {
        self.end_repeat();
        if self.overlay_open() {
            return Vec::new();
        }
        let Some(terminal) = to.or_else(|| self.ready()) else {
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

    /// A scroll request; remembers when it is meant to land on the live screen, and where
    /// the copy cursor's view will be.
    pub(crate) fn scroll(&mut self, terminal: TerminalId, to: ScrollTo) -> Out {
        let Some(t) = self.mirror.terminal(terminal) else {
            return Out::Send(ClientMsg::Scroll { terminal, to });
        };
        let mirror_top = i64::try_from(t.top_row).unwrap_or(i64::MAX);
        let live_top = mirror_top + i64::from(t.display_offset);
        let rows = i64::from(t.size.1);
        let oldest = (live_top - i64::from(t.history)).max(0);
        let expected = self
            .copy
            .as_ref()
            .filter(|c| c.terminal == terminal)
            .and_then(|c| c.expected_top)
            .map_or(mirror_top, |top| i64::try_from(top).unwrap_or(i64::MAX));
        let new_top = match to {
            ScrollTo::Bottom => live_top,
            ScrollTo::Top => oldest,
            ScrollTo::Lines(n) => expected - i64::from(n),
            ScrollTo::Pages(n) => expected - i64::from(n) * rows,
            ScrollTo::Row(row) => {
                let row = i64::try_from(row).unwrap_or(i64::MAX);
                if (expected..expected + rows).contains(&row) {
                    expected
                } else {
                    row - rows / 2
                }
            }
        }
        .clamp(oldest, live_top.max(oldest));
        let toward_top = match to {
            ScrollTo::Top => true,
            ScrollTo::Lines(n) => n > 0,
            ScrollTo::Pages(n) => n > 0,
            ScrollTo::Bottom | ScrollTo::Row(_) => false,
        };
        if self.scroll_mode.is_some() && !toward_top && new_top >= live_top {
            self.bottom_expected = true;
        }
        if let Some(copy) = self.copy.as_mut().filter(|c| c.terminal == terminal) {
            copy.expected_top = Some(u64::try_from(new_top).unwrap_or(0));
        }
        Out::Send(ClientMsg::Scroll { terminal, to })
    }

    /// Leaves scroll mode and returns the terminal to the bottom.
    pub(crate) fn leave_scroll(&mut self) -> Vec<Out> {
        self.bottom_expected = false;
        self.end_copy();
        match self.scroll_mode.take() {
            Some(terminal) => vec![Out::Send(ClientMsg::Scroll {
                terminal,
                to: ScrollTo::Bottom,
            })],
            None => Vec::new(),
        }
    }

    /// Ends scroll mode when its terminal is no longer the focused one.
    fn check_scroll_target(&mut self) {
        if self
            .scroll_mode
            .is_some_and(|t| self.mirror.focused_id() != Some(t))
        {
            self.scroll_mode = None;
            self.bottom_expected = false;
            self.end_copy();
        }
    }

    fn on_key(&mut self, key: KeyEvent, now: Instant) -> Vec<Out> {
        self.key(key, KeyEventKind::Press, now)
    }

    /// The release of a key whose press went to a terminal goes there too, for a program that
    /// asked for it. Deck's own keys have no use for releases.
    fn on_key_release(&mut self, key: KeyEvent) -> Vec<Out> {
        let code = held_code(key.code);
        let Some(at) = self.held.iter().position(|(_, held)| *held == code) else {
            return Vec::new();
        };
        let (terminal, _) = self.held.remove(at);
        if self.ready() != Some(terminal) {
            return Vec::new();
        }
        self.send(&Queued::Key(key, KeyEventKind::Release))
            .into_iter()
            .collect()
    }

    /// Whether a key that stands alone (a modifier, a media key) can go to the terminal:
    /// nothing else has the keyboard.
    fn typing(&self) -> bool {
        !(self.overlay_open()
            || self.armed
            || self.scroll_mode.is_some()
            || self.sidebar_focus
            || self.row_drag.is_some())
    }

    /// Sends a key to the focused terminal and remembers its release is due.
    fn send_key(&mut self, key: KeyEvent, kind: KeyEventKind) -> Vec<Out> {
        // Eight keys held at once are plenty; a stuck entry costs one stray release.
        if let Some(terminal) = self.mirror.focused_id() {
            let code = held_code(key.code);
            if !self.held.contains(&(terminal, code)) {
                if self.held.len() >= 8 {
                    self.held.remove(0);
                }
                self.held.push((terminal, code));
            }
        }
        self.send_or_queue(Queued::Key(key, kind))
    }

    /// The kitty keyboard flags the outer terminal should have pushed: those the focused
    /// terminal's program needs, when the outer terminal has the protocol and no overlay takes
    /// the keyboard (deck's own keys use the legacy form).
    pub fn outer_keyboard(&self) -> u8 {
        if !self.outer_kitty || self.overlay_open() {
            return 0;
        }
        self.mirror
            .focused_id()
            .and_then(|id| self.mirror.terminal(id))
            .map_or(0, |t| outer_flags(t.modes.kitty_flags()))
    }

    fn key(&mut self, key: KeyEvent, kind: KeyEventKind, now: Instant) -> Vec<Out> {
        // Modifier, media and lock keys are the program's alone: they never touch the prefix
        // or an open overlay (the Option key goes down before the prefix's space does).
        if matches!(
            key.code,
            KeyCode::Modifier(_) | KeyCode::Media(_) | KeyCode::Special(_)
        ) {
            return if self.typing() {
                self.send_key(key, kind)
            } else {
                Vec::new()
            };
        }
        self.dirty = true;
        // A key ends a sidebar drag, so an overlay it opens never shows one; the rest of the
        // gesture is still the drag's and goes to no program.
        self.end_sidebar_drag();
        // Esc cancels a row drag and goes no further; any other key cancels it and acts.
        let dragging = self.row_drag.is_some();
        if dragging {
            self.end_row_gesture(true);
            if key.code == KeyCode::Esc && !key.mods.ctrl && !key.mods.alt {
                return Vec::new();
            }
        }
        if self.modal.is_some() {
            return self.modal_key(key);
        }
        if self.picker.is_some() {
            return self.picker_key(key);
        }
        if self.context.is_some() {
            return self.context_key(key);
        }
        if self.menu.is_some() {
            return self.menu_key(key);
        }
        if self.restore.is_some() {
            return self.restore_key(key, now);
        }
        if !self.armed
            && let Some(terminal) = self.scroll_mode
            && self.search.prompt.is_some()
        {
            // The query prompt takes every key, the prefix included.
            return self.search_prompt_key(terminal, key);
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
            return self.send_key(key, kind);
        }
        self.armed = false;
        if self.bindings.is_prefix(&key) {
            // The key goes to the terminal, which returns to the bottom.
            self.scroll_mode = None;
            self.bottom_expected = false;
            self.end_copy();
            return self.send_key(key, kind);
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
            Action::Restore => self.open_restore(now),
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
                    self.enter_scroll();
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
            Action::CloseTerminal => match self.mirror.focused_id() {
                Some(terminal) => self.confirm_close_terminal(terminal, now),
                None => {
                    self.set_flash("no terminal to close".into(), now, false);
                    Vec::new()
                }
            },
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
                    self.set_flash("no space to add a panel to".into(), now, false);
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
            Action::NewMenu => {
                if let Some(project) = active_project {
                    self.open_new_menu(project);
                }
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
            Action::CollapseSidebar => self.toggle_sidebar_collapsed(now),
            Action::FocusSidebar => {
                if self.sidebar_visible() {
                    self.enter_sidebar()
                } else {
                    Vec::new()
                }
            }
            Action::NextAttention => match self.mirror.view().and_then(agent::next_attention) {
                Some((panel, terminal)) => vec![
                    self.act(Act::FocusPanel { panel }, None),
                    self.act(Act::FocusTerminal { terminal }, None),
                ],
                None => {
                    self.set_flash("no agent needs you".into(), now, false);
                    Vec::new()
                }
            },
            Action::Help => {
                self.modal = Some(Modal::Help);
                self.help_scroll = 0;
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
        if let Some(restore) = &mut self.restore {
            restore.paste(&text);
            self.restore_settle(true, None);
            return Vec::new();
        }
        if self.menu.is_some() || self.context.is_some() {
            return Vec::new();
        }
        if self.scroll_mode.is_some() && self.search.prompt.is_some() {
            self.search_paste(&text);
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
    pub(crate) fn ready(&self) -> Option<TerminalId> {
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
            Queued::Key(key, kind) => encode_key(key, *kind, modes),
            Queued::Paste(text) => {
                let mut bytes = Vec::with_capacity(text.len() + 12);
                if modes.has(Modes::BRACKETED_PASTE) {
                    // Every ESC byte is left out, as the daemon does for `deck send --paste`, so
                    // pasted text can never end the paste early.
                    bytes.extend_from_slice(b"\x1b[200~");
                    bytes.extend(text.bytes().filter(|b| *b != 0x1b));
                    bytes.extend_from_slice(b"\x1b[201~");
                } else {
                    bytes.extend_from_slice(text.as_bytes());
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
            // Still scrolled back: whatever was in flight did not bring it to the bottom,
            // unless more of our scrolls are on their way to it.
            self.bottom_expected &= self.scrolls_in_flight(frame);
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
            self.end_copy();
        }
    }

    pub fn on_server(&mut self, msg: ServerMsg) -> Vec<Out> {
        let outs = self.handle_server(msg);
        // A prompt reopened by an answer ends the hover too.
        if self.overlay_open() {
            self.set_hover(None);
        }
        outs
    }

    fn handle_server(&mut self, msg: ServerMsg) -> Vec<Out> {
        self.dirty = true;
        match msg {
            ServerMsg::Welcome { config, .. } => {
                let mut outs = self.apply_config(*config);
                if self.focus_seen {
                    outs.push(Out::Send(ClientMsg::Focus(self.focused)));
                }
                if !self.configured {
                    self.configured = true;
                    outs.extend(self.start(Instant::now()));
                }
                outs
            }
            ServerMsg::Config(config) => self.apply_config(*config),
            ServerMsg::AgentAlert {
                kind,
                status,
                project,
                panel,
                panel_id,
            } => {
                let bytes = agent::alert_bytes(self.config.notify, kind, status, &project, &panel);
                // deck's own notification opens its panel when clicked: no jump on focus.
                // Without a notifier here, `system` is `terminal`, the jump included.
                let own = (self.config.notify == NotifyMode::System && self.own_notifications)
                    .then(|| agent::alert_body(kind, status, &project, &panel))
                    .flatten()
                    .zip(self.mirror.view().map(|v| v.name.clone()));
                if let Some((body, session)) = own {
                    vec![Out::SystemNotify(crate::notify::SystemAlert {
                        body,
                        session,
                        panel: panel_id,
                        fallback: bytes,
                    })]
                } else if bytes.is_empty() {
                    Vec::new()
                } else {
                    if !self.focused {
                        self.alert_jump = Some((panel_id, Instant::now()));
                    }
                    vec![Out::Notify(bytes)]
                }
            }
            ServerMsg::Image {
                terminal,
                screen,
                image,
                generation,
                width,
                height,
                format,
                offset,
                total,
                data,
            } => {
                if let Some(images) = &mut self.images {
                    images.program.piece(Piece {
                        slot: (terminal, screen, image),
                        generation,
                        width,
                        height,
                        format,
                        offset,
                        total,
                        data,
                    });
                    self.dirty = true;
                }
                Vec::new()
            }
            ServerMsg::ImageGone {
                terminal,
                screen,
                image,
                generation,
            } => {
                if let Some(images) = &mut self.images {
                    images.program.gone((terminal, screen, image), generation);
                    self.dirty = true;
                }
                Vec::new()
            }
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
                if let Some(images) = &mut self.images {
                    let mirror = &self.mirror;
                    images
                        .program
                        .retain_terminals(|id| mirror.find_terminal(id).is_some());
                }
                self.check_row_gesture();
                self.arm_spinner(Instant::now());
                self.check_scroll_target();
                self.check_modal_target(Instant::now());
                self.check_menu_target(Instant::now());
                self.check_context_target(Instant::now());
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
                self.refresh_hover();
                if self.panel_area_empty() {
                    self.queued.clear();
                }
                // The daemon sized the terminals for the config's sidebar; ours may differ
                // (a width dragged before a switch, a sidebar hidden with the prefix).
                let mut outs = Vec::new();
                if std::mem::take(&mut self.check_viewport) && !too_small(self.size) {
                    let width = if self.config.sidebar_collapsed {
                        layout::COLLAPSED_SIDEBAR_WIDTH
                    } else {
                        layout::clamp_sidebar_width(self.size.0, self.config.sidebar_width)
                    };
                    let shown = self.config.sidebar;
                    if self.viewport() != layout::panel_area(self.size, width, shown) {
                        outs.push(self.viewport_msg());
                    }
                }
                outs.extend(self.flush());
                outs
            }
            ServerMsg::Frame(frame) => {
                if let Some(sel) = &self.text_sel
                    && let Some(term) = self.mirror.terminal(sel.terminal)
                    && sel.stale(&frame, term)
                {
                    // The rest of a drag cut short is nobody's, not the program's.
                    if sel.dragging {
                        self.swallow_release = true;
                    }
                    self.text_sel = None;
                    self.edge = None;
                    self.dirty = true;
                }
                self.follow_scroll(&frame);
                self.copy_follows(&frame);
                self.mirror.apply_frame(frame);
                self.flush()
            }
            ServerMsg::Clipboard { selection, text } => vec![osc52(&selection, &text)],
            ServerMsg::Bye { reason } => vec![Out::Exit(match reason {
                ByeReason::SessionEnded => Exit::SessionEnded(self.session_name()),
                ByeReason::ServerShutdown => Exit::DaemonStopped,
                ByeReason::Error(text) => Exit::Error(text),
                ByeReason::VersionMismatch { daemon } => Exit::Mismatch(daemon),
                ByeReason::Upgrading { exe } => Exit::Upgrading {
                    exe,
                    session: self.mirror.session_name().map(str::to_string),
                },
            })],
        }
    }
}

/// The OSC 52 sequence that puts `text` on the outer terminal's clipboard `selection`.
pub(crate) fn osc52(selection: &str, text: &str) -> Out {
    let b64 = deck_core::base64::encode(text.as_bytes());
    Out::Clipboard(format!("\x1b]52;{selection};{b64}\x07").into_bytes())
}

/// A key as `held` remembers it: a release can report the character without the Shift the
/// press had.
fn held_code(code: KeyCode) -> KeyCode {
    match code {
        KeyCode::Char(c) => KeyCode::Char(c.to_lowercase().next().unwrap_or(c)),
        other => other,
    }
}

fn key_mods(mods: ct::KeyModifiers) -> Modifiers {
    Modifiers {
        ctrl: mods.contains(ct::KeyModifiers::CONTROL),
        alt: mods.contains(ct::KeyModifiers::ALT),
        shift: mods.contains(ct::KeyModifiers::SHIFT),
        ..Modifiers::NONE
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
        // A paste that carries the end marker cannot end the paste: its ESC bytes are dropped.
        assert_eq!(
            a.on_event(ct::Event::Paste("a\x1b[201~rm -rf\x1b".into()), now),
            [input(b"\x1b[200~a[201~rm -rf\x1b[201~")]
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
                rows: 30,
                cell: None,
            })]
        );
        assert!(a.on_event(ct::Event::Resize(19, 30), now).is_empty());
        assert!(a.on_event(ct::Event::Resize(80, 4), now).is_empty());
        assert!(too_small(a.size));
        assert!(!too_small((20, 5)));
        assert_eq!(
            a.on_event(ct::Event::Resize(20, 5), now),
            [Out::Send(ClientMsg::Viewport {
                cols: 20,
                rows: 5,
                cell: None
            })]
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
            a.on_server(bye(ByeReason::Upgrading {
                exe: "/opt/deck".into()
            })),
            [Out::Exit(Exit::Upgrading {
                exe: "/opt/deck".into(),
                session: Some("main".into())
            })]
        );
        // Before the first view there is no session to come back to.
        let mut early = App::new(&Config::default(), (80, 24), &[]);
        assert_eq!(
            early.on_server(bye(ByeReason::Upgrading {
                exe: "/opt/deck".into()
            })),
            [Out::Exit(Exit::Upgrading {
                exe: "/opt/deck".into(),
                session: None
            })]
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
        // The cursor sits on the top row of a screen whose history is 100 rows, with the ids
        // starting at 1000: the vi keys that scroll at once.
        let mut f = frame(true, (78, 21), vec![]);
        f.top_row = 1000;
        f.history = 100;
        f.display_offset = 50;
        f.cursor.row = 0;
        a.on_server(ServerMsg::Frame(f));
        let cases = [
            (pressed(ct::KeyCode::PageUp), ScrollTo::Pages(1)),
            (ctrl('b'), ScrollTo::Pages(1)),
            (pressed(ct::KeyCode::PageDown), ScrollTo::Pages(-1)),
            (ctrl('f'), ScrollTo::Pages(-1)),
            // The screen has 21 rows.
            (ctrl('u'), ScrollTo::Lines(10)),
            (ctrl('d'), ScrollTo::Lines(-10)),
            (ch('g'), ScrollTo::Row(1050 - 100)),
            (pressed(ct::KeyCode::Home), ScrollTo::Row(1050 - 100)),
            (
                key(ct::KeyCode::Char('G'), ct::KeyModifiers::SHIFT),
                ScrollTo::Row(1050 + 20),
            ),
            (pressed(ct::KeyCode::End), ScrollTo::Row(1050 + 20)),
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
            [scroll_msg(ScrollTo::Row(25))]
        );
        show(&mut a, 0, 0, 100);
        assert_eq!(a.scroll_mode, Some(TerminalId(4)));
        // Scrolling down half a screen from line 1 lands on the bottom too.
        show(&mut a, 0, 1, 100);
        a.on_event(ctrl('d'), now);
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
        // The panel area is the whole window: columns 0 to 79, rows 0 to 23; the terminal fills
        // 78x21 of it.
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

        // A terminal larger than the window (another client sized it): the rows and the columns
        // past the window send nothing.
        let mut f = frame(true, (100, 30), vec![]);
        f.modes = Modes(SGR_CLICK | Modes::MOUSE_MOTION);
        a.on_server(ServerMsg::Frame(f));
        for (column, row) in [(5, 24), (79, 24), (80, 3), (200, 23)] {
            assert!(
                a.on_event(mouse0(ct::MouseEventKind::Down(Left), column, row), now)
                    .is_empty(),
                "{column},{row}"
            );
        }
        assert_eq!(
            a.on_event(mouse0(ct::MouseEventKind::Down(Left), 79, 23), now),
            [input(b"\x1b[<0;80;24M")]
        );
    }

    fn focus(focused: bool) -> Out {
        Out::Send(ClientMsg::Focus(focused))
    }

    #[test]
    fn focus_events_reach_a_program_that_asked_for_them() {
        let now = Instant::now();
        let mut a = app();
        show(&mut a, 0, 0, 0);
        // The daemon is told either way; the program only when it asked.
        assert_eq!(a.on_event(ct::Event::FocusGained, now), [focus(true)]);
        assert_eq!(a.on_event(ct::Event::FocusLost, now), [focus(false)]);
        show(&mut a, Modes::FOCUS_REPORTING, 0, 0);
        assert_eq!(
            a.on_event(ct::Event::FocusGained, now),
            [focus(true), input(b"\x1b[I")]
        );
        assert_eq!(
            a.on_event(ct::Event::FocusLost, now),
            [focus(false), input(b"\x1b[O")]
        );
    }

    #[test]
    fn focus_events_are_forwarded_in_scroll_mode_too() {
        let mut a = in_scroll_mode();
        show(&mut a, Modes::FOCUS_REPORTING, 5, 100);
        let now = Instant::now();
        assert_eq!(
            a.on_event(ct::Event::FocusLost, now),
            [focus(false), input(b"\x1b[O")]
        );
        assert_eq!(a.scroll_mode, Some(TerminalId(4)));
    }

    #[test]
    fn the_focus_is_reported_again_on_every_attach_once_it_was_seen() {
        let welcome = || ServerMsg::Welcome {
            session: deck_core::model::SessionId(1),
            config: Box::new(no_sidebar()),
        };
        // Never reported: the daemon counts the client as focused.
        let mut a = app();
        assert!(a.on_server(welcome()).is_empty());
        let now = Instant::now();
        a.on_event(ct::Event::FocusLost, now);
        assert_eq!(a.on_server(welcome()), [focus(false)]);
        a.on_event(ct::Event::FocusGained, now);
        assert_eq!(a.on_server(welcome()), [focus(true)]);
    }

    fn alert(status: deck_core::agents::AgentStatus, project: &str, panel: &str) -> ServerMsg {
        ServerMsg::AgentAlert {
            kind: deck_core::agents::AgentKind::Claude,
            status,
            project: project.into(),
            panel: panel.into(),
            panel_id: deck_core::model::PanelId(1),
        }
    }

    #[test]
    fn an_alert_notifies_through_the_outer_terminal_as_configured() {
        use deck_core::agents::AgentStatus;
        let waiting = alert(AgentStatus::Waiting, "proj", "review");
        let config = Config::parse("notify = \"terminal\"").unwrap().config;
        let mut a = App::new(&config, (80, 24), &[]);
        assert_eq!(
            a.on_server(waiting.clone()),
            [Out::Notify(
                "\x1b]9;claude needs you · proj › review\x07"
                    .as_bytes()
                    .to_vec()
            )]
        );
        assert_eq!(
            a.on_server(alert(AgentStatus::Idle, "proj", "docs")),
            [Out::Notify(
                "\x1b]9;claude finished · proj › docs\x07"
                    .as_bytes()
                    .to_vec()
            )]
        );
        // Control characters never reach the outer terminal: an ESC or BEL in a name would
        // end the sequence early.
        assert_eq!(
            a.on_server(alert(AgentStatus::Waiting, "p\x07\x1b[2Jq", "a\nb\u{9b}c")),
            [Out::Notify(
                "\x1b]9;claude needs you · p[2Jq › abc\x07"
                    .as_bytes()
                    .to_vec()
            )]
        );
        for (text, bytes) in [("bell", Some(b"\x07".to_vec())), ("none", None)] {
            let config = Config::parse(&format!("notify = \"{text}\""))
                .unwrap()
                .config;
            let mut a = App::new(&config, (80, 24), &[]);
            let outs = a.on_server(waiting.clone());
            assert_eq!(outs, bytes.map(Out::Notify).into_iter().collect::<Vec<_>>());
        }
    }

    /// An app showing the mockup with terminal 41 (panel 4) waiting and terminal 91
    /// (panel 9) idle and unseen; returns the view too.
    fn jump_app(notify: &str) -> (App, deck_core::model::SessionView) {
        use deck_core::agents::{AgentKind, AgentSource, AgentState, AgentStatus};
        let mut v = crate::mirror::tests::mockup();
        for (id, status) in [(41, AgentStatus::Waiting), (91, AgentStatus::Idle)] {
            let t = v
                .projects
                .iter_mut()
                .flat_map(|p| &mut p.panels)
                .flat_map(|p| &mut p.terminals)
                .find(|t| t.id == TerminalId(id))
                .unwrap();
            t.agent = Some(AgentState {
                kind: AgentKind::Claude,
                status,
                source: AgentSource::Hooks,
                session_id: None,
                first_prompt: None,
                since: Duration::ZERO,
                activity: None,
            });
        }
        v.projects[3].panels[0].unseen = true;
        let config = Config::parse(&format!("notify = \"{notify}\""))
            .unwrap()
            .config;
        let mut a = App::new(&config, (100, 24), &[]);
        a.on_server(ServerMsg::SessionView(v.clone()));
        (a, v)
    }

    fn alert_for(panel: u64, status: deck_core::agents::AgentStatus) -> ServerMsg {
        let ServerMsg::AgentAlert {
            kind,
            status,
            project,
            panel: name,
            ..
        } = alert(status, "proj", "p")
        else {
            unreachable!()
        };
        ServerMsg::AgentAlert {
            kind,
            status,
            project,
            panel: name,
            panel_id: PanelId(panel),
        }
    }

    fn jump_to(project: u64, panel: u64, terminal: u64) -> Vec<Out> {
        vec![
            Out::Send(ClientMsg::Action(Act::FocusProject {
                project: deck_core::model::ProjectId(project),
            })),
            Out::Send(ClientMsg::Action(Act::FocusPanel {
                panel: PanelId(panel),
            })),
            Out::Send(ClientMsg::Action(Act::FocusTerminal {
                terminal: TerminalId(terminal),
            })),
        ]
    }

    fn unfocus_and_alert(a: &mut App, panel: u64, status: deck_core::agents::AgentStatus) {
        a.on_event(ct::Event::FocusLost, Instant::now());
        a.on_server(alert_for(panel, status));
    }

    #[test]
    fn a_system_alert_is_decks_own_notification_and_needs_no_jump() {
        use deck_core::agents::AgentStatus;
        let (mut a, v) = jump_app("system");
        a.own_notifications = true;
        a.on_event(ct::Event::FocusLost, Instant::now());
        let outs = a.on_server(alert_for(4, AgentStatus::Waiting));
        assert_eq!(
            outs,
            [Out::SystemNotify(crate::notify::SystemAlert {
                body: "claude needs you · proj › p".into(),
                session: v.name.clone(),
                panel: PanelId(4),
                fallback: "\x1b]9;claude needs you · proj › p\x07".as_bytes().to_vec(),
            })]
        );
        // A click opens the panel: coming back to the terminal does not jump.
        let outs = a.on_event(ct::Event::FocusGained, Instant::now());
        assert!(
            !outs
                .iter()
                .any(|o| matches!(o, Out::Send(ClientMsg::Action(_)))),
            "{outs:?}"
        );
    }

    #[test]
    fn system_without_a_notifier_is_the_terminals_notification_with_its_jump() {
        use deck_core::agents::AgentStatus;
        let (mut a, _) = jump_app("system");
        assert!(!a.own_notifications);
        a.on_event(ct::Event::FocusLost, Instant::now());
        assert_eq!(
            a.on_server(alert_for(4, AgentStatus::Waiting)),
            [Out::Notify(
                "\x1b]9;claude needs you · proj › p\x07".as_bytes().to_vec()
            )]
        );
        let mut outs = a.on_event(ct::Event::FocusGained, Instant::now());
        outs.retain(|o| matches!(o, Out::Send(ClientMsg::Action(_))));
        assert_eq!(outs, jump_to(1, 4, 41));
    }

    #[test]
    fn a_focus_gain_soon_after_an_alert_jumps_to_its_panel() {
        use deck_core::agents::AgentStatus;
        let (mut a, _) = jump_app("osc9");
        unfocus_and_alert(&mut a, 4, AgentStatus::Waiting);
        let mut outs = a.on_event(
            ct::Event::FocusGained,
            Instant::now() + Duration::from_secs(3),
        );
        assert_eq!(outs.remove(0), focus(true));
        assert_eq!(outs, jump_to(1, 4, 41));
        // Forgotten: the next focus gain does nothing.
        a.on_event(ct::Event::FocusLost, Instant::now());
        assert_eq!(
            a.on_event(ct::Event::FocusGained, Instant::now()),
            [focus(true)]
        );
    }

    #[test]
    fn an_unseen_idle_panel_is_jumped_to_too() {
        use deck_core::agents::AgentStatus;
        let (mut a, _) = jump_app("osc9");
        unfocus_and_alert(&mut a, 9, AgentStatus::Idle);
        let outs = a.on_event(ct::Event::FocusGained, Instant::now());
        assert_eq!(outs[1..], jump_to(8, 9, 91));
    }

    #[test]
    fn a_late_focus_gain_does_not_jump() {
        use deck_core::agents::AgentStatus;
        let (mut a, _) = jump_app("osc9");
        unfocus_and_alert(&mut a, 4, AgentStatus::Waiting);
        let outs = a.on_event(
            ct::Event::FocusGained,
            Instant::now() + Duration::from_secs(11),
        );
        assert_eq!(outs, [focus(true)]);
    }

    #[test]
    fn a_seen_or_closed_panel_does_not_jump() {
        use deck_core::agents::AgentStatus;
        let (mut a, mut v) = jump_app("osc9");
        unfocus_and_alert(&mut a, 9, AgentStatus::Idle);
        v.projects[3].panels[0].unseen = false;
        a.on_server(ServerMsg::SessionView(v.clone()));
        assert_eq!(
            a.on_event(ct::Event::FocusGained, Instant::now()),
            [focus(true)]
        );
        // Closed: the panel is gone from the view.
        a.on_event(ct::Event::FocusLost, Instant::now());
        a.on_server(alert_for(4, AgentStatus::Waiting));
        v.projects[0].panels.retain(|p| p.id != PanelId(4));
        a.on_server(ServerMsg::SessionView(v));
        assert_eq!(
            a.on_event(ct::Event::FocusGained, Instant::now()),
            [focus(true)]
        );
    }

    #[test]
    fn no_jump_with_notify_none_or_without_a_focus_loss() {
        use deck_core::agents::AgentStatus;
        let (mut a, _) = jump_app("none");
        unfocus_and_alert(&mut a, 4, AgentStatus::Waiting);
        assert_eq!(
            a.on_event(ct::Event::FocusGained, Instant::now()),
            [focus(true)]
        );
        // Never reported focus out: the client counts as focused and does not remember.
        let (mut a, _) = jump_app("osc9");
        a.on_server(alert_for(4, AgentStatus::Waiting));
        assert_eq!(
            a.on_event(ct::Event::FocusGained, Instant::now()),
            [focus(true)]
        );
    }

    #[test]
    fn the_focus_in_report_goes_to_the_terminal_the_jump_lands_on() {
        use deck_core::agents::AgentStatus;
        let (mut a, _) = jump_app("osc9");
        let mut f = frame(true, (78, 21), vec![]);
        f.terminal = TerminalId(41);
        f.modes = Modes(Modes::FOCUS_REPORTING);
        a.on_server(ServerMsg::Frame(f));
        unfocus_and_alert(&mut a, 4, AgentStatus::Waiting);
        let outs = a.on_event(ct::Event::FocusGained, Instant::now());
        assert_eq!(outs[0], focus(true));
        assert_eq!(
            outs[1],
            Out::Send(ClientMsg::Input {
                terminal: TerminalId(41),
                bytes: b"\x1b[I".to_vec()
            })
        );
        assert_eq!(outs[2..], jump_to(1, 4, 41));
    }

    #[test]
    fn an_open_overlay_forgets_the_alert_without_jumping() {
        use deck_core::agents::AgentStatus;
        let (mut a, _) = jump_app("osc9");
        unfocus_and_alert(&mut a, 4, AgentStatus::Waiting);
        a.modal = Some(Modal::Help);
        assert_eq!(
            a.on_event(ct::Event::FocusGained, Instant::now()),
            [focus(true)]
        );
        a.modal = None;
        a.on_event(ct::Event::FocusLost, Instant::now());
        assert_eq!(
            a.on_event(ct::Event::FocusGained, Instant::now()),
            [focus(true)]
        );
    }

    #[test]
    fn the_newer_alert_wins() {
        use deck_core::agents::AgentStatus;
        let (mut a, _) = jump_app("osc9");
        unfocus_and_alert(&mut a, 9, AgentStatus::Idle);
        a.on_server(alert_for(4, AgentStatus::Waiting));
        let outs = a.on_event(ct::Event::FocusGained, Instant::now());
        assert_eq!(outs[1..], jump_to(1, 4, 41));
    }

    #[test]
    fn next_attention_cycles_waiting_then_unseen_idle_then_flashes() {
        use deck_core::agents::{AgentKind, AgentSource, AgentState, AgentStatus};
        use deck_core::model::PanelId;
        let set = |view: &mut deck_core::model::SessionView, terminal: u64, status| {
            let t = view
                .projects
                .iter_mut()
                .flat_map(|p| &mut p.panels)
                .flat_map(|p| &mut p.terminals)
                .find(|t| t.id == TerminalId(terminal))
                .unwrap();
            t.agent = Some(AgentState {
                kind: AgentKind::Claude,
                status,
                source: AgentSource::Hooks,
                session_id: None,
                first_prompt: None,
                since: Duration::ZERO,
                activity: None,
            });
        };
        let mut v = crate::mirror::tests::mockup();
        set(&mut v, 41, AgentStatus::Waiting);
        set(&mut v, 61, AgentStatus::Waiting);
        set(&mut v, 91, AgentStatus::Idle);
        v.projects[3].panels[0].unseen = true;
        let mut a = App::new(&Config::default(), (100, 24), &[]);
        a.on_server(ServerMsg::SessionView(v.clone()));
        let now = Instant::now();
        let jump = |a: &mut App| {
            a.on_event(prefix(), now);
            a.on_event(ch('a'), now)
        };
        let focus_to = |panel: u64, terminal: u64| {
            vec![
                Out::Send(ClientMsg::Action(Act::FocusPanel {
                    panel: PanelId(panel),
                })),
                Out::Send(ClientMsg::Action(Act::FocusTerminal {
                    terminal: TerminalId(terminal),
                })),
            ]
        };
        // From the editor panel (3): the waiting ones in sidebar order, wrapping.
        assert_eq!(jump(&mut a), focus_to(4, 41));
        v.active_panel = Some(PanelId(4));
        v.projects[0].active_panel = Some(PanelId(4));
        a.on_server(ServerMsg::SessionView(v.clone()));
        assert_eq!(jump(&mut a), focus_to(6, 61));
        v.active_panel = Some(PanelId(6));
        v.active_project = Some(deck_core::model::ProjectId(2));
        v.projects[1].active_panel = Some(PanelId(6));
        a.on_server(ServerMsg::SessionView(v.clone()));
        assert_eq!(jump(&mut a), focus_to(4, 41));
        // Nobody waits: the unseen idle one, in a collapsed project too.
        set(&mut v, 41, AgentStatus::Working);
        set(&mut v, 61, AgentStatus::Working);
        a.on_server(ServerMsg::SessionView(v.clone()));
        assert_eq!(jump(&mut a), focus_to(9, 91));
        // Seen, it no longer counts.
        v.projects[3].panels[0].unseen = false;
        a.on_server(ServerMsg::SessionView(v));
        assert!(jump(&mut a).is_empty());
        assert_eq!(a.flash.as_ref().unwrap().0, "no agent needs you");
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
