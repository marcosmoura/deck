//! The sidebar: its rows, the keyboard focus and mouse handling that belong to it, and its
//! drawing.

use deck_core::action::Action as Act;
use deck_core::icons::{IconKind, icon_for};
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
use crate::convert::image_id_color;
use crate::draw::{color, selection, truncate, width};
use crate::graphics::{Logo, Pair, placeholder};

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
/// The column the icon of a panel row starts in, and the one its name starts in with icons.
const ICON_X: u16 = 3;
const NAME_X_ICONS: u16 = ICON_X + ICON_CELLS + 1;

/// Rows the bottom bar and its spacer take in a sidebar `height` rows tall.
fn bar_rows(height: u16) -> u16 {
    if height >= BAR_MIN_HEIGHT {
        BAR_ROWS
    } else {
        0
    }
}

/// The `New space` chip of the header: `" + New space "`, or `" + "` in a narrow sidebar.
fn new_project_chip(width: u16) -> &'static str {
    if width < BAR_WIDE {
        " + "
    } else {
        " + New space "
    }
}

/// The `New` chip: `" + New "`, or `" + "` in a narrow sidebar.
fn new_chip(width: u16) -> &'static str {
    if width < BAR_WIDE { " + " } else { " + New " }
}

/// The columns the `New space` chip takes in a sidebar `w` columns wide: right-aligned on
/// the header row, with a margin of one column.
fn new_project_span(w: u16) -> std::ops::Range<u16> {
    let cells = width(new_project_chip(w));
    let x = w.saturating_sub(1 + cells);
    x..x + cells
}

/// The columns the `New` chip takes: from the second column.
fn new_span(w: u16) -> std::ops::Range<u16> {
    1..1 + width(new_chip(w))
}

/// The `Resume` chip: `" Resume "`, or `" ↻ "` in a narrow sidebar.
fn restore_chip(width: u16) -> &'static str {
    if width < BAR_WIDE {
        " ↻ "
    } else {
        " Resume "
    }
}

/// The columns the `Resume` chip takes in a sidebar `w` columns wide: right-aligned, with a
/// margin of one column.
fn restore_span(w: u16) -> std::ops::Range<u16> {
    let cells = width(restore_chip(w));
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
}

