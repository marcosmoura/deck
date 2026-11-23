//! The sidebar: its rows, the keyboard focus and mouse handling that belong to it, and its
//! drawing.

use deck_core::action::Action as Act;
use deck_core::icons::{Icon, IconKind, icon_for};
use deck_core::keys::{KeyCode, KeyEvent};
use deck_core::model::{PanelId, PanelView, ProjectId, SessionView};
use deck_core::naming::source_terminal_view;
use deck_core::theme::Theme;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event as ct;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::agent;
use crate::app::{App, Out, too_small};
use crate::convert::{claude_color, image_id_color};
use crate::draw::{color, selection, truncate, width};
use crate::graphics::{Pair, has_image, placeholder};
use crate::tier::Tier;

/// The session name and a blank row above the project rows.
pub const HEADER_ROWS: u16 = 2;
/// Rows the bottom bar and the blank row above it take.
const BAR_ROWS: u16 = 2;
/// A sidebar shorter than this has no bottom bar.
pub(crate) const BAR_MIN_HEIGHT: u16 = 8;
/// Under this width the bar's chips shorten to their icon.
const BAR_WIDE: u16 = 26;
/// Rows the wheel scrolls per notch.
const WHEEL_ROWS: usize = 3;
/// What the floating bar shows while the sidebar has the keyboard, as key and label pairs.
pub const SIDEBAR_HINTS: &[(&str, &str)] = &[
    ("↑↓", "move"),
    ("⏎", "open"),
    ("←→", "fold"),
    ("c", "new"),
    ("n", "menu"),
    ("r", "rename"),
    ("x", "close"),
    ("esc", "back"),
];

/// A row being dragged to another place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RowDrag {
    pub item: Sel,
    /// The index the item ends at if dropped now.
    pub to: usize,
}

/// The row the sidebar selection is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sel {
    Project(ProjectId),
    Panel(ProjectId, PanelId),
}

/// What the pointer is over in the sidebar. Client-local: never sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hover {
    /// A space or panel row of the list.
    Row(Sel),
    /// The `+ New space` chip of the header.
    NewProject,
    /// The `+ New` chip of the bottom bar.
    New,
    Restore,
}

/// Cells an icon takes: a text icon and a blank, or a logo.
pub const ICON_CELLS: u16 = 2;
/// The column of a panel row's icon, and the one its name starts after it. The status slot is
/// on the right.
const ICON_X: u16 = 1;
const NAME_X: u16 = ICON_X + ICON_CELLS + 1;

/// Rows the bottom bar and its spacer take in a sidebar `height` rows tall.
fn bar_rows(height: u16) -> u16 {
    if height >= BAR_MIN_HEIGHT {
        BAR_ROWS
    } else {
        0
    }
}

/// The label of the `New space` chip of the header.
const NEW_PROJECT_LABEL: &str = "New space";
/// The label of the `New` chip of the bottom bar.
const NEW_LABEL: &str = "New";
/// The label of the `Resume` chip of the bottom bar.
const RESUME_LABEL: &str = "Resume";

/// The columns a chip takes in a sidebar `w` columns wide: a space, the icon's slot and a
/// space; in a wide sidebar a space and the label come before the last space.
fn chip_cells(w: u16, label: &str) -> u16 {
    let text = if w < BAR_WIDE { 0 } else { 1 + width(label) };
    1 + ICON_CELLS + text + 1
}

/// Draws a chip's icon and label at column `x` of row `y`; the caller styled its cells.
fn draw_chip(
    app: &App,
    buf: &mut Buffer,
    (x, y): (u16, u16),
    w: u16,
    icon: Icon,
    label: &str,
    style: Style,
) {
    let (slot, slot_style) = icon_slot(app, icon, style);
    buf.set_stringn(x + 1, y, &slot, usize::from(ICON_CELLS), slot_style);
    if w >= BAR_WIDE {
        buf.set_stringn(
            x + 1 + ICON_CELLS,
            y,
            format!(" {label}"),
            usize::from(1 + width(label)),
            style,
        );
    }
}

/// The columns the `New space` chip takes in a sidebar `w` columns wide: right-aligned on
/// the header row, with a margin of one column.
fn new_project_span(w: u16) -> std::ops::Range<u16> {
    let cells = chip_cells(w, NEW_PROJECT_LABEL);
    let x = w.saturating_sub(1 + cells);
    x..x + cells
}

/// The columns the `New` chip takes: from the second column.
fn new_span(w: u16) -> std::ops::Range<u16> {
    1..1 + chip_cells(w, NEW_LABEL)
}

/// The columns the `Resume` chip takes in a sidebar `w` columns wide: right-aligned, with a
/// margin of one column.
fn restore_span(w: u16) -> std::ops::Range<u16> {
    let cells = chip_cells(w, RESUME_LABEL);
    let x = w.saturating_sub(1 + cells);
    x..x + cells
}

impl Sel {
    pub fn project(self) -> ProjectId {
        match self {
            Self::Project(p) | Self::Panel(p, _) => p,
        }
    }
}

