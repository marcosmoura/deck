//! Agent logos as images, through the kitty graphics protocol (Ghostty and kitty).
//!
//! Each logo is a 64x64 alpha mask (`assets/logos`). The client tints it with the colour its
//! text icon would have had and sends it once as a virtual placement of two cells
//! (`U=1,c=2,r=1`, direct RGBA). The image is then drawn like text: a cell holding the
//! placeholder character U+10EEEE with the row and column diacritics of its top-left cell,
//! whose foreground colour carries the image id (see
//! <https://sw.kovidgoyal.net/kitty/graphics-protocol/#unicode-placeholders>).
//!
//! Every command has `q=2`, so the terminal never answers into the input.

use std::collections::HashMap;
use std::io::{self, Write};

use deck_core::base64;
use deck_core::icons::IconKind;
use deck_core::model::PanelView;
use deck_core::theme::{IconImages, IconPreset};
use ratatui::style::{Color, Modifier, Style};

use crate::app::App;
use crate::colors::{Palette, Rgb};
use crate::convert::{self, Resolved};
use crate::sidebar::{self, Row};

/// The placeholder character of the kitty protocol's Unicode placeholders.
pub const PLACEHOLDER: char = '\u{10EEEE}';
/// The combining diacritic that stands for 0 in the protocol's row and column table.
pub const DIACRITIC_ZERO: char = '\u{0305}';
/// The one that stands for 1: column 1 of the second cell.
pub const DIACRITIC_ONE: char = '\u{030D}';

/// The first image id; the counter goes up from here, so an id is never 0.
const FIRST_ID: u32 = 0xDEC001;
/// Ids are 24 bits: they travel in the foreground colour of the placeholder cell.
pub(crate) const LAST_ID: u32 = 0x00FF_FFFF;
/// The side of a mask, in pixels.
const SIDE: usize = 64;
/// The largest payload of one chunk, in base64 bytes.
const CHUNK: usize = 4096;
/// The foreground assumed when the terminal did not answer OSC 10.
const DEFAULT_FG: Rgb = (0xc0, 0xc0, 0xc0);

/// The standard xterm values of the 16 base colours, for an index the terminal did not give.
const XTERM: [Rgb; 16] = [
    (0x00, 0x00, 0x00),
    (0xcd, 0x00, 0x00),
    (0x00, 0xcd, 0x00),
    (0xcd, 0xcd, 0x00),
    (0x00, 0x00, 0xee),
    (0xcd, 0x00, 0xcd),
    (0x00, 0xcd, 0xcd),
    (0xe5, 0xe5, 0xe5),
    (0x7f, 0x7f, 0x7f),
    (0xff, 0x00, 0x00),
    (0x00, 0xff, 0x00),
    (0xff, 0xff, 0x00),
    (0x5c, 0x5c, 0xff),
    (0xff, 0x00, 0xff),
    (0x00, 0xff, 0xff),
    (0xff, 0xff, 0xff),
];

/// Whether logos are drawn as images: `icon_images` is `auto`, the outer terminal is Ghostty or
/// kitty (by `TERM`, `TERM_PROGRAM` or `KITTY_WINDOW_ID`), not behind tmux, Zellij, screen or
/// another deck, and it draws in 24-bit colour.
pub fn detect(env: &HashMap<String, String>, icon_images: IconImages, truecolor: bool) -> bool {
    let var = |name: &str| env.get(name).map(String::as_str).filter(|v| !v.is_empty());
    if icon_images != IconImages::Auto || !truecolor {
        return false;
    }
    if ["TMUX", "ZELLIJ", "DECK"]
        .into_iter()
        .any(|n| var(n).is_some())
    {
        return false;
    }
    let term = var("TERM").unwrap_or("");
    if term.starts_with("screen") || term.starts_with("tmux") {
        return false;
    }
    matches!(term, "xterm-kitty" | "xterm-ghostty")
        || var("TERM_PROGRAM") == Some("ghostty")
        || var("KITTY_WINDOW_ID").is_some()
}

/// A logo with a mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Logo {
    Claude,
    Openai,
    Opencode,
}

impl Logo {
    /// The logo of an icon kind, if it has one.
    pub fn of(kind: IconKind) -> Option<Self> {
        match kind {
            IconKind::Claude => Some(Self::Claude),
            IconKind::Codex => Some(Self::Openai),
            IconKind::OpenCode => Some(Self::Opencode),
            _ => None,
        }
    }

    /// The 64x64 alpha mask, row-major.
    pub fn mask(self) -> &'static [u8] {
        match self {
            Self::Claude => include_bytes!("../assets/logos/claude.mask"),
            Self::Openai => include_bytes!("../assets/logos/openai.mask"),
            Self::Opencode => include_bytes!("../assets/logos/opencode.mask"),
        }
    }
}

/// An image: a logo in one colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Pair {
    pub logo: Logo,
    pub rgb: Rgb,
}

