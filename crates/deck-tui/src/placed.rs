//! Images the programs in the terminals place (kitty graphics, `a=p` with a position).
//!
//! The daemon sends each image's pixels once, in pieces, and every frame lists the placements
//! that touch the terminal. The outer terminal fits a virtual placement's whole image into its
//! box and ignores source rectangles, so for each placement the client builds the pixels the
//! program asked for: the source rectangle, scaled to the rendered size, on a transparent canvas
//! of exactly `cols × cell width` by `rows × cell height`. That canvas goes to the outer
//! terminal as its own image with one virtual placement, and the cells the placement covers
//! hold Unicode placeholders (see `draw.rs`).

use std::collections::{HashMap, HashSet};
use std::io::{self, Write};

use deck_core::base64;
use deck_core::model::TerminalId;
use deck_core::protocol::{CellPx, ImageFormat, ImageScreen, Placement, PlacementKind, SourceRect};

use crate::diacritics::COUNT;
use crate::graphics::{Images, delete};

/// The most RGBA bytes of a canvas; a placement that needs more is not drawn.
pub const MAX_CANVAS: u64 = 64 << 20;
/// The most canvas bytes built and written before one draw; the placements over it wait for
/// the next draw, so a screen of large placements cannot hold the client in one of them.
pub const DRAW_BUDGET: u64 = MAX_CANVAS;
/// The most canvas bytes of the outer images of what is on screen; a placement over it gets
/// no outer image until others leave. Well under the 320 MB that kitty and Ghostty store by
/// default: the images a draw is about to delete and the virtual placements count there too.
pub const LIVE_BUDGET: u64 = 2 * MAX_CANVAS;
/// The most pixel bytes of one received image: the daemon's storage limit of a screen.
const MAX_IMAGE: u64 = 64 << 20;
/// Raw bytes of one chunk; 3072 bytes are 4096 in base64.
const RAW_CHUNK: usize = 3072;

/// An image of a terminal: the program's id, on one of the two screens.
pub type Slot = (TerminalId, ImageScreen, u32);

/// One `ServerMsg::Image`.
#[derive(Debug)]
pub struct Piece {
    pub slot: Slot,
    pub generation: u64,
    pub width: u32,
    pub height: u32,
    pub format: ImageFormat,
    pub offset: u32,
    pub total: u32,
    pub data: Vec<u8>,
}

fn bytes_per_pixel(format: ImageFormat) -> u64 {
    match format {
        ImageFormat::Rgb => 3,
        ImageFormat::Rgba => 4,
    }
}

/// A whole image.
#[derive(Debug)]
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub format: ImageFormat,
    pub data: Vec<u8>,
}

impl Pixels {
    /// The pixel at `x`, `y` as RGBA; transparent outside the image.
    fn at(&self, x: u32, y: u32) -> [u8; 4] {
        if x >= self.width || y >= self.height {
            return [0; 4];
        }
        let n = u64::from(y) * u64::from(self.width) + u64::from(x);
        let bpp = bytes_per_pixel(self.format);
        let Some(start) = usize::try_from(n * bpp).ok() else {
            return [0; 4];
        };
        match (self.format, self.data.get(start..start + bpp as usize)) {
            (ImageFormat::Rgba, Some(&[r, g, b, a])) => [r, g, b, a],
            (ImageFormat::Rgb, Some(&[r, g, b])) => [r, g, b, 255],
            _ => [0; 4],
        }
    }
}

#[derive(Debug)]
struct Assembly {
    generation: u64,
    width: u32,
    height: u32,
    format: ImageFormat,
    total: usize,
    data: Vec<u8>,
}

/// What makes one outer image: the pixels' identity, how they are cut and scaled, the cells
/// they fill (cut to the diacritics table) and the cell size they were built for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OuterKey {
    pub slot: Slot,
    pub generation: u64,
    pub source: (u32, u32, u32, u32),
    pub pixels: (u32, u32),
    pub cols: u16,
    pub rows: u16,
    pub cell: (u16, u16),
}

