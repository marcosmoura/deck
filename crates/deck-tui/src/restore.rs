//! The Restore dialog: three columns of recent agent sessions (Claude Code, OpenCode, Codex),
//! a filter across all of them, and the resume of the chosen one in a new panel. It opens
//! from the sidebar's `Restore` chip, `prefix R` and `R` in sidebar focus.

use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use deck_core::action::{Action as Act, PanelLaunch};
use deck_core::agent_sessions::{AgentSession, age, matches, project_for, short_dir};
use deck_core::agents::AgentKind;
use deck_core::keys::{KeyCode, KeyEvent};
use deck_core::model::ProjectId;
use deck_core::protocol::{ClientMsg, Request};
use deck_core::theme::Theme;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event as ct;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Clear, Widget};

use crate::app::{App, Out};
use crate::draw::{border_type, color, selection, truncate, width};
use crate::modal::Editor;

/// The columns, left to right.
pub const COLUMNS: [AgentKind; 3] = [AgentKind::Claude, AgentKind::OpenCode, AgentKind::Codex];
/// Sessions asked for per agent.
const LIMIT: u16 = 50;
/// Under this many window columns the dialog shows one column at a time.
const NARROW: u16 = 60;
/// The dialog is at least this wide, window permitting.
const MIN_WIDTH: u16 = 60;
/// The dialog is at least this high, window permitting.
const MIN_HEIGHT: u16 = 10;
/// Sessions the wheel scrolls per notch.
const WHEEL_SESSIONS: usize = 2;
const HINTS: &str = "⏎ resume  ←→ column  ↑↓ row  type to filter  esc close";
const HINTS_NARROW: &str = "⏎ resume  tab column  esc close";
/// The flash when there is no project to open the session in.
const NO_PROJECT: &str = "no project to restore into";

/// What a column shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Load {
    Loading,
    Error(String),
    Ready(Vec<AgentSession>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub load: Load,
    /// Index into the filtered list.
    pub selected: usize,
    /// The first session shown, in the filtered list.
    pub offset: usize,
}

impl Column {
    fn new() -> Self {
        Self {
            load: Load::Loading,
            selected: 0,
            offset: 0,
        }
    }
}

/// The open dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restore {
    pub filter: Editor,
    pub cols: [Column; 3],
    /// The column that has the selection.
    pub active: usize,
    /// The user moved the selection: it no longer follows the first non-empty column.
    moved: bool,
    /// The hovered row: a column and an index into its filtered list.
    pub hover: Option<(usize, usize)>,
    /// The time ages are counted from, epoch seconds.
    pub now: u64,
}

