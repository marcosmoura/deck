//! One terminal emulator (`ghostty_vt::Terminal`, Ghostty's `libghostty-vt`) and its
//! conversion to protocol frames.

use std::fmt;
use std::time::Duration;

use deck_core::model::TerminalId;
use deck_core::protocol::{
    CellPx, Color, Cursor, CursorShape, Frame, Line as FrameLine, Modes, Placement, Run, ScrollTo,
    Style,
};
use ghostty_vt::render::{
    CellIterator, CursorVisualStyle, Dirty, RenderState, RowIterator, Snapshot,
};
use ghostty_vt::screen::{Cell, CellContentTag, CellWide, Screen};
use ghostty_vt::style::{Palette, RgbColor, Style as GStyle, StyleColor, Underline};
use ghostty_vt::terminal::{
    ClipboardLocation, ConformanceLevel, CursorStyle, DeviceAttributes, DeviceType, Mode,
    PrimaryDeviceAttributes, ScrollViewport, SecondaryDeviceAttributes, TertiaryDeviceAttributes,
};
use ghostty_vt::{Error, Event, Terminal, TerminalOptions};

mod capture;
pub(crate) mod files;
mod graphics;

pub use graphics::ImagePixels;

/// The emulator needs at least two columns (a wide character does not fit in one) and a
/// huge size would allocate gigabytes of cells.
pub fn clamp_size(cols: u16, rows: u16) -> (u16, u16) {
    (cols.clamp(2, 1000), rows.clamp(1, 1000))
}

/// A program that sets synchronized output (mode 2026) and never resets it gets its frame
/// after this long, as in alacritty (`SYNC_UPDATE_TIMEOUT`).
pub const SYNC_TIMEOUT: Duration = Duration::from_millis(150);

/// The cell size, in pixels, that size reports (`CSI 14/16/18 t`) use while no client has
/// reported the real one.
const CELL_PX: (u32, u32) = (8, 16);

/// `libghostty-vt` budgets scrollback in bytes (its `max_scrollback` says an estimate; it
/// prunes whole pages of about 400 KiB), and only the creation of the terminal sets it.
/// `Emu` turns the configured number of lines into bytes for a terminal of this many
/// columns: wider terminals keep fewer lines, narrower ones more (a terminal of 80 columns
/// holds about three times the lines), which `Emu::history` hides but does not free.
const BUDGET_COLS: usize = 250;
/// Bytes per line of `BUDGET_COLS` columns. Measured on `libghostty-vt` 1.0.1 with full lines
/// (`scrollback_budget_holds_the_configured_lines`): the cells take 8 bytes each (2,000 per
/// line), and 2,560 keeps at least 10,000 lines (11,673 measured; 2,300 keeps 10,527 and
/// 2,200 only 9,954).
const BYTES_PER_LINE: usize = 8 * BUDGET_COLS + 560;
/// Added to a non-empty budget: pages are pruned whole, so a small budget holds fewer lines
/// than its size says (1,000 lines at 2,400 bytes each kept 931).
const BUDGET_SLACK: usize = 1 << 20;

/// The scrollback budget in bytes for `lines` lines; 10,000 lines (the default) is
/// 26,648,576 bytes.
fn scrollback_budget(lines: usize) -> usize {
    if lines == 0 {
        return 0;
    }
    lines
        .saturating_mul(BYTES_PER_LINE)
        .saturating_add(BUDGET_SLACK)
}

/// What the daemon does about an emulator event.
pub enum EmuEvent {
    /// A title change; `None` resets it.
    Title(Option<String>),
    /// Bytes to send to the program (answers to queries).
    Reply(String),
    /// OSC 52 copy; `selection` is `"c"` or `"p"`.
    ClipboardStore {
        selection: &'static str,
        text: String,
    },
}

impl fmt::Debug for EmuEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Title(t) => write!(f, "Title({t:?})"),
            Self::Reply(s) => write!(f, "Reply({s:?})"),
            Self::ClipboardStore { selection, text } => {
                write!(f, "ClipboardStore({selection}, {text:?})")
            }
        }
    }
}

/// One cell as a frame needs it.
#[derive(Default)]
struct Glyph {
    /// The grapheme cluster; empty for a cell without text.
    text: String,
    style: Style,
    /// The second half of a wide character, or the filler before one at the end of a line.
    spacer: bool,
}

impl Glyph {
    fn is_blank(&self) -> bool {
        matches!(self.text.as_str(), "" | " ") && self.style == Style::default()
    }
}

pub struct Emu {
    term: Terminal,
    render: RenderState,
    rows: RowIterator,
    cells: CellIterator,
    /// The most lines of history the display can reach.
    scrollback: usize,
    /// The outer terminal's colours the emulator answers OSC 10 and 11 with.
    colors: ((u8, u8, u8), (u8, u8, u8)),
    /// The outer terminal's name, which XTVERSION is answered with; `None`: no answer.
    xtversion: Option<String>,
    /// When the current synchronized output was first seen.
    sync_since: Option<Duration>,
    /// The outer terminal's cell size in pixels, when a client reported it.
    cell: Option<CellPx>,
    /// What `images_changed` compared last, and what the graphics code keeps between calls.
    graphics: graphics::State,
}

impl fmt::Debug for Emu {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Emu")
            .field("size", &self.size())
            .finish_non_exhaustive()
    }
}

/// The screen as one read of the render state gives it.
struct Screenful {
    lines: Vec<(u16, FrameLine)>,
    full: bool,
    cursor: ghostty_vt::render::Cursor,
}

impl Emu {
    pub fn new(cols: u16, rows: u16, scrollback: usize) -> Result<Self, Error> {
        let (cols, rows) = clamp_size(cols, rows);
        graphics::install_png_decoder()?;
        let mut term = Terminal::new(TerminalOptions {
            cols,
            rows,
            max_scrollback: scrollback_budget(scrollback),
        })?;
        term.resize(cols, rows, CELL_PX.0, CELL_PX.1)?;
        // No escape sequence selects the hollow block, so as the default it marks a terminal
        // whose program never chose a shape.
        term.set_default_cursor_style(Some(CursorStyle::BlockHollow))?;
        term.set_device_attributes(Some(DeviceAttributes {
            primary: PrimaryDeviceAttributes::new(ConformanceLevel::VT102, &[]),
            secondary: SecondaryDeviceAttributes {
                device_type: DeviceType::VT100,
                firmware_version: 2600,
                rom_cartridge: 1,
            },
            tertiary: TertiaryDeviceAttributes::default(),
        }));
        graphics::enable(&mut term)?;
        let mut palette = Palette::default();
        for (index, color) in palette.0.iter_mut().enumerate() {
            *color = default_palette(index);
        }
        term.set_default_color_palette(Some(palette))?;
        let colors = (
            crate::server::FALLBACK_COLORS.fg,
            crate::server::FALLBACK_COLORS.bg,
        );
        let rgb = |(r, g, b): (u8, u8, u8)| RgbColor { r, g, b };
        term.set_default_fg_color(Some(rgb(colors.0)))?;
        term.set_default_bg_color(Some(rgb(colors.1)))?;
        Ok(Self {
            term,
            render: RenderState::new()?,
            rows: RowIterator::new()?,
            cells: CellIterator::new()?,
            scrollback,
            colors,
            xtversion: None,
            sync_since: None,
            cell: None,
            graphics: graphics::State::new()?,
        })
    }