impl OuterKey {
    /// The key of a placed placement; `None` for a virtual one, an empty one, or without a
    /// cell size.
    pub fn of(terminal: TerminalId, p: &Placement, cell: Option<CellPx>) -> Option<Self> {
        let PlacementKind::Placed { .. } = p.kind else {
            return None;
        };
        let cell = cell?;
        let cut = |n: u16| n.min(u16::try_from(COUNT).unwrap_or(u16::MAX));
        let (cols, rows) = (cut(p.cols), cut(p.rows));
        (cols > 0 && rows > 0).then_some(Self {
            slot: (terminal, p.screen, p.image),
            generation: p.generation,
            source: (p.source.x, p.source.y, p.source.width, p.source.height),
            pixels: p.pixels,
            cols,
            rows,
            cell: (cell.width, cell.height),
        })
    }

    /// The RGBA bytes of the canvas of this placement.
    fn canvas_bytes(&self) -> u64 {
        let width = u64::from(self.cols) * u64::from(self.cell.0);
        let height = u64::from(self.rows) * u64::from(self.cell.1);
        width * height * 4
    }
}

/// What makes one outer image of a program's virtual placement: the image's own pixels, shown
/// in the cells the placement says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VirtualKey {
    pub slot: Slot,
    pub generation: u64,
    pub cols: u16,
    pub rows: u16,
}

impl VirtualKey {
    /// The key of a virtual placement; `None` for a placed one or an empty one.
    pub fn of(terminal: TerminalId, p: &Placement) -> Option<Self> {
        (p.kind == PlacementKind::Virtual && p.cols > 0 && p.rows > 0).then_some(Self {
            slot: (terminal, p.screen, p.image),
            generation: p.generation,
            cols: p.cols,
            rows: p.rows,
        })
    }
}

/// What the client keeps of the programs' images.
#[derive(Debug, Default)]
pub struct Program {
    assembling: HashMap<Slot, Assembly>,
    pixels: HashMap<Slot, (u64, Pixels)>,
    /// The outer image of each placed placement, by its outer id.
    outer: HashMap<OuterKey, u32>,
    /// The outer image of each virtual placement, by its outer id.
    virtuals: HashMap<VirtualKey, u32>,
    /// What the draw in progress wants.
    used: HashSet<OuterKey>,
    used_virtual: HashSet<VirtualKey>,
    /// Placements whose canvas is too big; not tried again while they stay wanted.
    refused: HashSet<OuterKey>,
    /// Outer ids whose delete is owed.
    doomed: Vec<u32>,
}

impl Program {
    /// Takes in one piece of an image.
    pub fn piece(&mut self, p: Piece) {
        let slot = p.slot;
        let known = self
            .assembling
            .get(&slot)
            .map(|a| a.generation)
            .into_iter()
            .chain(self.pixels.get(&slot).map(|(g, _)| *g))
            .max();
        if known.is_some_and(|g| p.generation < g) {
            return;
        }
        if p.offset == 0 {
            let expected = u64::from(p.width) * u64::from(p.height) * bytes_per_pixel(p.format);
            let total = u64::from(p.total);
            if total == 0 || total != expected || total > MAX_IMAGE {
                self.assembling.remove(&slot);
                return;
            }
            self.assembling.insert(
                slot,
                Assembly {
                    generation: p.generation,
                    width: p.width,
                    height: p.height,
                    format: p.format,
                    total: usize::try_from(total).unwrap_or(0),
                    data: Vec::with_capacity(usize::try_from(total).unwrap_or(0)),
                },
            );
        }
        let Some(a) = self.assembling.get_mut(&slot) else {
            return;
        };
        let at_end = usize::try_from(p.offset).ok() == Some(a.data.len());
        let fits = a.data.len() + p.data.len() <= a.total;
        if a.generation != p.generation || !at_end || !fits || p.total as usize != a.total {
            self.assembling.remove(&slot);
            return;
        }
        a.data.extend_from_slice(&p.data);
        if a.data.len() == a.total
            && let Some(a) = self.assembling.remove(&slot)
        {
            self.pixels.insert(
                slot,
                (
                    a.generation,
                    Pixels {
                        width: a.width,
                        height: a.height,
                        format: a.format,
                        data: a.data,
                    },
                ),
            );
        }
    }