/// One row of the sidebar's list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Project(ProjectId),
    Panel(ProjectId, PanelId),
    /// The muted `no panels` of an empty project.
    NoPanels,
    /// A blank row between two spaces (`sidebar_space_gap`).
    Gap,
}

impl Row {
    pub(crate) fn sel(self) -> Option<Sel> {
        match self {
            Self::Project(p) => Some(Sel::Project(p)),
            Self::Panel(p, panel) => Some(Sel::Panel(p, panel)),
            Self::NoPanels | Self::Gap => None,
        }
    }
}

/// An item shown at another place than the daemon's, while it is dragged or its move is on
/// its way: the item ends at index `to` of its list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reorder {
    pub item: Sel,
    pub to: usize,
}

/// `items` with the one at `from` moved to `to` (clamped to the last index).
fn moved<T>(items: &mut Vec<T>, from: usize, to: usize) {
    let item = items.remove(from);
    items.insert(to.min(items.len()), item);
}

/// The rows of the session's projects, collapsed ones without their panels, with `gap` blank
/// rows between two projects.
pub fn rows(view: &SessionView, gap: u8) -> Vec<Row> {
    rows_with(view, gap, None)
}

/// [`rows`] as they will be once `reorder` is applied.
pub fn rows_with(view: &SessionView, gap: u8, reorder: Option<Reorder>) -> Vec<Row> {
    let mut projects: Vec<&deck_core::model::ProjectView> = view.projects.iter().collect();
    if let Some(Reorder {
        item: Sel::Project(id),
        to,
    }) = reorder
        && let Some(from) = projects.iter().position(|p| p.id == id)
    {
        moved(&mut projects, from, to);
    }
    let mut rows = Vec::new();
    for (i, project) in projects.into_iter().enumerate() {
        if i > 0 {
            rows.extend(std::iter::repeat_n(Row::Gap, usize::from(gap)));
        }
        rows.push(Row::Project(project.id));
        if project.collapsed {
            continue;
        }
        if project.panels.is_empty() {
            rows.push(Row::NoPanels);
        }
        let mut panels: Vec<PanelId> = project.panels.iter().map(|p| p.id).collect();
        if let Some(Reorder {
            item: Sel::Panel(owner, id),
            to,
        }) = reorder
            && owner == project.id
            && let Some(from) = panels.iter().position(|p| *p == id)
        {
            moved(&mut panels, from, to);
        }
        rows.extend(panels.into_iter().map(|p| Row::Panel(project.id, p)));
    }
    rows
}

impl App {
    pub(crate) fn sidebar_rows(&self) -> Vec<Row> {
        let gap = self.config.sidebar_space_gap;
        self.mirror
            .view()
            .map_or_else(Vec::new, |view| rows(view, gap))
    }

    fn selectable(&self) -> Vec<Sel> {
        self.sidebar_rows()
            .into_iter()
            .filter_map(Row::sel)
            .collect()
    }

    /// Rows of the list that fit under the header.
    fn list_height(&self) -> usize {
        let height = self.size.1;
        usize::from(height.saturating_sub(HEADER_ROWS + bar_rows(height)))
    }

    /// What the cell is over: a row of the list, the `New space` chip of the header, or the
    /// chips of the bottom bar when it is drawn.
    pub(crate) fn hover_at(&self, column: u16, row: u16) -> Option<Hover> {
        let height = self.size.1;
        let w = self.sidebar_width;
        if column >= w.min(self.sidebar_cols()) {
            return None;
        }
        if row == 0 {
            return new_project_span(w)
                .contains(&column)
                .then_some(Hover::NewProject);
        }
        if bar_rows(height) == 0 || row != height - 1 {
            let line = usize::from(row.checked_sub(HEADER_ROWS)?);
            if line >= self.list_height() {
                return None;
            }
            let rows = self.sidebar_rows();
            return rows
                .get(self.sidebar_offset + line)
                .and_then(|r| r.sel())
                .map(Hover::Row);
        }
        if new_span(w).contains(&column) {
            Some(Hover::New)
        } else if restore_span(w).contains(&column) {
            Some(Hover::Restore)
        } else {
            None
        }
    }

    /// The project the bottom `+ New` button opens the menu for: the selection in sidebar
    /// focus, else the active project.
    pub(crate) fn new_target(&self) -> Option<ProjectId> {
        // A selection whose project another client closed falls back to the active project.
        if self.sidebar_focus
            && let Some(sel) = self.selection
            && self.mirror.find_project(sel.project()).is_some()
        {
            return Some(sel.project());
        }
        self.mirror.active_project().map(|p| p.id)
    }

    /// Sets the hover, redrawing only when it changed.
    pub(crate) fn set_hover(&mut self, hover: Option<Hover>) {
        if self.hover != hover {
            self.hover = hover;
            self.dirty = true;
        }
    }

