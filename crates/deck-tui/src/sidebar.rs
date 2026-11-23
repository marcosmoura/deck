//! The sidebar: its rows, the keyboard focus and mouse handling that belong to it, and its
//! drawing.

use deck_core::action::Action as Act;
use deck_core::icons::icon_for;
use deck_core::keys::{KeyCode, KeyEvent};
use deck_core::model::{PanelId, PanelView, ProjectId, SessionView};
use deck_core::naming::source_terminal_view;
use deck_core::theme::{IconPreset, Theme};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event as ct;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::agent;
use crate::app::{App, Out};
use crate::draw::{color, selection, truncate, width};

/// The session name and a blank row above the project rows.
pub const HEADER_ROWS: u16 = 2;
/// Rows the bottom bar and the blank row above it take.
const BAR_ROWS: u16 = 2;
/// A sidebar shorter than this has no bottom bar.
const BAR_MIN_HEIGHT: u16 = 8;
/// Under this width the bar's chips shorten to their icon.
const BAR_WIDE: u16 = 26;
/// Rows the wheel scrolls per notch.
const WHEEL_ROWS: usize = 3;
/// What the status bar shows while the sidebar has the keyboard.
pub const SIDEBAR_HINTS: &str =
    "↑↓ move  ⏎ open  ←→ fold  c new  n menu  r rename  x close  esc back";

/// The row the sidebar selection is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sel {
    Project(ProjectId),
    Panel(ProjectId, PanelId),
}

/// What the pointer is over in the sidebar. Client-local: never sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hover {
    NewProject,
    // Set by DECK-11.
    #[expect(dead_code, reason = "Restore is set by DECK-11")]
    Restore,
    /// A project row, with its New button on show.
    ProjectNew(ProjectId),
}

/// Columns of the project row's button, and its margin from the right edge.
const BUTTON_MARGIN: u16 = 1;
/// Cells kept between the name and the button, besides the name's own `…`.
const BUTTON_GAP: u16 = 2;

/// The text of the project row's button.
fn button_text(theme: &Theme) -> String {
    format!("[ New {} ]", theme.glyphs.expanded)
}

/// Rows the bottom bar and its spacer take in a sidebar `height` rows tall.
fn bar_rows(height: u16) -> u16 {
    if height >= BAR_MIN_HEIGHT {
        BAR_ROWS
    } else {
        0
    }
}

/// The `New project` chip: `" + New project "`, or `" + "` in a narrow sidebar.
fn chip(width: u16) -> &'static str {
    if width < BAR_WIDE {
        " + "
    } else {
        " + New project "
    }
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
}

impl Row {
    fn sel(self) -> Option<Sel> {
        match self {
            Self::Project(p) => Some(Sel::Project(p)),
            Self::Panel(p, panel) => Some(Sel::Panel(p, panel)),
            Self::NoPanels => None,
        }
    }
}

/// The rows of the session's projects, collapsed ones without their panels.
pub fn rows(view: &SessionView) -> Vec<Row> {
    let mut rows = Vec::new();
    for project in &view.projects {
        rows.push(Row::Project(project.id));
        if project.collapsed {
            continue;
        }
        if project.panels.is_empty() {
            rows.push(Row::NoPanels);
        }
        rows.extend(project.panels.iter().map(|p| Row::Panel(project.id, p.id)));
    }
    rows
}

impl App {
    pub(crate) fn sidebar_rows(&self) -> Vec<Row> {
        self.mirror.view().map_or_else(Vec::new, rows)
    }

    fn selectable(&self) -> Vec<Sel> {
        self.sidebar_rows()
            .into_iter()
            .filter_map(Row::sel)
            .collect()
    }

    /// Rows of the list that fit under the header.
    fn list_height(&self) -> usize {
        let height = self.size.1.saturating_sub(1);
        usize::from(height.saturating_sub(HEADER_ROWS + bar_rows(height)))
    }

    /// The window row of a project's row, when it is in the list on screen.
    pub(crate) fn project_row_y(&self, project: ProjectId) -> Option<u16> {
        let at = self
            .sidebar_rows()
            .iter()
            .position(|r| *r == Row::Project(project))?;
        let line = at.checked_sub(self.sidebar_offset)?;
        (line < self.list_height()).then(|| HEADER_ROWS + u16::try_from(line).unwrap_or(0))
    }

    /// Whether the project row shows its button: the pointer is on the row, or the row is
    /// the selection of the sidebar focus, or its menu is open.
    pub(crate) fn button_shown(&self, project: ProjectId) -> bool {
        self.hover == Some(Hover::ProjectNew(project))
            || (self.sidebar_focus && self.selection == Some(Sel::Project(project)))
            || self.menu.as_ref().is_some_and(|m| m.project == project)
    }

