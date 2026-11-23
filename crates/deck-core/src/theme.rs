//! The colours, border style and glyphs of deck's own UI (`[theme]` in `config.toml`):
//! colour syntax, role references and the resolved [`Theme`].

use serde::{Deserialize, Serialize};
use toml::{Table, Value};
use unicode_width::UnicodeWidthStr;

use crate::config::Warning;
use crate::icons::IconKind;
use crate::protocol::Color;

/// The colour roles, in the order of [`Theme::colors`].
pub const ROLES: [&str; 15] = [
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Glyphs {
    pub expanded: String,
    pub collapsed: String,
    pub active: String,
    /// The frames of the spinner of a working agent, one cell wide each.
    pub spinner: Vec<String>,
    /// An agent that needs the user.
    pub waiting: String,
    /// An agent that finished its turn.
    pub idle: String,
    /// An agent found by its process that is not printing.
    pub quiet: String,
    /// Marks a panel whose agent needed the user or finished while nobody looked.
    pub unseen: String,
}

/// The default spinner: braille frames.
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

impl Default for Glyphs {
    fn default() -> Self {
        Self {
            expanded: "▾".into(),
            collapsed: "▸".into(),
            active: "▌".into(),
            spinner: SPINNER.iter().map(ToString::to_string).collect(),
            waiting: "!".into(),
            idle: "●".into(),
            quiet: "○".into(),
            unseen: "•".into(),
        }
    }
}

/// Which glyphs the program icons use: `[theme] icons`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IconPreset {
    /// Plain Unicode that every font has.
    Unicode,
    /// Font Awesome glyphs of the Nerd Fonts.
    Nerd,
    /// No icons are drawn.
    None,
}

impl IconPreset {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "unicode" => Some(Self::Unicode),
            "nerd" => Some(Self::Nerd),
            "none" => Some(Self::None),
            _ => None,
        }
    }
}

/// The glyph of each icon kind, one cell wide.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IconGlyphs {
    pub claude: String,
    pub codex: String,
    pub opencode: String,
    pub terminal: String,
    pub editor: String,
    pub git: String,
    pub monitor: String,
    pub remote: String,
    pub build: String,
    pub pager: String,
    pub repl: String,
}

impl IconGlyphs {
    /// The glyphs of a preset (`None` has the `Unicode` ones; they are not drawn).
    pub fn preset(preset: IconPreset) -> Self {
        let g = |text: &str| text.to_string();
        match preset {
            IconPreset::Unicode | IconPreset::None => Self {
                claude: g("✻"),
                codex: g("◎"),
                opencode: g("◇"),
                terminal: g("❯"),
                editor: g("✎"),
                git: g("±"),
                monitor: g("◔"),
                remote: g("⇄"),
                build: g("▲"),
                pager: g("¶"),
                repl: g("λ"),
            },
            // Codicons for the agents and Font Awesome for the rest, checked against Nerd
            // Fonts' `glyphnames.json` (3.5.1). Nerd Fonts has no OpenCode logo: `▣` stands in.
            IconPreset::Nerd => Self {
                claude: g("\u{ec82}"),   // nf-cod-claude
                codex: g("\u{ec81}"),    // nf-cod-openai
                opencode: g("▣"),        // no Nerd Font glyph
                terminal: g("\u{f120}"), // nf-fa-terminal
                editor: g("\u{f040}"),   // nf-fa-pencil
                git: g("\u{f126}"),      // nf-fa-code_fork
                monitor: g("\u{f0e4}"),  // nf-fa-tachometer
                remote: g("\u{f0ec}"),   // nf-fa-exchange
                build: g("\u{f013}"),    // nf-fa-cog
                pager: g("\u{f02d}"),    // nf-fa-book
                repl: g("\u{f121}"),     // nf-fa-code
            },
        }
    }

    pub fn get(&self, kind: IconKind) -> &str {
        match kind {
            IconKind::Claude => &self.claude,
            IconKind::Codex => &self.codex,
            IconKind::OpenCode => &self.opencode,
            IconKind::Terminal => &self.terminal,
            IconKind::Editor => &self.editor,
            IconKind::Git => &self.git,
            IconKind::Monitor => &self.monitor,
            IconKind::Remote => &self.remote,
            IconKind::Build => &self.build,
            IconKind::Pager => &self.pager,
            IconKind::Repl => &self.repl,
        }
    }

