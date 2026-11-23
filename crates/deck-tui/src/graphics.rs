//! Icons as images, through the kitty graphics protocol (Ghostty and kitty).
//!
//! Each icon is a 64x64 alpha mask (`assets/icons`). The client tints it with the colour its
//! text icon would have had and sends it once as a virtual placement of two cells
//! (`U=1,c=2,r=1`, direct RGBA). The image is then drawn like text: a cell holding the
//! placeholder character U+10EEEE with the row and column diacritics of its top-left cell,
//! whose foreground colour carries the image id (see
//! <https://sw.kovidgoyal.net/kitty/graphics-protocol/#unicode-placeholders>).
//!
//! Every command has `q=2`, so the terminal never answers into the input.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::OnceLock;

use deck_core::base64;
use deck_core::icons::{Icon, IconKind};
use ratatui::Terminal;
use ratatui::backend::Backend;
#[cfg(test)]
use ratatui::backend::TestBackend;
use ratatui::style::{Color, Modifier, Style};

use crate::app::App;
use crate::colors::{Palette, Rgb};
use crate::convert::{self, Resolved};
use crate::tier::Tier;

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

/// Whether icons are drawn as images: the outer terminal is Ghostty or
/// kitty (by `TERM`, `TERM_PROGRAM` or `KITTY_WINDOW_ID`), not behind tmux, Zellij, screen or
/// another deck, and it draws in 24-bit colour.
pub fn detect(env: &HashMap<String, String>, truecolor: bool) -> bool {
    let var = |name: &str| env.get(name).map(String::as_str).filter(|v| !v.is_empty());
    if !truecolor || multiplexed(env) {
        return false;
    }
    matches!(var("TERM"), Some("xterm-kitty" | "xterm-ghostty"))
        || var("TERM_PROGRAM") == Some("ghostty")
        || var("KITTY_WINDOW_ID").is_some()
}

/// Whether deck runs behind tmux, Zellij, screen or another deck, where images are off.
pub fn multiplexed(env: &HashMap<String, String>) -> bool {
    let var = |name: &str| env.get(name).map(String::as_str).filter(|v| !v.is_empty());
    let term = var("TERM").unwrap_or("");
    ["TMUX", "ZELLIJ", "DECK"]
        .into_iter()
        .any(|n| var(n).is_some())
        || term.starts_with("screen")
        || term.starts_with("tmux")
}

/// The bytes of a mask.
const MASK_LEN: usize = SIDE * SIDE;

/// The 64x64 alpha mask of an icon, row-major; none for the icons that stay text (the drop
/// caret).
pub fn mask(icon: Icon) -> Option<&'static [u8; MASK_LEN]> {
    Some(match icon {
        Icon::Kind(kind) => match kind {
            IconKind::Claude => include_bytes!("../assets/icons/claude.mask"),
            IconKind::Codex => include_bytes!("../assets/icons/openai.mask"),
            IconKind::OpenCode => include_bytes!("../assets/icons/opencode.mask"),
            IconKind::Terminal => include_bytes!("../assets/icons/terminal.mask"),
            IconKind::Editor => include_bytes!("../assets/icons/editor.mask"),
            IconKind::Git => include_bytes!("../assets/icons/git.mask"),
            IconKind::Monitor => include_bytes!("../assets/icons/monitor.mask"),
            IconKind::Remote => include_bytes!("../assets/icons/remote.mask"),
            IconKind::Build => include_bytes!("../assets/icons/build.mask"),
            IconKind::Pager => include_bytes!("../assets/icons/pager.mask"),
            IconKind::Repl => include_bytes!("../assets/icons/repl.mask"),
        },
        Icon::Expanded => include_bytes!("../assets/icons/expanded.mask"),
        Icon::Collapsed => include_bytes!("../assets/icons/collapsed.mask"),
        Icon::Spinner => include_bytes!("../assets/icons/spinner.mask"),
        Icon::Waiting => include_bytes!("../assets/icons/waiting.mask"),
        Icon::Idle => include_bytes!("../assets/icons/idle.mask"),
        Icon::Quiet => include_bytes!("../assets/icons/quiet.mask"),
        Icon::Unseen => include_bytes!("../assets/icons/unseen.mask"),
        Icon::New => include_bytes!("../assets/icons/new.mask"),
        Icon::Resume => include_bytes!("../assets/icons/resume.mask"),
        Icon::DropTarget => return None,
    })
}

/// Whether an icon is drawn as an image in the `Image` tier: all but the drop caret.
pub fn has_image(icon: Icon) -> bool {
    !matches!(icon, Icon::DropTarget)
}

/// How many frames one revolution of the loader has: one per degree step of 6.
pub const SPINNER_FRAMES: usize = 60;

/// The centre of a mask, in pixel coordinates.
const CENTRE: f32 = (SIDE as f32 - 1.0) / 2.0;

/// The `spinner` mask turned clockwise by `6 * k` degrees around its centre, sampled bilinearly,
/// pixels outside the source transparent. `k` wraps at 60. Computed once and cached.
pub fn spinner_frame(k: usize) -> [u8; MASK_LEN] {
    static FRAMES: OnceLock<Vec<[u8; MASK_LEN]>> = OnceLock::new();
    let frames = FRAMES.get_or_init(|| {
        let source = include_bytes!("../assets/icons/spinner.mask");
        (0..SPINNER_FRAMES).map(|k| rotate(source, k)).collect()
    });
    frames
        .get(k % SPINNER_FRAMES)
        .copied()
        .unwrap_or([0; MASK_LEN])
}