    /// What the cell is over: a project row, or the `New project` chip if the bar is drawn
    /// and shows it.
    pub(crate) fn hover_at(&self, column: u16, row: u16) -> Option<Hover> {
        let height = self.size.1.saturating_sub(1);
        if column >= self.sidebar_width.min(self.sidebar_cols()) {
            return None;
        }
        if let Some(line) = row.checked_sub(HEADER_ROWS)
            && usize::from(line) < self.list_height()
        {
            return match self
                .sidebar_rows()
                .get(self.sidebar_offset + usize::from(line))
            {
                Some(Row::Project(id)) => Some(Hover::ProjectNew(*id)),
                _ => None,
            };
        }
        if bar_rows(height) == 0 || row != height - 1 {
            return None;
        }
        let chip_w = width(chip(self.sidebar_width));
        (1..1 + chip_w)
            .contains(&column)
            .then_some(Hover::NewProject)
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
            Some((column, row)) if !self.overlay_open() => self.hover_at(column, row),
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

    /// Whether a sidebar column is on the project row's button.
    fn on_button(&self, column: u16) -> bool {
        let button = width(&button_text(&self.theme));
        let x = self.sidebar_width.saturating_sub(button + BUTTON_MARGIN);
        (x..x + button).contains(&column)
    }

    /// Whether the cell is on a project row's button.
    pub(crate) fn on_button_at(&self, column: u16, row: u16) -> bool {
        matches!(self.hover_at(column, row), Some(Hover::ProjectNew(_))) && self.on_button(column)
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
                if self.hover_at(event.column, event.row) == Some(Hover::NewProject) {
                    self.chip_down = true;
                    return Vec::new();
                }
                let Some(line) = event.row.checked_sub(HEADER_ROWS) else {
                    return Vec::new();
                };
                let line = usize::from(line);
                if line >= self.list_height() {
                    return Vec::new();
                }
                match rows.get(self.sidebar_offset + line) {
                    Some(Row::Panel(project, panel)) => {
                        self.selection = Some(Sel::Panel(*project, *panel));
                        vec![self.act(Act::FocusPanel { panel: *panel }, None)]
                    }
                    Some(Row::Project(project)) => {
                        // A click on the button opens the menu; on the rest of the row it
                        // folds the project.
                        if self.button_shown(*project) && self.on_button(event.column) {
                            self.open_new_menu(*project);
                            return Vec::new();
                        }
                        self.selection = Some(Sel::Project(*project));
                        self.toggle_collapsed(*project)
                    }
                    _ => Vec::new(),
                }
            }
            _ => Vec::new(),
        }
    }
}

/// The icon of a panel: the one of its source terminal's agent or program; none with
/// `icons = "none"` or without terminals.
pub fn panel_icon<'a>(panel: &PanelView, theme: &'a Theme) -> Option<&'a str> {
    if theme.icons == IconPreset::None {
        return None;
    }
    let term = source_terminal_view(panel)?;
    let kind = icon_for(term.agent.as_ref().map(|a| a.kind), Some(&term.program));
    Some(theme.icon_glyphs.get(kind))
}