    /// The colours (foreground, background) OSC 10 and 11 queries are answered with, and the
    /// default colours of the screen they describe.
    pub fn set_colors(&mut self, fg: (u8, u8, u8), bg: (u8, u8, u8)) {
        if self.colors == (fg, bg) {
            return;
        }
        self.colors = (fg, bg);
        let rgb = |(r, g, b): (u8, u8, u8)| RgbColor { r, g, b };
        let set = self
            .term
            .set_default_fg_color(Some(rgb(fg)))
            .and_then(|t| t.set_default_bg_color(Some(rgb(bg))))
            .map(|_| ());
        if let Err(e) = set {
            tracing::warn!("cannot set the emulator's colours: {e}");
        }
    }

    /// The name XTVERSION (`CSI > q`) is answered with, the outer terminal's; `None` leaves
    /// the query unanswered.
    pub fn set_xtversion(&mut self, name: Option<&str>) {
        if self.xtversion.as_deref() == name {
            return;
        }
        self.xtversion = name.map(str::to_owned);
        self.term.set_xtversion(self.xtversion.clone());
    }

    pub fn size(&self) -> (u16, u16) {
        (self.term.cols().unwrap_or(0), self.term.rows().unwrap_or(0))
    }

    /// Resizes with the cell size the emulator has.
    #[cfg(test)]
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.resize_with_cell(cols, rows, self.cell);
    }

    /// Resizes, and takes `cell` as the outer terminal's cell size in pixels (`None`: unknown,
    /// 8 by 16 for size reports). Image grid sizes and `CSI 14/16 t` answers use it.
    pub fn resize_with_cell(&mut self, cols: u16, rows: u16, cell: Option<CellPx>) {
        let (cols, rows) = clamp_size(cols, rows);
        self.cell = cell.filter(|c| c.width > 0 && c.height > 0);
        let (width, height) = self
            .cell
            .map_or(CELL_PX, |c| (u32::from(c.width), u32::from(c.height)));
        if let Err(e) = self.term.resize(cols, rows, width, height) {
            tracing::warn!("cannot resize the emulator: {e}");
        }
    }

    /// The cell size in pixels the emulator was given, `None` while unknown.
    pub fn cell(&self) -> Option<CellPx> {
        self.cell
    }

    /// Feeds program output and returns what it asked of the daemon.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<EmuEvent> {
        self.term.vt_write(bytes);
        self.keep_view_within_history();
        let raised: Vec<Event> = self.term.events().collect();
        let cell_known = self.cell.is_some();
        let named = self.xtversion.is_some();
        raised
            .into_iter()
            .filter_map(|event| convert(event, cell_known, named))
            .collect()
    }

    /// The lines of history the display can reach: what the emulator keeps, up to the
    /// configured number.
    pub(crate) fn history(&self) -> usize {
        self.term
            .scrollback_rows()
            .unwrap_or(0)
            .min(self.scrollback)
    }

    /// How many rows the display is scrolled back from the live screen.
    fn display_offset(&self) -> usize {
        let Ok(bar) = self.term.scrollbar() else {
            return 0;
        };
        let above = usize::try_from(bar.total.saturating_sub(bar.len)).unwrap_or(0);
        let from_top = usize::try_from(bar.offset).unwrap_or(0);
        above.saturating_sub(from_top).min(self.history())
    }

    /// A scrolled-back view stays on its content while output arrives, so it can sink deeper
    /// than the configured history (the emulator keeps more lines than that at narrow widths):
    /// put it back at the limit, and the content moves under it.
    fn keep_view_within_history(&mut self) {
        if self.term.is_viewport_active().unwrap_or(true) {
            return;
        }
        let Ok(bar) = self.term.scrollbar() else {
            return;
        };
        let above = usize::try_from(bar.total.saturating_sub(bar.len)).unwrap_or(0);
        let from_top = usize::try_from(bar.offset).unwrap_or(0);
        if above.saturating_sub(from_top) > self.scrollback {
            self.set_display_offset(self.scrollback);
        }
    }

    fn set_display_offset(&mut self, offset: usize) {
        let to = if offset == 0 {
            ScrollViewport::Bottom
        } else {
            let Ok(bar) = self.term.scrollbar() else {
                return;
            };
            let above = usize::try_from(bar.total.saturating_sub(bar.len)).unwrap_or(0);
            ScrollViewport::Row(above.saturating_sub(offset))
        };
        self.term.scroll_viewport(to);
    }

    /// Moves the display through the history, clamped to it.
    pub fn scroll(&mut self, to: ScrollTo) {
        let rows = i64::from(self.size().1);
        let history = i64::try_from(self.history()).unwrap_or(i64::MAX);
        let now = i64::try_from(self.display_offset()).unwrap_or(0);
        let target = match to {
            ScrollTo::Lines(n) => now.saturating_add(i64::from(n)),
            ScrollTo::Pages(n) => now.saturating_add(i64::from(n).saturating_mul(rows)),
            ScrollTo::Top => history,
            ScrollTo::Bottom => 0,
        };
        self.set_display_offset(usize::try_from(target.clamp(0, history)).unwrap_or(0));
    }

    /// Returns the display to the live screen; whether it was scrolled back.
    pub fn scroll_to_bottom(&mut self) -> bool {
        let scrolled = self.display_offset() > 0;
        if scrolled {
            self.set_display_offset(0);
        }
        scrolled
    }

    /// Every row of the screen. The damage stays.
    pub fn full_frame(&mut self, terminal: TerminalId) -> Frame {
        let screen = self.read_screen(false);
        let images = self.placements();
        self.frame(terminal, screen, images)
    }

    /// The rows changed since the last call, or every row when the emulator says so or the
    /// display is scrolled back (its damage is of the live screen, not of what shows). `None`
    /// while the program holds synchronized output: the damage waits for the end of the hold,
    /// which `now` times out after [`SYNC_TIMEOUT`].
    pub fn damage_frame(&mut self, terminal: TerminalId, now: Duration) -> Option<Frame> {
        if self.held(now) {
            return None;
        }
        let screen = self.read_screen(true);
        // A graphics change counts as damage: the frame is built even when no cell changed,
        // and the list of placements in it is the one the change detection compared.
        let (_, images) = self.images_state();
        Some(self.frame(terminal, screen, images))
    }

    /// The damage as a frame whatever the program holds: for the last output of a terminal.
    pub fn last_frame(&mut self, terminal: TerminalId) -> Frame {
        self.end_hold();
        let screen = self.read_screen(true);
        let images = self.placements();
        self.frame(terminal, screen, images)
    }

    /// How long until a held synchronized output times out, `None` when none is held (as far
    /// as `damage_frame` has seen).
    pub fn hold_remaining(&self, now: Duration) -> Option<Duration> {
        let since = self.sync_since?;
        Some((since + SYNC_TIMEOUT).saturating_sub(now))
    }

    fn held(&mut self, now: Duration) -> bool {
        if !self.term.mode(Mode::SYNC_OUTPUT).unwrap_or(false) {
            self.sync_since = None;
            return false;
        }
        let since = *self.sync_since.get_or_insert(now);
        if now.saturating_sub(since) >= SYNC_TIMEOUT {
            self.end_hold();
            return false;
        }
        true
    }

    fn end_hold(&mut self) {
        self.sync_since = None;
        if self.term.mode(Mode::SYNC_OUTPUT).unwrap_or(false) {
            let _ = self.term.set_mode(Mode::SYNC_OUTPUT, false);
        }
    }

    pub fn reset_damage(&mut self) {
        if let Ok(snapshot) = self.render.update(&self.term) {
            let _ = snapshot.clean();
        }
    }

    /// Reads the rows of the display: all of them, or with `damage` the dirty ones (all when
    /// the whole screen is dirty or the display is scrolled back), and then marks them clean.
    fn read_screen(&mut self, damage: bool) -> Screenful {
        let scrolled = self.display_offset() > 0;
        let (cols, _) = self.size();
        let read = (|| {
            let snapshot = self.render.update(&self.term)?;
            let full = !damage || scrolled || snapshot.dirty()? == Dirty::Full;
            let cursor = snapshot.cursor()?;
            let lines = read_rows(&snapshot, &mut self.rows, &mut self.cells, full, cols)?;
            if damage {
                snapshot.clean()?;
            }
            Ok::<_, Error>(Screenful {
                lines,
                full,
                cursor,
            })
        })();
        read.unwrap_or_else(|e| {
            tracing::warn!("cannot read the emulator's screen: {e}");
            Screenful {
                lines: Vec::new(),
                full: true,
                cursor: ghostty_vt::render::Cursor {
                    viewport: None,
                    visible: false,
                    blinking: false,
                    password_input: false,
                    visual_style: CursorVisualStyle::Block,
                },
            }
        })
    }

    fn frame(&self, terminal: TerminalId, screen: Screenful, images: Vec<Placement>) -> Frame {
        let (cols, rows) = self.size();
        let offset = self.display_offset();
        // The cursor sits on the live screen: scrolled back, it moves down with the content
        // and is hidden once it falls below the last row.
        let (row, col) = match screen.cursor.viewport {
            Some(v) => (v.y, v.x),
            None => {
                let live = i64::from(self.term.cursor_y().unwrap_or(0));
                let row = live.saturating_add(i64::try_from(offset).unwrap_or(0));
                (
                    u16::try_from(row).unwrap_or(u16::MAX),
                    self.term.cursor_x().unwrap_or(0),
                )
            }
        };
        Frame {
            terminal,
            size: (cols, rows),
            cursor: Cursor {
                row: row.min(rows.saturating_sub(1)),
                col: col.min(cols.saturating_sub(1)),
                visible: screen.cursor.visible && screen.cursor.viewport.is_some(),
                shape: match screen.cursor.visual_style {
                    CursorVisualStyle::Bar => CursorShape::Bar,
                    CursorVisualStyle::Underline => CursorShape::Underline,
                    CursorVisualStyle::Block => CursorShape::Block,
                    _ => CursorShape::Default,
                },
            },
            modes: self.modes(),
            display_offset: u32::try_from(offset).unwrap_or(0),
            history: u32::try_from(self.history()).unwrap_or(0),
            lines: screen.lines,
            full: screen.full,
            images,
        }
    }
}

