//! Drawing: the terminal's frame and contents, the overlays and the too-small message.
//! Every colour and glyph of deck's own UI comes from the [`Theme`].

use deck_core::config::HelpGroup;
use deck_core::model::{TerminalId, TerminalView};
use std::borrow::Cow;
use std::collections::HashMap;

use deck_core::protocol::{CursorShape, ImageScreen, Modes, NoticeLevel, Placement, PlacementKind};
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
use crate::convert::{image_id_color, program_id_color};
use crate::diacritics::{COUNT, DIACRITICS, PLACEHOLDER, diacritic};
use crate::graphics::Images;
use crate::grid::Grid;
use crate::input::Action;
use crate::mirror::TermMirror;
use crate::modal::{Modal, Prompt};
use crate::sidebar;

pub const TOO_SMALL: &str = "deck: window too small";

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

/// Draws the whole screen; the cursor is shown where the focused terminal's cursor is (the
/// client draws through `App::draw_frame`).
#[cfg(test)]
pub fn draw(frame: &mut Frame, app: &App) {
    if let Some(pos) = draw_screen(frame, app) {
        frame.set_cursor_position(pos);
    }
}

/// Draws the whole screen and returns where the cursor goes, if it is shown.
pub(crate) fn draw_screen(frame: &mut Frame, app: &App) -> Option<Position> {
    let theme = &app.theme;
    let area = frame.area();
    if too_small((area.width, area.height)) {
        draw_too_small(frame.buffer_mut(), area);
        return None;
    }
    let side = app.sidebar_cols();
    if side > 0 {
        sidebar::draw(
            frame.buffer_mut(),
            Rect::new(0, 0, side - 1, area.height),
            app,
            theme,
        );
    }
    let panel = Rect::new(side, 0, area.width - side, area.height);
    let cursor = if app.panel_area_empty() {
        draw_empty(frame.buffer_mut(), panel, app, theme);
        None
    } else {
        draw_panel(frame.buffer_mut(), panel, app)
    };
    if matches!(app.modal, Some(Modal::Help)) {
        draw_help(frame.buffer_mut(), area, app, theme);
    }
    if let Some(picker) = &app.picker {
        crate::picker::draw(frame.buffer_mut(), area, app, picker, theme);
    }
    crate::menu::draw(frame.buffer_mut(), app, theme);
    crate::context::draw(frame.buffer_mut(), app, theme);
    crate::restore::draw(frame.buffer_mut(), app, theme);
    draw_dialog(frame.buffer_mut(), area, app, theme);
    crate::bar::draw(frame.buffer_mut(), app, theme);
    if !app.truecolor {
        downsample(frame.buffer_mut());
    }
    // The cursor stays hidden on a cell the floating bar covers.
    cursor.filter(|pos| !app.overlay_open() && !app.bar_covers(pos.x, pos.y))
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
        None => format!("No spaces — {} to add one", label(Action::NewProject)),
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
    let room = usize::from(app.size.1).saturating_sub(2);
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
                draw_images(&mut tmp, area, app, id);
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
            let cell = std::mem::take(&mut tmp[(x, y)]);
            // A wide character cut in half by the crop would spill over.
            if x + 1 == cw && x + 1 < vw && width(cell.symbol()) > 1 {
                buf[(panel.x + x, panel.y + y)].set_symbol(" ");
            } else {
                buf[(panel.x + x, panel.y + y)] = cell;
            }
        }
    }

    let Position { x, y } = cursor?;
    (x < cw && y < ch).then_some(Position::new(panel.x + x, panel.y + y))
}

/// What a terminal is called on its border: its title, else its program, then
/// `exited <code>` once a kept terminal's process ended.
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
        match agent::title_mark(view, theme) {
            // An agent's status slot and name take the place of the number:
            // `╭─ <status slot> <kind> · title ─╮`.
            Some((icon, icon_style, kind)) => {
                let (x, _) =
                    buf.set_stringn(rect.x + 2, rect.y, " ", usize::from(room), title_style);
                let (slot, slot_style) = sidebar::icon_slot(app, icon, icon_style);
                let (x, _) = buf.set_stringn(x, rect.y, &slot, usize::from(room), slot_style);
                let used = x.saturating_sub(rect.x + 2);
                let label = truncate(
                    &format!("{kind} · {name}"),
                    rect.width.saturating_sub(6 + sidebar::ICON_CELLS),
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
        draw_images(buf, inner, app, view.id);
        draw_selection(buf, inner, app, view.id);
        // Scroll mode draws its own cursor.
        let scrolling = app.scroll_mode == Some(view.id);
        if let Some(c) = term
            .map(|t| t.cursor)
            .filter(|c| focused && c.visible && !scrolling)
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
            let (x, max) = (area.x + run.col, usize::from(area.width - run.col));
            if !put_ascii(buf, (x, y), &run.text, max, style(run.style)) {
                buf.set_stringn(x, y, blank_placeholders(&run.text), max, style(run.style));
            }
        }
    }
}

