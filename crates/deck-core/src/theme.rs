//! The colours and border style of deck's own UI (`[theme]` in `config.toml`):
//! colour syntax, role references and the resolved [`Theme`].

use serde::{Deserialize, Serialize};
use toml::{Table, Value};

use crate::config::Warning;
use crate::protocol::Color;

/// The colour roles, in the order of [`Theme::colors`].
pub const ROLES: [&str; 20] = [
    "accent",
    "muted",
    "ok",
    "warning",
    "error",
    "border",
    "border_focused",
    "title",
    "sidebar_fg",
    "sidebar_bg",
    "status_fg",
    "status_bg",
    "selection_fg",
    "selection_bg",
    "overlay_bg",
    "hover",
    "search_match_fg",
    "search_match_bg",
    "search_current_fg",
    "search_current_bg",
];

const ANSI_NAMES: [&str; 16] = [
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "white",
    "bright-black",
    "bright-red",
    "bright-green",
    "bright-yellow",
    "bright-blue",
    "bright-magenta",
    "bright-cyan",
    "bright-white",
];

/// A colour as written in the config; it may name another role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeColor {
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
    /// The name of another role, one of [`ROLES`].
    Role(&'static str),
}

const COLOR_HELP: &str =
    "expected \"default\", a colour name, an integer 0 to 255, \"#rrggbb\" or a role name";

impl ThemeColor {
    /// Parses `"default"`, an ANSI name, `"#rrggbb"` or a role name.
    pub fn parse(text: &str) -> Result<Self, String> {
        if text == "default" {
            return Ok(Self::Default);
        }
        if let Some((index, _)) = (0u8..).zip(ANSI_NAMES).find(|&(_, name)| name == text) {
            return Ok(Self::Indexed(index));
        }
        if let Some(hex) = text.strip_prefix('#') {
            if hex.len() == 6
                && hex.bytes().all(|b| b.is_ascii_hexdigit())
                && let Ok(value) = u32::from_str_radix(hex, 16)
            {
                let [_, r, g, b] = value.to_be_bytes();
                return Ok(Self::Rgb(r, g, b));
            }
            return Err(format!("`{text}` is not a colour of the form #rrggbb"));
        }
        if let Some(role) = ROLES.iter().find(|r| **r == text) {
            return Ok(Self::Role(role));
        }
        Err(format!("unknown colour or role `{text}`; {COLOR_HELP}"))
    }

