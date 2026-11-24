//! Kitty graphics in the emulator: what is placed in the viewport, the pixels of an image,
//! and whether either changed.
//!
//! `libghostty-vt` hands out `Graphics` and `Image` handles that are valid only until the
//! next mutating call on the terminal, so every method here copies what it needs first.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, PoisonError};

use deck_core::protocol::{ImageFormat, ImageScreen, Placement, PlacementKind, SourceRect};
use ghostty_vt::alloc::{Allocator, Bytes};
use ghostty_vt::kitty::graphics as kg;
use ghostty_vt::screen::Screen;
use ghostty_vt::{Error, Terminal};

use super::Emu;

/// Kitty image storage per screen of a terminal.
const STORAGE_LIMIT: u64 = 64 << 20;
/// Remembered "cannot express this image" warnings, so a log line appears once per image.
const WARNED_CAP: usize = 256;

/// Each `set_png_decoder` also sets a process-global C option, so installs take turns.
static PNG_INSTALL: Mutex<()> = Mutex::new(());

/// The pixels of an image, copied out of the emulator: `Rgb` or `Rgba`, row by row, without
/// padding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagePixels {
    pub generation: u64,
    pub width: u32,
    pub height: u32,
    pub format: ImageFormat,
    pub data: Vec<u8>,
}

/// What the emulator keeps between graphics calls.
pub(super) struct State {
    iterator: kg::PlacementIterator,
    /// The storage generation and placements `images_changed` saw last.
    seen: (u64, Vec<Placement>),
    /// Images already logged as impossible to express.
    warned: HashSet<(ImageScreen, u32, u64)>,
}

impl State {
    pub(super) fn new() -> Result<Self, Error> {
        Ok(Self {
            iterator: kg::PlacementIterator::new()?,
            seen: (0, Vec::new()),
            warned: HashSet::new(),
        })
    }
}

/// Installs the crate's PNG decoder for the current thread (the thread that writes to the
/// terminal). Call it on the emulator's thread before creating the terminal.
pub(super) fn install_png_decoder() -> Result<(), Error> {
    let _installing = PNG_INSTALL.lock().unwrap_or_else(PoisonError::into_inner);
    kg::set_png_decoder(Some(Box::new(PngDecoder(kg::RustPngDecoder::new()))))
}

/// The crate's PNG decoder, plus the grey PNGs it leaves as 1 or 2 bytes per pixel: it
/// expands palette and 16-bit PNGs to RGBA8 but not greyscale ones, which Ghostty then
/// rejects as invalid data.
struct PngDecoder(kg::RustPngDecoder);

impl kg::DecodePng for PngDecoder {
    fn decode_png(&mut self, alloc: &Allocator, data: &[u8]) -> Option<kg::DecodedImage> {
        let mut image = self.0.decode_png(alloc, data)?;
        let pixels = usize::try_from(u64::from(image.width) * u64::from(image.height)).ok()?;
        let grey_alpha = match image.data.len() {
            n if n == pixels => false,
            n if n == pixels.checked_mul(2)? => true,
            _ => return Some(image),
        };
        let mut rgba = Bytes::new_with_alloc(alloc, pixels.checked_mul(4)?).ok()?;
        let mut out = rgba.iter_mut();
        let mut source = image.data.iter();
        while let Some(&grey) = source.next() {
            let alpha = if grey_alpha { *source.next()? } else { 255 };
            for value in [grey, grey, grey, alpha] {
                *out.next()? = value;
            }
        }
        image.data = rgba;
        Some(image)
    }
}

/// Turns kitty graphics on: the storage limit on both screens, and only direct transmission:
/// the file, temporary-file and shared-memory mediums are off, because opening a path a
/// program names (a FIFO, `/dev/shm/<name>`) can block the emulator thread for good.
pub(super) fn enable(term: &mut Terminal) -> Result<(), Error> {
    term.set_kitty_image_storage_limit(STORAGE_LIMIT)?;
    term.set_kitty_image_from_file_allowed(false)?;
    term.set_kitty_image_from_shared_mem_allowed(false)?;
    term.set_kitty_image_temp_file_dir(None)?;
    Ok(())
}