impl Restore {
    fn new() -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        Self {
            filter: Editor::default(),
            cols: [Column::new(), Column::new(), Column::new()],
            active: 0,
            moved: false,
            hover: None,
            now,
        }
    }

    /// The sessions of a column the filter lets through.
    pub fn visible(&self, col: usize) -> Vec<&AgentSession> {
        let query = self.filter.text();
        match self.cols.get(col).map(|c| &c.load) {
            Some(Load::Ready(list)) => list.iter().filter(|s| matches(s, &query)).collect(),
            _ => Vec::new(),
        }
    }

    fn len(&self, col: usize) -> usize {
        self.visible(col).len()
    }

    pub fn selected_session(&self) -> Option<&AgentSession> {
        let col = self.cols.get(self.active)?;
        self.visible(self.active).get(col.selected).copied()
    }

    pub(crate) fn paste(&mut self, text: &str) {
        self.filter.insert_str(text);
    }

    /// Puts the selection on a row that exists: the first non-empty column's first row until
    /// the user moves it, the nearest column with rows after that.
    fn settle(&mut self) {
        let lens = [self.len(0), self.len(1), self.len(2)];
        if !self.moved {
            if let Some(i) = lens.iter().position(|&l| l > 0) {
                self.active = i;
            }
            if let Some(col) = self.cols.get_mut(self.active) {
                col.selected = 0;
            }
        }
        for (col, len) in self.cols.iter_mut().zip(lens) {
            col.selected = col.selected.min(len.saturating_sub(1));
            col.offset = col.offset.min(len.saturating_sub(1));
        }
    }

    /// After the filter changed: leaves an active column that lost its rows for the nearest
    /// one that has some. Moving to an empty column on purpose (`Tab`) is not undone.
    fn relocate(&mut self) {
        let lens = [self.len(0), self.len(1), self.len(2)];
        if self.moved && lens.get(self.active).copied().unwrap_or(0) == 0 {
            let near = (0..3usize)
                .filter(|&i| lens.get(i).copied().unwrap_or(0) > 0)
                .min_by_key(|&i| (i.abs_diff(self.active), i));
            if let Some(i) = near {
                self.active = i;
            }
        }
        self.settle();
    }

    /// Moves to the next column in `delta`'s direction; with `skip_empty`, columns without
    /// rows are skipped while any column has rows. `wrap` goes around the ends.
    fn step(&mut self, delta: isize, wrap: bool, skip_empty: bool) {
        self.moved = true;
        let lens = [self.len(0), self.len(1), self.len(2)];
        let any = lens.iter().any(|&l| l > 0);
        let mut at = self.active;
        for _ in 0..3 {
            let next = at.checked_add_signed(delta).filter(|&n| n < 3);
            at = match (next, wrap) {
                (Some(n), _) => n,
                (None, true) if delta < 0 => 2,
                (None, true) => 0,
                (None, false) => return,
            };
            if !any || !skip_empty || lens.get(at).copied().unwrap_or(0) > 0 {
                self.active = at;
                self.settle();
                return;
            }
        }
    }

    fn select(&mut self, col: usize, index: usize) {
        self.moved = true;
        self.active = col;
        let len = self.len(col);
        if let Some(c) = self.cols.get_mut(col) {
            c.selected = index.min(len.saturating_sub(1));
        }
    }

    /// Moves the selection in the active column by `delta` rows.
    fn move_row(&mut self, delta: isize) {
        self.moved = true;
        let len = self.len(self.active);
        if let Some(c) = self.cols.get_mut(self.active) {
            c.selected = c
                .selected
                .saturating_add_signed(delta)
                .min(len.saturating_sub(1));
        }
    }

    fn row_to(&mut self, last: bool) {
        self.moved = true;
        let len = self.len(self.active);
        if let Some(c) = self.cols.get_mut(self.active) {
            c.selected = if last { len.saturating_sub(1) } else { 0 };
        }
    }

    /// Scrolls the active column the least that shows its selection, `per_page` sessions high.
    fn fit(&mut self, col: usize, per_page: usize) {
        let len = self.len(col);
        let per_page = per_page.max(1);
        if let Some(c) = self.cols.get_mut(col) {
            if c.selected < c.offset {
                c.offset = c.selected;
            } else if c.selected >= c.offset + per_page {
                c.offset = c.selected + 1 - per_page;
            }
            c.offset = c.offset.min(len.saturating_sub(per_page));
        }
    }
}

/// Where the dialog's parts are, in window cells.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Geometry {
    pub(crate) rect: Rect,
    pub(crate) narrow: bool,
    /// The filter row, the header row and the first list row.
    pub(crate) filter_y: u16,
    pub(crate) header_y: u16,
    pub(crate) list_y: u16,
    pub(crate) list_rows: u16,
    pub(crate) hint_y: u16,
    /// Left edge and width of what the columns share.
    pub(crate) x: u16,
    pub(crate) w: u16,
}

impl Geometry {
    /// Sessions the list shows at once.
    fn per_page(&self) -> usize {
        usize::from(self.list_rows / 2).max(1)
    }

    /// The cells of column `i`, with its separator before it: `(x, width)`.
    pub(crate) fn column(&self, i: usize) -> (u16, u16) {
        if self.narrow {
            return (self.x, self.w);
        }
        let each = self.w.saturating_sub(2) / 3;
        let i16 = u16::try_from(i).unwrap_or(2);
        let x = self.x + i16 * (each + 1);
        let w = if i >= 2 {
            self.w.saturating_sub(2 * (each + 1))
        } else {
            each
        };
        (x, w)
    }

    /// Where the text of column `i` goes: inside its padding.
    fn content(&self, i: usize) -> (u16, u16) {
        let (x, w) = self.column(i);
        if self.narrow {
            return (x, w);
        }
        let lead = u16::from(i > 0);
        let trail = u16::from(i < 2);
        (x + lead, w.saturating_sub(lead + trail))
    }

    /// The columns on screen: all three, or the active one alone.
    fn shown(&self, active: usize) -> Vec<usize> {
        if self.narrow {
            vec![active]
        } else {
            vec![0, 1, 2]
        }
    }
}

