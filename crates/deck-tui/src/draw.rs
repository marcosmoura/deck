//! Drawing: the terminal's frame and contents, the status bar and the too-small message.
//! Every colour and glyph of deck's own UI comes from the [`Theme`].

use deck_core::protocol::{self, CursorShape};
use deck_core::theme::{BorderStyle, Theme};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Widget};

use crate::app::{App, too_small};
use crate::mirror::TermMirror;

pub const TOO_SMALL: &str = "deck: window too small";
/// The status-bar hints while scroll mode is on.
pub const SCROLL_HINTS: &str = "q exit  g top  G bottom";

pub fn color(c: protocol::Color) -> Color {
    match c {
        protocol::Color::Default => Color::Reset,
        protocol::Color::Indexed(n) => match n {
            0 => Color::Black,
            1 => Color::Red,
            2 => Color::Green,
            3 => Color::Yellow,
            4 => Color::Blue,
            5 => Color::Magenta,
            6 => Color::Cyan,
            7 => Color::Gray,
            8 => Color::DarkGray,
            9 => Color::LightRed,
            10 => Color::LightGreen,
            11 => Color::LightYellow,
            12 => Color::LightBlue,
            13 => Color::LightMagenta,
            14 => Color::LightCyan,
            15 => Color::White,
            n => Color::Indexed(n),
        },
        protocol::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

pub fn style(s: protocol::Style) -> Style {
    let flags = [
        (protocol::Style::BOLD, Modifier::BOLD),
        (protocol::Style::DIM, Modifier::DIM),
        (protocol::Style::ITALIC, Modifier::ITALIC),
        (protocol::Style::UNDERLINE, Modifier::UNDERLINED),
        (protocol::Style::REVERSE, Modifier::REVERSED),
        (protocol::Style::STRIKETHROUGH, Modifier::CROSSED_OUT),
        (protocol::Style::HIDDEN, Modifier::HIDDEN),
    ];
    let modifiers = flags
        .into_iter()
        .filter(|(flag, _)| s.has(*flag))
        .fold(Modifier::empty(), |acc, (_, m)| acc | m);
    Style::new()
        .fg(color(s.fg))
        .bg(color(s.bg))
        .add_modifier(modifiers)
}

fn border_type(style: BorderStyle) -> BorderType {
    match style {
        BorderStyle::Rounded => BorderType::Rounded,
        BorderStyle::Plain => BorderType::Plain,
        BorderStyle::Thick => BorderType::Thick,
        BorderStyle::Double => BorderType::Double,
    }
}

fn width(text: &str) -> u16 {
    u16::try_from(Span::raw(text).width()).unwrap_or(u16::MAX)
}

/// `text` cut to at most `max` columns, with `…` in place of what was cut.
fn truncate(text: &str, max: u16) -> String {
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
    let theme = Theme::default();
    let area = frame.area();
    if too_small((area.width, area.height)) {
        draw_too_small(frame.buffer_mut(), area);
        return;
    }
    let panel = Rect::new(0, 0, area.width, area.height - 1);
    let cursor = draw_panel(frame.buffer_mut(), panel, app, &theme);
    draw_status(
        frame.buffer_mut(),
        Rect::new(0, area.height - 1, area.width, 1),
        app,
        &theme,
    );
    if let Some(pos) = cursor {
        frame.set_cursor_position(pos);
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
fn draw_panel(buf: &mut Buffer, panel: Rect, app: &App, theme: &Theme) -> Option<Position> {
    let (vw, vh) = app
        .mirror
        .view()
        .and_then(|v| v.layout_viewport)
        .unwrap_or((panel.width, panel.height));
    if vw < 3 || vh < 3 {
        return None;
    }
    let term = app
        .mirror
        .focused_id()
        .and_then(|id| app.mirror.terminal(id));
    let title = app
        .mirror
        .focused()
        .and_then(|t| t.title.clone())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| app.program.clone());

    let mut tmp = Buffer::empty(Rect::new(0, 0, vw, vh));
    draw_terminal(&mut tmp, Rect::new(0, 0, vw, vh), &title, term, theme);

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

    let cursor = term?.cursor;
    if !cursor.visible {
        return None;
    }
    let (x, y) = (1 + cursor.col, 1 + cursor.row);
    (x < vw - 1 && y < vh - 1 && x < cw && y < ch)
        .then_some(Position::new(panel.x + x, panel.y + y))
}

/// The focused terminal's border, title and lines inside `rect`.
fn draw_terminal(
    buf: &mut Buffer,
    rect: Rect,
    title: &str,
    term: Option<&TermMirror>,
    theme: &Theme,
) {
    let border = Style::new().fg(color(theme.border_focused));
    Block::bordered()
        .border_type(border_type(theme.border_style))
        .border_style(border)
        .render(rect, buf);

    // "[offset/history]" at the right of the top border while scrolled back.
    let indicator = term
        .filter(|t| t.display_offset > 0)
        .map(|t| format!(" [{}/{}] ", t.display_offset, t.history))
        .filter(|text| width(text) + 4 <= rect.width);
    let indicator_w = indicator.as_deref().map_or(0, width);
    if let Some(text) = &indicator {
        buf.set_stringn(
            rect.x + rect.width - 1 - indicator_w,
            rect.y,
            text,
            usize::from(indicator_w),
            border.fg(color(theme.warning)),
        );
    }

    // "─ title ─": the title sits after one border cell, with a space on each side.
    let room = rect.width.saturating_sub(6 + indicator_w);
    if room > 0 && !title.is_empty() {
        let text = format!(" {} ", truncate(title, room));
        buf.set_stringn(
            rect.x + 2,
            rect.y,
            &text,
            usize::from(rect.width.saturating_sub(3 + indicator_w)),
            border.fg(color(theme.title)),
        );
    }

    let Some(term) = term else { return };
    let inner = Rect::new(rect.x + 1, rect.y + 1, rect.width - 2, rect.height - 2);
    for (row, line) in term
        .lines
        .iter()
        .enumerate()
        .take(usize::from(inner.height))
    {
        let y = inner.y + u16::try_from(row).unwrap_or(0);
        for run in &line.runs {
            if run.col >= inner.width {
                break;
            }
            buf.set_stringn(
                inner.x + run.col,
                y,
                &run.text,
                usize::from(inner.width - run.col),
                style(run.style),
            );
        }
    }
}

fn draw_status(buf: &mut Buffer, bar: Rect, app: &App, theme: &Theme) {
    let base = Style::new()
        .fg(color(theme.status_fg))
        .bg(color(theme.status_bg));
    buf.set_style(bar, base);

    let name = app.mirror.session_name().unwrap_or("");
    let (badge, badge_style) = if app.armed || app.scroll_mode.is_some() {
        (
            if app.armed { " PREFIX " } else { " SCROLL " }.to_string(),
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
    let (right, right_style) = match &app.flash {
        Some((text, _)) => (text.clone(), Style::new().fg(color(theme.warning))),
        None if app.scroll_mode.is_some() => (
            SCROLL_HINTS.to_string(),
            Style::new().fg(color(theme.muted)),
        ),
        // The hints wait for the session, like the name and the project.
        None if app.mirror.view().is_none() => (String::new(), Style::new()),
        None => (app.bindings.hints(), Style::new().fg(color(theme.muted))),
    };

    let right_w = width(&right);
    let (x, _) = buf.set_stringn(
        bar.x,
        bar.y,
        &badge,
        usize::from(bar.width),
        base.patch(badge_style),
    );
    let room = bar
        .width
        .saturating_sub(right_w + 1)
        .saturating_sub(x - bar.x);
    if room > 1 && !project.is_empty() {
        let text = format!(" {}", truncate(project, room - 1));
        buf.set_stringn(
            x,
            bar.y,
            &text,
            usize::from(room),
            base.fg(color(theme.muted)),
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

/// The outer terminal's cursor style for a terminal cursor shape.
pub fn cursor_style(shape: CursorShape) -> ratatui::crossterm::cursor::SetCursorStyle {
    use ratatui::crossterm::cursor::SetCursorStyle;
    match shape {
        CursorShape::Block => SetCursorStyle::SteadyBlock,
        CursorShape::Bar => SetCursorStyle::SteadyBar,
        CursorShape::Underline => SetCursorStyle::SteadyUnderScore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mirror::tests::{frame, line, view};
    use deck_core::config::Config;
    use deck_core::model::TerminalId;
    use deck_core::protocol::{Cursor, Line, Run, ServerMsg};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::time::{Duration, Instant};

    fn prefix_hint() -> &'static str {
        if cfg!(target_os = "macos") {
            "⌥Space"
        } else {
            "M-Space"
        }
    }

    fn app(size: (u16, u16), vp: Option<(u16, u16)>, title: Option<&str>) -> App {
        let mut app = App::new(&Config::default(), size, "sh".into(), &[]);
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
        let mut a = app((80, 24), Some((80, 23)), None);
        put(
            &mut a,
            (78, 21),
            vec![
                (0, line("$ echo hello")),
                (1, line("hello")),
                (2, line("$ ")),
            ],
        );
        let (mut t, text) = render(&a);
        assert_eq!(text[0], format!("╭─ sh {}╮", "─".repeat(80 - 7)));
        assert_eq!(text[1], format!("│$ echo hello{}│", " ".repeat(66)));
        assert_eq!(text[2], format!("│hello{}│", " ".repeat(73)));
        assert_eq!(text[22], format!("╰{}╯", "─".repeat(78)));
        assert_eq!(
            text[23],
            format!(
                " main  proj{}{} d detach",
                " ".repeat(80 - 11 - 9 - prefix_hint().chars().count()),
                prefix_hint()
            )
        );
        // The border is the accent, the hints are muted, the session name is bold.
        let buf = t.backend().buffer();
        assert_eq!(buf[(0, 0)].fg, Color::Blue);
        assert_eq!(buf[(1, 23)].fg, Color::Blue);
        assert!(buf[(1, 23)].modifier.contains(Modifier::BOLD));
        assert_eq!(buf[(79, 23)].fg, Color::DarkGray);
        assert_eq!(buf[(7, 23)].fg, Color::DarkGray);
        // The cursor frame has it at row 0, col 1.
        assert_eq!(t.get_cursor_position().unwrap(), Position::new(2, 1));
    }

    #[test]
    fn the_title_is_the_osc_title_and_is_truncated_with_an_ellipsis() {
        let a = app(
            (30, 10),
            Some((30, 9)),
            Some("vim: a very long title indeed"),
        );
        let (_, text) = render(&a);
        assert!(
            text[0].starts_with("╭─ vim: a very long title"),
            "{}",
            text[0]
        );
        assert!(text[0].contains("… ─"), "{}", text[0]);
        assert_eq!(width(&text[0]), 30);
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
        let mut f = frame(true, (78, 21), vec![(0, line("line 3880"))]);
        f.display_offset = 120;
        f.history = 4096;
        a.on_server(ServerMsg::Frame(f));
        a.scroll_mode = Some(TerminalId(4));
        let (t, text) = render(&a);
        assert_eq!(
            text[0],
            format!("╭─ sh {} [120/4096] ╮", "─".repeat(80 - 6 - 12 - 1))
        );
        assert!(text[1].starts_with("│line 3880"));
        assert_eq!(
            text[23],
            format!(
                " SCROLL  proj{}q exit  g top  G bottom",
                " ".repeat(80 - 13 - 23)
            )
        );
        let buf = t.backend().buffer();
        // The indicator is the warning colour, the badge has the PREFIX style.
        assert_eq!(buf[(68, 0)].fg, Color::Yellow);
        assert_eq!(buf[(1, 23)].fg, Color::Yellow);
        assert!(buf[(1, 23)].modifier.contains(Modifier::REVERSED));

        // The wheel alone shows the indicator and neither the badge nor the scroll hints.
        a.scroll_mode = None;
        let (_, text) = render(&a);
        assert!(text[0].contains("[120/4096]"), "{}", text[0]);
        assert!(text[23].starts_with(" main "), "{}", text[23]);
        assert!(
            text[23].ends_with(&format!("{} d detach", prefix_hint())),
            "{}",
            text[23]
        );

        // Back at the bottom there is no indicator.
        a.on_server(ServerMsg::Frame(frame(true, (78, 21), vec![])));
        let (_, text) = render(&a);
        assert!(!text[0].contains('['), "{}", text[0]);
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
        let a = App::new(&Config::default(), (80, 24), "sh".into(), &[]);
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
        put(&mut a, (18, 3), vec![(0, l)]);
        let (t, _) = render(&a);
        let buf = t.backend().buffer();
        assert_eq!(buf[(1, 1)].symbol(), "日");
        assert_eq!(buf[(3, 1)].symbol(), "本");
        assert_eq!(buf[(6, 1)].symbol(), "🙂");
        assert_eq!(buf[(9, 1)].symbol(), "e\u{301}");
        assert_eq!(buf[(10, 1)].symbol(), "x");
        assert_eq!(buf[(19, 1)].symbol(), "│");
        assert_eq!(buf[(19, 3)].symbol(), "│");
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
        put(&mut a, (18, 3), vec![(0, l)]);
        let (t, _) = render(&a);
        let cell = &t.backend().buffer()[(1, 1)];
        assert_eq!(cell.fg, Color::Red);
        assert_eq!(cell.bg, Color::Rgb(1, 2, 3));
        assert!(
            cell.modifier
                .contains(Modifier::BOLD | Modifier::REVERSED | Modifier::ITALIC)
        );
    }

    #[test]
    fn a_smaller_client_draws_the_larger_layout_cropped() {
        let mut a = app((30, 8), Some((60, 20)), None);
        put(&mut a, (58, 18), vec![(0, line("0123456789"))]);
        let (mut t, text) = render(&a);
        assert_eq!(text[0].chars().count(), 30);
        assert!(text[1].starts_with("│0123456789"));
        assert_eq!(text[6].chars().next(), Some('│'));
        assert!(text[7].starts_with(" main"));
        // The cursor is in the visible part.
        assert!(t.get_cursor_position().is_ok());
    }

    #[test]
    fn a_larger_client_pads_with_blanks() {
        let mut a = app((40, 10), Some((20, 6)), None);
        put(&mut a, (18, 4), vec![(0, line("hi"))]);
        let (_, text) = render(&a);
        assert_eq!(text[0].chars().filter(|c| *c == '╮').count(), 1);
        assert_eq!(text[0].trim_end().chars().count(), 20);
        assert_eq!(text[3].trim_end(), "│                  │");
        assert_eq!(text[6].trim(), "");
    }

    #[test]
    fn a_wide_character_cut_by_the_crop_is_blanked() {
        let mut a = app((20, 6), Some((30, 5)), None);
        let l = Line {
            runs: vec![Run {
                col: 18,
                text: "日".into(),
                style: protocol::Style::default(),
            }],
        };
        put(&mut a, (28, 3), vec![(0, l)]);
        let (_, text) = render(&a);
        assert!(!text[1].contains('日'), "{}", text[1]);
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
