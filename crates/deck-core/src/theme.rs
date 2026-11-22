//! The colours, border style and glyphs of deck's own UI. A plain struct with the defaults
//! of the theme spec (DECK-08); parsing comes with that spec.

use crate::protocol::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderStyle {
    Rounded,
    Plain,
    Thick,
    Double,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Glyphs {
    pub expanded: String,
    pub collapsed: String,
    pub active: String,
}

impl Default for Glyphs {
    fn default() -> Self {
        Self {
            expanded: "▾".into(),
            collapsed: "▸".into(),
            active: "▌".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    pub accent: Color,
    pub muted: Color,
    pub ok: Color,
    pub warning: Color,
    pub error: Color,
    pub border: Color,
    pub border_focused: Color,
    pub title: Color,
    pub sidebar_fg: Color,
    pub sidebar_bg: Color,
    pub status_fg: Color,
    pub status_bg: Color,
    pub selection_fg: Color,
    pub selection_bg: Color,
    pub overlay_bg: Color,
    pub border_style: BorderStyle,
    pub glyphs: Glyphs,
}

impl Default for Theme {
    fn default() -> Self {
        let blue = Color::Indexed(4);
        Self {
            accent: blue,
            muted: Color::Indexed(8),
            ok: Color::Indexed(2),
            warning: Color::Indexed(3),
            error: Color::Indexed(1),
            border: Color::Indexed(8),
            border_focused: blue,
            title: Color::Default,
            sidebar_fg: Color::Default,
            sidebar_bg: Color::Default,
            status_fg: Color::Default,
            status_bg: Color::Default,
            selection_fg: Color::Default,
            selection_bg: Color::Default,
            overlay_bg: Color::Default,
            border_style: BorderStyle::Rounded,
            glyphs: Glyphs::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_the_role_table() {
        let t = Theme::default();
        assert_eq!(t.accent, Color::Indexed(4));
        assert_eq!(t.border_focused, t.accent);
        assert_eq!(t.muted, Color::Indexed(8));
        assert_eq!(t.border, Color::Indexed(8));
        assert_eq!(
            (t.ok, t.warning, t.error),
            (Color::Indexed(2), Color::Indexed(3), Color::Indexed(1))
        );
        assert_eq!(t.status_bg, Color::Default);
        assert_eq!(t.border_style, BorderStyle::Rounded);
        assert_eq!(t.glyphs.collapsed, "▸");
    }
}