fn screen_of(screen: Screen) -> ImageScreen {
    match screen {
        Screen::Alternate => ImageScreen::Alternate,
        _ => ImageScreen::Primary,
    }
}

fn saturate(n: u32) -> u16 {
    u16::try_from(n).unwrap_or(u16::MAX)
}

/// What reading one image gave.
enum Read {
    Absent,
    Pixels(ImagePixels),
    /// Pixels that cannot be sent as RGB or RGBA, and why.
    Unsupported(u64, String),
}

impl Emu {
    /// Every placement of the active screen that touches the viewport as shown, lower `z`
    /// first. A virtual placement is listed once per image, whether or not its placeholder
    /// cells are on screen.
    pub fn placements(&mut self) -> Vec<Placement> {
        self.read_placements().unwrap_or_else(|e| {
            tracing::warn!("cannot list the image placements: {e}");
            Vec::new()
        })
    }

    fn read_placements(&mut self) -> Result<Vec<Placement>, Error> {
        let screen = screen_of(self.term.active_screen()?);
        let graphics = self.term.kitty_graphics()?;
        let mut iteration = self.graphics.iterator.update(&graphics)?;
        let mut out: Vec<(i32, Placement)> = Vec::new();
        let mut virtuals = HashSet::new();
        while let Some(p) = iteration.next() {
            let Ok(id) = p.image_id() else { continue };
            let Some(image) = graphics.image(id) else {
                continue;
            };
            let listed = (|| {
                let kind = if p.is_virtual()? {
                    if virtuals.contains(&id) {
                        return Ok(None);
                    }
                    PlacementKind::Virtual
                } else {
                    match p.viewport_pos(&image, &self.term)? {
                        Some(pos) => PlacementKind::Placed {
                            col: pos.col,
                            row: pos.row,
                        },
                        None => return Ok(None),
                    }
                };
                let grid = p.grid_size(&image, &self.term)?;
                let src = p.source_rect(&image)?;
                let px = p.pixel_size(&image, &self.term)?;
                Ok::<_, Error>(Some((
                    p.z()?,
                    Placement {
                        screen,
                        image: id,
                        generation: image.generation()?,
                        kind,
                        cols: saturate(grid.cols),
                        rows: saturate(grid.rows),
                        source: SourceRect {
                            x: src.x,
                            y: src.y,
                            width: src.width,
                            height: src.height,
                        },
                        pixels: (px.width, px.height),
                    },
                )))
            })();
            match listed {
                Ok(Some(entry)) => {
                    if entry.1.kind == PlacementKind::Virtual {
                        virtuals.insert(id);
                    }
                    out.push(entry);
                }
                Ok(None) => {}
                Err(e) => tracing::debug!(image = id, "skipping an image placement: {e}"),
            }
        }
        out.sort_by_key(|(z, _)| *z);
        Ok(out.into_iter().map(|(_, p)| p).collect())
    }

    /// The pixels of image `id` of `screen`, when that is the active screen, as `Rgb` or
    /// `Rgba` (grey formats are expanded). `None` for an image that is not there, is still
    /// being sent, or whose pixels cannot be expressed so (logged once).
    pub fn image(&mut self, screen: ImageScreen, id: u32) -> Option<ImagePixels> {
        let active = screen_of(self.term.active_screen().ok()?);
        if active != screen {
            return None;
        }
        match read_image(&self.term, id) {
            Read::Absent => None,
            Read::Pixels(pixels) => Some(pixels),
            Read::Unsupported(generation, why) => {
                let warned = &mut self.graphics.warned;
                if warned.len() >= WARNED_CAP {
                    warned.clear();
                }
                if warned.insert((screen, id, generation)) {
                    tracing::warn!(image = id, "an image cannot be shown: {why}");
                }
                None
            }
        }
    }

    /// Whether the graphics changed since this was last asked: the storage's generation, or
    /// the placements (they move when the display scrolls or the terminal resizes).
    #[cfg(test)]
    pub fn images_changed(&mut self) -> bool {
        self.images_state().0
    }

