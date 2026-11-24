//! The context menu: a small overlay opened by a right click on a terminal of the active
//! panel, a panel row or a space row, at the pointer.

use std::time::Instant;

use deck_core::action::Action as Act;
use deck_core::keys::{KeyCode, KeyEvent};
use deck_core::model::{PanelId, ProjectId, TerminalId};
use deck_core::theme::Theme;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event as ct;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, Clear, Widget};

use crate::app::{App, Out};
use crate::draw::{border_type, color, selection, width};
use crate::menu::item_style;
use crate::sidebar::Sel;

const TERMINAL_ITEMS: &[&str] = &["New terminal", "Rename panel", "Close terminal"];
const PANEL_ITEMS: &[&str] = &["Rename", "Close"];
const PROJECT_ITEMS: &[&str] = &["Rename", "New panel", "Close"];
/// Columns inside the borders, at least.
const MIN_INNER: u16 = 16;
/// What an item's row takes besides its label: a padding cell and the digit's two cells on
/// either side.
const ROW_CHROME: u16 = 4;

/// What the menu acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextTarget {
    Terminal(TerminalId),
    Panel(PanelId),
    Project(ProjectId),
}

/// The open context menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextMenu {
    pub target: ContextTarget,
    /// The name of the panel or space when the menu opened, for the closing message.
    pub name: String,
    /// The pressed cell: the menu's top-left corner, and the New menu's anchor.
    pub at: (u16, u16),
    pub selected: usize,
    /// The sidebar row the menu was opened from; none when opened from the grid.
    row: Option<Sel>,
}

impl ContextMenu {
    pub fn items(&self) -> &'static [&'static str] {
        match self.target {
            ContextTarget::Terminal(_) => TERMINAL_ITEMS,
            ContextTarget::Panel(_) => PANEL_ITEMS,
            ContextTarget::Project(_) => PROJECT_ITEMS,
        }
    }

    /// The sidebar row the menu was opened from, which keeps its hover background.
    pub(crate) fn row(&self) -> Option<Sel> {
        self.row
    }
}

impl App {
    fn open_context(
        &mut self,
        target: ContextTarget,
        name: String,
        at: (u16, u16),
        row: Option<Sel>,
    ) {
        self.context = Some(ContextMenu {
            target,
            name,
            at,
            selected: 0,
            row,
        });
        self.swallow_release = true;
        self.dirty = true;
    }

    /// Opens the menu of a terminal of the active panel.
    pub(crate) fn open_terminal_menu(&mut self, terminal: TerminalId, at: (u16, u16)) {
        if self.mirror.find_terminal(terminal).is_some() {
            self.open_context(ContextTarget::Terminal(terminal), String::new(), at, None);
        }
    }

    /// Opens the menu of a sidebar row.
    pub(crate) fn open_row_menu(&mut self, sel: Sel, at: (u16, u16)) {
        match sel {
            Sel::Panel(_, panel) => {
                if let Some((_, p)) = self.mirror.find_panel(panel) {
                    let name = p.name.clone();
                    self.open_context(ContextTarget::Panel(panel), name, at, Some(sel));
                }
            }
            Sel::Terminal(_, _, terminal) => {
                if self.mirror.find_terminal(terminal).is_some() {
                    self.open_context(
                        ContextTarget::Terminal(terminal),
                        String::new(),
                        at,
                        Some(sel),
                    );
                }
            }
            Sel::Project(project) => {
                if let Some(p) = self.mirror.find_project(project) {
                    let name = p.name.clone();
                    self.open_context(ContextTarget::Project(project), name, at, Some(sel));
                }
            }
        }
    }

    fn close_context(&mut self) {
        if self.context.take().is_some() {
            self.dirty = true;
        }
    }

    /// Where the menu is: its corner on the pressed cell, moved left and up to stay in the
    /// window.
    pub(crate) fn context_rect(&self) -> Option<Rect> {
        let menu = self.context.as_ref()?;
        let (cols, rows) = self.size;
        let items = menu.items();
        let longest = items.iter().map(|i| width(i)).max().unwrap_or(0);
        let w = (longest + ROW_CHROME).max(MIN_INNER) + 2;
        let h = u16::try_from(items.len()).unwrap_or(0) + 2;
        let (w, h) = (w.min(cols), h.min(rows));
        let x = menu.at.0.min(cols - w);
        let y = menu.at.1.min(rows - h);
        Some(Rect::new(x, y, w, h))
    }

    fn context_item_at(&self, column: u16, row: u16) -> Option<usize> {
        let rect = self.context_rect()?;
        let items = self.context.as_ref()?.items().len();
        let inside = column > rect.x && column + 1 < rect.x + rect.width;
        let line = usize::from(row.checked_sub(rect.y + 1)?);
        (inside && line < items && row + 1 < rect.y + rect.height).then_some(line)
    }

