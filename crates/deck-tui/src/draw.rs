//! Drawing: the terminal's frame and contents, the status bar and the too-small message.
//! Every colour and glyph of deck's own UI comes from the [`Theme`].

use deck_core::config::HelpGroup;
use deck_core::model::{NameSource, TerminalId, TerminalView};
use deck_core::naming::source_terminal_view;
use deck_core::protocol::{CursorShape, NoticeLevel};
use deck_core::theme::{BorderStyle, Theme};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Clear, Widget};

use crate::agent;
use crate::app::{App, too_small};
pub(crate) use crate::convert::{color, downsample, style};
use crate::grid::Grid;
use crate::input::Action;
use crate::mirror::TermMirror;
use crate::modal::{Modal, Prompt};
use crate::sidebar::{self, SIDEBAR_HINTS};

pub const TOO_SMALL: &str = "deck: window too small";
/// The status-bar hints while scroll mode is on.
pub const SCROLL_HINTS: &str = "q exit  g top  G bottom";

pub(crate) fn width(text: &str) -> u16 {
    u16::try_from(Span::raw(text).width()).unwrap_or(u16::MAX)
}

/// `text` cut to at most `max` columns, with `…` in place of what was cut.
pub(crate) fn truncate(text: &str, max: u16) -> String {
    if width(text) <= max {
        return text.to_string();
    }
    let mut out = String::new();
    for c in text.chars() {
        let next = format!("{out}{c}");
        if width(&next) + 1 > max {
            break;
        }
        out = next;
    }
    if max > 0 {
        out.push('…');
    }
    out
}

/// Draws the whole screen; the cursor is shown where the focused terminal's cursor is.
pub fn draw(frame: &mut Frame, app: &App) {
    let theme = &app.theme;
    let area = frame.area();
    if too_small((area.width, area.height)) {
        draw_too_small(frame.buffer_mut(), area);
        return;
    }
    let side = app.sidebar_cols();
    if side > 0 {
        sidebar::draw(
            frame.buffer_mut(),
            Rect::new(0, 0, side - 1, area.height - 1),
            app,
            theme,
        );
    }
    let panel = Rect::new(side, 0, area.width - side, area.height - 1);
    let cursor = if app.panel_area_empty() {
        draw_empty(frame.buffer_mut(), panel, app, theme);
        None
    } else {
        draw_panel(frame.buffer_mut(), panel, app)
    };
    let bar = Rect::new(0, area.height - 1, area.width, 1);
    draw_status(frame.buffer_mut(), bar, app, theme);
    if matches!(app.modal, Some(Modal::Help)) {
        draw_help(
            frame.buffer_mut(),
            Rect::new(0, 0, area.width, area.height - 1),
            app,
            theme,
        );
    }
    if let Some(picker) = &app.picker {
        crate::picker::draw(
            frame.buffer_mut(),
            Rect::new(0, 0, area.width, area.height - 1),
            app,
            picker,
            theme,
        );
    }
    crate::menu::draw(frame.buffer_mut(), app, theme);
    crate::restore::draw(frame.buffer_mut(), app, theme);
    if !app.truecolor {
        downsample(frame.buffer_mut());
    }
    if !app.overlay_open()
        && let Some(pos) = cursor
    {
        frame.set_cursor_position(pos);
    }
}

/// The ratatui border set of a theme's border style.
pub(crate) fn border_type(style: BorderStyle) -> BorderType {
    match style {
        BorderStyle::Rounded => BorderType::Rounded,
        BorderStyle::Plain => BorderType::Plain,
        BorderStyle::Thick => BorderType::Thick,
        BorderStyle::Double => BorderType::Double,
    }
}

/// `base` as a selected row: the theme's selection colours, or reversed while both are
/// `default`.
pub(crate) fn selection(theme: &Theme, base: Style) -> Style {
    let none = Default::default();
    if theme.selection_fg == none && theme.selection_bg == none {
        base.add_modifier(Modifier::REVERSED)
    } else {
        base.fg(color(theme.selection_fg))
            .bg(color(theme.selection_bg))
    }
}

/// The message centered in a panel area that has no panel.
fn draw_empty(buf: &mut Buffer, panel: Rect, app: &App, theme: &Theme) {
    let label = |action| app.bindings.key_label(action).unwrap_or_default();
    let text = match app.mirror.active_project() {
        Some(p) => format!(
            "No panels in {} — {} to create one",
            p.name,
            label(Action::NewPanel)
        ),
        None => format!("No projects — {} to add one", label(Action::NewProject)),
    };
    let text = truncate(&text, panel.width);
    let x = panel.x + panel.width.saturating_sub(width(&text)) / 2;
    buf.set_stringn(
        x,
        panel.y + panel.height / 2,
        &text,
        usize::from(panel.width),
        Style::new().fg(color(theme.muted)),
    );
}