/// What a cell of the window is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Hit {
    Outside,
    Inside,
    Header(usize),
    Row(usize, usize),
    /// A column's list area, without a row under the cell.
    List(usize),
}

impl App {
    /// The dialog's geometry for the window, when it is open.
    pub(crate) fn restore_geometry(&self) -> Option<Geometry> {
        self.restore.as_ref()?;
        let (cols, rows) = (self.size.0, self.size.1.saturating_sub(1));
        let narrow = cols < NARROW;
        let w = if narrow {
            cols * 9 / 10
        } else {
            (cols * 9 / 10).max(MIN_WIDTH).min(cols)
        }
        .max(cols.min(6));
        let h = (rows * 8 / 10).max(MIN_HEIGHT).min(rows);
        let rect = Rect::new((cols - w) / 2, (rows - h) / 2, w, h);
        let inner_h = h.saturating_sub(2);
        let chrome = if narrow { 3 } else { 6 };
        let filter_y = rect.y + 1;
        let header_y = filter_y + if narrow { 1 } else { 2 };
        let list_y = header_y + if narrow { 1 } else { 2 };
        Some(Geometry {
            rect,
            narrow,
            filter_y,
            header_y,
            list_y,
            list_rows: inner_h.saturating_sub(chrome),
            hint_y: rect.y + h.saturating_sub(2),
            x: rect.x + 2,
            w: w.saturating_sub(4),
        })
    }

    pub(crate) fn restore_hit(&self, column: u16, row: u16) -> Hit {
        let (Some(g), Some(restore)) = (self.restore_geometry(), self.restore.as_ref()) else {
            return Hit::Outside;
        };
        let r = g.rect;
        if !((r.x..r.x + r.width).contains(&column) && (r.y..r.y + r.height).contains(&row)) {
            return Hit::Outside;
        }
        for i in g.shown(restore.active) {
            let (x, w) = g.column(i);
            if !(x..x + w).contains(&column) {
                continue;
            }
            if row == g.header_y {
                return Hit::Header(i);
            }
            if row >= g.list_y && row < g.list_y + g.list_rows {
                let line = usize::from(row - g.list_y) / 2;
                if line >= g.per_page() {
                    return Hit::List(i);
                }
                let offset = restore.cols.get(i).map_or(0, |c| c.offset);
                let index = offset + line;
                return if index < restore.len(i) {
                    Hit::Row(i, index)
                } else {
                    Hit::List(i)
                };
            }
        }
        Hit::Inside
    }

    /// Opens the dialog and asks each agent for its sessions; the answers are routed by kind.
    pub(crate) fn open_restore(&mut self, now: Instant) -> Vec<Out> {
        let has_project = self.mirror.view().is_some_and(|v| !v.projects.is_empty());
        if !has_project {
            self.set_flash(NO_PROJECT.into(), now, false);
            return Vec::new();
        }
        self.restore = Some(Restore::new());
        self.dirty = true;
        COLUMNS
            .into_iter()
            .map(|kind| {
                Out::Send(ClientMsg::Request(Request::ListAgentSessions {
                    kind,
                    limit: LIMIT,
                }))
            })
            .collect()
    }

    fn close_restore(&mut self) {
        if self.restore.take().is_some() {
            self.dirty = true;
            self.refresh_hover();
        }
    }

    /// Scrolls the columns so every selection is in sight, after a change of rows or size.
    pub(crate) fn restore_fit(&mut self) {
        self.restore_fit_cols(&[0, 1, 2]);
    }

    /// Scrolls only `cols` to their selections; the others keep where the wheel left them.
    fn restore_fit_cols(&mut self, cols: &[usize]) {
        let Some(per_page) = self.restore_geometry().map(|g| g.per_page()) else {
            return;
        };
        if let Some(restore) = &mut self.restore {
            for &col in cols {
                restore.fit(col, per_page);
            }
        }
    }

    /// Settles the selection after the lists changed or a key moved it, fitting the active
    /// column and `also`. A change of the filter (`filtered`) fits every column.
    pub(crate) fn restore_settle(&mut self, filtered: bool, also: Option<usize>) {
        if let Some(restore) = &mut self.restore {
            restore.settle();
            if filtered {
                restore.relocate();
            }
        }
        if filtered {
            self.restore_fit();
        } else {
            let active = self.restore.as_ref().map_or(0, |r| r.active);
            let mut cols = vec![active];
            cols.extend(also);
            self.restore_fit_cols(&cols);
        }
        self.dirty = true;
    }