    fn set(&mut self, kind: IconKind, glyph: String) {
        let slot = match kind {
            IconKind::Claude => &mut self.claude,
            IconKind::Codex => &mut self.codex,
            IconKind::OpenCode => &mut self.opencode,
            IconKind::Terminal => &mut self.terminal,
            IconKind::Editor => &mut self.editor,
            IconKind::Git => &mut self.git,
            IconKind::Monitor => &mut self.monitor,
            IconKind::Remote => &mut self.remote,
            IconKind::Build => &mut self.build,
            IconKind::Pager => &mut self.pager,
            IconKind::Repl => &mut self.repl,
        };
        *slot = glyph;
    }
}

/// A glyph: a string one cell wide.
fn one_cell(path: &str, value: &toml::Value) -> Result<String, ThemeError> {
    let text = value
        .as_str()
        .ok_or_else(|| bad(path, "expected a string"))?;
    let cells = text.width();
    if cells != 1 {
        return Err(bad(
            path,
            format!("a glyph must be one cell wide, `{text}` is {cells}"),
        ));
    }
    Ok(text.to_string())
}

/// The spinner: a non-empty array of glyphs.
fn spinner_frames(path: &str, value: &toml::Value) -> Result<Vec<String>, ThemeError> {
    let items = value
        .as_array()
        .ok_or_else(|| bad(path, "expected an array of glyphs"))?;
    if items.is_empty() {
        return Err(bad(path, "the spinner needs at least one frame"));
    }
    items.iter().map(|v| one_cell(path, v)).collect()
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
    pub border_style: BorderStyle,
    pub glyphs: Glyphs,
    pub icons: IconPreset,
    pub icon_glyphs: IconGlyphs,
}

/// The default of each role, in the order of [`ROLES`].
const DEFAULTS: [ThemeColor; 15] = [
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
];

/// Follows role references until a colour; a loop is an error naming the keys in it.
fn resolve(raw: &[ThemeColor; 15]) -> Result<[Color; 15], ThemeError> {
    let mut out = [Color::Default; 15];
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
    fn from_parts(
        c: [Color; 15],
        border_style: BorderStyle,
        glyphs: Glyphs,
        icons: IconPreset,
        icon_glyphs: IconGlyphs,
    ) -> Self {
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
            border_style,
            glyphs,
            icons,
            icon_glyphs,
        }
    }

    /// Reads the `[theme]` table. Unknown keys are added to `warnings`.
    pub fn parse(table: &Table, warnings: &mut Vec<Warning>) -> Result<Self, ThemeError> {
        let mut raw = DEFAULTS;
        let mut border_style = BorderStyle::Rounded;
        let mut glyphs = Glyphs::default();
        let mut icons = IconPreset::Unicode;
        let mut overrides = Vec::new();
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
                "glyphs" => {
                    let table = value
                        .as_table()
                        .ok_or_else(|| bad(&path, "expected a table"))?;
                    for (name, value) in table {
                        let path = format!("{path}.{name}");
                        if name == "spinner" {
                            glyphs.spinner = spinner_frames(&path, value)?;
                            continue;
                        }
                        let slot = match name.as_str() {
                            "expanded" => &mut glyphs.expanded,
                            "collapsed" => &mut glyphs.collapsed,
                            "active" => &mut glyphs.active,
                            "waiting" => &mut glyphs.waiting,
                            "idle" => &mut glyphs.idle,
                            "quiet" => &mut glyphs.quiet,
                            "unseen" => &mut glyphs.unseen,
                            _ => {
                                warnings.push(Warning::unknown_key(&path));
                                continue;
                            }
                        };
                        *slot = one_cell(&path, value)?;
                    }
                }
                "icons" => {
                    icons = value
                        .as_str()
                        .and_then(IconPreset::parse)
                        .ok_or_else(|| bad(&path, "expected \"unicode\", \"nerd\" or \"none\""))?;
                }
                "icon_glyphs" => {
                    let table = value
                        .as_table()
                        .ok_or_else(|| bad(&path, "expected a table"))?;
                    for (name, value) in table {
                        let path = format!("{path}.{name}");
                        let kind = IconKind::parse(name).ok_or_else(|| {
                            let kinds = IconKind::ALL.map(IconKind::name).join(", ");
                            bad(
                                &path,
                                format!("unknown icon `{name}`; expected one of {kinds}"),
                            )
                        })?;
                        overrides.push((kind, one_cell(&path, value)?));
                    }
                }
                _ => warnings.push(Warning::unknown_key(&path)),
            }
        }
        // `none` ignores the overrides; they are still checked above.
        let mut icon_glyphs = IconGlyphs::preset(icons);
        if icons != IconPreset::None {
            for (kind, glyph) in overrides {
                icon_glyphs.set(kind, glyph);
            }
        }
        Ok(Self::from_parts(
            resolve(&raw)?,
            border_style,
            glyphs,
            icons,
            icon_glyphs,
        ))
    }
}