    /// Recomputes the hover from the last pointer position, after rows moved under it.
    pub(crate) fn refresh_hover(&mut self) {
        let hover = match self.pointer {
            Some((column, row)) if !self.overlay_open() && self.row_drag.is_none() => {
                self.hover_at(column, row)
            }
            _ => None,
        };
        self.set_hover(hover);
    }

    /// The selection a sidebar focus starts with: the active panel's row, else the active
    /// project's.
    fn default_selection(&self) -> Option<Sel> {
        let project = self.mirror.active_project()?;
        match self.mirror.active_panel() {
            Some(panel) if !project.collapsed => Some(Sel::Panel(project.id, panel.id)),
            _ => Some(Sel::Project(project.id)),
        }
    }

    pub(crate) fn enter_sidebar(&mut self) -> Vec<Out> {
        let outs = self.leave_scroll();
        self.sidebar_focus = true;
        self.selection = self.default_selection();
        self.ensure_visible();
        outs
    }

    /// Keeps the selection on a row that exists after the view changed.
    pub(crate) fn normalize_selection(&mut self) {
        let Some(sel) = self.selection else { return };
        let rows = self.selectable();
        if rows.contains(&sel) {
            return;
        }
        let project = Sel::Project(sel.project());
        self.selection = if rows.contains(&project) {
            Some(project)
        } else {
            self.default_selection()
        };
    }

    /// Scrolls the list the least that shows the row that matters: the selection in sidebar
    /// focus, else the active panel (or project).
    pub(crate) fn ensure_visible(&mut self) {
        let rows = self.sidebar_rows();
        let height = self.list_height();
        let target = if self.sidebar_focus {
            self.selection
        } else {
            self.default_selection()
        }
        .and_then(|sel| rows.iter().position(|r| r.sel() == Some(sel)));
        let max = rows.len().saturating_sub(height);
        let mut offset = self.sidebar_offset.min(max);
        if let Some(target) = target {
            if target < offset {
                offset = target;
            } else if height > 0 && target >= offset + height {
                offset = target + 1 - height;
            }
        }
        self.sidebar_offset = offset.min(max);
    }

    /// Keeps the first shown row inside the list.
    pub(crate) fn clamp_offset(&mut self) {
        let max = self.sidebar_rows().len().saturating_sub(self.list_height());
        self.sidebar_offset = self.sidebar_offset.min(max);
    }

    fn move_selection(&mut self, delta: isize) {
        let rows = self.selectable();
        if rows.is_empty() {
            return;
        }
        let at = self
            .selection
            .and_then(|s| rows.iter().position(|r| *r == s))
            .unwrap_or(0);
        let to = at.saturating_add_signed(delta).min(rows.len() - 1);
        self.selection = rows.get(to).copied();
    }

    fn set_collapsed(&mut self, project: ProjectId, collapsed: bool) -> Vec<Out> {
        let Some(p) = self.mirror.find_project(project) else {
            return Vec::new();
        };
        if p.collapsed == collapsed {
            return Vec::new();
        }
        if collapsed {
            self.selection = Some(Sel::Project(project));
        }
        vec![self.act(Act::SetProjectCollapsed { project, collapsed }, None)]
    }

    fn toggle_collapsed(&mut self, project: ProjectId) -> Vec<Out> {
        let collapsed = self
            .mirror
            .find_project(project)
            .is_some_and(|p| p.collapsed);
        self.set_collapsed(project, !collapsed)
    }