    /// An agent's list arrived; one that arrives after the dialog closed is dropped.
    pub(crate) fn restore_response(
        &mut self,
        kind: AgentKind,
        sessions: Vec<AgentSession>,
        error: Option<String>,
    ) {
        let Some(restore) = &mut self.restore else {
            return;
        };
        let Some(i) = COLUMNS.iter().position(|k| *k == kind) else {
            return;
        };
        if let Some(col) = restore.cols.get_mut(i) {
            col.load = match error {
                Some(text) if sessions.is_empty() => Load::Error(text),
                _ => Load::Ready(sessions),
            };
        }
        self.restore_settle(false, Some(i));
    }

    /// A key while the dialog is open; it never goes anywhere else.
    pub(crate) fn restore_key(&mut self, key: KeyEvent, now: Instant) -> Vec<Out> {
        let Some(per_page) = self.restore_geometry().map(|g| g.per_page()) else {
            return Vec::new();
        };
        let Some(restore) = self.restore.as_mut() else {
            return Vec::new();
        };
        let (ctrl, alt, shift) = (key.mods.ctrl, key.mods.alt, key.mods.shift);
        if alt {
            return Vec::new();
        }
        let page = isize::try_from(per_page).unwrap_or(1);
        let mut filtered = false;
        match key.code {
            KeyCode::Esc if !ctrl => {
                if restore.filter.text().is_empty() {
                    self.close_restore();
                    return Vec::new();
                }
                restore.filter = Editor::default();
                filtered = true;
            }
            KeyCode::Enter => return self.restore_resume(now),
            KeyCode::Left => restore.step(-1, false, true),
            KeyCode::Right => restore.step(1, false, true),
            KeyCode::Tab => restore.step(if shift { -1 } else { 1 }, true, false),
            KeyCode::Up => restore.move_row(-1),
            KeyCode::Down => restore.move_row(1),
            KeyCode::Char('p') if ctrl => restore.move_row(-1),
            KeyCode::Char('n') if ctrl => restore.move_row(1),
            KeyCode::PageUp => restore.move_row(-page),
            KeyCode::PageDown => restore.move_row(page),
            KeyCode::Home => restore.row_to(false),
            KeyCode::End => restore.row_to(true),
            _ => {
                restore.filter.on_key(&key);
                filtered = true;
            }
        }
        self.restore_settle(filtered, None);
        Vec::new()
    }

    /// Resumes the selected session in a new panel of the project that holds its directory.
    fn restore_resume(&mut self, now: Instant) -> Vec<Out> {
        let Some(session) = self
            .restore
            .as_ref()
            .and_then(|r| r.selected_session().cloned())
        else {
            return Vec::new();
        };
        self.close_restore();
        let Some(view) = self.mirror.view() else {
            return Vec::new();
        };
        let roots: Vec<(ProjectId, PathBuf)> = view
            .projects
            .iter()
            .map(|p| (p.id, p.root.clone()))
            .collect();
        let Some(project) = project_for(&session.cwd, &roots, view.active_project) else {
            self.set_flash(NO_PROJECT.into(), now, false);
            return Vec::new();
        };
        let folded = self
            .mirror
            .find_project(project)
            .is_some_and(|p| p.collapsed);
        let action = Act::NewPanelWith {
            project,
            launch: PanelLaunch::Resume {
                kind: session.kind,
                session_id: session.id,
                cwd: session.cwd,
            },
        };
        let mut outs = vec![self.act(action, None)];
        // The keyboard leaves the sidebar once the daemon accepted the launch.
        self.launch_result = Some(self.pending.len().saturating_sub(1));
        // The new panel must be seen: a folded project unfolds.
        if folded {
            outs.push(self.act(
                Act::SetProjectCollapsed {
                    project,
                    collapsed: false,
                },
                None,
            ));
        }
        outs
    }