    /// The daemon dropped an image.
    pub fn gone(&mut self, slot: Slot, generation: u64) {
        if self
            .pixels
            .get(&slot)
            .is_some_and(|(g, _)| *g == generation)
        {
            self.pixels.remove(&slot);
        }
        if self
            .assembling
            .get(&slot)
            .is_some_and(|a| a.generation == generation)
        {
            self.assembling.remove(&slot);
        }
    }

    /// Keeps the pixels of the terminals `alive` says are still in the session.
    pub fn retain_terminals(&mut self, alive: impl Fn(TerminalId) -> bool) {
        self.pixels.retain(|(t, ..), _| alive(*t));
        self.assembling.retain(|(t, ..), _| alive(*t));
    }

    /// Forgets everything (another session): the outer images are deleted by the next draw.
    pub fn clear(&mut self) {
        self.pixels.clear();
        self.assembling.clear();
        self.doomed.extend(self.outer.drain().map(|(_, id)| id));
        self.doomed.extend(self.virtuals.drain().map(|(_, id)| id));
        self.used.clear();
        self.used_virtual.clear();
        self.refused.clear();
    }

    /// Whether the pixels of an image are complete, for tests.
    #[cfg(test)]
    pub fn has_pixels(&self, slot: Slot, generation: u64) -> bool {
        self.pixels
            .get(&slot)
            .is_some_and(|(g, _)| *g == generation)
    }

    /// Whether an image is partly received, for tests.
    #[cfg(test)]
    pub fn assembling(&self, slot: Slot) -> bool {
        self.assembling.contains_key(&slot)
    }

    /// The outer ids in use, and the ones owed a delete.
    pub fn ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.outer
            .values()
            .copied()
            .chain(self.virtuals.values().copied())
            .chain(self.doomed.iter().copied())
    }
}

/// The pixels of a canvas.
#[derive(Debug, PartialEq, Eq)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// For each destination pixel along one axis: the two source pixels and the weight of the
/// second, with the pixel centres of both aligned.
fn axis(from: u32, to: u32, count: u32) -> Vec<(u32, u32, f32)> {
    let ratio = from as f32 / to as f32;
    let last = from.saturating_sub(1);
    (0..count)
        .map(|i| {
            let at = ((i as f32 + 0.5) * ratio - 0.5).clamp(0.0, last as f32);
            let a = (at.floor() as u32).min(last);
            let b = (a + 1).min(last);
            (a, b, at - a as f32)
        })
        .collect()
}

/// The canvas of a placement: `cols × rows` cells of `cell` pixels, transparent, with the
/// `source` rectangle of `image` scaled to `pixels` (bilinear, when the sizes differ) at its
/// top left, only the part that lands on the canvas computed. `None` when the canvas has no
/// pixels or is over [`MAX_CANVAS`].
pub fn canvas(
    image: &Pixels,
    source: SourceRect,
    pixels: (u32, u32),
    cols: u16,
    rows: u16,
    cell: CellPx,
) -> Option<Canvas> {
    let width = u64::from(cols) * u64::from(cell.width);
    let height = u64::from(rows) * u64::from(cell.height);
    if width == 0 || height == 0 || width * height * 4 > MAX_CANVAS {
        return None;
    }
    let (width, height) = (u32::try_from(width).ok()?, u32::try_from(height).ok()?);
    let mut rgba = vec![0u8; usize::try_from(u64::from(width) * u64::from(height) * 4).ok()?];

    let sx = source.x.min(image.width);
    let sy = source.y.min(image.height);
    let room = (image.width - sx, image.height - sy);
    let sw = if source.width == 0 {
        room.0
    } else {
        source.width.min(room.0)
    };
    let sh = if source.height == 0 {
        room.1
    } else {
        source.height.min(room.1)
    };
    if sw == 0 || sh == 0 {
        return Some(Canvas {
            width,
            height,
            rgba,
        });
    }
    let (pw, ph) = if pixels.0 == 0 || pixels.1 == 0 {
        (sw, sh)
    } else {
        pixels
    };
    let (dw, dh) = (pw.min(width), ph.min(height));
    let same = (pw, ph) == (sw, sh);
    let xs = if same { Vec::new() } else { axis(sw, pw, dw) };
    let ys = if same { Vec::new() } else { axis(sh, ph, dh) };
    for y in 0..dh {
        let row = usize::try_from(u64::from(y) * u64::from(width) * 4).ok()?;
        for x in 0..dw {
            let px = if same {
                image.at(sx + x, sy + y)
            } else {
                let (Some(&(x0, x1, tx)), Some(&(y0, y1, ty))) =
                    (xs.get(x as usize), ys.get(y as usize))
                else {
                    continue;
                };
                blend(
                    [
                        image.at(sx + x0, sy + y0),
                        image.at(sx + x1, sy + y0),
                        image.at(sx + x0, sy + y1),
                        image.at(sx + x1, sy + y1),
                    ],
                    tx,
                    ty,
                )
            };
            let at = row + x as usize * 4;
            if let Some(dst) = rgba.get_mut(at..at + 4) {
                dst.copy_from_slice(&px);
            }
        }
    }
    Some(Canvas {
        width,
        height,
        rgba,
    })
}