    /// A key while the sidebar has the keyboard.
    pub(crate) fn sidebar_key(&mut self, key: KeyEvent) -> Vec<Out> {
        if key.mods.ctrl || key.mods.alt {
            return Vec::new();
        }
        self.normalize_selection();
        let sel = self.selection;
        let outs = match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_selection(-1);
                Vec::new()
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_selection(1);
                Vec::new()
            }
            KeyCode::Enter => match sel {
                Some(Sel::Panel(_, panel)) => {
                    self.sidebar_focus = false;
                    vec![self.act(Act::FocusPanel { panel }, None)]
                }
                Some(Sel::Project(project)) => self.toggle_collapsed(project),
                None => Vec::new(),
            },
            KeyCode::Left | KeyCode::Char('h') => {
                sel.map_or_else(Vec::new, |s| self.set_collapsed(s.project(), true))
            }
            KeyCode::Right | KeyCode::Char('l') => {
                sel.map_or_else(Vec::new, |s| self.set_collapsed(s.project(), false))
            }
            KeyCode::Char('c') => sel.map_or_else(Vec::new, |s| {
                vec![self.act(
                    Act::NewPanel {
                        project: s.project(),
                        name: None,
                    },
                    None,
                )]
            }),
            KeyCode::Char('n') => {
                if let Some(s) = sel {
                    self.open_new_menu(s.project());
                }
                Vec::new()
            }
            KeyCode::Char('P') => {
                self.open_new_project();
                Vec::new()
            }
            KeyCode::Char('R') => self.open_restore(std::time::Instant::now()),
            KeyCode::Char('r') => {
                match sel {
                    Some(Sel::Panel(_, panel)) => self.open_rename_panel(panel),
                    Some(Sel::Project(project)) => self.open_rename_project(project),
                    None => {}
                }
                Vec::new()
            }
            KeyCode::Char('x') => {
                match sel {
                    Some(Sel::Panel(_, panel)) => self.confirm_close_panel(panel),
                    Some(Sel::Project(project)) => self.confirm_close_project(project),
                    None => {}
                }
                Vec::new()
            }
            KeyCode::Esc | KeyCode::Tab => {
                self.sidebar_focus = false;
                Vec::new()
            }
            _ => Vec::new(),
        };
        self.ensure_visible();
        outs
    }

    /// The release of a click on `item`: focuses the panel, or folds or unfolds the space.
    fn click_row(&mut self, item: Sel) -> Vec<Out> {
        self.selection = Some(item);
        match item {
            Sel::Panel(_, panel) => vec![self.act(Act::FocusPanel { panel }, None)],
            Sel::Project(project) => self.toggle_collapsed(project),
        }
    }

    /// Ends the press or drag of a row, for a reason that is not its release: the rest of the
    /// gesture goes to no program.
    /// The rest of the gesture is swallowed only when `swallow` (its release is still to come).
    pub(crate) fn end_row_gesture(&mut self, swallow: bool) {
        let pressed = self.row_press.take().is_some();
        let dragged = self.row_drag.take().is_some();
        if pressed || dragged {
            self.swallow_release |= swallow;
            self.dirty = true;
        }
    }

    /// Ends a gesture whose row is gone or whose sidebar cannot be drawn.
    pub(crate) fn check_row_gesture(&mut self) {
        let item = self.row_drag.map(|d| d.item).or(self.row_press);
        let Some(item) = item else { return };
        let alive = !too_small(self.size)
            && self.sidebar_visible()
            && !self.overlay_open()
            && self.sidebar_rows().iter().any(|r| r.sel() == Some(item));
        if !alive {
            self.end_row_gesture(true);
        }
    }

    /// The rows as they are drawn: the daemon's, with the item just dropped at its new place until
    /// the daemon answers.
    pub(crate) fn preview_rows(&self) -> Vec<Row> {
        // The list stays still while dragging; a move just dropped shows until its answer.
        let reorder = self.row_drop.map(|(r, _)| r);
        let gap = self.config.sidebar_space_gap;
        self.mirror
            .view()
            .map_or_else(Vec::new, |view| rows_with(view, gap, reorder))
    }

    /// The item whose row is the drop target of the drag: the one whose place the dragged item
    /// would take. None without a drag, or when the target is the item's own place.
    pub(crate) fn drag_target_sel(&self) -> Option<Sel> {
        let drag = self.row_drag?;
        if self.place_of(drag.item)? == drag.to {
            return None;
        }
        let view = self.mirror.view()?;
        match drag.item {
            Sel::Project(_) => view.projects.get(drag.to).map(|p| Sel::Project(p.id)),
            Sel::Panel(project, _) => {
                let panels = &view.projects.iter().find(|p| p.id == project)?.panels;
                panels.get(drag.to).map(|p| Sel::Panel(project, p.id))
            }
        }
    }

    /// The index `item` is at in its list now.
    fn place_of(&self, item: Sel) -> Option<usize> {
        let view = self.mirror.view()?;
        match item {
            Sel::Project(id) => view.projects.iter().position(|p| p.id == id),
            Sel::Panel(project, id) => view
                .projects
                .iter()
                .find(|p| p.id == project)?
                .panels
                .iter()
                .position(|p| p.id == id),
        }
    }

    /// The index `item` would end at with the pointer on row `at` of the daemon's `rows`.
    fn drop_target(&self, rows: &[Row], item: Sel, at: usize) -> Option<usize> {
        let view = self.mirror.view()?;
        match item {
            Sel::Project(_) => {
                // A blank row or `no panels` belongs to the space above it.
                let owner = rows.get(..=at)?.iter().rev().find_map(|r| match r {
                    Row::Project(p) | Row::Panel(p, _) => Some(*p),
                    Row::NoPanels | Row::Gap => None,
                })?;
                view.projects.iter().position(|p| p.id == owner)
            }
            Sel::Panel(project, _) => {
                let panels = &view.projects.iter().find(|p| p.id == project)?.panels;
                if let Some(Row::Panel(p, panel)) = rows.get(at)
                    && *p == project
                {
                    return panels.iter().position(|x| x.id == *panel);
                }
                // Above the space's panels is the first, below is the last.
                let first = rows
                    .iter()
                    .position(|r| matches!(r, Row::Panel(p, _) if *p == project))?;
                Some(if at < first {
                    0
                } else {
                    panels.len().saturating_sub(1)
                })
            }
        }
    }

    /// The press, drag and release of a row: `Some` with what to send when the event is the
    /// gesture's and goes no further.
    pub(crate) fn row_drag_mouse(&mut self, event: ct::MouseEvent) -> Option<Vec<Out>> {
        use ct::MouseButton::Left;
        use ct::MouseEventKind as Kind;
        if self.row_press.is_none() && self.row_drag.is_none() {
            return None;
        }
        match event.kind {
            Kind::Drag(Left) => {
                self.row_drag_to(event);
                Some(Vec::new())
            }
            Kind::Up(Left) => {
                let press = self.row_press.take();
                let drag = self.row_drag.take();
                self.dirty = true;
                if let Some(drag) = drag {
                    return Some(self.drop_row(drag, event));
                }
                let item = press?;
                if self.hover_at(event.column, event.row) == Some(Hover::Row(item)) {
                    self.hover = None;
                    return Some(self.click_row(item));
                }
                Some(Vec::new())
            }
            // The release was lost: the gesture is over, and this event is handled as usual.
            Kind::Moved | Kind::Down(_) => {
                self.row_press = None;
                self.row_drag = None;
                self.dirty = true;
                None
            }
            _ => Some(Vec::new()),
        }
    }

    /// The release of a drag: the move, when the item ends at another place. A release over
    /// the panel area drops nothing.
    fn drop_row(&mut self, drag: RowDrag, event: ct::MouseEvent) -> Vec<Out> {
        if event.column >= self.sidebar_width || self.place_of(drag.item) == Some(drag.to) {
            return Vec::new();
        }
        self.row_drop = Some((
            Reorder {
                item: drag.item,
                to: drag.to,
            },
            self.pending.len(),
        ));
        let action = match drag.item {
            Sel::Project(project) => Act::MoveProject {
                project,
                to: drag.to,
            },
            Sel::Panel(_, panel) => Act::MovePanel { panel, to: drag.to },
        };
        vec![self.act(action, None)]
    }

    /// The pointer moved with the button down after a press on a row: starts the drag when it
    /// reaches another row, then follows it.
    fn row_drag_to(&mut self, event: ct::MouseEvent) {
        let rows = self.sidebar_rows();
        let height = self.list_height();
        let in_list = event.column < self.sidebar_width;
        let line = event
            .row
            .checked_sub(HEADER_ROWS)
            .map(usize::from)
            .filter(|l| in_list && *l < height);
        // A pointer below the last row is on it.
        let at = line
            .map(|l| (self.sidebar_offset + l).min(rows.len().saturating_sub(1)))
            .filter(|_| !rows.is_empty());
        let item = self.row_drag.map(|d| d.item).or(self.row_press);
        let Some(item) = item else { return };
        if self.row_drag.is_none() {
            // The pointer has to reach another row to start a drag.
            let reached = line
                .and_then(|l| rows.get(self.sidebar_offset + l))
                .is_some_and(|r| r.sel() != Some(item));
            if !reached {
                return;
            }
            self.row_press = None;
            self.row_drag = Some(RowDrag { item, to: 0 });
            self.set_hover(None);
            self.dirty = true;
            if let Some(to) = self.place_of(item) {
                self.row_drag = Some(RowDrag { item, to });
            }
        }
        if let Some(at) = at
            && let Some(to) = self.drop_target(&rows, item, at)
            && let Some(drag) = self.row_drag.as_mut()
            && drag.to != to
        {
            drag.to = to;
            self.dirty = true;
        }
        // The header scrolls the list up, the last row down.
        let max = rows.len().saturating_sub(height);
        if in_list && event.row < HEADER_ROWS && self.sidebar_offset > 0 {
            self.sidebar_offset -= 1;
            self.dirty = true;
        } else if in_list && height > 0 && line == Some(height - 1) && self.sidebar_offset < max {
            self.sidebar_offset += 1;
            self.dirty = true;
        }
    }

    /// A mouse event over the sidebar or its separator.
    pub(crate) fn sidebar_mouse(&mut self, event: ct::MouseEvent) -> Vec<Out> {
        if event.column >= self.sidebar_width {
            return Vec::new();
        }
        let rows = self.sidebar_rows();
        let max = rows.len().saturating_sub(self.list_height());
        match event.kind {
            ct::MouseEventKind::ScrollUp => {
                self.sidebar_offset = self.sidebar_offset.saturating_sub(WHEEL_ROWS);
                Vec::new()
            }
            ct::MouseEventKind::ScrollDown => {
                self.sidebar_offset = (self.sidebar_offset + WHEEL_ROWS).min(max);
                Vec::new()
            }
            ct::MouseEventKind::Down(ct::MouseButton::Left) => {
                let target = self.hover_at(event.column, event.row);
                if matches!(target, Some(h) if !matches!(h, Hover::Row(_))) {
                    self.chip_down = target;
                    return Vec::new();
                }
                let Some(line) = event.row.checked_sub(HEADER_ROWS) else {
                    return Vec::new();
                };
                let line = usize::from(line);
                if line >= self.list_height() {
                    return Vec::new();
                }
                // The press only records the row: its release acts, a drag moves it.
                self.row_press = rows.get(self.sidebar_offset + line).and_then(|r| r.sel());
                Vec::new()
            }
            _ => Vec::new(),
        }
    }
}