    /// Parses a TOML string or integer.
    pub fn from_value(value: &Value) -> Result<Self, String> {
        match value {
            Value::String(s) => Self::parse(s),
            Value::Integer(n) => u8::try_from(*n)
                .map(Self::Indexed)
                .map_err(|_| format!("{n} is outside the palette, 0 to 255")),
            _ => Err(COLOR_HELP.to_string()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BorderStyle {
    Rounded,
    Plain,
    Thick,
    Double,
}

impl BorderStyle {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "rounded" => Some(Self::Rounded),
            "plain" => Some(Self::Plain),
            "thick" => Some(Self::Thick),
            "double" => Some(Self::Double),
            _ => None,
        }
    }
}

/// A `[theme]` value that cannot be used; `key` is its full name, e.g. `theme.accent`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeError {
    pub key: String,
    pub message: String,
}

fn bad(key: impl Into<String>, message: impl Into<String>) -> ThemeError {
    ThemeError {
        key: key.into(),
        message: message.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    pub hover: Color,
    pub search_match_fg: Color,
    pub search_match_bg: Color,
    pub search_current_fg: Color,
    pub search_current_bg: Color,
    pub border_style: BorderStyle,
}

/// The default of each role, in the order of [`ROLES`].
const DEFAULTS: [ThemeColor; 20] = [
    ThemeColor::Indexed(4),
    ThemeColor::Indexed(8),
    ThemeColor::Indexed(2),
    ThemeColor::Indexed(3),
    ThemeColor::Indexed(1),
    ThemeColor::Indexed(8),
    ThemeColor::Role("accent"),
    ThemeColor::Default,
    ThemeColor::Default,
    ThemeColor::Default,
    ThemeColor::Default,
    ThemeColor::Default,
    ThemeColor::Default,
    ThemeColor::Default,
    ThemeColor::Default,
    ThemeColor::Indexed(8),
    ThemeColor::Indexed(0),
    ThemeColor::Indexed(3),
    ThemeColor::Indexed(0),
    ThemeColor::Indexed(11),
];

/// Follows role references until a colour; a loop is an error naming the keys in it.
fn resolve(raw: &[ThemeColor; 20]) -> Result<[Color; 20], ThemeError> {
    let mut out = [Color::Default; 20];
    let roles = ROLES.iter().copied().zip(raw.iter().copied());
    for ((start, mut current), slot) in roles.clone().zip(&mut out) {
        let mut chain = vec![start];
        *slot = loop {
            match current {
                ThemeColor::Default => break Color::Default,
                ThemeColor::Indexed(n) => break Color::Indexed(n),
                ThemeColor::Rgb(r, g, b) => break Color::Rgb(r, g, b),
                ThemeColor::Role(name) => {
                    if chain.contains(&name) {
                        chain.push(name);
                        return Err(bad(
                            format!("theme.{name}"),
                            format!("colour roles refer to each other: {}", chain.join(" -> ")),
                        ));
                    }
                    let Some((_, next)) = roles.clone().find(|&(role, _)| role == name) else {
                        return Err(bad(
                            format!("theme.{start}"),
                            format!("unknown role `{name}`"),
                        ));
                    };
                    chain.push(name);
                    current = next;
                }
            }
        };
    }
    Ok(out)
}

impl Theme {
    fn from_parts(c: [Color; 20], border_style: BorderStyle) -> Self {
        let [
            accent,
            muted,
            ok,
            warning,
            error,
            border,
            border_focused,
            title,
            sidebar_fg,
            sidebar_bg,
            status_fg,
            status_bg,
            selection_fg,
            selection_bg,
            overlay_bg,
            hover,
            search_match_fg,
            search_match_bg,
            search_current_fg,
            search_current_bg,
        ] = c;
        Self {
            accent,
            muted,
            ok,
            warning,
            error,
            border,
            border_focused,
            title,
            sidebar_fg,
            sidebar_bg,
            status_fg,
            status_bg,
            selection_fg,
            selection_bg,
            overlay_bg,
            hover,
            search_match_fg,
            search_match_bg,
            search_current_fg,
            search_current_bg,
            border_style,
        }
    }

    /// Reads the `[theme]` table. Unknown keys are added to `warnings`.
    pub fn parse(table: &Table, warnings: &mut Vec<Warning>) -> Result<Self, ThemeError> {
        let mut raw = DEFAULTS;
        let mut border_style = BorderStyle::Rounded;
        for (key, value) in table {
            let path = format!("theme.{key}");
            if let Some((_, slot)) = ROLES
                .into_iter()
                .zip(&mut raw)
                .find(|(role, _)| role == key)
            {
                *slot = ThemeColor::from_value(value).map_err(|m| bad(&path, m))?;
                continue;
            }
            match key.as_str() {
                "border_style" => {
                    border_style =
                        value.as_str().and_then(BorderStyle::parse).ok_or_else(|| {
                            bad(
                                &path,
                                "expected \"rounded\", \"plain\", \"thick\" or \"double\"",
                            )
                        })?;
                }
                "icons" | "icon_images" | "glyphs" | "icon_glyphs" | "icon_colors" => {
                    warnings.push(Warning {
                        key: path.clone(),
                        message: format!("{path} was removed: deck ships its own icons"),
                    });
                }
                _ => warnings.push(Warning::unknown_key(&path)),
            }
        }
        Ok(Self::from_parts(resolve(&raw)?, border_style))
    }
}

impl Default for Theme {
    #[expect(
        clippy::expect_used,
        reason = "the built-in roles are verified by the default theme tests"
    )]
    fn default() -> Self {
        let colors = resolve(&DEFAULTS).expect("the default roles do not loop");
        Self::from_parts(colors, BorderStyle::Rounded)
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
    }

    fn parse(text: &str) -> Result<Theme, ThemeError> {
        let table: Table = text.parse().unwrap();
        Theme::parse(&table, &mut Vec::new())
    }

    #[test]
    fn every_colour_form_parses() {
        let t = parse(
            r##"
            accent = "#CBA6F7"
            muted = 0
            ok = 255
            warning = "default"
            error = "bright-white"
            border = "black"
            border_focused = "muted"
            title = "bright-black"
            "##,
        )
        .unwrap();
        assert_eq!(t.accent, Color::Rgb(0xcb, 0xa6, 0xf7));
        assert_eq!(t.muted, Color::Indexed(0));
        assert_eq!(t.ok, Color::Indexed(255));
        assert_eq!(t.warning, Color::Default);
        assert_eq!(t.error, Color::Indexed(15));
        assert_eq!(t.border, Color::Indexed(0));
        assert_eq!(t.border_focused, Color::Indexed(0));
        assert_eq!(t.title, Color::Indexed(8));
        for (i, name) in ANSI_NAMES.iter().enumerate() {
            let t = parse(&format!("accent = \"{name}\"")).unwrap();
            assert_eq!(t.accent, Color::Indexed(u8::try_from(i).unwrap()), "{name}");
        }
    }