/// Bilinear mix of the four pixels around a point (top left, top right, bottom left, bottom
/// right) with the weights `tx` and `ty`; colours are weighted by their alpha.
fn blend(p: [[u8; 4]; 4], tx: f32, ty: f32) -> [u8; 4] {
    let weights = [
        (1.0 - tx) * (1.0 - ty),
        tx * (1.0 - ty),
        (1.0 - tx) * ty,
        tx * ty,
    ];
    let mut alpha = 0.0;
    let mut color = [0.0f32; 3];
    for (px, w) in p.iter().zip(weights) {
        let a = f32::from(px[3]) * w;
        alpha += a;
        for (c, v) in color.iter_mut().zip(px) {
            *c += f32::from(*v) * a;
        }
    }
    if alpha <= 0.0 {
        return [0; 4];
    }
    let channel = |c: f32| (c / alpha).round().clamp(0.0, 255.0) as u8;
    [
        channel(color[0]),
        channel(color[1]),
        channel(color[2]),
        alpha.round().clamp(0.0, 255.0) as u8,
    ]
}

/// Writes outer image `id` (`a=t`, chunked) and its virtual placement of `cols × rows` cells.
fn write_outer(
    out: &mut impl Write,
    id: u32,
    canvas: &Canvas,
    cols: u16,
    rows: u16,
) -> io::Result<()> {
    write_pixels(
        out,
        id,
        (canvas.width, canvas.height),
        ImageFormat::Rgba,
        &canvas.rgba,
        (cols, rows),
    )
}

/// Writes outer image `id` from `data` (`a=t`, chunked) and its virtual placement.
fn write_pixels(
    out: &mut impl Write,
    id: u32,
    (w, h): (u32, u32),
    format: ImageFormat,
    data: &[u8],
    (cols, rows): (u16, u16),
) -> io::Result<()> {
    let f = match format {
        ImageFormat::Rgb => 24,
        ImageFormat::Rgba => 32,
    };
    let chunks = data.chunks(RAW_CHUNK).count();
    for (n, chunk) in data.chunks(RAW_CHUNK).enumerate() {
        let more = u8::from(n + 1 < chunks);
        if n == 0 {
            write!(out, "\x1b_Ga=t,f={f},s={w},v={h},i={id},q=2,m={more};")?;
        } else {
            write!(out, "\x1b_Gm={more},q=2;")?;
        }
        out.write_all(base64::encode(chunk).as_bytes())?;
        out.write_all(b"\x1b\\")?;
    }
    write!(out, "\x1b_Ga=p,U=1,i={id},c={cols},r={rows},q=2\x1b\\")
}