impl Default for Theme {
    #[expect(
        clippy::expect_used,
        reason = "the built-in roles are verified by the default theme tests"
    )]
    fn default() -> Self {
        let colors = resolve(&DEFAULTS).expect("the default roles do not loop");
        Self::from_parts(
            colors,
            BorderStyle::Rounded,
            Glyphs::default(),
            IconPreset::Unicode,
            IconGlyphs::preset(IconPreset::Unicode),
        )
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
            ("[glyphs]\ncollapsed = \"日\"", "theme.glyphs.collapsed"),
            ("[glyphs]\nactive = \"\"", "theme.glyphs.active"),
            ("[glyphs]\nexpanded = \"ab\"", "theme.glyphs.expanded"),
            ("glyphs = 1", "theme.glyphs"),
            ("icons = \"emoji\"", "theme.icons"),
            ("icons = true", "theme.icons"),
            ("icon_glyphs = 1", "theme.icon_glyphs"),
            ("[icon_glyphs]\nclaude = \"日\"", "theme.icon_glyphs.claude"),
            ("[icon_glyphs]\ngit = \"\"", "theme.icon_glyphs.git"),
            ("[icon_glyphs]\nbogus = \"x\"", "theme.icon_glyphs.bogus"),
            (
                "icons = \"none\"\n[icon_glyphs]\nrepl = \"ab\"",
                "theme.icon_glyphs.repl",
            ),
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
    fn styles_and_glyphs() {
        let t = parse("border_style = \"double\"\n[glyphs]\ncollapsed = \">\"").unwrap();
        assert_eq!(t.border_style, BorderStyle::Double);
        assert_eq!(t.glyphs.collapsed, ">");
        assert_eq!(t.glyphs.expanded, "▾");
        for (text, style) in [
            ("rounded", BorderStyle::Rounded),
            ("plain", BorderStyle::Plain),
            ("thick", BorderStyle::Thick),
        ] {
            let t = parse(&format!("border_style = \"{text}\"")).unwrap();
            assert_eq!(t.border_style, style);
        }
    }

    #[test]
    fn unknown_keys_warn_and_empty_is_the_default() {
        let table: Table = "colour = 1\n[glyphs]\nstar = \"*\"".parse().unwrap();
        let mut warnings = Vec::new();
        let t = Theme::parse(&table, &mut warnings).unwrap();
        assert_eq!(t, Theme::default());
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert_eq!(parse("").unwrap(), Theme::default());
    }

    #[test]
    fn the_icon_presets_are_one_cell_wide_and_distinct() {
        for preset in [IconPreset::Unicode, IconPreset::Nerd] {
            let glyphs = IconGlyphs::preset(preset);
            let mut seen = Vec::new();
            for kind in IconKind::ALL {
                let glyph = glyphs.get(kind);
                assert_eq!(glyph.width(), 1, "{preset:?} {}", kind.name());
                assert!(!seen.contains(&glyph), "{preset:?} repeats {glyph}");
                seen.push(glyph);
            }
        }
        assert_eq!(parse("").unwrap().icons, IconPreset::Unicode);
        assert_eq!(
            parse("icons = \"nerd\"").unwrap().icon_glyphs.claude,
            "\u{ec82}"
        );
        assert_eq!(
            parse("icons = \"unicode\"").unwrap().icon_glyphs.terminal,
            "❯"
        );
        assert_eq!(parse("icons = \"none\"").unwrap().icons, IconPreset::None);
    }

    #[test]
    fn single_icons_can_be_overridden() {
        let t = parse("[icon_glyphs]\nclaude = \"C\"").unwrap();
        assert_eq!(t.icon_glyphs.claude, "C");
        assert_eq!(t.icon_glyphs.codex, "◎");
        let t = parse("icons = \"nerd\"\n[icon_glyphs]\ngit = \"G\"").unwrap();
        assert_eq!(t.icon_glyphs.git, "G");
        assert_eq!(t.icon_glyphs.editor, "\u{f040}");
        // With `none` the overrides are ignored.
        let t = parse("icons = \"none\"\n[icon_glyphs]\ngit = \"G\"").unwrap();
        assert_eq!(t.icon_glyphs, IconGlyphs::preset(IconPreset::Unicode));
    }
}
