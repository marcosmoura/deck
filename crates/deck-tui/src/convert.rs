//! Conversion of protocol colours and styles to ratatui, and the fallback to the 256-colour
//! palette for outer terminals without truecolor.

use deck_core::protocol;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier, Style};

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

/// The foreground of an image placeholder cell: the image id's three bytes as an RGB colour.
///
/// This is the one place a colour is built outside the theme (DECK-08's rule): the colour is
/// not drawn but read by the terminal as the id. The 256-colour `downsample` never sees it,
/// since images are off without truecolor.
pub fn image_id_color(id: u32) -> Color {
    let [_, r, g, b] = id.to_be_bytes();
    Color::Rgb(r, g, b)
}

/// An RGB colour as a ratatui colour, for image cells whose reversed colours are resolved.
pub fn rgb(c: (u8, u8, u8)) -> Color {
    Color::Rgb(c.0, c.1, c.2)
}

/// What a colour is, for resolving it to RGB: its own value, the outer terminal's default
/// foreground, or a palette entry (the 16 named colours are entries 0 to 15).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolved {
    Rgb(u8, u8, u8),
    Default,
    Indexed(u8),
}

pub fn classify(c: Color) -> Resolved {
    match c {
        Color::Rgb(r, g, b) => Resolved::Rgb(r, g, b),
        Color::Reset => Resolved::Default,
        Color::Indexed(n) => Resolved::Indexed(n),
        Color::Black => Resolved::Indexed(0),
        Color::Red => Resolved::Indexed(1),
        Color::Green => Resolved::Indexed(2),
        Color::Yellow => Resolved::Indexed(3),
        Color::Blue => Resolved::Indexed(4),
        Color::Magenta => Resolved::Indexed(5),
        Color::Cyan => Resolved::Indexed(6),
        Color::Gray => Resolved::Indexed(7),
        Color::DarkGray => Resolved::Indexed(8),
        Color::LightRed => Resolved::Indexed(9),
        Color::LightGreen => Resolved::Indexed(10),
        Color::LightYellow => Resolved::Indexed(11),
        Color::LightBlue => Resolved::Indexed(12),
        Color::LightMagenta => Resolved::Indexed(13),
        Color::LightCyan => Resolved::Indexed(14),
        Color::White => Resolved::Indexed(15),
    }
}

/// Whether `COLORTERM` says the outer terminal does truecolor.
pub fn truecolor(colorterm: Option<&str>) -> bool {
    matches!(colorterm, Some("truecolor" | "24bit"))
}

/// The palette colour (16 to 255) nearest to an RGB colour, from the 6x6x6 cube and the
/// grey ramp; the 16 first colours depend on the user's terminal theme and are not used.
pub fn nearest_256(r: u8, g: u8, b: u8) -> u8 {
    const LEVELS: [i32; 6] = [0, 95, 135, 175, 215, 255];
    let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
    let level = |v: i32| {
        LEVELS
            .into_iter()
            .enumerate()
            .min_by_key(|&(_, level)| (level - v).abs())
            .unwrap_or((0, 0))
    };
    let (r_index, r_level) = level(r);
    let (g_index, g_level) = level(g);
    let (b_index, b_level) = level(b);
    let dist = |(x, y, z): (i32, i32, i32)| (x - r).pow(2) + (y - g).pow(2) + (z - b).pow(2);
    let cube = dist((r_level, g_level, b_level));
    let step = (((r + g + b) / 3 - 8 + 5) / 10).clamp(0, 23);
    let grey = 8 + 10 * step;
    if dist((grey, grey, grey)) < cube {
        u8::try_from(232 + step).unwrap_or(255)
    } else {
        u8::try_from(16 + 36 * r_index + 6 * g_index + b_index).unwrap_or(255)
    }
}

fn reduce(c: Color) -> Color {
    match c {
        Color::Rgb(r, g, b) => Color::Indexed(nearest_256(r, g, b)),
        other => other,
    }
}

/// Draws every RGB colour of `buf` as its nearest palette colour.
pub fn downsample(buf: &mut Buffer) {
    for cell in &mut buf.content {
        cell.fg = reduce(cell.fg);
        cell.bg = reduce(cell.bg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    #[test]
    fn colorterm_values() {
        assert!(truecolor(Some("truecolor")));
        assert!(truecolor(Some("24bit")));
        assert!(!truecolor(Some("256")));
        assert!(!truecolor(None));
    }

    #[test]
    fn nearest_palette_colours() {
        assert_eq!(nearest_256(0, 0, 0), 16);
        assert_eq!(nearest_256(255, 255, 255), 231);
        assert_eq!(nearest_256(255, 0, 0), 196);
        assert_eq!(nearest_256(128, 128, 128), 244);
        let n = nearest_256(0xcb, 0xa6, 0xf7);
        assert!(n >= 16);
    }

    #[test]
    fn downsampling_keeps_the_other_colours() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 3, 1));
        buf[(0, 0)].set_fg(Color::Rgb(255, 0, 0));
        buf[(1, 0)].set_bg(Color::Indexed(236));
        buf[(2, 0)].set_fg(Color::Blue);
        downsample(&mut buf);
        assert_eq!(buf[(0, 0)].fg, Color::Indexed(196));
        assert_eq!(buf[(1, 0)].bg, Color::Indexed(236));
        assert_eq!(buf[(2, 0)].fg, Color::Blue);
        assert_eq!(buf[(0, 0)].bg, Color::Reset);
    }
}