/// The kind of icon of a panel: the one of its source terminal's agent or program; none
/// without terminals.
pub fn panel_icon_kind(panel: &PanelView) -> Option<IconKind> {
    let term = source_terminal_view(panel)?;
    Some(icon_for(
        term.agent.as_ref().map(|a| a.kind),
        Some(&term.program),
    ))
}

/// The style of an icon of `kind` in a row styled `style`: Claude's is always its orange. On
/// a reversed row the colour goes in the background, which is what a reversed cell shows as
/// text.
pub fn icon_style(kind: IconKind, style: Style) -> Style {
    if kind != IconKind::Claude {
        return style;
    }
    let reversed = style.add_modifier.contains(Modifier::REVERSED)
        && !style.sub_modifier.contains(Modifier::REVERSED);
    if reversed {
        style.bg(claude_color())
    } else {
        style.fg(claude_color())
    }
}

/// An icon as it is drawn: a two-cell slot and the style to draw it in. In the `Image` tier an
/// icon with an image that was sent is the placeholder cells (the visible colours of `style`,
/// no modifiers, the id as foreground; see [`crate::graphics::Images::cell_for`]); every other
/// icon is its glyph of the tier in the first cell and a blank second, in `style`. An image
/// not sent yet is noted for the pre-pass ([`App::send_images`]) and drawn as its glyph.
pub fn icon_slot(app: &App, icon: Icon, style: Style) -> (String, Style) {
    let style = match icon {
        Icon::Kind(kind) => icon_style(kind, style),
        _ => style,
    };
    let image = app
        .live_images()
        .filter(|_| has_image(icon))
        .and_then(|images| {
            let (rgb, cell) = images.cell_for(style);
            let frame = if icon == Icon::Spinner {
                app.spin_frame
            } else {
                0
            };
            let pair = Pair { icon, rgb, frame };
            images.want(pair);
            images.id(&pair).map(|id| cell.fg(image_id_color(id)))
        });
    match image {
        Some(cell) => (placeholder(), cell),
        None => {
            // `spin` steps at 60 Hz in the image tier: its glyph frame follows the clock at 100 ms.
            let frame = if app.tier == Tier::Image {
                usize::from(app.spin_frame) / 6
            } else {
                app.spin
            };
            (format!("{} ", app.tier.glyph(icon, frame)), style)
        }
    }
}