/// The help overlay: every binding, grouped, in a rounded box in the middle.
/// The lines of the help overlay for a window area `room` rows tall inside the border.
pub(crate) fn help_lines(app: &App, theme: &Theme, room: usize) -> Vec<(String, Style)> {
    let rows = app.bindings.help_rows();
    let key_w = rows.iter().map(|r| width(&r.keys)).max().unwrap_or(0);
    let mut lines: Vec<(String, Style)> = Vec::new();
    let heading = Style::new()
        .fg(color(theme.accent))
        .add_modifier(Modifier::BOLD);
    // The group headings go when the window is too short for them and every binding.
    let headings = rows.len() + HelpGroup::ALL.len() <= room;
    for group in HelpGroup::ALL {
        let mut in_group = rows.iter().filter(|r| r.group == group).peekable();
        if in_group.peek().is_none() {
            continue;
        }
        if headings {
            lines.push((format!(" {}", group.title()), heading));
        }
        for r in in_group {
            let pad = " ".repeat(usize::from(key_w - width(&r.keys)));
            lines.push((format!("  {}{pad}  {}", r.keys, r.text), Style::new()));
        }
    }
    lines
}

/// Rows of the help list that fit in the overlay, and the lines of its content.
pub(crate) fn help_room(app: &App) -> (usize, usize) {
    let room = usize::from(app.size.1.saturating_sub(1)).saturating_sub(2);
    (room, help_lines(app, &app.theme, room).len())
}

fn draw_help(buf: &mut Buffer, area: Rect, app: &App, theme: &Theme) {
    let room = usize::from(area.height.saturating_sub(2));
    let lines = help_lines(app, theme, room);
    let inner_w = lines.iter().map(|(l, _)| width(l)).max().unwrap_or(0) + 2;
    let w = (inner_w + 2).min(area.width);
    let h = (u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .saturating_add(2))
    .min(area.height);
    let rect = Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    );
    Clear.render(rect, buf);
    let base = Style::new().bg(color(theme.overlay_bg));
    buf.set_style(rect, base);
    let shown = usize::from(h.saturating_sub(2));
    let top = app.help_scroll.min(lines.len().saturating_sub(shown));
    let below = lines.len().saturating_sub(top + shown);
    let mut hint = String::new();
    if top > 0 {
        hint.push_str(&format!("↑ {top} more · "));
    }
    if below > 0 {
        hint.push_str(&format!("↓ {below} more · "));
    }
    Block::bordered()
        .border_type(border_type(theme.border_style))
        .border_style(base.fg(color(theme.accent)))
        .title(" Help ")
        .title_bottom(format!(" {hint}esc close "))
        .render(rect, buf);
    for (i, (line, style)) in lines.iter().skip(top).take(shown).enumerate() {
        buf.set_stringn(
            rect.x + 1,
            rect.y + 1 + u16::try_from(i).unwrap_or(0),
            line,
            usize::from(w.saturating_sub(2)),
            base.patch(*style),
        );
    }
}

