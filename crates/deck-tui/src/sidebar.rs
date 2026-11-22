//! The sidebar: its rows, the keyboard focus and mouse handling that belong to it, and its
//! drawing.

use deck_core::action::Action as Act;
use deck_core::keys::{KeyCode, KeyEvent};
use deck_core::model::{PanelId, ProjectId, SessionView};
use deck_core::theme::Theme;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event as ct;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::app::{App, Out};
use crate::draw::{color, truncate, width};

/// The panel area never gets narrower than this; the sidebar hides instead.
pub const MIN_PANEL_COLS: u16 = 20;
/// The session name and a blank row above the project rows.
pub const HEADER_ROWS: u16 = 2;
/// Rows the wheel scrolls per notch.
const WHEEL_ROWS: usize = 3;
/// What the status bar shows while the sidebar has the keyboard.
pub const SIDEBAR_HINTS: &str = "↑↓ move  ⏎ open  ←→ fold  c new  r rename  x close  esc back";

/// The row the sidebar selection is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sel {
    Project(ProjectId),
    Panel(ProjectId, PanelId),
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
        usize::from(self.size.1).saturating_sub(usize::from(1 + HEADER_ROWS))
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
        self.selection = Some(rows[to]);
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
    for y in area.y..area.y + area.height {
        buf.set_string(
            area.x + area.width,
            y,
            "│",
            Style::new().fg(color(theme.border)),
        );
    }
    let muted = base.fg(color(theme.muted));
    let accent = base.fg(color(theme.accent));
    let w = area.width;
    let put = |buf: &mut Buffer, y: u16, x: u16, text: &str, style: Style| {
        buf.set_stringn(area.x + x, y, text, usize::from(w.saturating_sub(x)), style);
    };

    let header = truncate(&view.name, w.saturating_sub(2));
    put(buf, area.y, 1, &header, base.add_modifier(Modifier::BOLD));

    let rows = rows(view);
    let height = usize::from(area.height.saturating_sub(HEADER_ROWS));
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
        if row.sel().is_some() && row.sel() == selected {
            row_style = row_style.add_modifier(Modifier::REVERSED);
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
                put(buf, y, 1, glyph, row_style.patch(muted));
                let (count, count_w) = if p.collapsed {
                    side(p.panels.len().to_string())
                } else {
                    (String::new(), 0)
                };
                let room = w.saturating_sub(4 + if count_w > 0 { count_w + 1 } else { 0 });
                let mut style = row_style;
                if view.active_project == Some(id) {
                    style = style.add_modifier(Modifier::BOLD);
                }
                put(buf, y, 3, &truncate(&p.name, room), style);
                if count_w > 0 {
                    put(
                        buf,
                        y,
                        w.saturating_sub(count_w + 1),
                        &count,
                        row_style.patch(muted),
                    );
                }
            }
            Row::Panel(_, id) => {
                let Some((_, p)) = app.mirror.find_panel(id) else {
                    continue;
                };
                let active = view.active_panel == Some(id);
                let mut style = row_style;
                if active {
                    put(buf, y, 0, &theme.glyphs.active, row_style.patch(accent));
                    style = style.patch(accent).add_modifier(Modifier::BOLD);
                }
                let (count, count_w) = if p.terminals.len() > 1 {
                    side(p.terminals.len().to_string())
                } else {
                    (String::new(), 0)
                };
                let room = w.saturating_sub(4 + if count_w > 0 { count_w + 1 } else { 0 });
                put(buf, y, 3, &truncate(&p.name, room), style);
                if count_w > 0 {
                    put(
                        buf,
                        y,
                        w.saturating_sub(count_w + 1),
                        &count,
                        row_style.patch(muted),
                    );
                }
            }
            Row::NoPanels => put(buf, y, 3, "no panels", row_style.patch(muted)),
        }
    }
}