/// Draws the sidebar in `area` (its columns, without the separator) and the separator
/// column after it.
pub fn draw(buf: &mut Buffer, area: Rect, app: &App, theme: &Theme) {
    let Some(view) = app.mirror.view() else {
        return;
    };
    let base = Style::new()
        .fg(color(theme.sidebar_fg))
        .bg(color(theme.sidebar_bg));
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
    let w = area.width;
    let put = |buf: &mut Buffer, y: u16, x: u16, text: &str, style: Style| {
        buf.set_stringn(area.x + x, y, text, usize::from(w.saturating_sub(x)), style);
    };

    let header = truncate(&view.name, w.saturating_sub(2));
    put(buf, area.y, 1, &header, accent.add_modifier(Modifier::BOLD));

    let rows = rows(view);
    let height = usize::from(
        area.height
            .saturating_sub(HEADER_ROWS + bar_rows(area.height)),
    );
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
        // The accent and muted patches must not replace a custom selection colour.
        let keep = |style: Style| {
            if is_selected && theme.selection_fg != Default::default() {
                style.fg(color(theme.selection_fg))
            } else {
                style
            }
        };
        if is_selected {
            row_style = selection(theme, row_style);
            buf.set_style(Rect::new(area.x, y, w, 1), row_style);
        }
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
                let glyph = if p.collapsed {
                    &theme.glyphs.collapsed
                } else {
                    &theme.glyphs.expanded
                };
                put(buf, y, 1, glyph, keep(row_style.patch(muted)));
                // The button, while shown, wins the right edge over the count and the badge.
                let button = app
                    .button_shown(id)
                    .then(|| button_text(theme))
                    .map(|text| (width(&text), text));
                let (count, count_w) = if p.collapsed && button.is_none() {
                    side(p.panels.len().to_string())
                } else {
                    (String::new(), 0)
                };
                // `!n` after the name of a collapsed project with n waiting panels.
                let waiting = if p.collapsed && button.is_none() {
                    agent::waiting_panels(p)
                } else {
                    0
                };
                let badge = (waiting > 0).then(|| format!("{}{waiting}", theme.glyphs.waiting));
                let badge_w = badge.as_deref().map_or(0, |b| width(b) + 1);
                let room = w
                    .saturating_sub(4 + if count_w > 0 { count_w + 1 } else { 0 })
                    .saturating_sub(badge_w)
                    .saturating_sub(
                        button
                            .as_ref()
                            .map_or(0, |(n, _)| n + BUTTON_MARGIN + BUTTON_GAP),
                    );
                let mut style = row_style;
                if view.active_project == Some(id) {
                    style = style.add_modifier(Modifier::BOLD);
                }
                let name = truncate(&p.name, room);
                put(buf, y, 3, &name, style);
                if let Some(badge) = badge {
                    let warning = Style::new()
                        .fg(color(theme.warning))
                        .add_modifier(Modifier::BOLD);
                    put(
                        buf,
                        y,
                        4 + width(&name),
                        &badge,
                        keep(row_style.patch(warning)),
                    );
                }
                if count_w > 0 {
                    put(
                        buf,
                        y,
                        w.saturating_sub(count_w + 1),
                        &count,
                        keep(row_style.patch(muted)),
                    );
                }
                if let Some((button_w, text)) = button {
                    let x = w.saturating_sub(button_w + BUTTON_MARGIN);
                    let on_it = app.hover == Some(Hover::ProjectNew(id))
                        && app
                            .pointer
                            .is_some_and(|(px, py)| py == y && (x..x + button_w).contains(&px));
                    let style = if on_it {
                        selection(theme, base)
                    } else {
                        keep(row_style.patch(accent))
                    };
                    buf.set_style(Rect::new(area.x + x, y, button_w, 1), style);
                    put(buf, y, x, &text, style);
                }
            }
            Row::Panel(_, id) => {
                let Some((_, p)) = app.mirror.find_panel(id) else {
                    continue;
                };
                let active = view.active_panel == Some(id);
                let mut style = row_style;
                if active {
                    put(
                        buf,
                        y,
                        0,
                        &theme.glyphs.active,
                        keep(row_style.patch(accent)),
                    );
                    style = keep(style.patch(accent)).add_modifier(Modifier::BOLD);
                }
                // The status column: the panel's most urgent agent.
                if let Some((text, glyph_style)) =
                    agent::glyph(agent::panel_status(p), app.spin, theme)
                {
                    put(buf, y, 1, &text, keep(row_style.patch(glyph_style)));
                }
                let (count, count_w) = if p.terminals.len() > 1 {
                    side(p.terminals.len().to_string())
                } else {
                    (String::new(), 0)
                };
                let marker_w = if p.unseen {
                    width(&theme.glyphs.unseen) + 1
                } else {
                    0
                };
                // The icon and its space come before the name.
                let icon = panel_icon(p, theme);
                let name_x = icon.map_or(3, |i| 4 + width(i));
                let room = w
                    .saturating_sub(name_x + 1 + if count_w > 0 { count_w + 1 } else { 0 })
                    .saturating_sub(marker_w);
                if p.unseen {
                    style = style.add_modifier(Modifier::BOLD);
                }
                if let Some(icon) = icon {
                    put(buf, y, 3, icon, style);
                }
                let name = truncate(&p.name, room);
                put(buf, y, name_x, &name, style);
                if p.unseen {
                    put(
                        buf,
                        y,
                        name_x + 1 + width(&name),
                        &theme.glyphs.unseen,
                        keep(row_style.patch(accent)),
                    );
                }
                if count_w > 0 {
                    put(
                        buf,
                        y,
                        w.saturating_sub(count_w + 1),
                        &count,
                        keep(row_style.patch(muted)),
                    );
                }
            }
            Row::NoPanels => put(buf, y, 3, "no panels", keep(row_style.patch(muted))),
        }
    }
    if bar_rows(area.height) > 0 {
        let y = area.y + area.height - 1;
        let style = if app.hover == Some(Hover::NewProject) {
            selection(theme, base)
        } else {
            muted
        };
        let text = chip(w);
        let cells = width(text);
        buf.set_style(Rect::new(area.x + 1, y, cells, 1), style);
        put(buf, y, 1, text, style);
    }
}