/// Turns what the emulator raised into what the daemon acts on.
fn convert(event: Event, cell_known: bool, named: bool) -> Option<EmuEvent> {
    Some(match event {
        Event::TitleChanged(title) if title.is_empty() => EmuEvent::Title(None),
        Event::TitleChanged(title) => EmuEvent::Title(Some(title)),
        Event::PtyWrite(bytes) if is_unanswered(&bytes, cell_known, named) => return None,
        Event::PtyWrite(bytes) => EmuEvent::Reply(String::from_utf8_lossy(&bytes).into_owned()),
        Event::ClipboardWrite(write) => {
            let selection = match write.location {
                ClipboardLocation::Standard => "c",
                ClipboardLocation::Primary => "p",
                _ => return None,
            };
            let content = write.contents.into_iter().next()?;
            EmuEvent::ClipboardStore {
                selection,
                text: String::from_utf8_lossy(&content.data).into_owned(),
            }
        }
        _ => return None,
    })
}

/// Answers `libghostty-vt` gives and the daemon has never given, which cannot be switched
/// off one by one, so they are dropped here:
///
/// - the kitty keyboard query (`CSI ? u`) is answered with `CSI ? <flags> u`; deck encodes
///   keys the legacy way, so programs keep seeing no support for the protocol;
/// - the cell size report (`CSI 16 t`) is answered with `CSI 6 ; <height> ; <width> t`, while
///   the size reports (`CSI 14 t`, `CSI 18 t`) are kept. It is dropped only while no client
///   has reported the cell size (alacritty never answered it); with a real size it passes;
/// - XTVERSION (`CSI > q`) is answered with `DCS > | libghostty ST` whatever version is set.
///   It passes once a name is set (the outer terminal's, see `Emu::set_xtversion`).
fn is_unanswered(bytes: &[u8], cell_known: bool, named: bool) -> bool {
    let digits = |b: &[u8]| !b.is_empty() && b.iter().all(u8::is_ascii_digit);
    let kitty_keyboard = bytes
        .strip_prefix(b"\x1b[?")
        .and_then(|rest| rest.strip_suffix(b"u"))
        .is_some_and(|flags| digits(flags) || flags.is_empty());
    let cell_size = bytes
        .strip_prefix(b"\x1b[6;")
        .and_then(|rest| rest.strip_suffix(b"t"))
        .and_then(|rest| {
            let mut parts = rest.split(|b| *b == b';');
            Some((parts.next()?, parts.next()?, parts.next().is_none()))
        })
        .is_some_and(|(h, w, end)| end && digits(h) && digits(w));
    let xtversion = bytes.starts_with(b"\x1bP>|");
    kitty_keyboard || (cell_size && !cell_known) || (xtversion && !named)
}