    fn context_choose(&mut self, index: usize) -> Vec<Out> {
        let Some(menu) = self.context.take() else {
            return Vec::new();
        };
        self.dirty = true;
        match (menu.target, index) {
            (ContextTarget::Terminal(terminal), 0) => {
                if let Some((panel, _, _)) = self.mirror.find_terminal(terminal) {
                    let panel = panel.id;
                    vec![
                        self.act(Act::FocusPanel { panel }, None),
                        self.act(Act::NewTerminal { panel: Some(panel) }, None),
                    ]
                } else {
                    self.set_flash(
                        "no panel to add a terminal to".into(),
                        Instant::now(),
                        false,
                    );
                    Vec::new()
                }
            }
            (ContextTarget::Terminal(terminal), 1) => {
                if let Some((panel, _, _)) = self.mirror.find_terminal(terminal) {
                    let panel = panel.id;
                    self.open_rename_panel(panel);
                }
                Vec::new()
            }
            (ContextTarget::Terminal(terminal), 2) => {
                if self.mirror.find_terminal(terminal).is_none() {
                    return Vec::new();
                }
                self.confirm_close_terminal(terminal, Instant::now())
            }
            (ContextTarget::Panel(panel), 0) => {
                self.open_rename_panel(panel);
                Vec::new()
            }
            (ContextTarget::Panel(panel), 1) => {
                self.confirm_close_panel(panel);
                Vec::new()
            }
            (ContextTarget::Project(project), 0) => {
                self.open_rename_project(project);
                Vec::new()
            }
            (ContextTarget::Project(project), 1) => {
                self.open_new_menu_at(project, Some(menu.at));
                Vec::new()
            }
            (ContextTarget::Project(project), 2) => {
                self.confirm_close_project(project);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// A key while the menu is open; it never goes anywhere else.
    pub(crate) fn context_key(&mut self, key: KeyEvent) -> Vec<Out> {
        let Some(menu) = self.context.as_mut() else {
            return Vec::new();
        };
        let ctrl = key.mods.ctrl;
        if key.mods.alt {
            return Vec::new();
        }
        let len = menu.items().len();
        let up = match key.code {
            KeyCode::Up | KeyCode::Char('k') => !ctrl,
            KeyCode::Char('p') => ctrl,
            _ => false,
        };
        let down = match key.code {
            KeyCode::Down | KeyCode::Char('j') => !ctrl,
            KeyCode::Char('n') => ctrl,
            _ => false,
        };
        match key.code {
            _ if up => menu.selected = menu.selected.saturating_sub(1),
            _ if down => menu.selected = (menu.selected + 1).min(len - 1),
            KeyCode::Enter => {
                let index = menu.selected;
                return self.context_choose(index);
            }
            KeyCode::Char(c @ '1'..='9') if !ctrl => {
                let index = usize::from(c as u8 - b'1');
                if index < len {
                    return self.context_choose(index);
                }
            }
            KeyCode::Esc | KeyCode::Char('q') if !ctrl => {
                self.close_context();
                self.refresh_hover();
            }
            _ => {}
        }
        Vec::new()
    }

    /// A mouse event while the menu is open: it has the mouse before everything else but the
    /// Restore dialog.
    pub(crate) fn context_mouse(&mut self, event: ct::MouseEvent) -> Vec<Out> {
        let item = self.context_item_at(event.column, event.row);
        match event.kind {
            ct::MouseEventKind::Moved => {
                if let (Some(i), Some(menu)) = (item, self.context.as_mut())
                    && menu.selected != i
                {
                    menu.selected = i;
                    self.dirty = true;
                }
                Vec::new()
            }
            ct::MouseEventKind::Down(button) => {
                // The rest of the click (its release) goes to nobody.
                self.swallow_release = true;
                if let (Some(i), ct::MouseButton::Left) = (item, button) {
                    return self.context_choose(i);
                }
                let inside = self.context_rect().is_some_and(|r| {
                    (r.x..r.x + r.width).contains(&event.column)
                        && (r.y..r.y + r.height).contains(&event.row)
                });
                if !inside {
                    self.close_context();
                    self.hover = None;
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Closes the menu when its target is gone: with a message for a panel or a space, silently
    /// for a terminal.
    pub(crate) fn check_context_target(&mut self, now: Instant) {
        let Some(menu) = &self.context else { return };
        let (gone, what) = match menu.target {
            ContextTarget::Terminal(t) => (self.mirror.find_terminal(t).is_none(), ""),
            ContextTarget::Panel(p) => (self.mirror.find_panel(p).is_none(), "panel"),
            ContextTarget::Project(p) => (self.mirror.find_project(p).is_none(), "space"),
        };
        if !gone {
            return;
        }
        let text = format!("{what} \"{}\" was closed", menu.name);
        self.context = None;
        self.dirty = true;
        if !what.is_empty() {
            self.set_flash(text, now, false);
        }
    }
}

/// Draws the menu over the panels and the sidebar, under the dialogs.
pub fn draw(buf: &mut Buffer, app: &App, theme: &Theme) {
    let (Some(menu), Some(rect)) = (&app.context, app.context_rect()) else {
        return;
    };
    Clear.render(rect, buf);
    let base = Style::new().bg(color(theme.overlay_bg));
    buf.set_style(rect, base);
    Block::bordered()
        .border_type(border_type(theme.border_style))
        .border_style(base.fg(color(theme.border)))
        .render(rect, buf);
    let inner_w = rect.width.saturating_sub(2);
    for (i, label) in menu.items().iter().enumerate() {
        let y = rect.y + 1 + u16::try_from(i).unwrap_or(0);
        if y + 1 >= rect.y + rect.height {
            break;
        }
        let selected = i == menu.selected;
        let style = item_style(theme, selected);
        buf.set_style(Rect::new(rect.x + 1, y, inner_w, 1), style);
        buf.set_stringn(
            rect.x + 2,
            y,
            label,
            usize::from(inner_w.saturating_sub(2)),
            style,
        );
        let digit = if selected {
            selection(theme, base)
        } else {
            base.fg(color(theme.muted))
        };
        buf.set_stringn(
            rect.x + rect.width.saturating_sub(3),
            y,
            (i + 1).to_string(),
            1,
            digit,
        );
    }
}