    /// `images_changed`, and the placements it read.
    pub(super) fn images_state(&mut self) -> (bool, Vec<Placement>) {
        let generation = self
            .term
            .kitty_graphics()
            .and_then(|g| g.generation())
            .unwrap_or(0);
        let placements = self.placements();
        let now = (generation, placements);
        if self.graphics.seen == now {
            return (false, now.1);
        }
        self.graphics.seen = now.clone();
        (true, now.1)
    }

    /// The active screen, and for each of the images `ids` that its storage holds, the
    /// generation and the byte size its pixels have when sent (`image`). Cheap: no pixels
    /// are read.
    pub fn image_generations(&mut self, ids: &[u32]) -> (ImageScreen, HashMap<u32, (u64, usize)>) {
        let screen = self
            .term
            .active_screen()
            .map_or(ImageScreen::Primary, screen_of);
        let mut stored = HashMap::new();
        if let Ok(graphics) = self.term.kitty_graphics() {
            for &id in ids {
                let Some(image) = graphics.image(id) else {
                    continue;
                };
                let (Ok(generation), Ok(width), Ok(height)) =
                    (image.generation(), image.width(), image.height())
                else {
                    continue;
                };
                let per_pixel = match image.format() {
                    Ok(kg::ImageFormat::Rgb | kg::ImageFormat::Gray) => 3,
                    _ => 4,
                };
                let bytes = usize::try_from(u64::from(width) * u64::from(height) * per_pixel)
                    .unwrap_or(usize::MAX);
                stored.insert(id, (generation, bytes));
            }
        }
        (screen, stored)
    }
}

fn read_image(term: &Terminal, id: u32) -> Read {
    let Ok(graphics) = term.kitty_graphics() else {
        return Read::Absent;
    };
    let Some(image) = graphics.image(id) else {
        return Read::Absent;
    };
    let (Ok(generation), Ok(width), Ok(height), Ok(format), Ok(compression), Ok(data)) = (
        image.generation(),
        image.width(),
        image.height(),
        image.format(),
        image.compression(),
        image.data(),
    ) else {
        return Read::Absent;
    };
    // The pixels of an image still being sent are not there yet.
    if data.is_empty() {
        return Read::Absent;
    }
    if compression != kg::Compression::None {
        return Read::Unsupported(generation, format!("compressed data ({compression:?})"));
    }
    match pixels_of(format, width, height, data) {
        Ok((format, data)) => Read::Pixels(ImagePixels {
            generation,
            width,
            height,
            format,
            data,
        }),
        Err(why) => Read::Unsupported(generation, why),
    }
}

/// `data` as RGB or RGBA: grey formats are expanded; anything else, or a length that does not
/// fit the size, is an error naming what is wrong.
fn pixels_of(
    format: kg::ImageFormat,
    width: u32,
    height: u32,
    data: &[u8],
) -> Result<(ImageFormat, Vec<u8>), String> {
    let count = u64::from(width) * u64::from(height);
    let (bytes, out) = match format {
        kg::ImageFormat::Rgb => (3, ImageFormat::Rgb),
        kg::ImageFormat::Rgba => (4, ImageFormat::Rgba),
        kg::ImageFormat::Gray => (1, ImageFormat::Rgb),
        kg::ImageFormat::GrayAlpha => (2, ImageFormat::Rgba),
        other => return Err(format!("the pixel format {other:?}")),
    };
    if u64::try_from(data.len()).ok() != count.checked_mul(bytes) {
        return Err(format!(
            "{} bytes for {width}x{height} pixels of {format:?}",
            data.len()
        ));
    }
    let data = match format {
        kg::ImageFormat::Gray => data.iter().flat_map(|&g| [g, g, g]).collect(),
        kg::ImageFormat::GrayAlpha => data
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|&[g, a]| [g, g, g, a])
            .collect(),
        _ => data.to_vec(),
    };
    Ok((out, data))
}

#[cfg(test)]
mod tests;