/// The icon of a panel as it is drawn (see [`icon_slot`]); none without terminals.
pub fn panel_icon_cell(app: &App, panel: &PanelView, style: Style) -> Option<(String, Style)> {
    let kind = panel_icon_kind(panel)?;
    Some(icon_slot(app, Icon::Kind(kind), style))
}

/// The style of the sidebar's rows before any of them is selected or active.
pub fn base_style(theme: &Theme) -> Style {
    Style::new()
        .fg(color(theme.sidebar_fg))
        .bg(color(theme.sidebar_bg))
}

/// The style of a panel row's name and icon (before the unseen mark makes it bold).
///
/// `band` is whether the row has the selection band: the selected row in sidebar focus, else
/// the active panel's row. An active row without the band is drawn in the accent colour.
pub fn panel_style(theme: &Theme, base: Style, band: bool, active: bool) -> Style {
    let style = if band {
        selection(theme, base)
    } else if active {
        base.fg(color(theme.accent))
    } else {
        base
    };
    if active {
        style.add_modifier(Modifier::BOLD)
    } else {
        style
    }
}

/// The caret of the drop target's row: `DropTarget`'s glyph in the tier.
pub fn caret_glyph(tier: Tier) -> &'static str {
    tier.glyph(Icon::DropTarget, 0)
}

/// The style of the dragged row, left in its place: all muted.
pub fn dimmed_style(theme: &Theme, base: Style) -> Style {
    base.fg(color(theme.muted))
}