impl Pair {
    /// The tinted image as RGBA: the colour in every pixel, the mask as alpha.
    pub fn rgba(&self) -> Vec<u8> {
        let (r, g, b) = self.rgb;
        self.logo
            .mask()
            .iter()
            .flat_map(|&alpha| [r, g, b, alpha])
            .collect()
    }
}

/// The cell text of an image, two cells wide: the placeholders of row 0, column 0 and row 0,
/// column 1.
pub fn placeholder() -> String {
    [
        PLACEHOLDER,
        DIACRITIC_ZERO,
        DIACRITIC_ZERO,
        PLACEHOLDER,
        DIACRITIC_ZERO,
        DIACRITIC_ONE,
    ]
    .into_iter()
    .collect()
}

/// The commands that send `pair` as image `id`: a virtual placement of two cells (`c=2,r=1`), chunked.
pub fn transmit(id: u32, pair: &Pair) -> Vec<u8> {
    let data = base64::encode(&pair.rgba());
    let chunks: Vec<&[u8]> = data.as_bytes().chunks(CHUNK).collect();
    let mut out = Vec::with_capacity(data.len() + 64 * chunks.len());
    for (n, chunk) in chunks.iter().enumerate() {
        let more = u8::from(n + 1 < chunks.len());
        if n == 0 {
            out.extend_from_slice(
                format!("\x1b_Ga=T,U=1,f=32,s={SIDE},v={SIDE},c=2,r=1,i={id},q=2,m={more};")
                    .as_bytes(),
            );
        } else {
            out.extend_from_slice(format!("\x1b_Gm={more},q=2;").as_bytes());
        }
        out.extend_from_slice(chunk);
        out.extend_from_slice(b"\x1b\\");
    }
    out
}

/// The command that deletes image `id` and its data.
pub fn delete(id: u32) -> Vec<u8> {
    format!("\x1b_Ga=d,d=I,i={id},q=2\x1b\\").into_bytes()
}

/// The images of one client run: the colours of the outer terminal, the id of each image
/// and the ones not sent yet.
#[derive(Debug)]
pub struct Images {
    fg: Option<Rgb>,
    bg: Option<Rgb>,
    palette: Palette,
    ids: HashMap<Pair, u32>,
    /// Ids in the order they were given, to send and to delete.
    pub(crate) order: Vec<(u32, Pair)>,
    sent: usize,
    pub(crate) next: u32,
}

impl Images {
    pub fn new(fg: Option<Rgb>, bg: Option<Rgb>, palette: Palette) -> Self {
        Self {
            fg,
            bg,
            palette,
            ids: HashMap::new(),
            order: Vec::new(),
            sent: 0,
            next: FIRST_ID,
        }
    }

    /// `color` as RGB: its own value, or the outer terminal's answer for the default
    /// foreground and the 16 base colours.
    pub fn resolve(&self, color: Color) -> Rgb {
        let base = |n: usize| {
            self.palette
                .get(n)
                .copied()
                .flatten()
                .or_else(|| XTERM.get(n).copied())
                .unwrap_or(DEFAULT_FG)
        };
        match convert::classify(color) {
            Resolved::Rgb(r, g, b) => (r, g, b),
            Resolved::Default => self.fg.unwrap_or(DEFAULT_FG),
            Resolved::Indexed(n @ 0..=15) => base(usize::from(n)),
            Resolved::Indexed(n @ 16..=231) => cube(n),
            Resolved::Indexed(n) => {
                let v = 8 + 10 * (n - 232);
                (v, v, v)
            }
        }
    }

    /// How an image is drawn in a cell styled `style`: the tint (the colour the text shows,
    /// with `REVERSED` resolved) and the cell's style: the visible background, no modifiers
    /// (kitty reserves every attribute but the colours). The caller sets the foreground to
    /// the image id.
    pub fn cell_for(&self, style: Style) -> (Rgb, Style) {
        let reversed = style.add_modifier.contains(Modifier::REVERSED)
            && !style.sub_modifier.contains(Modifier::REVERSED);
        let fg = style.fg.unwrap_or_default();
        let bg = style.bg.unwrap_or_default();
        // A reversed `default` is the outer terminal's other colour.
        let outer = |c: Color, rgb: Option<Rgb>, fallback: Rgb| {
            if convert::classify(c) == Resolved::Default {
                convert::rgb(rgb.unwrap_or(fallback))
            } else {
                c
            }
        };
        let (text, back) = if reversed {
            (
                outer(bg, self.bg, (0, 0, 0)),
                outer(fg, self.fg, DEFAULT_FG),
            )
        } else {
            (fg, bg)
        };
        (
            self.resolve(text),
            Style::new().bg(back).remove_modifier(Modifier::all()),
        )
    }

    /// The id of an image already registered.
    pub fn id(&self, pair: &Pair) -> Option<u32> {
        self.ids.get(pair).copied()
    }