/// Writes a run of printable ASCII, one cell per byte, without the grapheme segmentation of
/// `set_stringn`, which is most of a draw's time. `false` (nothing written) for any other text.
fn put_ascii(buf: &mut Buffer, (x, y): (u16, u16), text: &str, max: usize, style: Style) -> bool {
    if !text.bytes().all(|b| (0x20..0x7f).contains(&b)) {
        return false;
    }
    for (i, byte) in text.bytes().take(max).enumerate() {
        let Ok(offset) = u16::try_from(i) else { break };
        let Some(col) = x.checked_add(offset) else {
            break;
        };
        let Some(cell) = buf.cell_mut((col, y)) else {
            break;
        };
        cell.set_char(char::from(byte)).set_style(style);
    }
    true
}

/// `text` with every Unicode placeholder cell (U+10EEEE and its diacritics) replaced by a
/// space: a program's own placeholders never reach the outer terminal, where their image id
/// could match one of the client's.
fn blank_placeholders(text: &str) -> Cow<'_, str> {
    if !text.contains(PLACEHOLDER) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut after = false;
    for c in text.chars() {
        if c == PLACEHOLDER {
            out.push(' ');
            after = true;
        } else if !(after && DIACRITICS.contains(&c)) {
            out.push(c);
            after = false;
        }
    }
    Cow::Owned(out)
}

/// The placed images of terminal `id`, over its text and under the selection highlight (which
/// skips them), in `area`: each cell of
/// a placement that has its outer image holds U+10EEEE, the diacritics of its row and column
/// inside the image, and the outer id as its foreground, on the background underneath.
fn draw_images(buf: &mut Buffer, area: Rect, app: &App, id: TerminalId) {
    let (Some(images), Some(term)) = (app.images.as_ref(), app.mirror.terminal(id)) else {
        return;
    };
    draw_program_placeholders(buf, area, images, term, id);
    for p in &term.images {
        let PlacementKind::Placed { col, row } = p.kind else {
            continue;
        };
        let Some((outer, cols, rows)) = images.placed_id(id, p, app.cell) else {
            continue;
        };
        for r in 0..rows {
            let y = row.saturating_add(i32::from(r));
            let Ok(y) = u16::try_from(y) else { continue };
            if y >= area.height {
                break;
            }
            for c in 0..cols {
                let x = col.saturating_add(i32::from(c));
                let Ok(x) = u16::try_from(x) else { continue };
                if x >= area.width {
                    break;
                }
                put_placeholder(buf, area.x, (area.x + x, area.y + y), outer, (r, c));
            }
        }
    }
}

/// Puts the placeholder cell of image row `r` and column `c` of outer image `outer` at the
/// cell `(bx, by)`, keeping the background underneath; `min_x` is the area's left edge.
fn put_placeholder(
    buf: &mut Buffer,
    min_x: u16,
    (bx, by): (u16, u16),
    outer: u32,
    (r, c): (u16, u16),
) {
    // A wide character whose second half this cell takes would spill over it.
    if bx > min_x
        && let Some(left) = buf.cell_mut((bx - 1, by))
        && width(left.symbol()) > 1
    {
        left.set_symbol(" ");
    }
    let Some(cell) = buf.cell_mut((bx, by)) else {
        return;
    };
    let bg = cell.bg;
    cell.reset();
    let symbol: String = [
        PLACEHOLDER,
        diacritic(usize::from(r)),
        diacritic(usize::from(c)),
    ]
    .into_iter()
    .collect();
    cell.set_symbol(&symbol)
        .set_fg(image_id_color(outer))
        .set_bg(bg);
}