/// Draws the sidebar in `area` (its columns, without the separator) and the separator
/// column after it.
pub fn draw(buf: &mut Buffer, area: Rect, app: &App, theme: &Theme) {
    let Some(view) = app.mirror.view() else {
        return;
    };
    let base = base_style(theme);
    buf.set_style(area, base);
    // The separator is drawn in the accent colour while it is dragged.
    let (bar, bar_color) = if app.sidebar_drag.is_some() {
        ("┃", theme.accent)
    } else {
        ("│", theme.border)
    };
    for y in area.y..area.y + area.height {
        buf.set_string(
            area.x + area.width,
            y,
            bar,
            Style::new().fg(color(bar_color)),
        );
    }
    let muted = base.fg(color(theme.muted));
    let accent = base.fg(color(theme.accent));
    // Foreground-only patches for the cells of a row, which keep the row's background.
    let fg = |c| Style::new().fg(color(c));
    let accent_fg = fg(theme.accent);
    let hover_bg = color(theme.hover);
    let w = area.width;
    let put = |buf: &mut Buffer, y: u16, x: u16, text: &str, style: Style| {
        buf.set_stringn(area.x + x, y, text, usize::from(w.saturating_sub(x)), style);
    };

    // The chips: the hover background under the pointer.
    let style_of = |hover| {
        if app.hover == Some(hover) {
            base.bg(hover_bg).fg(color(theme.sidebar_fg))
        } else {
            muted
        }
    };
    // The title, cut before the `New space` chip, which it gives way to.
    let span = new_project_span(w);
    let header = truncate(
        app.mirror.session_name().unwrap_or(""),
        span.start.saturating_sub(2),
    );
    put(buf, area.y, 1, &header, accent.add_modifier(Modifier::BOLD));
    let style = style_of(Hover::NewProject);
    buf.set_style(
        Rect::new(area.x + span.start, area.y, span.end - span.start, 1),
        style,
    );
    draw_chip(
        app,
        buf,
        (area.x + span.start, area.y),
        w,
        Icon::New,
        NEW_PROJECT_LABEL,
        style,
    );

    let rows = app.preview_rows();
    let dragged = app.row_drag.map(|d| d.item);
    let target = app.drag_target_sel();
    let height = usize::from(
        area.height
            .saturating_sub(HEADER_ROWS + bar_rows(area.height)),
    );
    // The row a context menu belongs to keeps the hover background.
    let menu_row = app.context.as_ref().and_then(|menu| menu.row(app));
    let selected = if app.sidebar_focus {
        app.selection
    } else {
        None
    };
    for (line, row) in rows
        .iter()
        .skip(app.sidebar_offset)
        .take(height)
        .enumerate()
    {
        let y = area.y + HEADER_ROWS + u16::try_from(line).unwrap_or(0);
        let mut row_style = base;
        let is_selected = row.sel().is_some() && row.sel() == selected;
        let active = matches!(*row, Row::Panel(_, id) if view.active_panel == Some(id));
        // The selection in sidebar focus, else the active panel, has the band.
        // The drop target is drawn two columns to the right, after a caret; the dragged row
        // is dimmed.
        let shifted = row.sel().is_some() && row.sel() == target;
        let dimmed = row.sel().is_some() && row.sel() == dragged;
        // A band or hover background starts after the caret.
        let (lead, full) = if shifted {
            (2, w.saturating_sub(2))
        } else {
            (0, w)
        };
        let band = !dimmed && (is_selected || (active && !app.sidebar_focus));
        let hovered = !dimmed
            && !band
            && row.sel().is_some()
            && (app.hover == row.sel().map(Hover::Row) || menu_row == row.sel());
        let muted = if hovered {
            fg(theme.sidebar_fg)
        } else {
            fg(theme.muted)
        };
        // The accent and muted patches must not replace a custom selection colour.
        let keep = |style: Style| {
            let none = Default::default();
            if dimmed {
                style.fg(color(theme.muted))
            } else if band && theme.selection_fg != none {
                style.fg(color(theme.selection_fg))
            } else if band && theme.selection_bg == none {
                // Reversed: a colour would show as a block, so the text colour stays.
                style.fg(color(theme.sidebar_fg))
            } else {
                style
            }
        };
        if dimmed {
            row_style = dimmed_style(theme, row_style);
        } else if band {
            row_style = selection(theme, row_style);
            buf.set_style(Rect::new(area.x + lead, y, full, 1), row_style);
        } else if hovered {
            row_style = row_style.bg(hover_bg);
            buf.set_style(Rect::new(area.x + lead, y, full, 1), row_style);
        }
        // The caret, then the row's content in the room that is left.
        let (shift, w) = (lead, full);
        if shifted {
            let caret = Style::new()
                .fg(color(theme.accent))
                .add_modifier(Modifier::BOLD)
                .remove_modifier(Modifier::REVERSED);
            buf.set_stringn(area.x, y, format!("{} ", caret_glyph(app.tier)), 2, caret);
        }
        let put = |buf: &mut Buffer, y: u16, x: u16, text: &str, style: Style| {
            buf.set_stringn(
                area.x + shift + x,
                y,
                text,
                usize::from(w.saturating_sub(x)),
                style,
            );
        };
        // The right-aligned count, if any, takes its room off the name.
        let side = |text: String| -> (String, u16) {
            let n = width(&text);
            (text, n)
        };
        match *row {
            Row::Project(id) => {
                let Some(p) = view.projects.iter().find(|p| p.id == id) else {
                    continue;
                };
                let caret = if p.collapsed {
                    Icon::Collapsed
                } else {
                    Icon::Expanded
                };
                // The caret's slot and the one-cell margin close the row on the right; the
                // count of a collapsed space comes before it.
                let slot_x = w.saturating_sub(ICON_CELLS + 1);
                let (text, caret_style) = icon_slot(app, caret, keep(row_style.patch(muted)));
                put(buf, y, slot_x, &text, caret_style);
                let (count, count_w) = if p.collapsed {
                    side(p.panels.len().to_string())
                } else {
                    (String::new(), 0)
                };
                // `!n` after the name of a collapsed project with n waiting panels, with the
                // Unicode glyph in every tier.
                let waiting = if p.collapsed {
                    agent::waiting_panels(p)
                } else {
                    0
                };
                let badge = (waiting > 0)
                    .then(|| format!("{}{waiting}", deck_core::icons::unicode(Icon::Waiting, 0)));
                let badge_w = badge.as_deref().map_or(0, |b| width(b) + 1);
                let room = slot_x
                    .saturating_sub(2 + if count_w > 0 { count_w + 1 } else { 0 })
                    .saturating_sub(badge_w);
                let mut style = row_style;
                if view.active_project == Some(id) && !dimmed {
                    style = style.add_modifier(Modifier::BOLD);
                }
                let name = truncate(&p.name, room);
                put(buf, y, 1, &name, style);
                if let Some(badge) = badge {
                    let warning = Style::new()
                        .fg(color(theme.warning))
                        .add_modifier(Modifier::BOLD);
                    put(
                        buf,
                        y,
                        2 + width(&name),
                        &badge,
                        keep(row_style.patch(warning)),
                    );
                }
                if count_w > 0 {
                    put(
                        buf,
                        y,
                        slot_x.saturating_sub(count_w + 1),
                        &count,
                        keep(row_style.patch(muted)),
                    );
                }
            }
            Row::Panel(_, id) => {
                let Some((_, p)) = app.mirror.find_panel(id) else {
                    continue;
                };
                let mut style = panel_style(theme, base, band, active);
                if hovered {
                    style = style.bg(hover_bg);
                }
                if dimmed {
                    style = row_style;
                }
                // The status slot closes the row before the margin: the panel's most urgent
                // agent. Without one there is no slot and no room kept for it.
                let status = agent::status_icon(agent::panel_status(p), theme);
                let status_w = if status.is_some() { ICON_CELLS + 1 } else { 0 };
                if let Some((icon, glyph_style)) = status {
                    // The muted `quiet` glyph turns to the sidebar text under the pointer.
                    let glyph_style = if hovered && glyph_style.fg == Some(color(theme.muted)) {
                        fg(theme.sidebar_fg)
                    } else {
                        glyph_style
                    };
                    let (text, slot_style) =
                        icon_slot(app, icon, keep(row_style.patch(glyph_style)));
                    put(buf, y, w.saturating_sub(ICON_CELLS + 1), &text, slot_style);
                }
                let (count, count_w) = if p.terminals.len() > 1 {
                    side(p.terminals.len().to_string())
                } else {
                    (String::new(), 0)
                };
                // The unseen marker is a slot after the name and one space.
                let marker_w = if p.unseen { ICON_CELLS + 1 } else { 0 };
                // The icon and its space come before the name.
                let icon = panel_icon_cell(app, p, style);
                let name_x = NAME_X;
                let room = w
                    .saturating_sub(name_x + 1 + if count_w > 0 { count_w + 1 } else { 0 })
                    .saturating_sub(marker_w + status_w);
                if p.unseen && !dimmed {
                    style = style.add_modifier(Modifier::BOLD);
                }
                if let Some((icon, icon_style)) = icon {
                    put(buf, y, ICON_X, &icon, icon_style);
                }
                // A kept terminal that exited and is alone in its panel: ` · exited <code>`
                // after the name, which is cut first.
                let exited = match p.terminals.as_slice() {
                    [t] => t.exit_status.map(|code| format!(" · exited {code}")),
                    _ => None,
                }
                .unwrap_or_default();
                let name = truncate(&p.name, room.saturating_sub(width(&exited)));
                put(buf, y, name_x, &name, style);
                let exited = truncate(&exited, room.saturating_sub(width(&name)));
                put(
                    buf,
                    y,
                    name_x + width(&name),
                    &exited,
                    keep(row_style.patch(muted)),
                );
                let name_end = width(&name) + width(&exited);
                if p.unseen {
                    let (text, slot_style) =
                        icon_slot(app, Icon::Unseen, keep(row_style.patch(accent_fg)));
                    put(buf, y, name_x + 1 + name_end, &text, slot_style);
                }
                if count_w > 0 {
                    put(
                        buf,
                        y,
                        w.saturating_sub(count_w + 1 + status_w),
                        &count,
                        keep(row_style.patch(muted)),
                    );
                }
            }
            Row::NoPanels => put(buf, y, 3, "no panels", keep(row_style.patch(muted))),
            Row::Gap => {}
        }
    }
    if bar_rows(area.height) > 0 {
        let y = area.y + area.height - 1;
        let style = style_of(Hover::New);
        let cells = chip_cells(w, NEW_LABEL);
        buf.set_style(Rect::new(area.x + 1, y, cells, 1), style);
        draw_chip(app, buf, (area.x + 1, y), w, Icon::New, NEW_LABEL, style);
        let style = style_of(Hover::Restore);
        let span = restore_span(w);
        buf.set_style(
            Rect::new(area.x + span.start, y, span.end - span.start, 1),
            style,
        );
        draw_chip(
            app,
            buf,
            (area.x + span.start, y),
            w,
            Icon::Resume,
            RESUME_LABEL,
            style,
        );
    }
}