    /// Gives `pair` an id if it has none. `false` when the 24-bit ids ran out.
    pub fn register(&mut self, pair: Pair) -> bool {
        if self.ids.contains_key(&pair) {
            return true;
        }
        if self.next > LAST_ID {
            return false;
        }
        self.ids.insert(pair, self.next);
        self.order.push((self.next, pair));
        self.next += 1;
        true
    }

    /// Writes the images registered since the last call, then flushes.
    pub fn send_pending(&mut self, out: &mut impl Write) -> io::Result<()> {
        let Some(pending) = self.order.get(self.sent..).filter(|p| !p.is_empty()) else {
            return Ok(());
        };
        for (id, pair) in pending {
            out.write_all(&transmit(*id, pair))?;
        }
        self.sent = self.order.len();
        out.flush()
    }

    /// Deletes every image that was sent, then flushes.
    pub fn delete_all(&self, out: &mut impl Write) -> io::Result<()> {
        let sent = self.order.get(..self.sent).unwrap_or_default();
        if sent.is_empty() {
            return Ok(());
        }
        for (id, _) in sent {
            out.write_all(&delete(*id))?;
        }
        out.flush()
    }
}

/// An entry of the 6x6x6 cube (16 to 231).
fn cube(n: u8) -> Rgb {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let i = usize::from(n - 16);
    let level = |k: usize| LEVELS.get(k).copied().unwrap_or(0);
    (level(i / 36), level(i / 6 % 6), level(i % 6))
}

impl App {
    /// The images, when the logos are drawn as images now: the terminal shows them, the
    /// theme asks for them and has icons, and the colours are 24-bit.
    pub(crate) fn logos(&self) -> Option<&Images> {
        self.images.as_ref().filter(|_| {
            self.theme.icon_images == IconImages::Auto
                && self.theme.icons != IconPreset::None
                && self.truecolor
        })
    }

    /// The images the next frame draws, as the drawing picks them: every panel row of the
    /// sidebar in its row colour.
    fn wanted_pairs(&self) -> Vec<Pair> {
        let (Some(images), Some(view)) = (self.logos(), self.mirror.view()) else {
            return Vec::new();
        };
        let theme = &self.theme;
        let base = sidebar::base_style(theme);
        let selected = self.sidebar_focus.then_some(self.selection).flatten();
        let pair = |panel: &PanelView, style: Style| {
            let kind = sidebar::panel_icon_kind(panel, theme)?;
            Some(Pair {
                logo: Logo::of(kind)?,
                rgb: images.cell_for(sidebar::icon_style(theme, kind, style)).0,
            })
        };
        let dragged = self.row_drag.map(|d| d.item);
        let mut out = Vec::new();
        for row in sidebar::rows(view, 0) {
            let Row::Panel(_, id) = row else { continue };
            let Some((_, panel)) = self.mirror.find_panel(id) else {
                continue;
            };
            let is_selected = row.sel().is_some() && row.sel() == selected;
            let active = view.active_panel == Some(id);
            let band = is_selected || (active && !self.sidebar_focus);
            let style = if row.sel().is_some() && row.sel() == dragged {
                sidebar::dimmed_style(theme, base)
            } else {
                sidebar::panel_style(theme, base, band, active)
            };
            out.extend(pair(panel, style));
        }
        // The menu's items and the restore dialog's headings, as they are drawn.
        let kind_pair = |kind: IconKind, style: Style| {
            Some(Pair {
                logo: Logo::of(kind)?,
                rgb: images.cell_for(sidebar::icon_style(theme, kind, style)).0,
            })
        };
        if self.menu.is_some() {
            for (i, kind) in crate::menu::ICONS.iter().enumerate() {
                let selected = self.menu.as_ref().is_some_and(|m| m.selected == i);
                out.extend(kind_pair(*kind, crate::menu::item_style(theme, selected)));
            }
        }
        if let (Some(restore), Some(g)) = (&self.restore, self.restore_geometry()) {
            for i in g.shown(restore.active) {
                let Some(kind) = crate::restore::COLUMNS.get(i) else {
                    continue;
                };
                let style = crate::restore::heading_style(theme, i == restore.active || g.narrow);
                out.extend(kind_pair(crate::restore::agent_icon_kind(*kind), style));
            }
        }
        out
    }

    /// Before a frame: sends the images it draws that the terminal does not have yet.
    pub fn send_images(&mut self, out: &mut impl Write) -> io::Result<()> {
        let pairs = self.wanted_pairs();
        match &mut self.images {
            Some(images) if !pairs.is_empty() => {
                for pair in pairs {
                    images.register(pair);
                }
                images.send_pending(out)
            }
            _ => Ok(()),
        }
    }

    /// On the way out: deletes the images that were sent.
    pub fn delete_images(&self, out: &mut impl Write) -> io::Result<()> {
        self.images
            .as_ref()
            .map_or(Ok(()), |images| images.delete_all(out))
    }
}
