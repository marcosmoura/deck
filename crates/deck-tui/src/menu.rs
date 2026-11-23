//! The New menu: a small overlay that starts a separate panel with a shell or an agent in a
//! project. It opens from the `+ New` button of the sidebar's bottom bar, from `prefix N` and
//! from `n` in sidebar focus.

use std::time::Instant;

use deck_core::action::{Action as Act, PanelLaunch};
use deck_core::agents::AgentKind;
use deck_core::icons::{Icon, IconKind};
use deck_core::keys::{KeyCode, KeyEvent};
use deck_core::model::ProjectId;
use deck_core::theme::Theme;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event as ct;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, Clear, Widget};

use crate::app::{App, Out};
use crate::draw::{border_type, color, selection, truncate, width};
use crate::sidebar::{ICON_CELLS, icon_slot};

/// The menu's items, in order; the digit that chooses one is its position from 1.
pub const ITEMS: [&str; 4] = ["Terminal", "Claude", "OpenCode", "Codex"];
/// The icon of each item, in the order of [`ITEMS`].
pub const ICONS: [IconKind; 4] = [
    IconKind::Terminal,
    IconKind::Claude,
    IconKind::OpenCode,
    IconKind::Codex,
];
/// Columns inside the borders, at least.
const INNER: u16 = 20;
/// Rows with the borders: the items and the hint row.
const HEIGHT: u16 = 7;
/// Columns the longest project name shown in the title may take.
const NAME_MAX: u16 = 24;
/// What the top border takes besides the name: the corners, a dash, `" New in "`, a space
/// after it and a dash.
const TITLE_CHROME: u16 = 13;
const HINT: &str = " ⏎ choose  esc close";

/// The open menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMenu {
    pub project: ProjectId,
    /// The project's name when the menu opened, for the title and the closing message.
    pub name: String,
    /// The highlighted item.
    pub selected: usize,
    /// Where the menu's top-left corner sits, when it was opened at a point.
    pub anchor: Option<(u16, u16)>,
}

/// What an item sends for `project`.
pub fn item_action(project: ProjectId, index: usize) -> Option<Act> {
    let agent = |kind| Act::NewPanelWith {
        project,
        launch: PanelLaunch::Agent(kind),
    };
    match index {
        0 => Some(Act::NewPanel {
            project,
            name: None,
        }),
        1 => Some(agent(AgentKind::Claude)),
        2 => Some(agent(AgentKind::OpenCode)),
        3 => Some(agent(AgentKind::Codex)),
        _ => None,
    }
}

impl App {
    /// Opens the menu for a project; nothing for one that is not there.
    pub(crate) fn open_new_menu(&mut self, project: ProjectId) {
        self.open_new_menu_at(project, None);
    }

    /// Opens the menu for a project with its corner on a cell of the window.
    pub(crate) fn open_new_menu_at(&mut self, project: ProjectId, anchor: Option<(u16, u16)>) {
        let Some(p) = self.mirror.find_project(project) else {
            return;
        };
        let name = p.name.clone();
        self.menu = Some(NewMenu {
            project,
            name,
            selected: 0,
            anchor,
        });
        self.dirty = true;
    }

    fn close_menu(&mut self) {
        if self.menu.take().is_some() {
            self.dirty = true;
        }
    }

    /// The window the menu sits in.
    fn menu_area(&self) -> (u16, u16) {
        self.size
    }

    /// Where the menu is: above the sidebar's bottom bar with its left edge on the sidebar's
    /// first column (the bottom of the window area when the sidebar has no bar), clamped to
    /// the window, and at the top right of the window when the sidebar is not on screen.
    pub(crate) fn menu_rect(&self) -> Option<Rect> {
        let menu = self.menu.as_ref()?;
        let (cols, rows) = self.menu_area();
        let name = truncate(&menu.name, NAME_MAX.min(cols.saturating_sub(TITLE_CHROME)));
        let w = (width(&name) + TITLE_CHROME).max(INNER + 2).min(cols);
        let h = HEIGHT.min(rows);
        let (x, y) = if let Some((x, y)) = menu.anchor {
            (x, y.min(rows - h))
        } else if self.sidebar_visible() {
            let anchor = if rows >= crate::sidebar::BAR_MIN_HEIGHT {
                rows - 1
            } else {
                rows
            };
            (0, anchor.saturating_sub(h))
        } else {
            (cols - w, 0)
        };
        Some(Rect::new(x.min(cols - w), y, w, h))
    }

    /// The item under a cell of the window, when the menu has one there.
    fn menu_item_at(&self, column: u16, row: u16) -> Option<usize> {
        let rect = self.menu_rect()?;
        let inside = column > rect.x && column + 1 < rect.x + rect.width;
        let line = usize::from(row.checked_sub(rect.y + 1)?);
        (inside && line < ITEMS.len() && row + 1 < rect.y + rect.height).then_some(line)
    }

    fn menu_choose(&mut self, index: usize) -> Vec<Out> {
        let Some(menu) = self.menu.take() else {
            return Vec::new();
        };
        self.dirty = true;
        let Some(action) = item_action(menu.project, index) else {
            return Vec::new();
        };
        let mut outs = vec![self.act(action, None)];
        // The keyboard leaves the sidebar once the daemon accepted the launch.
        self.launch_result = Some(self.pending.len().saturating_sub(1));
        // The new panel must be seen: a folded project unfolds.
        if self
            .mirror
            .find_project(menu.project)
            .is_some_and(|p| p.collapsed)
        {
            outs.push(self.act(
                Act::SetProjectCollapsed {
                    project: menu.project,
                    collapsed: false,
                },
                None,
            ));
        }
        outs
    }