/// The rows of a snapshot as lines: every row, or the dirty ones.
fn read_rows(
    snapshot: &Snapshot<'_>,
    rows: &mut RowIterator,
    cells: &mut CellIterator,
    all: bool,
    cols: u16,
) -> Result<Vec<(u16, FrameLine)>, Error> {
    let mut lines = Vec::new();
    let mut iteration = rows.update(snapshot)?;
    let mut next_row = 0u16;
    loop {
        let row = if all {
            if iteration.next().is_none() {
                break;
            }
            let row = next_row;
            next_row = next_row.saturating_add(1);
            row
        } else {
            match iteration.next_dirty() {
                Some((row, _)) => row,
                None => break,
            }
        };
        let mut glyphs = Vec::with_capacity(usize::from(cols));
        let mut row_cells = cells.update(&iteration)?;
        while let Some(cell) = row_cells.next() {
            let raw = cell.raw_cell()?;
            let mut text = String::new();
            if raw.has_text().unwrap_or(false) {
                cell.graphemes_utf8(&mut text)?;
            }
            let style = if raw.has_styling().unwrap_or(false) {
                cell.style().ok()
            } else {
                None
            };
            glyphs.push(glyph(raw, style, text));
        }
        lines.push(line(row, &glyphs));
    }
    Ok(lines)
}

fn glyph(raw: Cell, style: Option<GStyle>, text: String) -> Glyph {
    Glyph {
        text,
        style: style_of(raw, style),
        spacer: matches!(raw.wide(), Ok(CellWide::SpacerTail | CellWide::SpacerHead)),
    }
}

/// A row as runs: a run per style, placed at the column of its first cell, without the
/// spacers of wide characters and the trailing blanks.
fn line(row: u16, glyphs: &[Glyph]) -> (u16, FrameLine) {
    let end = glyphs
        .iter()
        .rposition(|g| !g.is_blank())
        .map_or(0, |i| i + 1);
    let mut runs: Vec<Run> = Vec::new();
    for (col, glyph) in glyphs.iter().enumerate().take(end) {
        if glyph.spacer {
            continue;
        }
        let run = match runs.last_mut() {
            Some(run) if run.style == glyph.style => run,
            _ => runs.push_mut(Run {
                col: u16::try_from(col).unwrap_or(u16::MAX),
                text: String::new(),
                style: glyph.style,
            }),
        };
        run.text.push_str(if glyph.text.is_empty() {
            " "
        } else {
            &glyph.text
        });
    }
    (row, FrameLine { runs })
}

fn modes_of(term: &Terminal) -> Modes {
    let on = |mode: Mode| term.mode(mode).unwrap_or(false);
    let table = [
        (on(Mode::DECCKM), Modes::APPLICATION_CURSOR),
        (on(Mode::BRACKETED_PASTE), Modes::BRACKETED_PASTE),
        (on(Mode::NORMAL_MOUSE), Modes::MOUSE_CLICK),
        (on(Mode::BUTTON_MOUSE), Modes::MOUSE_DRAG),
        (on(Mode::ANY_MOUSE), Modes::MOUSE_MOTION),
        (on(Mode::SGR_MOUSE), Modes::SGR_MOUSE),
        (on(Mode::FOCUS_EVENT), Modes::FOCUS_REPORTING),
        (
            term.active_screen() == Ok(Screen::Alternate),
            Modes::ALTERNATE_SCREEN,
        ),
        (on(Mode::ALT_SCROLL), Modes::ALTERNATE_SCROLL),
    ];
    Modes(
        table
            .iter()
            .filter(|(set, _)| *set)
            .fold(0, |acc, (_, bit)| acc | bit),
    )
}

/// The style of a cell: its style (if it has one), and the background colour of a cell that
/// holds only that.
fn style_of(raw: Cell, style: Option<GStyle>) -> Style {
    let mut out = Style::default();
    if let Some(s) = style {
        let table = [
            (s.bold, Style::BOLD),
            (s.faint, Style::DIM),
            (s.italic, Style::ITALIC),
            (s.underline != Underline::None, Style::UNDERLINE),
            (s.inverse, Style::REVERSE),
            (s.strikethrough, Style::STRIKETHROUGH),
            (s.invisible, Style::HIDDEN),
        ];
        out = Style {
            fg: color(s.fg_color),
            bg: color(s.bg_color),
            flags: table
                .iter()
                .filter(|(set, _)| *set)
                .fold(0, |acc, (_, bit)| acc | bit),
        };
    }
    match raw.content_tag() {
        Ok(CellContentTag::BgColorPalette) => {
            if let Ok(index) = raw.bg_color_palette() {
                out.bg = Color::Indexed(index.0);
            }
        }
        Ok(CellContentTag::BgColorRgb) => {
            if let Ok(RgbColor { r, g, b }) = raw.bg_color_rgb() {
                out.bg = Color::Rgb(r, g, b);
            }
        }
        _ => {}
    }
    out
}

fn color(color: StyleColor) -> Color {
    match color {
        StyleColor::None => Color::Default,
        StyleColor::Palette(index) => Color::Indexed(index.0),
        StyleColor::Rgb(RgbColor { r, g, b }) => Color::Rgb(r, g, b),
    }
}