/// Frame `k` of the loader: `source` turned clockwise by `6 * k` degrees.
fn rotate(source: &[u8; MASK_LEN], k: usize) -> [u8; MASK_LEN] {
    let angle = (k * 360 / SPINNER_FRAMES) as f32 * std::f32::consts::PI / 180.0;
    let (sin, cos) = angle.sin_cos();
    let at = |x: i64, y: i64| -> f32 {
        let side = SIDE as i64;
        if (0..side).contains(&x) && (0..side).contains(&y) {
            source
                .get((y * side + x) as usize)
                .copied()
                .map_or(0.0, f32::from)
        } else {
            0.0
        }
    };
    let mut out = [0u8; MASK_LEN];
    for y in 0..SIDE {
        for x in 0..SIDE {
            // The source point that lands on this pixel: the inverse turn.
            let (dx, dy) = (x as f32 - CENTRE, y as f32 - CENTRE);
            let sx = CENTRE + cos * dx + sin * dy;
            let sy = CENTRE - sin * dx + cos * dy;
            let (x0, y0) = (sx.floor(), sy.floor());
            let (fx, fy) = (sx - x0, sy - y0);
            let (x0, y0) = (x0 as i64, y0 as i64);
            let top = at(x0, y0) * (1.0 - fx) + at(x0 + 1, y0) * fx;
            let bottom = at(x0, y0 + 1) * (1.0 - fx) + at(x0 + 1, y0 + 1) * fx;
            let value = top * (1.0 - fy) + bottom * fy;
            if let Some(px) = out.get_mut(y * SIDE + x) {
                *px = value.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// An image: an icon in one colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Pair {
    pub icon: Icon,
    pub rgb: Rgb,
    /// The loader's frame (0..60); 0 for every other icon.
    pub frame: u8,
}

impl Pair {
    /// The tinted image as RGBA: the colour in every pixel, the mask as alpha.
    pub fn rgba(&self) -> Vec<u8> {
        let (r, g, b) = self.rgb;
        let tint = |mask: &[u8; MASK_LEN]| mask.iter().flat_map(|&a| [r, g, b, a]).collect();
        if self.icon == Icon::Spinner {
            return tint(&spinner_frame(usize::from(self.frame)));
        }
        mask(self.icon).map(tint).unwrap_or_default()
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
    /// The pairs the drawing asked for since the last pre-pass.
    wanted: RefCell<Vec<Pair>>,
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
            wanted: RefCell::new(Vec::new()),
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

    /// Notes that the drawing wants `pair` (the pre-pass registers it).
    pub fn want(&self, pair: Pair) {
        self.wanted.borrow_mut().push(pair);
    }

    /// The pairs wanted since the last call.
    fn take_wanted(&self) -> Vec<Pair> {
        std::mem::take(&mut *self.wanted.borrow_mut())
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
    /// The images, when the icons are drawn as images now: the tier is `Image` and the
    /// colours are 24-bit.
    pub(crate) fn live_images(&self) -> Option<&Images> {
        self.images
            .as_ref()
            .filter(|_| self.tier == Tier::Image && self.truecolor)
    }

    /// Draws a frame. Every image slot (`sidebar::icon_slot`) notes the pair it wants, in the
    /// colour it draws in; when the frame brought a pair the terminal does not have, the pair is
    /// sent (flushed, so it goes out before any cell of the frame) and the frame is drawn again,
    /// now with the image. In the steady state the frame is drawn once.
    pub fn draw_frame<B>(&mut self, terminal: &mut Terminal<B>) -> io::Result<()>
    where
        B: Backend<Error = io::Error> + Write,
    {
        terminal.autoresize()?;
        if let Some(images) = self.live_images() {
            images.take_wanted();
        }
        let mut cursor = crate::draw::draw_screen(&mut terminal.get_frame(), self);
        if self.send_wanted(terminal.backend_mut())? {
            terminal.current_buffer_mut().reset();
            cursor = crate::draw::draw_screen(&mut terminal.get_frame(), self);
        }
        terminal.apply_buffer_with_cursor(cursor)?;
        Ok(())
    }

    /// Registers the pairs the last draw wanted and sends the new ones; whether there were any.
    fn send_wanted(&mut self, out: &mut impl Write) -> io::Result<bool> {
        let Some(images) = self.live_images() else {
            return Ok(false);
        };
        let pairs = images.take_wanted();
        let Some(images) = &mut self.images else {
            return Ok(false);
        };
        let mut fresh = false;
        for pair in pairs {
            if pair.icon == Icon::Spinner {
                // All 60 frames of a colour go out together, before the first one is drawn.
                for frame in 0..SPINNER_FRAMES {
                    let frame = Pair {
                        frame: u8::try_from(frame).unwrap_or(0),
                        ..pair
                    };
                    fresh |= images.id(&frame).is_none() && images.register(frame);
                }
            } else {
                fresh |= images.id(&pair).is_none() && images.register(pair);
            }
        }
        if fresh {
            images.send_pending(out)?;
        }
        Ok(fresh)
    }

    /// Sends the images a frame draws, as the pre-pass of a test: draws once into a scratch
    /// buffer.
    #[cfg(test)]
    pub fn send_images(&mut self, out: &mut impl Write) -> io::Result<()> {
        let Some(images) = self.live_images() else {
            return Ok(());
        };
        images.take_wanted();
        let Ok(mut scratch) = Terminal::new(TestBackend::new(self.size.0, self.size.1));
        let _ = scratch.draw(|f| crate::draw::draw(f, self));
        self.send_wanted(out)?;
        Ok(())
    }

    /// On the way out: deletes the images that were sent.
    pub fn delete_images(&self, out: &mut impl Write) -> io::Result<()> {
        self.images
            .as_ref()
            .map_or(Ok(()), |images| images.delete_all(out))
    }
}