    /// A key while the menu is open; it never goes anywhere else.
    pub(crate) fn menu_key(&mut self, key: KeyEvent) -> Vec<Out> {
        let Some(menu) = self.menu.as_mut() else {
            return Vec::new();
        };
        let ctrl = key.mods.ctrl;
        if key.mods.alt {
            return Vec::new();
        }
        let last = ITEMS.len() - 1;
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
            _ if down => menu.selected = (menu.selected + 1).min(last),
            KeyCode::Enter => {
                let index = menu.selected;
                return self.menu_choose(index);
            }
            KeyCode::Char(c @ '1'..='4') if !ctrl => {
                let index = usize::from(c as u8 - b'1');
                return self.menu_choose(index);
            }
            KeyCode::Esc | KeyCode::Char('q') if !ctrl => {
                self.close_menu();
                self.refresh_hover();
            }
            _ => {}
        }
        Vec::new()
    }

    /// A mouse event while the menu is open: the menu has it before the sidebar.
    pub(crate) fn menu_mouse(&mut self, event: ct::MouseEvent) -> Vec<Out> {
        let item = self.menu_item_at(event.column, event.row);
        match event.kind {
            ct::MouseEventKind::Moved => {
                if let (Some(i), Some(menu)) = (item, self.menu.as_mut())
                    && menu.selected != i
                {
                    menu.selected = i;
                    self.dirty = true;
                }
                Vec::new()
            }
            ct::MouseEventKind::Down(_) => {
                // The rest of the click (its release) goes to nobody.
                self.swallow_release = true;
                if let (Some(i), ct::MouseEventKind::Down(ct::MouseButton::Left)) =
                    (item, event.kind)
                {
                    return self.menu_choose(i);
                }
                let inside = self.menu_rect().is_some_and(|r| {
                    (r.x..r.x + r.width).contains(&event.column)
                        && (r.y..r.y + r.height).contains(&event.row)
                });
                if !inside {
                    self.close_menu();
                    self.hover = None;
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Closes the menu with a message when its project is gone.
    pub(crate) fn check_menu_target(&mut self, now: Instant) {
        let Some(menu) = &self.menu else { return };
        if self.mirror.find_project(menu.project).is_some() {
            return;
        }
        let text = format!("space \"{}\" was closed", menu.name);
        self.menu = None;
        self.set_flash(text, now, false);
    }
}

/// The style of a menu item, highlighted or not.
pub(crate) fn item_style(theme: &Theme, selected: bool) -> Style {
    let base = Style::new().bg(color(theme.overlay_bg));
    if selected {
        selection(theme, base)
    } else {
        base
    }
}

/// Draws the menu over everything else but a dialog and the floating bar.
pub fn draw(buf: &mut Buffer, app: &App, theme: &Theme) {
    let (Some(menu), Some(rect)) = (&app.menu, app.menu_rect()) else {
        return;
    };
    Clear.render(rect, buf);
    let base = Style::new().bg(color(theme.overlay_bg));
    buf.set_style(rect, base);
    let border = base.fg(color(theme.border));
    Block::bordered()
        .border_type(border_type(theme.border_style))
        .border_style(border)
        .render(rect, buf);
    let name = truncate(
        &menu.name,
        NAME_MAX.min(rect.width.saturating_sub(TITLE_CHROME)),
    );
    buf.set_stringn(
        rect.x + 2,
        rect.y,
        format!(" New in {name} "),
        usize::from(rect.width.saturating_sub(3)),
        border,
    );
    let inner_w = rect.width.saturating_sub(2);
    for (i, label) in ITEMS.iter().enumerate() {
        let y = rect.y + 1 + u16::try_from(i).unwrap_or(0);
        if y + 1 >= rect.y + rect.height {
            break;
        }
        let selected = i == menu.selected;
        let style = item_style(theme, selected);
        buf.set_style(Rect::new(rect.x + 1, y, inner_w, 1), style);
        // ` <icon, two cells> <label>`.
        let (icon, icon_style) = ICONS
            .get(i)
            .map(|kind| icon_slot(app, Icon::Kind(*kind), style))
            .unwrap_or_default();
        buf.set_stringn(rect.x + 2, y, icon, usize::from(ICON_CELLS), icon_style);
        let label_x = rect.x + 2 + ICON_CELLS + 1;
        buf.set_stringn(
            label_x,
            y,
            label,
            usize::from((rect.x + 1 + inner_w).saturating_sub(label_x)),
            style,
        );
        buf.set_stringn(
            rect.x + rect.width.saturating_sub(3),
            y,
            (i + 1).to_string(),
            1,
            style,
        );
    }
    let hint_y = rect.y + 1 + u16::try_from(ITEMS.len()).unwrap_or(0);
    if hint_y + 1 < rect.y + rect.height {
        buf.set_stringn(
            rect.x + 1,
            hint_y,
            HINT,
            usize::from(inner_w),
            base.fg(color(theme.muted)),
        );
    }
}