fn draw_too_small(buf: &mut Buffer, area: Rect) {
    let mut lines: Vec<String> = Vec::new();
    for word in TOO_SMALL.split(' ') {
        match lines.last_mut() {
            Some(last) if width(last) + 1 + width(word) <= area.width => {
                last.push(' ');
                last.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    let rows = u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .min(area.height);
    let top = area.y + (area.height - rows) / 2;
    for (i, line) in lines.iter().take(usize::from(rows)).enumerate() {
        let x = area.x + area.width.saturating_sub(width(line)) / 2;
        let y = top + u16::try_from(i).unwrap_or(0);
        buf.set_stringn(x, y, line, usize::from(area.width), Style::new());
    }
}

/// Lays the terminal out for the size the daemon used, then copies the part that fits the
/// panel area; returns the cursor position when it is visible there.
fn draw_panel(buf: &mut Buffer, panel: Rect, app: &App) -> Option<Position> {
    let (vw, vh) = app
        .mirror
        .view()
        .and_then(|v| v.layout_viewport)
        .unwrap_or((panel.width, panel.height));
    if vw == 0 || vh == 0 {
        return None;
    }
    let term = app
        .mirror
        .focused_id()
        .and_then(|id| app.mirror.terminal(id));

    let mut tmp = Buffer::empty(Rect::new(0, 0, vw, vh));
    let grid = Grid::of(app).filter(Grid::bordered);
    let cursor = match &grid {
        Some(grid) => draw_grid(&mut tmp, grid, app),
        None => {
            let area = tmp.area;
            draw_terminal(&mut tmp, area, term);
            if let Some(id) = app.mirror.focused_id() {
                draw_selection(&mut tmp, area, app, id);
            }
            term.map(|t| t.cursor)
                .filter(|c| c.visible)
                .map(|c| Position::new(c.col, c.row))
        }
    };

    let (cw, ch) = (vw.min(panel.width), vh.min(panel.height));
    for y in 0..ch {
        for x in 0..cw {
            let cell = &tmp[(x, y)];
            // A wide character cut in half by the crop would spill over.
            if x + 1 == cw && x + 1 < vw && width(cell.symbol()) > 1 {
                buf[(panel.x + x, panel.y + y)].set_symbol(" ");
            } else {
                buf[(panel.x + x, panel.y + y)] = cell.clone();
            }
        }
    }

    let Position { x, y } = cursor?;
    (x < cw && y < ch).then_some(Position::new(panel.x + x, panel.y + y))
}

/// What a terminal is called on its border or in the status bar: its title, else its
/// program, then `exited <code>` once a kept terminal's process ended.
fn terminal_label(view: &TerminalView) -> String {
    let name = view
        .title
        .clone()
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| view.program.clone());
    match view.exit_status {
        Some(code) => format!("{name} · exited {code}"),
        None => name,
    }
}

/// Every terminal of a grid of two or more in its rect, with its own border and title;
/// returns where the focused terminal's cursor is, when visible.
fn draw_grid(buf: &mut Buffer, grid: &Grid, app: &App) -> Option<Position> {
    let theme = &app.theme;
    let panel = app.mirror.active_panel()?;
    let mut cursor = None;
    for (i, rect) in grid.rects.iter().enumerate() {
        if rect.width < 2 || rect.height < 2 {
            continue;
        }
        let focused = grid.focused == Some(i);
        let Some(view) = panel.terminals.get(i) else {
            continue;
        };
        // An exited kept terminal is drawn muted, focused or not.
        let (border, title_color) = if view.exit_status.is_some() {
            (theme.muted, theme.muted)
        } else if focused {
            (theme.border_focused, theme.accent)
        } else {
            (theme.border, theme.title)
        };
        let name = terminal_label(view);
        let area = Rect::new(rect.x, rect.y, rect.width, rect.height);
        Block::bordered()
            .border_type(border_type(theme.border_style))
            .border_style(Style::new().fg(color(border)))
            .render(area, buf);
        let title_style = Style::new().fg(color(title_color));
        let room = rect.width.saturating_sub(3);
        match agent::title_mark(view, app.spin, theme) {
            // An agent's glyph and name take the place of the number: `╭─ <glyph> claude · title ─╮`.
            Some((glyph, glyph_style, kind)) => {
                let (x, _) =
                    buf.set_stringn(rect.x + 2, rect.y, " ", usize::from(room), title_style);
                let (x, _) = buf.set_stringn(x, rect.y, &glyph, usize::from(room), glyph_style);
                let used = x.saturating_sub(rect.x + 2);
                let label = truncate(
                    &format!("{kind} · {name}"),
                    rect.width.saturating_sub(6 + width(&glyph)),
                );
                buf.set_stringn(
                    x,
                    rect.y,
                    format!(" {label} "),
                    usize::from(room.saturating_sub(used)),
                    title_style,
                );
            }
            // One border cell, then the title: `╭─ 1 · nvim ───╮`.
            None => {
                let label = truncate(&format!("{} · {name}", i + 1), rect.width.saturating_sub(6));
                buf.set_stringn(
                    rect.x + 2,
                    rect.y,
                    format!(" {label} "),
                    usize::from(room),
                    title_style,
                );
            }
        }
        let inner = grid.inner(i);
        let inner = Rect::new(inner.x, inner.y, inner.width, inner.height);
        let term = app.mirror.terminal(view.id);
        draw_terminal(buf, inner, term);
        draw_selection(buf, inner, app, view.id);
        if let Some(c) = term.map(|t| t.cursor).filter(|c| focused && c.visible)
            && c.col < inner.width
            && c.row < inner.height
        {
            cursor = Some(Position::new(inner.x + c.col, inner.y + c.row));
        }
    }
    // The border being dragged is drawn in the accent colour.
    if let Some(drag) = &app.drag {
        for (x, y) in grid.boundary_cells(drag.boundary) {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_fg(color(theme.accent));
            }
        }
    }
    cursor
}

/// A terminal's lines, from the top left of `area` and not past it.
fn draw_terminal(buf: &mut Buffer, area: Rect, term: Option<&TermMirror>) {
    let Some(term) = term else { return };
    for (row, line) in term.lines.iter().enumerate().take(usize::from(area.height)) {
        let y = area.y + u16::try_from(row).unwrap_or(0);
        for run in &line.runs {
            if run.col >= area.width {
                break;
            }
            buf.set_stringn(
                area.x + run.col,
                y,
                &run.text,
                usize::from(area.width - run.col),
                style(run.style),
            );
        }
    }
}

/// The text selected in terminal `id`, in the selection colours over its lines in `area`.
fn draw_selection(buf: &mut Buffer, area: Rect, app: &App, id: TerminalId) {
    if area.is_empty() {
        return;
    }
    let Some(sel) = app.text_sel.as_ref().filter(|s| s.terminal == id) else {
        return;
    };
    let Some(term) = app.mirror.terminal(id) else {
        return;
    };
    let Some((from, to)) = sel.range(&term.lines, term.size.0) else {
        return;
    };
    let style = selection(&app.theme, Style::new());
    for row in from.1..=to.1.min(area.height - 1) {
        let start = if row == from.1 { from.0 } else { 0 };
        let end = if row == to.1 { to.0 } else { u16::MAX };
        for col in start..=end.min(area.width - 1) {
            if let Some(cell) = buf.cell_mut((area.x + col, area.y + row)) {
                cell.set_style(style);
            }
        }
    }
}

fn draw_status(buf: &mut Buffer, bar: Rect, app: &App, theme: &Theme) {
    let base = Style::new()
        .fg(color(theme.status_fg))
        .bg(color(theme.status_bg));
    buf.set_style(bar, base);

    let flash_color = match app.flash_level {
        Some(NoticeLevel::Info) => theme.muted,
        Some(NoticeLevel::Warning) => theme.warning,
        Some(NoticeLevel::Error) => theme.error,
        None if app.flash_error => theme.error,
        None => theme.warning,
    };
    let flash_style = Style::new().fg(color(flash_color));
    let flash = app.flash.as_ref().map(|(text, _)| text.clone());
    match &app.modal {
        Some(Modal::Prompt(prompt)) => {
            draw_prompt(
                buf,
                bar,
                prompt,
                flash.map(|t| (t, flash_style)),
                base,
                theme,
            );
            return;
        }
        Some(Modal::Confirm(confirm)) => {
            let text = format!(" {}", confirm.question);
            buf.set_stringn(
                bar.x,
                bar.y,
                &text,
                usize::from(bar.width),
                base.fg(color(theme.warning)),
            );
            if let Some(text) = flash {
                let w = width(&text);
                let used = width(&confirm.question) + 2;
                if used + w < bar.width {
                    buf.set_stringn(
                        bar.x + bar.width - w,
                        bar.y,
                        &text,
                        usize::from(w),
                        base.patch(flash_style),
                    );
                }
            }
            return;
        }
        _ => {}
    }

    let name = app.mirror.session_name().unwrap_or("");
    let resizing = app.resize_until.is_some();
    let dragging_sidebar = app.sidebar_drag.is_some();
    let (badge, badge_style) = if app.armed
        || app.scroll_mode.is_some()
        || app.sidebar_focus
        || resizing
        || dragging_sidebar
    {
        let label = if dragging_sidebar {
            format!(" SIDEBAR {} ", app.sidebar_width)
        } else if app.armed {
            " PREFIX ".to_string()
        } else if resizing && app.scroll_mode.is_none() && !app.sidebar_focus {
            " RESIZE ".to_string()
        } else if app.scroll_mode.is_some() {
            " SCROLL ".to_string()
        } else {
            " SIDEBAR ".to_string()
        };
        (
            label,
            Style::new()
                .fg(color(theme.warning))
                .add_modifier(Modifier::REVERSED),
        )
    } else {
        (
            format!(" {name} "),
            Style::new()
                .fg(color(theme.accent))
                .add_modifier(Modifier::BOLD),
        )
    };
    let project = app.mirror.project_name().unwrap_or("");
    // `project › <status> <icon> <name>` like the sidebar row (the status only with an
    // agent; coloured after the text is drawn); a manually named panel keeps its name, so
    // its source terminal's title follows it when it says something else.
    let mut mark = None;
    let mut icon_mark = None;
    let location = match app.mirror.active_panel() {
        Some(panel) => {
            let cell = sidebar::panel_icon_cell(app, panel, theme, base.fg(color(theme.muted)));
            let image_style = cell
                .as_ref()
                .filter(|(text, _)| text.starts_with(crate::graphics::PLACEHOLDER))
                .map(|(_, style)| *style);
            let icon = cell.map_or_else(String::new, |(i, _)| format!("{i} "));
            let before = format!("{project} › ");
            let status = agent::glyph(agent::panel_status(panel), app.spin, theme).map_or_else(
                String::new,
                |(glyph, style)| {
                    let text = format!("{glyph} ");
                    mark = Some((width(&before), glyph, style));
                    text
                },
            );
            if let Some(style) = image_style {
                icon_mark = Some((width(&before) + width(&status), style));
            }
            let mut text = format!("{before}{status}{icon}{}", panel.name);
            let title = source_terminal_view(panel)
                .and_then(|t| t.title.as_deref())
                .filter(|t| !t.is_empty() && *t != panel.name);
            if panel.name_source == NameSource::Manual
                && !app.panel_area_empty()
                && let Some(title) = title
            {
                text.push_str(" · ");
                text.push_str(title);
            }
            // A kept terminal whose process ended says so.
            if let Some(code) = app.mirror.focused().and_then(|t| t.exit_status) {
                text.push_str(&format!(" · exited {code}"));
            }
            text
        }
        None => project.to_string(),
    };
    // "[offset/history]" while the focused terminal is scrolled back.
    let indicator = app
        .mirror
        .focused_id()
        .and_then(|id| app.mirror.terminal(id))
        .filter(|t| t.display_offset > 0)
        .map(|t| format!(" [{}/{}]", t.display_offset, t.history))
        .unwrap_or_default();
    let waiting = app.mirror.view().map_or(0, agent::waiting_terminals);
    let (right, right_style) = match &app.flash {
        // A long message (a config error with its location) is cut to the bar.
        Some((text, _)) => (truncate(text, bar.width), flash_style),
        None if app.restore.is_some() => (
            crate::restore::hints(app).to_string(),
            Style::new().fg(color(theme.muted)),
        ),
        None if app.scroll_mode.is_some() => (
            SCROLL_HINTS.to_string(),
            Style::new().fg(color(theme.muted)),
        ),
        None if app.sidebar_focus && app.scroll_mode.is_none() => (
            SIDEBAR_HINTS.to_string(),
            Style::new().fg(color(theme.muted)),
        ),
        None if resizing => (
            app.bindings.resize_hints(),
            Style::new().fg(color(theme.muted)),
        ),
        // The hints wait for the session, like the name and the project.
        None if app.mirror.view().is_none() => (String::new(), Style::new()),
        None => (
            app.bindings.hints(waiting > 0),
            Style::new().fg(color(theme.muted)),
        ),
    };
    // `n waiting` to the left of the right-hand text, unless a message takes its place.
    let waiting_text = (waiting > 0 && app.flash.is_none()).then(|| format!("{waiting} waiting"));
    let waiting_w = waiting_text
        .as_deref()
        .map_or(0, |t| width(t) + if right.is_empty() { 0 } else { 2 });

    let right_w = width(&right);
    let (x, _) = buf.set_stringn(
        bar.x,
        bar.y,
        &badge,
        usize::from(bar.width),
        base.patch(badge_style),
    );
    let indicator_w = width(&indicator).min(bar.width.saturating_sub(x - bar.x));
    buf.set_stringn(
        x,
        bar.y,
        &indicator,
        usize::from(indicator_w),
        base.fg(color(theme.warning)),
    );
    let x = x + indicator_w;
    let room = bar
        .width
        .saturating_sub(right_w + waiting_w + 1)
        .saturating_sub(x - bar.x);
    if room > 1 && !location.is_empty() {
        let text = format!(" {}", truncate(&location, room - 1));
        buf.set_stringn(
            x,
            bar.y,
            &text,
            usize::from(room),
            base.fg(color(theme.muted)),
        );
        // One column for the space before the text.
        if let Some((col, glyph, glyph_style)) = mark {
            let at = x + 1 + col;
            if at < x + room
                && buf
                    .cell((at, bar.y))
                    .is_some_and(|cell| cell.symbol() == glyph)
            {
                buf.set_stringn(at, bar.y, &glyph, 1, base.patch(glyph_style));
            }
        }
        // An image is two placeholder cells with its id as colour. When the text was cut
        // inside it, the half that is left is blanked: it would show half a logo.
        if let Some((col, icon_style)) = icon_mark {
            let at = x + 1 + col;
            let is_image = |buf: &Buffer, x: u16| {
                buf.cell((x, bar.y))
                    .is_some_and(|c| c.symbol().starts_with(crate::graphics::PLACEHOLDER))
            };
            if at + 1 < x + room && is_image(buf, at) && is_image(buf, at + 1) {
                for dx in 0..2 {
                    if let Some(cell) = buf.cell_mut((at + dx, bar.y)) {
                        cell.set_style(icon_style);
                    }
                }
            } else if is_image(buf, at)
                && let Some(cell) = buf.cell_mut((at, bar.y))
            {
                cell.set_symbol(" ");
            }
        }
    }
    if let Some(text) = waiting_text
        && right_w + waiting_w <= bar.width
    {
        buf.set_stringn(
            bar.x + bar.width - right_w - waiting_w,
            bar.y,
            &text,
            usize::from(waiting_w),
            base.fg(color(theme.warning)),
        );
    }
    if right_w <= bar.width {
        buf.set_stringn(
            bar.x + bar.width - right_w,
            bar.y,
            &right,
            usize::from(right_w),
            base.patch(right_style),
        );
    }
}

/// The one-line editor of a prompt; a flash, when there is one, takes the hints' place.
fn draw_prompt(
    buf: &mut Buffer,
    bar: Rect,
    prompt: &Prompt,
    flash: Option<(String, Style)>,
    base: Style,
    theme: &Theme,
) {
    let (right, right_style) = flash.unwrap_or_else(|| {
        (
            "⏎ save  esc cancel".to_string(),
            Style::new().fg(color(theme.muted)),
        )
    });
    let right_w = width(&right).min(bar.width);
    let label = format!(" {} ", prompt.label());
    let (x, _) = buf.set_stringn(
        bar.x,
        bar.y,
        &label,
        usize::from(bar.width),
        base.fg(color(theme.accent)).add_modifier(Modifier::BOLD),
    );
    let (before, after) = prompt.editor.split();
    let avail = bar.width.saturating_sub(right_w + 1 + (x - bar.x));
    // The text keeps the cursor in sight: the start is cut when it does not fit.
    let mut before: Vec<char> = before.chars().collect();
    while width(&before.iter().collect::<String>()) + 1 > avail && !before.is_empty() {
        before.remove(0);
    }
    let before: String = before.into_iter().collect();
    let (x, _) = buf.set_stringn(x, bar.y, &before, usize::from(avail), base);
    let left = avail.saturating_sub(width(&before));
    let mut rest = after.chars();
    let cursor_cell = rest.next();
    match cursor_cell {
        None => {
            buf.set_stringn(
                x,
                bar.y,
                "█",
                usize::from(left),
                base.fg(color(theme.accent)),
            );
        }
        Some(c) => {
            let (cx, _) = buf.set_stringn(
                x,
                bar.y,
                c.to_string(),
                usize::from(left),
                base.add_modifier(Modifier::REVERSED),
            );
            let left = left.saturating_sub(cx - x);
            buf.set_stringn(cx, bar.y, rest.as_str(), usize::from(left), base);
        }
    }
    if right_w > 0 && width(&label) + width(&before) + right_w < bar.width {
        buf.set_stringn(
            bar.x + bar.width - right_w,
            bar.y,
            &right,
            usize::from(right_w),
            base.patch(right_style),
        );
    }
}

/// The outer terminal's cursor style for a terminal cursor shape.
pub fn cursor_style(shape: CursorShape) -> ratatui::crossterm::cursor::SetCursorStyle {
    use ratatui::crossterm::cursor::SetCursorStyle;
    match shape {
        CursorShape::Default => SetCursorStyle::DefaultUserShape,
        CursorShape::Block => SetCursorStyle::SteadyBlock,
        CursorShape::Bar => SetCursorStyle::SteadyBar,
        CursorShape::Underline => SetCursorStyle::SteadyUnderScore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mirror::tests::{frame, line, no_sidebar, view};
    use deck_core::model::TerminalId;
    use deck_core::protocol::{self, Cursor, Line, Run, ServerMsg};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;
    use std::time::{Duration, Instant};

    fn prefix_hint() -> &'static str {
        if cfg!(target_os = "macos") {
            "⌥Space"
        } else {
            "M-Space"
        }
    }

    fn app(size: (u16, u16), vp: Option<(u16, u16)>, title: Option<&str>) -> App {
        let mut app = App::new(&no_sidebar(), size, &[]);
        let mut v = view(title);
        v.layout_viewport = vp;
        app.on_server(ServerMsg::SessionView(v));
        app
    }

    /// The app of `app()` whose panel `p` was renamed by hand.
    fn manual_app(size: (u16, u16), vp: (u16, u16), title: Option<&str>) -> App {
        let mut app = App::new(&no_sidebar(), size, &[]);
        let mut v = view(title);
        v.layout_viewport = Some(vp);
        v.projects[0].panels[0].name_source = NameSource::Manual;
        app.on_server(ServerMsg::SessionView(v));
        app
    }

    fn render(app: &App) -> (Terminal<TestBackend>, Vec<String>) {
        let mut t = Terminal::new(TestBackend::new(app.size.0, app.size.1)).unwrap();
        t.draw(|f| draw(f, app)).unwrap();
        let text = rows(&t);
        (t, text)
    }

    fn rows(t: &Terminal<TestBackend>) -> Vec<String> {
        let buf = t.backend().buffer();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect()
    }

    fn put(app: &mut App, size: (u16, u16), lines: Vec<(u16, Line)>) {
        app.on_server(ServerMsg::Frame(frame(true, size, lines)));
    }

    #[test]
    fn the_mockup() {
        let mut a = app((80, 24), Some((80, 23)), None);
        put(
            &mut a,
            (80, 23),
            vec![
                (0, line("$ echo hello")),
                (1, line("hello")),
                (2, line("$ ")),
            ],
        );
        let (mut t, text) = render(&a);
        // The terminal fills the panel area: no border, no title.
        assert_eq!(text[0], format!("$ echo hello{}", " ".repeat(68)));
        assert_eq!(text[1], format!("hello{}", " ".repeat(75)));
        assert!(!text.iter().any(|r| r.contains('╭') || r.contains('│')));
        assert_eq!(
            text[23],
            format!(
                " main  proj › ❯  p{}{} ? help  d detach",
                " ".repeat(80 - 18 - 17 - prefix_hint().chars().count()),
                prefix_hint()
            )
        );
        // The hints are muted, the session name is bold.
        let buf = t.backend().buffer();
        assert_eq!(buf[(1, 23)].fg, Color::Blue);
        assert!(buf[(1, 23)].modifier.contains(Modifier::BOLD));
        assert_eq!(buf[(79, 23)].fg, Color::DarkGray);
        assert_eq!(buf[(7, 23)].fg, Color::DarkGray);
        // The cursor frame has it at row 0, col 1.
        assert_eq!(t.get_cursor_position().unwrap(), Position::new(1, 0));
    }

    #[test]
    fn the_status_bar_shows_the_osc_title_cut_with_an_ellipsis_beside_the_hints() {
        let a = manual_app((80, 24), (80, 23), Some("vim: a title"));
        let (_, text) = render(&a);
        assert!(
            text[23].starts_with(" main  proj › ❯  p · vim: a title "),
            "{}",
            text[23]
        );

        let a = manual_app((50, 10), (50, 9), Some("vim: a very long title indeed"));
        let (_, text) = render(&a);
        let bar = &text[9];
        assert!(bar.starts_with(" main  proj › ❯  p · vim"), "{bar}");
        assert!(bar.contains("… "), "{bar}");
        assert!(bar.ends_with("d detach"), "{bar}");
        assert_eq!(width(bar), 50);

        // A panel named after what runs in it has no title suffix, and neither has a manual
        // one whose title is its name.
        let a = app((80, 24), Some((80, 23)), Some("vim: a title"));
        let (_, text) = render(&a);
        assert!(!text[23].contains('·'), "{}", text[23]);
        let a = manual_app((80, 24), (80, 23), Some("p"));
        let (_, text) = render(&a);
        assert!(text[23].starts_with(" main  proj › ❯  p  "), "{}", text[23]);

        // No title when the panel area is empty.
        let mut v = view(Some("t"));
        v.projects[0].panels.clear();
        v.projects[0].active_panel = None;
        v.active_panel = None;
        let mut a = App::new(&no_sidebar(), (80, 24), &[]);
        a.on_server(ServerMsg::SessionView(v));
        let (_, text) = render(&a);
        assert!(!text[23].contains('·'), "{}", text[23]);
    }

    #[test]
    fn the_prefix_badge_replaces_the_session_name() {
        let mut a = app((80, 24), Some((80, 23)), None);
        a.armed = true;
        let (t, text) = render(&a);
        assert!(text[23].starts_with(" PREFIX  proj"), "{}", text[23]);
        let cell = &t.backend().buffer()[(1, 23)];
        assert_eq!(cell.fg, Color::Yellow);
        assert!(cell.modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn the_scroll_indicator_and_badge() {
        let mut a = app((80, 24), Some((80, 23)), None);
        let mut f = frame(true, (80, 23), vec![(0, line("line 3880"))]);
        f.display_offset = 120;
        f.history = 4096;
        a.on_server(ServerMsg::Frame(f));
        a.scroll_mode = Some(TerminalId(4));
        let (t, text) = render(&a);
        assert!(text[0].starts_with("line 3880"));
        assert_eq!(
            text[23],
            format!(
                " SCROLL  [120/4096] proj › ❯  p{}q exit  g top  G bottom",
                " ".repeat(80 - 31 - 23)
            )
        );
        let buf = t.backend().buffer();
        // The indicator is the warning colour, the badge has the PREFIX style.
        assert_eq!(buf[(10, 23)].fg, Color::Yellow);
        assert_eq!(buf[(1, 23)].fg, Color::Yellow);
        assert!(buf[(1, 23)].modifier.contains(Modifier::REVERSED));

        // The wheel alone shows the indicator and neither the badge nor the scroll hints.
        a.scroll_mode = None;
        let (_, text) = render(&a);
        assert!(
            text[23].starts_with(" main  [120/4096] proj"),
            "{}",
            text[23]
        );
        assert!(
            text[23].ends_with(&format!("{} ? help  d detach", prefix_hint())),
            "{}",
            text[23]
        );

        // Back at the bottom there is no indicator.
        a.on_server(ServerMsg::Frame(frame(true, (80, 23), vec![])));
        let (_, text) = render(&a);
        assert!(!text[23].contains('['), "{}", text[23]);
    }

    #[test]
    fn the_prefix_badge_wins_over_the_scroll_badge() {
        let mut a = app((80, 24), Some((80, 23)), None);
        a.scroll_mode = Some(TerminalId(4));
        a.armed = true;
        let (_, text) = render(&a);
        assert!(text[23].starts_with(" PREFIX "), "{}", text[23]);
    }

    #[test]
    fn the_status_bar_has_no_hints_before_the_first_session_view() {
        let a = App::new(&no_sidebar(), (80, 24), &[]);
        let (_, text) = render(&a);
        assert!(!text[23].contains("detach"), "{}", text[23]);
    }

    #[test]
    fn a_flash_message_replaces_the_hints() {
        let mut a = app((80, 24), Some((80, 23)), None);
        a.flash = Some((
            "no binding for x".into(),
            Instant::now() + Duration::from_secs(2),
        ));
        let (_, text) = render(&a);
        assert!(text[23].ends_with("no binding for x"), "{}", text[23]);
        assert!(!text[23].contains("detach"));
    }

    #[test]
    fn too_small_shows_only_the_message() {
        let a = app((40, 4), Some((40, 3)), None);
        let (mut t, text) = render(&a);
        assert!(text.iter().any(|r| r.trim() == TOO_SMALL), "{text:?}");
        assert!(!text.iter().any(|r| r.contains('╭') || r.contains("main")));
        assert!(t.get_cursor_position().is_ok());

        let a = app((15, 4), None, None);
        let (_, text) = render(&a);
        let all = text.join("\n");
        assert!(
            all.contains("deck: window") && all.contains("too small"),
            "{all}"
        );
    }

    #[test]
    fn wide_and_combining_characters_keep_their_columns() {
        let mut a = app((20, 6), Some((20, 5)), None);
        let run = |col: u16, text: &str| Run {
            col,
            text: text.into(),
            style: protocol::Style::default(),
        };
        let l = Line {
            runs: vec![run(0, "日本"), run(5, "🙂"), run(8, "e\u{301}x")],
        };
        put(&mut a, (20, 5), vec![(0, l)]);
        let (t, _) = render(&a);
        let buf = t.backend().buffer();
        assert_eq!(buf[(0, 0)].symbol(), "日");
        assert_eq!(buf[(2, 0)].symbol(), "本");
        assert_eq!(buf[(5, 0)].symbol(), "🙂");
        assert_eq!(buf[(8, 0)].symbol(), "e\u{301}");
        assert_eq!(buf[(9, 0)].symbol(), "x");
    }

    #[test]
    fn colours_and_attributes_are_kept() {
        let mut a = app((20, 6), Some((20, 5)), None);
        let l = Line {
            runs: vec![Run {
                col: 0,
                text: "red".into(),
                style: protocol::Style {
                    fg: protocol::Color::Indexed(1),
                    bg: protocol::Color::Rgb(1, 2, 3),
                    flags: protocol::Style::BOLD
                        | protocol::Style::REVERSE
                        | protocol::Style::ITALIC,
                },
            }],
        };
        put(&mut a, (20, 5), vec![(0, l)]);
        let (t, _) = render(&a);
        let cell = &t.backend().buffer()[(0, 0)];
        assert_eq!(cell.fg, Color::Red);
        assert_eq!(cell.bg, Color::Rgb(1, 2, 3));
        assert!(
            cell.modifier
                .contains(Modifier::BOLD | Modifier::REVERSED | Modifier::ITALIC)
        );
    }

    #[test]
    fn a_smaller_client_draws_the_larger_layout_cropped() {
        let mut a = app((30, 8), Some((60, 19)), None);
        put(&mut a, (60, 19), vec![(0, line("0123456789"))]);
        let (mut t, text) = render(&a);
        assert_eq!(text[0].chars().count(), 30);
        assert!(text[0].starts_with("0123456789"));
        assert!(text[7].starts_with(" main"));
        // The cursor is in the visible part.
        assert!(t.get_cursor_position().is_ok());
    }

    #[test]
    fn a_larger_client_pads_with_blanks() {
        let mut a = app((40, 10), Some((20, 6)), None);
        put(&mut a, (20, 5), vec![(0, line("hi"))]);
        let (_, text) = render(&a);
        assert_eq!(text[0].trim_end(), "hi");
        assert_eq!(text[4].trim(), "");
        assert_eq!(text[6].trim(), "");
    }

    #[test]
    fn a_wide_character_cut_by_the_crop_is_blanked() {
        let mut a = app((20, 6), Some((30, 5)), None);
        let l = Line {
            runs: vec![Run {
                col: 19,
                text: "日".into(),
                style: protocol::Style::default(),
            }],
        };
        put(&mut a, (30, 5), vec![(0, l)]);
        let (_, text) = render(&a);
        assert!(!text[0].contains('日'), "{}", text[0]);
    }

    #[test]
    fn a_hidden_cursor_is_not_shown() {
        let mut a = app((20, 6), Some((20, 5)), None);
        let mut f = frame(true, (18, 3), vec![]);
        f.cursor = Cursor {
            visible: false,
            ..f.cursor
        };
        a.on_server(ServerMsg::Frame(f));
        let mut t = Terminal::new(TestBackend::new(20, 6)).unwrap();
        t.draw(|f| draw(f, &a)).unwrap();
        t.backend_mut().assert_cursor_position(Position::new(0, 0));
    }
}