/// A program-made placeholder cell, as decoded: the image id, and the image row and column.
#[derive(Debug, Clone, Copy)]
struct Decoded {
    at: u16,
    id: u32,
    row: usize,
    col: usize,
}

/// The program-made placeholder cells of terminal `id` (U+10EEEE with up to three diacritics,
/// the image id in the foreground colour), drawn over the client's outer image of the
/// program's virtual placement. Cells whose image is unknown or not sent stay blank.
fn draw_program_placeholders(
    buf: &mut Buffer,
    area: Rect,
    images: &Images,
    term: &TermMirror,
    id: TerminalId,
) {
    let screen = if term.modes.has(Modes::ALTERNATE_SCREEN) {
        ImageScreen::Alternate
    } else {
        ImageScreen::Primary
    };
    let mut newest: HashMap<u32, &Placement> = HashMap::new();
    for p in &term.images {
        if p.kind == PlacementKind::Virtual
            && p.screen == screen
            && newest
                .get(&p.image)
                .is_none_or(|n| n.generation < p.generation)
        {
            newest.insert(p.image, p);
        }
    }
    if newest.is_empty() {
        return;
    }
    for (row, line) in term.lines.iter().enumerate().take(usize::from(area.height)) {
        let y = area.y + u16::try_from(row).unwrap_or(0);
        let mut left: Option<Decoded> = None;
        for run in &line.runs {
            let mut at = run.col;
            let mut chars = run.text.chars().peekable();
            while let Some(c) = chars.next() {
                if c != PLACEHOLDER {
                    at = at.saturating_add(width(c.encode_utf8(&mut [0; 4])));
                    continue;
                }
                let mut marks = [None; 3];
                let mut n = 0;
                while let Some(&d) = chars.peek()
                    && let Ok(i) = DIACRITICS.binary_search(&d)
                {
                    if let Some(slot) = marks.get_mut(n) {
                        *slot = Some(i);
                    }
                    n += 1;
                    chars.next();
                }
                let cell_at = at;
                at = at.saturating_add(1);
                let low = program_id_color(run.style.fg);
                // The cell to the left, when it is next to this one and has the same colour.
                let near = left.filter(|l| l.at + 1 == cell_at && l.id & 0xFF_FFFF == low);
                // The third diacritic is the high byte of a 32-bit id; a cell with no
                // diacritics at all takes everything from the cell to the left.
                let (image, r, col) = match (marks[0], marks[1], marks[2]) {
                    (None, ..) => match near {
                        Some(l) => (l.id, l.row, l.col + 1),
                        None => (low, 0, 0),
                    },
                    (Some(r), col, high) => {
                        // Without a third diacritic, a cell that continues the one to the
                        // left (same row, the next column) keeps its high byte; else 0.
                        let high = match high {
                            Some(h) => u8::try_from(h).ok(),
                            None => Some(
                                near.filter(|l| l.row == r && col.is_none_or(|c| c == l.col + 1))
                                    .map_or(0, |l| (l.id >> 24) as u8),
                            ),
                        };
                        let Some(high) = high else {
                            left = None;
                            continue;
                        };
                        let image = (u32::from(high) << 24) | low;
                        let col = col.unwrap_or_else(|| {
                            near.filter(|l| l.id == image && l.row == r)
                                .map_or(0, |l| l.col + 1)
                        });
                        (image, r, col)
                    }
                };
                left = Some(Decoded {
                    at: cell_at,
                    id: image,
                    row: r,
                    col,
                });
                if r >= COUNT || col >= COUNT || cell_at >= area.width {
                    continue;
                }
                let Some(outer) = newest.get(&image).and_then(|p| images.virtual_id(id, p)) else {
                    continue;
                };
                // Both are below 297.
                let (r, col) = (
                    u16::try_from(r).unwrap_or(0),
                    u16::try_from(col).unwrap_or(0),
                );
                put_placeholder(buf, area.x, (area.x + cell_at, y), outer, (r, col));
            }
        }
    }
}