/// The standard xterm colour for an index below 256: the palette the emulator answers OSC 4
/// queries with until a program sets the entry itself.
pub fn default_palette(index: usize) -> RgbColor {
    const ANSI: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (205, 0, 0),
        (0, 205, 0),
        (205, 205, 0),
        (0, 0, 238),
        (205, 0, 205),
        (0, 205, 205),
        (229, 229, 229),
        (127, 127, 127),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (92, 92, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    if let Some(&(r, g, b)) = ANSI.get(index) {
        return RgbColor { r, g, b };
    }
    let (r, g, b) = match index {
        16..=231 => {
            let i = u8::try_from(index - 16).unwrap_or(0);
            let level = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
            (level(i / 36), level(i / 6 % 6), level(i % 6))
        }
        232..=255 => {
            let v = u8::try_from(8 + 10 * (index - 232)).unwrap_or(238);
            (v, v, v)
        }
        _ => (0, 0, 0),
    };
    RgbColor { r, g, b }
}

#[cfg(test)]
mod tests {
    use ghostty_vt::terminal::{Point, PointCoordinate};

    use super::*;

    fn emu(cols: u16, rows: u16) -> Emu {
        Emu::new(cols, rows, 100).expect("emulator")
    }

    fn new(cols: u16, rows: u16, scrollback: usize) -> Emu {
        Emu::new(cols, rows, scrollback).expect("emulator")
    }

    fn damage(e: &mut Emu) -> Frame {
        e.damage_frame(TerminalId(1), Duration::ZERO)
            .expect("a frame")
    }

    fn text(frame: &Frame, row: u16) -> String {
        frame
            .lines
            .iter()
            .find(|(r, _)| *r == row)
            .map(|(_, l)| l.runs.iter().map(|r| r.text.as_str()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn wide_and_combining_characters_keep_their_columns() {
        let mut e = emu(20, 3);
        e.feed("日本 🙂 e\u{301}x".as_bytes());
        // The emulator's own grid: where each character sits and which cells are spacers.
        let cell = |c: u16| {
            e.term
                .grid_ref(Point::Active(PointCoordinate { x: c, y: 0 }))
                .and_then(|g| g.cell())
                .expect("cell")
        };
        for (col, ch) in [(0, '日'), (2, '本'), (5, '🙂')] {
            assert_eq!(cell(col).codepoint().unwrap(), u32::from(ch));
            assert_eq!(cell(col).wide().unwrap(), CellWide::Wide, "{ch}");
            assert_eq!(cell(col + 1).wide().unwrap(), CellWide::SpacerTail, "{ch}");
        }
        assert_eq!(cell(4).codepoint().unwrap(), u32::from(' '));
        assert_eq!(cell(7).codepoint().unwrap(), u32::from(' '));
        assert_eq!(cell(8).codepoint().unwrap(), u32::from('e'));
        let mut graphemes = ['\0'; 4];
        let len = e
            .term
            .grid_ref(Point::Active(PointCoordinate { x: 8, y: 0 }))
            .and_then(|g| g.graphemes(&mut graphemes))
            .expect("graphemes");
        assert_eq!(graphemes[..len], ['e', '\u{301}']);
        assert_eq!(cell(9).codepoint().unwrap(), u32::from('x'));

        // The frame is one run from column 0 with the spacers left out, so every character
        // keeps its emulator column.
        let frame = e.full_frame(TerminalId(1));
        let runs = &frame.lines[0].1.runs;
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].col, 0);
        assert_eq!(runs[0].text, "日本 🙂 e\u{301}x");
        assert_eq!(frame.cursor.col, 10);
    }

    #[test]
    fn the_smallest_emulator_survives_wide_output() {
        assert_eq!(clamp_size(0, 0), (2, 1));
        assert_eq!(clamp_size(u16::MAX, u16::MAX), (1000, 1000));
        let mut e = emu(1, 1);
        assert_eq!(e.size(), (2, 1));
        e.feed("日本🙂\r\n日".as_bytes());
        let _ = e.full_frame(TerminalId(1));
        e.resize(0, 0);
        e.feed("日本".as_bytes());
        e.resize(u16::MAX, u16::MAX);
        assert_eq!(e.size(), (1000, 1000));
    }

    #[test]
    fn style_changes_split_runs_and_trailing_blanks_are_dropped() {
        let mut e = emu(20, 2);
        e.feed(b"a\x1b[1;31mb\x1b[0m   ");
        let frame = e.full_frame(TerminalId(1));
        let runs = &frame.lines[0].1.runs;
        assert_eq!(runs.len(), 2, "{runs:?}");
        assert_eq!(runs[0].text, "a");
        assert_eq!((runs[1].col, runs[1].text.as_str()), (1, "b"));
        assert!(runs[1].style.has(Style::BOLD));
        assert_eq!(runs[1].style.fg, Color::Indexed(1));
        assert!(frame.lines[1].1.runs.is_empty());
        assert!(frame.full);
    }

    #[test]
    fn damage_frames_carry_only_changed_rows() {
        let mut e = emu(10, 4);
        e.feed(b"one\r\ntwo\r\nthree");
        let first = damage(&mut e);
        assert!(first.full, "a new terminal is fully damaged");
        e.feed(b"\x1b[2;1Hxx");
        let second = damage(&mut e);
        assert!(!second.full);
        assert!(second.lines.iter().any(|(r, _)| *r == 1));
        assert!(!second.lines.iter().any(|(r, _)| *r == 0));
        assert_eq!(text(&second, 1), "xxo");
        e.resize(12, 5);
        let resized = damage(&mut e);
        assert!(resized.full);
        assert_eq!(resized.size, (12, 5));
    }

    #[test]
    fn modes_and_cursor_are_reported() {
        let mut e = emu(10, 4);
        e.feed(b"\x1b[?1h\x1b[?2004h\x1b[?25l\x1b[6 q\x1b[3;4H");
        let frame = e.full_frame(TerminalId(1));
        assert!(frame.modes.has(Modes::APPLICATION_CURSOR));
        assert!(frame.modes.has(Modes::BRACKETED_PASTE));
        assert!(!frame.cursor.visible);
        assert_eq!(frame.cursor.shape, CursorShape::Bar);
        assert_eq!((frame.cursor.row, frame.cursor.col), (2, 3));
    }

    #[test]
    fn a_cursor_shape_nobody_chose_is_the_outer_terminals_default() {
        let mut e = emu(10, 4);
        let frame = e.full_frame(TerminalId(1));
        assert!(frame.cursor.visible);
        assert_eq!(frame.cursor.shape, CursorShape::Default);
        // Blinking mode alone does not choose a shape.
        e.feed(b"\x1b[?12h");
        assert_eq!(
            e.full_frame(TerminalId(1)).cursor.shape,
            CursorShape::Default
        );
        e.feed(b"\x1b[2 q");
        assert_eq!(e.full_frame(TerminalId(1)).cursor.shape, CursorShape::Block);
        // DECSCUSR 0 goes back to the default.
        e.feed(b"\x1b[0 q");
        assert_eq!(
            e.full_frame(TerminalId(1)).cursor.shape,
            CursorShape::Default
        );
    }

    #[test]
    fn scrolling_clamps_to_the_history_and_frames_say_where_it_is() {
        let mut e = new(10, 4, 100);
        for n in 1..=20 {
            e.feed(format!("line {n}\r\n").as_bytes());
        }
        let live = e.full_frame(TerminalId(1));
        assert_eq!((live.display_offset, live.history), (0, 17));

        e.scroll(ScrollTo::Lines(2));
        assert_eq!(e.full_frame(TerminalId(1)).display_offset, 2);
        e.scroll(ScrollTo::Pages(1));
        assert_eq!(e.full_frame(TerminalId(1)).display_offset, 6);
        e.scroll(ScrollTo::Lines(i32::MAX));
        e.scroll(ScrollTo::Pages(i32::MAX));
        let top = e.full_frame(TerminalId(1));
        assert_eq!((top.display_offset, top.history), (17, 17));
        assert_eq!(text(&top, 0), "line 1");
        e.scroll(ScrollTo::Lines(i32::MIN));
        assert_eq!(e.full_frame(TerminalId(1)).display_offset, 0);
        e.scroll(ScrollTo::Top);
        assert_eq!(e.full_frame(TerminalId(1)).display_offset, 17);
        e.scroll(ScrollTo::Pages(-2));
        assert_eq!(e.full_frame(TerminalId(1)).display_offset, 9);
        assert!(e.scroll_to_bottom());
        assert!(!e.scroll_to_bottom());
        assert_eq!(e.full_frame(TerminalId(1)).display_offset, 0);
    }

    #[test]
    fn output_while_scrolled_back_keeps_the_view_and_grows_the_history() {
        let mut e = new(10, 4, 100);
        for n in 1..=10 {
            e.feed(format!("line {n}\r\n").as_bytes());
        }
        e.reset_damage();
        e.scroll(ScrollTo::Lines(2));
        let before = damage(&mut e);
        let shown = text(&before, 0);
        e.feed(b"more\r\n");
        let after = damage(&mut e);
        assert!(after.full, "a scrolled display is sent in full");
        assert_eq!(after.history, before.history + 1);
        assert_eq!(after.display_offset, before.display_offset + 1);
        assert_eq!(text(&after, 0), shown);
    }

    #[test]
    fn the_cursor_follows_the_content_when_scrolled_back_and_hides_below_the_screen() {
        let mut e = new(10, 4, 100);
        for n in 1..=10 {
            e.feed(format!("line {n}\r\n").as_bytes());
        }
        // The cursor is on the last row of the live screen.
        let live = e.full_frame(TerminalId(1)).cursor;
        assert_eq!((live.row, live.visible), (3, true));
        e.scroll(ScrollTo::Lines(1));
        assert!(!e.full_frame(TerminalId(1)).cursor.visible);
        e.scroll(ScrollTo::Top);
        assert!(!e.full_frame(TerminalId(1)).cursor.visible);
        // With the cursor higher on the screen it is still shown, one row lower.
        e.scroll(ScrollTo::Bottom);
        e.feed(b"\x1b[2;1H");
        assert_eq!(e.full_frame(TerminalId(1)).cursor.row, 1);
        e.scroll(ScrollTo::Lines(1));
        let c = e.full_frame(TerminalId(1)).cursor;
        assert_eq!((c.row, c.visible), (2, true));
        e.scroll(ScrollTo::Lines(5));
        assert!(!e.full_frame(TerminalId(1)).cursor.visible);
    }

    #[test]
    fn the_default_cursor_shape_survives_the_alternate_screen_and_a_reset() {
        let shape = |e: &mut Emu| e.full_frame(TerminalId(1)).cursor.shape;
        let mut e = emu(10, 4);
        e.feed(b"\x1b[5 q");
        assert_eq!(shape(&mut e), CursorShape::Bar);
        e.feed(b"\x1b[?1049h");
        assert_eq!(shape(&mut e), CursorShape::Bar);
        e.feed(b"\x1b[3 q\x1b[?1049l");
        assert_eq!(shape(&mut e), CursorShape::Bar);
        e.feed(b"\x1b[0 q");
        assert_eq!(shape(&mut e), CursorShape::Default);
        // The alternate screen of a terminal whose program never chose a shape.
        e.feed(b"\x1b[?1049h");
        assert_eq!(shape(&mut e), CursorShape::Default);
        e.feed(b"\x1b[4 q");
        assert_eq!(shape(&mut e), CursorShape::Underline);
        e.feed(b"\x1b[?1049l");
        // A full reset.
        e.feed(b"\x1b[2 q");
        assert_eq!(shape(&mut e), CursorShape::Block);
        e.feed(b"\x1bc");
        assert_eq!(shape(&mut e), CursorShape::Default);
        e.feed(b"\x1b[?1049h\x1bc");
        assert_eq!(shape(&mut e), CursorShape::Default);
    }

    #[test]
    fn every_style_flag_and_colour_reaches_the_frame() {
        let mut e = emu(40, 2);
        e.feed(
            b"\x1b[1mb\x1b[0;2md\x1b[0;3mi\x1b[0;4mu\x1b[0;4:3mc\x1b[0;7mr\x1b[0;9ms\x1b[0;8mh\x1b[0m \
              \x1b[31ma\x1b[93mb\x1b[38;5;200mc\x1b[38;2;1;2;3md\x1b[0;48;5;17me\x1b[48;2;4;5;6mf",
        );
        let frame = e.full_frame(TerminalId(1));
        let runs = &frame.lines[0].1.runs;
        let flags: Vec<u16> = runs.iter().map(|r| r.style.flags).collect();
        assert_eq!(
            flags[..7],
            [
                Style::BOLD,
                Style::DIM,
                Style::ITALIC,
                // Single and curly underline are one flag, so one run.
                Style::UNDERLINE,
                Style::REVERSE,
                Style::STRIKETHROUGH,
                Style::HIDDEN
            ]
        );
        let colors: Vec<(Color, Color)> =
            runs[8..].iter().map(|r| (r.style.fg, r.style.bg)).collect();
        assert_eq!(
            colors,
            [
                (Color::Indexed(1), Color::Default),
                (Color::Indexed(11), Color::Default),
                (Color::Indexed(200), Color::Default),
                (Color::Rgb(1, 2, 3), Color::Default),
                (Color::Default, Color::Indexed(17)),
                (Color::Default, Color::Rgb(4, 5, 6)),
            ]
        );
    }

    #[test]
    fn a_background_without_text_is_kept() {
        let mut e = emu(10, 2);
        // Erasing with a background leaves cells that hold only the colour.
        e.feed(b"ab\x1b[44m\x1b[K");
        let frame = e.full_frame(TerminalId(1));
        let runs = &frame.lines[0].1.runs;
        assert_eq!(runs.len(), 2, "{runs:?}");
        assert_eq!((runs[1].col, runs[1].text.len()), (2, 8));
        assert_eq!(runs[1].style.bg, Color::Indexed(4));
    }

    #[test]
    fn a_hold_of_synchronized_output_keeps_the_damage_until_its_end_or_timeout() {
        let ms = Duration::from_millis;
        let id = TerminalId(1);
        let mut e = emu(10, 4);
        assert!(e.damage_frame(id, ms(0)).is_some());
        assert_eq!(e.hold_remaining(ms(0)), None);

        // Held, then reset before the timeout: one frame with everything.
        e.feed(b"\x1b[?2026hhello");
        assert!(e.damage_frame(id, ms(10)).is_none());
        assert!(e.damage_frame(id, ms(100)).is_none());
        assert_eq!(e.hold_remaining(ms(100)), Some(ms(60)));
        e.feed(b"\x1b[2;1Hmore\x1b[?2026l");
        let frame = e.damage_frame(id, ms(120)).expect("frame after the reset");
        assert_eq!(
            (text(&frame, 0), text(&frame, 1)),
            ("hello".into(), "more".into())
        );
        assert!(!frame.full);
        assert_eq!(e.hold_remaining(ms(120)), None);

        // Held and never reset: the frame comes 150 ms after the hold was first seen.
        e.feed(b"\x1b[?2026h\x1b[3;1Hlate");
        assert!(e.damage_frame(id, ms(1_000)).is_none());
        assert!(e.damage_frame(id, ms(1_149)).is_none());
        assert_eq!(e.hold_remaining(ms(1_149)), Some(ms(1)));
        let frame = e.damage_frame(id, ms(1_150)).expect("frame at the timeout");
        assert_eq!(text(&frame, 2), "late");
        // The hold is over: the next change goes out at once.
        e.feed(b"x");
        assert!(e.damage_frame(id, ms(1_151)).is_some());

        // The last frame of a terminal goes out whatever it holds.
        e.feed(b"\x1b[?2026h\x1b[4;1Hbye");
        assert!(e.damage_frame(id, ms(2_000)).is_none());
        assert_eq!(text(&e.last_frame(id), 3), "bye");
    }

    #[test]
    fn scrollback_budget_holds_the_configured_lines() {
        // Full lines at the width the budget is for, twice the configured number of them:
        // the emulator keeps at least what is configured, and `history` stops there.
        for lines in [1usize, 10, 100, 1_000, 10_000] {
            let mut e = new(250, 24, lines);
            let line = format!("{}\r\n", "x".repeat(249));
            for _ in 0..(lines * 2 + 24) {
                e.feed(line.as_bytes());
            }
            let kept = e.term.scrollback_rows().unwrap();
            assert!(kept >= lines, "{lines} lines configured, {kept} kept");
            let frame = e.full_frame(TerminalId(1));
            assert_eq!(frame.history, u32::try_from(lines).unwrap());
            // Scrolling back stops at the limit.
            e.scroll(ScrollTo::Top);
            let top = e.full_frame(TerminalId(1));
            assert_eq!(top.display_offset, frame.history);
            e.scroll(ScrollTo::Lines(i32::MAX));
            assert_eq!(e.full_frame(TerminalId(1)).display_offset, frame.history);
        }
        assert_eq!(scrollback_budget(10_000), 26_648_576);
        assert_eq!(scrollback_budget(0), 0);
        // No scrollback: the emulator itself keeps nothing.
        let mut none = new(250, 24, 0);
        for _ in 0..200 {
            none.feed(b"line\r\n");
        }
        assert_eq!(none.term.scrollback_rows().unwrap(), 0);
        assert_eq!(none.history(), 0);
    }

    #[test]
    fn output_while_scrolled_back_never_moves_the_view_past_the_limit() {
        // At 10 columns the emulator keeps far more than 100 lines of history.
        let mut e = new(10, 4, 100);
        for n in 1..=1_000 {
            e.feed(format!("l{n}\r\n").as_bytes());
        }
        e.scroll(ScrollTo::Top);
        let top = e.full_frame(TerminalId(1));
        assert_eq!((top.display_offset, top.history), (100, 100));
        let shown = text(&top, 0);
        for n in 1..=50 {
            e.feed(format!("m{n}\r\n").as_bytes());
        }
        let after = e.full_frame(TerminalId(1));
        assert_eq!((after.display_offset, after.history), (100, 100));
        // The content moved under the view: it shows lines 50 further on.
        let row0: usize = text(&after, 0)[1..].parse().unwrap();
        let before: usize = shown[1..].parse().unwrap();
        assert_eq!(row0, before + 50);
        // One row up or down is one row.
        e.scroll(ScrollTo::Lines(-1));
        assert_eq!(e.full_frame(TerminalId(1)).display_offset, 99);
        e.scroll(ScrollTo::Lines(1));
        assert_eq!(e.full_frame(TerminalId(1)).display_offset, 100);
        e.scroll(ScrollTo::Lines(1));
        assert_eq!(e.full_frame(TerminalId(1)).display_offset, 100);
    }

    #[test]
    fn history_stays_at_the_limit_after_widening_the_terminal() {
        let mut e = new(80, 24, 2_000);
        let line = format!("{}\r\n", "y".repeat(79));
        for _ in 0..4_000 {
            e.feed(line.as_bytes());
        }
        let narrow = e.full_frame(TerminalId(1));
        assert_eq!(narrow.history, 2_000);
        e.resize(250, 24);
        let wide = e.full_frame(TerminalId(1));
        assert_eq!(wide.history, 2_000);
        let more = format!("{}\r\n", "z".repeat(249));
        for _ in 0..4_000 {
            e.feed(more.as_bytes());
        }
        assert_eq!(e.full_frame(TerminalId(1)).history, 2_000);
        // The capture holds the history and the lines of the screen: 23 are text, the
        // cursor's line is empty.
        let all = e.capture(TerminalId(1), None, true, false);
        assert_eq!(all.lines.len(), 2_000 + 23);
    }

    #[test]
    fn alternate_scroll_is_on_by_default_and_a_program_can_turn_it_off() {
        let mut e = emu(10, 4);
        assert!(
            e.full_frame(TerminalId(1))
                .modes
                .has(Modes::ALTERNATE_SCROLL)
        );
        e.feed(b"\x1b[?1007l");
        assert!(
            !e.full_frame(TerminalId(1))
                .modes
                .has(Modes::ALTERNATE_SCROLL)
        );
        e.feed(b"\x1b[?1007h");
        assert!(
            e.full_frame(TerminalId(1))
                .modes
                .has(Modes::ALTERNATE_SCROLL)
        );
    }

    #[test]
    fn events_are_converted() {
        let mut e = emu(10, 4);
        e.set_colors((1, 2, 3), (4, 5, 6));
        let events = e.feed(
            b"\x1b]0;hello\x07\x1b[6n\x1b]11;?\x07\x1b]52;c;aGk=\x07\x1b]52;p;aGk=\x07\x1b]52;c;?\x07\x1b[18t",
        );
        let mut seen = String::new();
        for ev in &events {
            seen.push_str(&format!("{ev:?};"));
        }
        assert!(seen.contains("Title(Some(\"hello\"))"), "{seen}");
        assert!(seen.contains("Reply(\"\\u{1b}[1;1R\")"), "{seen}");
        assert!(seen.contains("ClipboardStore(c, \"hi\")"), "{seen}");
        assert!(seen.contains("ClipboardStore(p, \"hi\")"), "{seen}");
        assert!(seen.contains("[8;4;10t"), "{seen}");
        // A clipboard read is ignored by the emulator: no event, no answer.
        assert_eq!(seen.matches("Clipboard").count(), 2, "{seen}");
        // OSC 11 is answered from the colours given to the emulator.
        assert!(
            seen.contains("Reply(\"\\u{1b}]11;rgb:0404/0505/0606\\u{7}\")"),
            "{seen}"
        );
        assert!(matches!(
            e.feed(b"\x1b]2;\x07").as_slice(),
            [EmuEvent::Title(None)]
        ));
    }

    #[test]
    fn colour_queries_use_the_outer_terminals_colours() {
        let mut e = emu(10, 4);
        // Nobody told the emulator: the fixed fallback.
        assert_eq!(
            answers(&mut e, b"\x1b]10;?\x07\x1b]11;?\x07"),
            [
                "reply \"\\u{1b}]10;rgb:c0c0/c0c0/c0c0\\u{7}\"",
                "reply \"\\u{1b}]11;rgb:0000/0000/0000\\u{7}\"",
            ]
        );
        e.set_colors((1, 2, 3), (255, 254, 253));
        assert_eq!(
            answers(&mut e, b"\x1b]10;?\x07\x1b]11;?\x07"),
            [
                "reply \"\\u{1b}]10;rgb:0101/0202/0303\\u{7}\"",
                "reply \"\\u{1b}]11;rgb:ffff/fefe/fdfd\\u{7}\"",
            ]
        );
        // The terminator of the question is the one of the answer.
        assert_eq!(
            answers(&mut e, b"\x1b]11;?\x1b\\"),
            ["reply \"\\u{1b}]11;rgb:ffff/fefe/fdfd\\u{1b}\\\\\""]
        );
        // A palette entry the program set answers with its own colour.
        assert_eq!(
            answers(&mut e, b"\x1b]4;5;rgb:10/20/30\x07\x1b]4;5;?\x07"),
            ["reply \"\\u{1b}]4;5;rgb:1010/2020/3030\\u{7}\""]
        );
    }

    /// What the emulator says back to a program after `input`, as the events show it.
    fn answers(e: &mut Emu, input: &[u8]) -> Vec<String> {
        e.feed(input)
            .iter()
            .map(|ev| match ev {
                EmuEvent::Reply(s) => format!("reply {s:?}"),
                other => format!("{other:?}"),
            })
            .collect()
    }

    /// The emulator swap kept these answers byte for byte.
    #[test]
    fn replies_to_queries_are_pinned() {
        let none: &[&str] = &[];
        let cases: [(&[u8], &[&str]); 13] = [
            // DA1 and DA2.
            (b"\x1b[c", &["reply \"\\u{1b}[?6c\""]),
            (b"\x1b[>c", &["reply \"\\u{1b}[>0;2600;1c\""]),
            // DSR: status, then the cursor position.
            (b"\x1b[5n", &["reply \"\\u{1b}[0n\""]),
            (b"\x1b[6n", &["reply \"\\u{1b}[1;1R\""]),
            // Window size in pixels (8x16 cells), cell size (no answer), size in cells.
            (b"\x1b[14t", &["reply \"\\u{1b}[4;64;80t\""]),
            (b"\x1b[16t", none),
            (b"\x1b[18t", &["reply \"\\u{1b}[8;4;10t\""]),
            // DECRQM: cursor visible (set), bracketed paste (unset).
            (b"\x1b[?25$p", &["reply \"\\u{1b}[?25;1$y\""]),
            (b"\x1b[?2004$p", &["reply \"\\u{1b}[?2004;2$y\""]),
            // XTVERSION, and the kitty keyboard query alone and after a push: no answer.
            (b"\x1b[>q", none),
            (b"\x1b[?u", none),
            (b"\x1b[>1u\x1b[?u", none),
            // OSC 4 for a palette index the program never set: the xterm colour.
            (
                b"\x1b]4;5;?\x07",
                &["reply \"\\u{1b}]4;5;rgb:cdcd/0000/cdcd\\u{7}\""],
            ),
        ];
        for (query, want) in cases {
            let mut e = emu(10, 4);
            assert_eq!(
                answers(&mut e, query),
                want,
                "{:?}",
                String::from_utf8_lossy(query)
            );
        }
    }

    #[test]
    fn xtversion_is_answered_with_the_name_once_set() {
        let mut e = emu(10, 4);
        e.set_xtversion(Some("ghostty 1.2.0"));
        assert_eq!(
            answers(&mut e, b"\x1b[>q"),
            ["reply \"\\u{1b}P>|ghostty 1.2.0\\u{1b}\\\\\""]
        );
        // The name goes away: unanswered again.
        e.set_xtversion(None);
        assert_eq!(answers(&mut e, b"\x1b[>q"), Vec::<String>::new());
    }

    #[test]
    fn size_reports_use_the_clients_cell_size_once_known() {
        let mut e = emu(80, 24);
        e.resize_with_cell(
            80,
            24,
            Some(CellPx {
                width: 9,
                height: 18,
            }),
        );
        assert_eq!(answers(&mut e, b"\x1b[16t"), ["reply \"\\u{1b}[6;18;9t\""]);
        assert_eq!(
            answers(&mut e, b"\x1b[14t"),
            ["reply \"\\u{1b}[4;432;720t\""]
        );
        // Unknown again: the answer to `CSI 16 t` is dropped, `CSI 14 t` uses 8x16.
        e.resize_with_cell(80, 24, None);
        assert_eq!(answers(&mut e, b"\x1b[16t"), Vec::<String>::new());
        assert_eq!(
            answers(&mut e, b"\x1b[14t"),
            ["reply \"\\u{1b}[4;384;640t\""]
        );
        // The kitty keyboard query and XTVERSION stay unanswered with a known size.
        e.resize_with_cell(
            80,
            24,
            Some(CellPx {
                width: 9,
                height: 18,
            }),
        );
        assert_eq!(answers(&mut e, b"\x1b[?u\x1b[>q"), Vec::<String>::new());
        assert_eq!(
            e.cell(),
            Some(CellPx {
                width: 9,
                height: 18
            })
        );
    }

    #[test]
    fn default_palette_has_the_xterm_cube_and_greys() {
        assert_eq!(default_palette(1), RgbColor { r: 205, g: 0, b: 0 });
        assert_eq!(default_palette(16), RgbColor { r: 0, g: 0, b: 0 });
        assert_eq!(
            default_palette(231),
            RgbColor {
                r: 255,
                g: 255,
                b: 255
            }
        );
        assert_eq!(default_palette(232), RgbColor { r: 8, g: 8, b: 8 });
        assert_eq!(
            default_palette(255),
            RgbColor {
                r: 238,
                g: 238,
                b: 238
            }
        );
    }
}