    /// A mouse event while the dialog is open: it has the mouse before everything else.
    pub(crate) fn restore_mouse(&mut self, event: ct::MouseEvent) -> Vec<Out> {
        let hit = self.restore_hit(event.column, event.row);
        let Some(per_page) = self.restore_geometry().map(|g| g.per_page()) else {
            return Vec::new();
        };
        let now = Instant::now();
        match event.kind {
            ct::MouseEventKind::Moved => {
                let hover = match hit {
                    Hit::Row(c, i) => Some((c, i)),
                    _ => None,
                };
                if let Some(restore) = &mut self.restore
                    && restore.hover != hover
                {
                    restore.hover = hover;
                    self.dirty = true;
                }
            }
            ct::MouseEventKind::Down(button) => {
                // The rest of the click (its release) goes to nobody.
                self.swallow_release = true;
                if button != ct::MouseButton::Left {
                    if hit == Hit::Outside {
                        self.close_restore();
                    }
                    return Vec::new();
                }
                match hit {
                    Hit::Outside => {
                        self.close_restore();
                        self.hover = None;
                    }
                    Hit::Header(c) => {
                        if let Some(restore) = &mut self.restore {
                            restore.moved = true;
                            restore.active = c;
                            restore.settle();
                        }
                        self.restore_fit_cols(&[c]);
                        self.dirty = true;
                    }
                    Hit::Row(c, i) => {
                        let on_selected = self.restore.as_ref().is_some_and(|r| {
                            r.active == c && r.cols.get(c).is_some_and(|col| col.selected == i)
                        });
                        if on_selected {
                            return self.restore_resume(now);
                        }
                        if let Some(restore) = &mut self.restore {
                            restore.select(c, i);
                        }
                        self.restore_fit_cols(&[c]);
                        self.dirty = true;
                    }
                    Hit::Inside | Hit::List(_) => {}
                }
            }
            ct::MouseEventKind::ScrollUp | ct::MouseEventKind::ScrollDown => {
                let col = match hit {
                    Hit::Row(c, _) | Hit::List(c) | Hit::Header(c) => Some(c),
                    _ => None,
                };
                let up = event.kind == ct::MouseEventKind::ScrollUp;
                if let (Some(c), Some(restore)) = (col, self.restore.as_mut()) {
                    let len = restore.len(c);
                    if let Some(column) = restore.cols.get_mut(c) {
                        let max = len.saturating_sub(per_page);
                        column.offset = if up {
                            column.offset.saturating_sub(WHEEL_SESSIONS)
                        } else {
                            (column.offset + WHEEL_SESSIONS).min(max)
                        };
                    }
                    // The row under the pointer changed.
                    restore.hover = None;
                    self.dirty = true;
                }
            }
            _ => {}
        }
        Vec::new()
    }
}

/// The hints for the status bar while the dialog is open.
pub fn hints(app: &App) -> &'static str {
    if app.size.0 < NARROW {
        HINTS_NARROW
    } else {
        HINTS
    }
}

/// The agent's name in the column header.
fn label(kind: AgentKind) -> &'static str {
    match kind {
        AgentKind::Claude => "Claude Code",
        AgentKind::OpenCode => "OpenCode",
        AgentKind::Codex => "Codex",
    }
}

/// `text` cut to `max` columns from its left, with `…` in place of what was cut.
fn truncate_left(text: &str, max: u16) -> String {
    if width(text) <= max {
        return text.to_string();
    }
    let mut tail: Vec<char> = Vec::new();
    for c in text.chars().rev() {
        let candidate: String = std::iter::once(c).chain(tail.iter().copied()).collect();
        if width(&candidate) + 1 > max {
            break;
        }
        tail.insert(0, c);
    }
    if max == 0 {
        return String::new();
    }
    format!("…{}", tail.into_iter().collect::<String>())
}

/// The second line of a session: `<directory> · <age> ago`, the directory cut from the left.
fn detail(session: &AgentSession, home: Option<&Path>, now: u64, room: u16) -> String {
    let age = age(now, session.updated);
    let suffix = if age == "now" {
        " · now".to_string()
    } else {
        format!(" · {age} ago")
    };
    let dir = short_dir(&session.cwd, home.unwrap_or(Path::new("")));
    let dir = truncate_left(&dir, room.saturating_sub(width(&suffix) + 2));
    format!("  {dir}{suffix}")
}