/// The text selected in terminal `id`, in the selection colours over its lines in `area`, and
/// the copy cursor.
fn draw_selection(buf: &mut Buffer, area: Rect, app: &App, id: TerminalId) {
    if area.is_empty() {
        return;
    }
    let Some(term) = app.mirror.terminal(id) else {
        return;
    };
    let selected = app
        .text_sel
        .as_ref()
        .filter(|s| s.terminal == id)
        .and_then(|s| s.range());
    // The search's matches go under the selection, whose colours win.
    for hit in crate::search::visible(app, id) {
        let (fg, bg) = if hit.current {
            (app.theme.search_current_fg, app.theme.search_current_bg)
        } else {
            (app.theme.search_match_fg, app.theme.search_match_bg)
        };
        let at = term.top_row + u64::from(hit.row);
        for col in hit.from..=hit.to.min(area.width.saturating_sub(1)) {
            let chosen = selected.is_some_and(|(from, to)| {
                (at, col) >= (from.row, from.col) && (at, col) <= (to.row, to.col)
            });
            if hit.row >= area.height || chosen {
                continue;
            }
            if let Some(cell) = buf.cell_mut((area.x + col, area.y + hit.row))
                && !cell.symbol().starts_with(PLACEHOLDER)
            {
                cell.modifier.remove(Modifier::REVERSED);
                cell.set_fg(color(fg)).set_bg(color(bg));
                if hit.current {
                    cell.modifier.insert(Modifier::BOLD);
                }
            }
        }
    }
    if let Some((from, to)) = selected {
        let style = selection(&app.theme, Style::new());
        let rows = area.height.min(term.size.1);
        for row in 0..rows {
            let at = term.top_row + u64::from(row);
            if at < from.row || at > to.row {
                continue;
            }
            let start = if at == from.row { from.col } else { 0 };
            let end = if at == to.row { to.col } else { u16::MAX };
            for col in start..=end.min(area.width - 1) {
                // An image cell keeps showing the image, without the highlight.
                if let Some(cell) = buf.cell_mut((area.x + col, area.y + row))
                    && !cell.symbol().starts_with(PLACEHOLDER)
                {
                    cell.set_style(style);
                }
            }
        }
    }
    // Not reversed: the accent must show inside a selection drawn reversed.
    if let Some((row, col)) = crate::copy::cursor_cell(app, id)
        && row < area.height
        && col < area.width
        && let Some(cell) = buf.cell_mut((area.x + col, area.y + row))
    {
        cell.modifier.remove(Modifier::REVERSED);
        cell.set_bg(color(app.theme.accent));
    }
}

/// The colour of the flash message.
pub(crate) fn flash_color(app: &App, theme: &Theme) -> deck_core::protocol::Color {
    match app.flash_level {
        Some(NoticeLevel::Info) => theme.muted,
        Some(NoticeLevel::Warning) => theme.warning,
        Some(NoticeLevel::Error) => theme.error,
        None if app.flash_error => theme.error,
        None => theme.warning,
    }
}

/// Rows of a dialog, borders included.
const DIALOG_ROWS: u16 = 5;
/// The least columns of a dialog.
const DIALOG_MIN_COLS: u16 = 40;

/// The prompt or confirmation dialog, centered in `area`; nothing when there is none or the
/// window is too small for it.
pub(crate) fn draw_dialog(buf: &mut Buffer, area: Rect, app: &App, theme: &Theme) {
    let (title, hint) = match &app.modal {
        Some(Modal::Prompt(p)) => (Some(p.label().trim_end_matches(':')), "⏎ save  esc cancel"),
        Some(Modal::Confirm(_)) => (None, "y yes  n no"),
        _ => return,
    };
    let question = match &app.modal {
        Some(Modal::Confirm(c)) => c.question.as_str(),
        _ => "",
    };
    if area.height < DIALOG_ROWS || area.width < 3 {
        return;
    }
    let flash = app.flash.as_ref().map_or(0, |(text, _)| width(text));
    let content = width(title.unwrap_or(question)).max(width(hint)).max(flash);
    let w = (content.saturating_add(4))
        .max(DIALOG_MIN_COLS)
        .min(area.width.saturating_sub(2).max(1));
    let rect = Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - DIALOG_ROWS) / 2,
        w,
        DIALOG_ROWS,
    );
    Clear.render(rect, buf);
    let base = Style::new().bg(color(theme.overlay_bg));
    buf.set_style(rect, base);
    let border = base.fg(color(theme.border));
    Block::bordered()
        .border_type(border_type(theme.border_style))
        .border_style(border)
        .render(rect, buf);
    let inner_w = w.saturating_sub(4);
    let x = rect.x + 2;
    if let Some(title) = title {
        let text = format!(" {} ", truncate(title, w.saturating_sub(4)));
        buf.set_stringn(
            rect.x + 1,
            rect.y,
            &text,
            usize::from(w.saturating_sub(2)),
            base.fg(color(theme.accent)).add_modifier(Modifier::BOLD),
        );
    }
    match &app.modal {
        Some(Modal::Prompt(prompt)) => {
            draw_editor(buf, x, rect.y + 1, inner_w, prompt, base, theme)
        }
        _ => {
            buf.set_stringn(
                x,
                rect.y + 1,
                truncate(question, inner_w),
                usize::from(inner_w),
                base.fg(color(theme.warning)),
            );
        }
    }
    let (text, style) = match &app.flash {
        Some((text, _)) => (
            text.as_str(),
            Style::new().fg(color(flash_color(app, theme))),
        ),
        None => (hint, Style::new().fg(color(theme.muted))),
    };
    buf.set_stringn(
        x,
        rect.y + 3,
        truncate(text, inner_w),
        usize::from(inner_w),
        base.patch(style),
    );
}