impl Row {
    pub(crate) fn sel(self) -> Option<Sel> {
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

/// The kind of icon of a panel: the one of its source terminal's agent or program; none with
/// `icons = "none"` or without terminals.
pub fn panel_icon_kind(panel: &PanelView, theme: &Theme) -> Option<IconKind> {
    if theme.icons == IconPreset::None {
        return None;
    }
    let term = source_terminal_view(panel)?;
    Some(icon_for(
        term.agent.as_ref().map(|a| a.kind),
        Some(&term.program),
    ))
}

/// The style of an icon of `kind` in a row styled `style`: its text in the kind's colour
/// (`[theme.icon_colors]`) when it has one. On a reversed row the colour goes in the
/// background, which is what a reversed cell shows as text.
pub fn icon_style(theme: &Theme, kind: IconKind, style: Style) -> Style {
    let Some(c) = theme.icon_colors.get(kind) else {
        return style;
    };
    let reversed = style.add_modifier.contains(Modifier::REVERSED)
        && !style.sub_modifier.contains(Modifier::REVERSED);
    if reversed {
        style.bg(color(c))
    } else {
        style.fg(color(c))
    }
}

/// The icon of an icon kind as it is drawn, two cells wide, with the style to draw it in. For a
/// logo that was sent, the placeholder cells: the visible colours of `style`, no modifiers, the
/// id as foreground (see [`crate::graphics::Images::cell_for`]); else the text icon and a
/// blank cell, in `style`.
pub fn kind_icon_cell(app: &App, theme: &Theme, kind: IconKind, style: Style) -> (String, Style) {
    let style = icon_style(theme, kind, style);
    let image = app.logos().and_then(|images| {
        let (rgb, cell) = images.cell_for(style);
        let pair = Pair {
            logo: Logo::of(kind)?,
            rgb,
        };
        images.id(&pair).map(|id| cell.fg(image_id_color(id)))
    });
    match image {
        Some(cell) => (placeholder(), cell),
        None => (format!("{} ", theme.icon_glyphs.get(kind)), style),
    }
}

/// The icon of a panel as it is drawn (see [`kind_icon_cell`]); none with `icons = "none"` or
/// without terminals.
pub fn panel_icon_cell(
    app: &App,
    panel: &PanelView,
    theme: &Theme,
    style: Style,
) -> Option<(String, Style)> {
    let kind = panel_icon_kind(panel, theme)?;
    Some(kind_icon_cell(app, theme, kind, style))
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
    put(buf, area.y, span.start, new_project_chip(w), style);

    let rows = rows(view);
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
        let band = is_selected || (active && !app.sidebar_focus);
        let hovered = !band
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
            if band && theme.selection_fg != none {
                style.fg(color(theme.selection_fg))
            } else if band && theme.selection_bg == none {
                // Reversed: a colour would show as a block, so the text colour stays.
                style.fg(color(theme.sidebar_fg))
            } else {
                style
            }
        };
        if band {
            row_style = selection(theme, row_style);
            buf.set_style(Rect::new(area.x, y, w, 1), row_style);
        } else if hovered {
            row_style = row_style.bg(hover_bg);
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
                let (count, count_w) = if p.collapsed {
                    side(p.panels.len().to_string())
                } else {
                    (String::new(), 0)
                };
                // `!n` after the name of a collapsed project with n waiting panels.
                let waiting = if p.collapsed {
                    agent::waiting_panels(p)
                } else {
                    0
                };
                let badge = (waiting > 0).then(|| format!("{}{waiting}", theme.glyphs.waiting));
                let badge_w = badge.as_deref().map_or(0, |b| width(b) + 1);
                let room = w
                    .saturating_sub(4 + if count_w > 0 { count_w + 1 } else { 0 })
                    .saturating_sub(badge_w);
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
            }
            Row::Panel(_, id) => {
                let Some((_, p)) = app.mirror.find_panel(id) else {
                    continue;
                };
                let mut style = panel_style(theme, base, band, active);
                if hovered {
                    style = style.bg(hover_bg);
                }
                // The status column: the panel's most urgent agent.
                if let Some((text, glyph_style)) =
                    agent::glyph(agent::panel_status(p), app.spin, theme)
                {
                    // The muted `quiet` glyph turns to the sidebar text under the pointer.
                    let glyph_style = if hovered && glyph_style.fg == Some(color(theme.muted)) {
                        fg(theme.sidebar_fg)
                    } else {
                        glyph_style
                    };
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
                let icon = panel_icon_cell(app, p, theme, style);
                // With icons on the names all start in the same column.
                let name_x = if theme.icons == IconPreset::None {
                    ICON_X
                } else {
                    NAME_X_ICONS
                };
                let room = w
                    .saturating_sub(name_x + 1 + if count_w > 0 { count_w + 1 } else { 0 })
                    .saturating_sub(marker_w);
                if p.unseen {
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
                    put(
                        buf,
                        y,
                        name_x + 1 + name_end,
                        &theme.glyphs.unseen,
                        keep(row_style.patch(accent_fg)),
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
        let style = style_of(Hover::New);
        let text = new_chip(w);
        let cells = width(text);
        buf.set_style(Rect::new(area.x + 1, y, cells, 1), style);
        put(buf, y, 1, text, style);
        let style = style_of(Hover::Restore);
        let span = restore_span(w);
        buf.set_style(
            Rect::new(area.x + span.start, y, span.end - span.start, 1),
            style,
        );
        put(buf, y, span.start, restore_chip(w), style);
    }
}