/// Draws the dialog over everything else but the status bar.
pub fn draw(buf: &mut Buffer, app: &App, theme: &Theme) {
    let (Some(restore), Some(g)) = (&app.restore, app.restore_geometry()) else {
        return;
    };
    let rect = g.rect;
    Clear.render(rect, buf);
    let base = Style::new().bg(color(theme.overlay_bg));
    buf.set_style(rect, base);
    Block::bordered()
        .border_type(border_type(theme.border_style))
        .border_style(base.fg(color(theme.accent)))
        .title("─ Restore agent session ")
        .render(rect, buf);
    let muted = base.fg(color(theme.muted));
    let accent = base.fg(color(theme.accent));
    // Everything is clipped to the inside of the border, so a short window never draws over it.
    let (top, bottom) = (rect.y + 1, rect.y + rect.height.saturating_sub(1));
    let right = rect.x + rect.width.saturating_sub(1);
    let put = |buf: &mut Buffer, x: u16, y: u16, text: &str, max: u16, style: Style| {
        if y < top || y >= bottom || x >= right {
            return;
        }
        buf.set_stringn(x, y, text, usize::from(max.min(right - x)), style);
    };

    let (before, after) = restore.filter.split();
    put(
        buf,
        g.x,
        g.filter_y,
        &format!("> {before}█{after}"),
        g.w,
        base,
    );

    let hint = if g.narrow { HINTS_NARROW } else { HINTS };
    if g.hint_y >= g.list_y {
        put(buf, g.x, g.hint_y, hint, g.w, muted);
    }
    let border = base.fg(color(theme.border));
    let home = app.home.as_deref();
    for i in g.shown(restore.active) {
        let Some(col) = restore.cols.get(i) else {
            continue;
        };
        let (x, _) = g.column(i);
        let (cx, cw) = g.content(i);
        let visible = restore.visible(i);
        // The header: the name and, once loaded, the count.
        let name = label(COLUMNS.get(i).copied().unwrap_or(AgentKind::Claude));
        let active = i == restore.active;
        let count = match col.load {
            Load::Ready(_) => format!(" ({})", visible.len()),
            _ => String::new(),
        };
        let header = if g.narrow {
            format!("‹ {name}{count} ›")
        } else {
            format!("{name}{count}")
        };
        let style = if active || g.narrow { accent } else { base };
        put(
            buf,
            cx,
            g.header_y,
            &header,
            cw,
            style.add_modifier(Modifier::BOLD),
        );
        if !g.narrow {
            put(
                buf,
                cx,
                g.header_y + 1,
                &"─".repeat(usize::from(cw)),
                cw,
                muted,
            );
            if i > 0 {
                for y in g.header_y..g.list_y + g.list_rows {
                    put(buf, x.saturating_sub(1), y, "│", 1, border);
                }
            }
        }
        let list_end = g.list_y + g.list_rows;
        match &col.load {
            _ if g.list_rows == 0 => {}
            Load::Loading => put(buf, cx, g.list_y, "loading…", cw, muted),
            Load::Error(text) => {
                // The text wraps over the list rows.
                let mut line = String::new();
                let mut y = g.list_y;
                for c in text.chars().filter(|c| !c.is_control()) {
                    if width(&format!("{line}{c}")) > cw && !line.is_empty() {
                        if y < list_end {
                            put(buf, cx, y, &line, cw, base.fg(color(theme.warning)));
                        }
                        y += 1;
                        line.clear();
                    }
                    line.push(c);
                }
                if !line.is_empty() && y < list_end {
                    put(buf, cx, y, &line, cw, base.fg(color(theme.warning)));
                }
            }
            Load::Ready(_) if visible.is_empty() => {
                put(buf, cx, g.list_y, "no sessions", cw, muted);
            }
            Load::Ready(_) => {
                for (line, session) in visible
                    .iter()
                    .enumerate()
                    .skip(col.offset)
                    .take(g.per_page())
                {
                    let y = g.list_y + u16::try_from((line - col.offset) * 2).unwrap_or(0);
                    if y >= list_end {
                        break;
                    }
                    let selected = active && line == col.selected;
                    let hovered = restore.hover == Some((i, line));
                    let (title_style, detail_style) = if selected {
                        (selection(theme, base), selection(theme, base))
                    } else if hovered {
                        let s = muted.add_modifier(Modifier::REVERSED);
                        (s, s)
                    } else {
                        (base, muted)
                    };
                    let rows = if y + 1 < list_end { 2 } else { 1 };
                    buf.set_style(Rect::new(cx, y, cw, rows), title_style);
                    put(buf, cx, y, &truncate(&session.title, cw), cw, title_style);
                    if rows == 2 {
                        let text = detail(session, home, restore.now, cw);
                        put(buf, cx, y + 1, &text, cw, detail_style);
                    }
                }
            }
        }
    }
}