    #[test]
    fn search_roles_default_to_black_on_yellow_and_take_a_role() {
        let t = parse("").unwrap();
        assert_eq!(
            (t.search_match_fg, t.search_match_bg),
            (Color::Indexed(0), Color::Indexed(3))
        );
        assert_eq!(
            (t.search_current_fg, t.search_current_bg),
            (Color::Indexed(0), Color::Indexed(11))
        );
        let t = parse("accent = 5\nsearch_match_bg = \"accent\"").unwrap();
        assert_eq!(t.search_match_bg, Color::Indexed(5));
    }

    #[test]
    fn hover_defaults_to_bright_black_and_takes_a_role() {
        assert_eq!(parse("").unwrap().hover, Color::Indexed(8));
        assert_eq!(
            parse("hover = \"#102030\"").unwrap().hover,
            Color::Rgb(16, 32, 48)
        );
        let t = parse("accent = 5\nhover = \"accent\"").unwrap();
        assert_eq!(t.hover, Color::Indexed(5));
    }

    #[test]
    fn each_removed_icon_key_warns_once_and_changes_nothing() {
        for (text, key) in [
            ("icons = \"nerd\"", "theme.icons"),
            ("icon_images = \"off\"", "theme.icon_images"),
            ("icons = 1", "theme.icons"),
            (
                "[glyphs]\nexpanded = \"a\"\ncollapsed = \"b\"\nidle = \"日\"",
                "theme.glyphs",
            ),
            ("[icon_glyphs]\nclaude = \"C\"", "theme.icon_glyphs"),
            ("[icon_colors]\nclaude = \"none\"", "theme.icon_colors"),
        ] {
            let table: Table = text.parse().unwrap();
            let mut warnings = Vec::new();
            let t = Theme::parse(&table, &mut warnings).unwrap();
            assert_eq!(t, Theme::default(), "{text}");
            assert_eq!(warnings.len(), 1, "{text}: {warnings:?}");
            assert_eq!(warnings[0].key, key);
            assert_eq!(
                warnings[0].message,
                format!("{key} was removed: deck ships its own icons")
            );
        }
    }

    #[test]
    fn the_default_follows_the_accent() {
        let t = parse("accent = \"#102030\"").unwrap();
        assert_eq!(t.border_focused, Color::Rgb(0x10, 0x20, 0x30));
    }

    #[test]
    fn bad_colours_and_references_name_the_key() {
        for (text, key) in [
            ("accent = 256", "theme.accent"),
            ("accent = -1", "theme.accent"),
            ("muted = \"#ggg\"", "theme.muted"),
            ("muted = \"#12345\"", "theme.muted"),
            ("ok = \"purple\"", "theme.ok"),
            ("ok = \"nonrole\"", "theme.ok"),
            ("error = 1.5", "theme.error"),
            ("accent = \"muted\"\nmuted = \"accent\"", "theme.accent"),
            ("title = \"title\"", "theme.title"),
            ("border_style = \"dotted\"", "theme.border_style"),
            ("border_style = 1", "theme.border_style"),
        ] {
            let err = parse(text).unwrap_err();
            assert_eq!(err.key, key, "{text}: {}", err.message);
        }
        let err = parse("accent = \"muted\"\nmuted = \"accent\"").unwrap_err();
        assert!(
            err.message.contains("accent -> muted -> accent"),
            "{}",
            err.message
        );
    }

    #[test]
    fn border_styles_parse() {
        for (text, style) in [
            ("rounded", BorderStyle::Rounded),
            ("plain", BorderStyle::Plain),
            ("thick", BorderStyle::Thick),
            ("double", BorderStyle::Double),
        ] {
            let t = parse(&format!("border_style = \"{text}\"")).unwrap();
            assert_eq!(t.border_style, style);
        }
    }

    #[test]
    fn unknown_keys_warn_and_empty_is_the_default() {
        let table: Table = "colour = 1\nother = 2".parse().unwrap();
        let mut warnings = Vec::new();
        let t = Theme::parse(&table, &mut warnings).unwrap();
        assert_eq!(t, Theme::default());
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert_eq!(parse("").unwrap(), Theme::default());
    }
}