/// The one-line editor of a prompt in `avail` columns from `x`.
fn draw_editor(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    avail: u16,
    prompt: &Prompt,
    base: Style,
    theme: &Theme,
) {
    let (before, after) = prompt.editor.split();
    // The text keeps the cursor in sight: the start is cut when it does not fit.
    let mut before: Vec<char> = before.chars().collect();
    while width(&before.iter().collect::<String>()) + 1 > avail && !before.is_empty() {
        before.remove(0);
    }
    let before: String = before.into_iter().collect();
    let (x, _) = buf.set_stringn(x, y, &before, usize::from(avail), base);
    let left = avail.saturating_sub(width(&before));
    let mut rest = after.chars();
    match rest.next() {
        None => {
            buf.set_stringn(x, y, "█", usize::from(left), base.fg(color(theme.accent)));
        }
        Some(c) => {
            let (cx, _) = buf.set_stringn(
                x,
                y,
                c.to_string(),
                usize::from(left),
                base.add_modifier(Modifier::REVERSED),
            );
            let left = left.saturating_sub(cx - x);
            buf.set_stringn(cx, y, rest.as_str(), usize::from(left), base);
        }
    }
}

/// The outer terminal's cursor style for a terminal cursor shape and its blinking. A shape
/// the program never chose stays the outer terminal's own, blinking or not.
pub fn cursor_style(shape: CursorShape, blink: bool) -> ratatui::crossterm::cursor::SetCursorStyle {
    use ratatui::crossterm::cursor::SetCursorStyle;
    match (shape, blink) {
        (CursorShape::Default, _) => SetCursorStyle::DefaultUserShape,
        (CursorShape::Block, false) => SetCursorStyle::SteadyBlock,
        (CursorShape::Block, true) => SetCursorStyle::BlinkingBlock,
        (CursorShape::Bar, false) => SetCursorStyle::SteadyBar,
        (CursorShape::Bar, true) => SetCursorStyle::BlinkingBar,
        (CursorShape::Underline, false) => SetCursorStyle::SteadyUnderScore,
        (CursorShape::Underline, true) => SetCursorStyle::BlinkingUnderScore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mirror::tests::{frame, line, no_sidebar, view};
    use deck_core::protocol::{self, Cursor, Line, Run, ServerMsg};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;

    #[test]
    fn ascii_text_past_the_buffer_edge_is_cut_not_a_panic() {
        let mut buf = Buffer::empty(ratatui::layout::Rect::new(0, 0, 4, 1));
        assert!(put_ascii(&mut buf, (2, 0), "abcdef", 6, Style::default()));
        assert_eq!(buf[(2, 0)].symbol(), "a");
        assert_eq!(buf[(3, 0)].symbol(), "b");
        // A row that is not there writes nothing and does not panic.
        assert!(put_ascii(&mut buf, (0, 5), "x", 1, Style::default()));
    }

    #[test]
    fn the_outer_cursor_blinks_when_the_program_asked() {
        use ratatui::crossterm::cursor::SetCursorStyle as S;
        for (shape, blink, style) in [
            (CursorShape::Block, false, S::SteadyBlock),
            (CursorShape::Block, true, S::BlinkingBlock),
            (CursorShape::Bar, false, S::SteadyBar),
            (CursorShape::Bar, true, S::BlinkingBar),
            (CursorShape::Underline, false, S::SteadyUnderScore),
            (CursorShape::Underline, true, S::BlinkingUnderScore),
            (CursorShape::Default, false, S::DefaultUserShape),
            (CursorShape::Default, true, S::DefaultUserShape),
        ] {
            assert_eq!(cursor_style(shape, blink), style, "{shape:?} blink {blink}");
        }
    }

    fn app(size: (u16, u16), vp: Option<(u16, u16)>, title: Option<&str>) -> App {
        let mut app = App::new(&no_sidebar(), size, &[]);
        let mut v = view(title);
        v.layout_viewport = vp;
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
        let mut a = app((80, 24), Some((80, 24)), None);
        put(
            &mut a,
            (80, 24),
            vec![
                (0, line("$ echo hello")),
                (1, line("hello")),
                (2, line("$ ")),
            ],
        );
        let (mut t, text) = render(&a);
        // The terminal fills the whole window: no border, no title, no status row.
        assert_eq!(text[0], format!("$ echo hello{}", " ".repeat(68)));
        assert_eq!(text[1], format!("hello{}", " ".repeat(75)));
        assert!(!text.iter().any(|r| r.contains('╭') || r.contains('│')));
        assert_eq!(text[23].trim(), "");
        // The cursor frame has it at row 0, col 1.
        assert_eq!(t.get_cursor_position().unwrap(), Position::new(1, 0));
    }

    #[test]
    fn too_small_shows_only_the_message() {
        let a = app((40, 4), Some((40, 4)), None);
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
        let mut a = app((20, 6), Some((20, 6)), None);
        let run = |col: u16, text: &str| Run {
            col,
            text: text.into(),
            style: protocol::Style::default(),
        };
        let l = Line {
            wrapped: false,
            runs: vec![run(0, "日本"), run(5, "🙂"), run(8, "e\u{301}x")],
        };
        put(&mut a, (20, 6), vec![(0, l)]);
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
        let mut a = app((20, 6), Some((20, 6)), None);
        let l = Line {
            wrapped: false,
            runs: vec![Run {
                col: 0,
                text: "red".into(),
                style: protocol::Style {
                    fg: protocol::Color::Indexed(1),
                    bg: protocol::Color::Rgb(1, 2, 3),
                    flags: protocol::Style::BOLD
                        | protocol::Style::REVERSE
                        | protocol::Style::ITALIC,
                    underline: protocol::Underline::Curly,
                    underline_color: protocol::Color::Rgb(9, 8, 7),
                },
            }],
        };
        put(&mut a, (20, 6), vec![(0, l)]);
        let (t, _) = render(&a);
        let cell = &t.backend().buffer()[(0, 0)];
        assert_eq!(cell.fg, Color::Red);
        assert_eq!(cell.bg, Color::Rgb(1, 2, 3));
        assert!(
            cell.modifier
                .contains(Modifier::BOLD | Modifier::REVERSED | Modifier::ITALIC)
        );
        assert_eq!(
            crate::backend::kind_bits(cell.modifier),
            protocol::Underline::Curly
        );
        assert_eq!(cell.underline_color, Color::Rgb(9, 8, 7));
    }

    #[test]
    fn a_placeholder_cell_carries_no_underline() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 1));
        buf[(1, 0)]
            .set_style(style(protocol::Style {
                underline: protocol::Underline::Curly,
                underline_color: protocol::Color::Rgb(1, 2, 3),
                ..protocol::Style::default()
            }))
            .set_bg(Color::Blue);
        put_placeholder(&mut buf, 0, (1, 0), 7, (0, 0));
        let cell = &buf[(1, 0)];
        assert_eq!(cell.underline_color, Color::Reset);
        assert_eq!(cell.modifier, Modifier::empty());
        assert_eq!(cell.bg, Color::Blue);
        assert_eq!(cell.fg, Color::Rgb(0, 0, 7));
    }

    #[test]
    fn a_smaller_client_draws_the_larger_layout_cropped() {
        let mut a = app((30, 8), Some((60, 19)), None);
        put(&mut a, (60, 19), vec![(0, line("0123456789"))]);
        let (mut t, text) = render(&a);
        assert_eq!(text[0].chars().count(), 30);
        assert!(text[0].starts_with("0123456789"));
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
            wrapped: false,
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
        let mut a = app((20, 6), Some((20, 6)), None);
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