impl Images {
    /// Before a draw: builds and sends the outer image of each wanted placement that has
    /// none and whose pixels are in. What the draw wants is remembered for [`Images::finish_placed`].
    ///
    /// The canvases are built within two budgets, in the order of `wanted`: [`DRAW_BUDGET`]
    /// for this draw and [`LIVE_BUDGET`] for everything on screen. `true` when a placement
    /// was left for the next draw, which the caller then asks for.
    pub(crate) fn ensure_placed(
        &mut self,
        out: &mut impl Write,
        wanted: &[(TerminalId, Placement)],
        cell: Option<CellPx>,
    ) -> io::Result<bool> {
        self.program.used.clear();
        self.program.used_virtual.clear();
        // The outer images this draw keeps; the others go right after it.
        let kept: HashSet<OuterKey> = wanted
            .iter()
            .filter_map(|(terminal, p)| OuterKey::of(*terminal, p, cell))
            .filter(|key| self.program.outer.contains_key(key))
            .collect();
        let mut live: u64 = kept.iter().map(OuterKey::canvas_bytes).sum();
        let mut built: u64 = 0;
        let mut deferred = false;
        let mut wrote = false;
        for (terminal, p) in wanted {
            if let Some(key) = VirtualKey::of(*terminal, p) {
                self.program.used_virtual.insert(key);
                if self.program.virtuals.contains_key(&key) {
                    continue;
                }
                let Some((generation, image)) = self.program.pixels.get(&key.slot) else {
                    continue;
                };
                if *generation != key.generation {
                    continue;
                }
                let sent = (image.width, image.height, image.format);
                let Some(id) = self.alloc() else { continue };
                if let Some((_, image)) = self.program.pixels.get(&key.slot) {
                    write_pixels(
                        out,
                        id,
                        (sent.0, sent.1),
                        sent.2,
                        &image.data,
                        (key.cols, key.rows),
                    )?;
                }
                self.program.virtuals.insert(key, id);
                wrote = true;
                continue;
            }
            let Some(key) = OuterKey::of(*terminal, p, cell) else {
                continue;
            };
            self.program.used.insert(key);
            if self.program.outer.contains_key(&key) || self.program.refused.contains(&key) {
                continue;
            }
            let Some(cell) = cell else { continue };
            let size = key.canvas_bytes();
            let canvas = match self.program.pixels.get(&key.slot) {
                Some((generation, image)) if *generation == key.generation => {
                    // A canvas over `MAX_CANVAS` is refused below, whatever the budgets.
                    if size <= MAX_CANVAS && live + size > LIVE_BUDGET {
                        continue;
                    }
                    if size <= MAX_CANVAS && built + size > DRAW_BUDGET {
                        deferred = true;
                        continue;
                    }
                    canvas(image, p.source, p.pixels, key.cols, key.rows, cell)
                }
                _ => continue,
            };
            let Some(canvas) = canvas else {
                self.program.refused.insert(key);
                continue;
            };
            let Some(id) = self.alloc() else { continue };
            write_outer(out, id, &canvas, key.cols, key.rows)?;
            self.program.outer.insert(key, id);
            built += size;
            live += size;
            wrote = true;
        }
        if wrote {
            out.flush()?;
        }
        Ok(deferred)
    }

    /// After a draw: deletes the outer images it did not use, and the ones owed a delete.
    pub(crate) fn finish_placed(&mut self, out: &mut impl Write) -> io::Result<()> {
        let program = &mut self.program;
        let unused: Vec<OuterKey> = program
            .outer
            .keys()
            .filter(|k| !program.used.contains(k))
            .copied()
            .collect();
        let unused_virtual: Vec<VirtualKey> = program
            .virtuals
            .keys()
            .filter(|k| !program.used_virtual.contains(k))
            .copied()
            .collect();
        let mut ids: Vec<u32> = std::mem::take(&mut program.doomed);
        for key in unused_virtual {
            ids.extend(program.virtuals.remove(&key));
        }
        for key in unused {
            ids.extend(program.outer.remove(&key));
        }
        program.refused.retain(|k| program.used.contains(k));
        if ids.is_empty() {
            return Ok(());
        }
        for id in ids {
            out.write_all(&delete(id))?;
            self.release(id);
        }
        out.flush()
    }

    /// The outer id of a placed placement, and the cells it fills, when its outer image was
    /// sent.
    pub(crate) fn placed_id(
        &self,
        terminal: TerminalId,
        p: &Placement,
        cell: Option<CellPx>,
    ) -> Option<(u32, u16, u16)> {
        let key = OuterKey::of(terminal, p, cell)?;
        let id = self.program.outer.get(&key)?;
        Some((*id, key.cols, key.rows))
    }
}

impl Images {
    /// The outer id of a program's virtual placement, when its outer image was sent.
    pub(crate) fn virtual_id(&self, terminal: TerminalId, p: &Placement) -> Option<u32> {
        self.program
            .virtuals
            .get(&VirtualKey::of(terminal, p)?)
            .copied()
    }
}
