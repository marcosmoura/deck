//! The differential compat harness: the same bytes shown on the reference emulator (the
//! harness's `alacritty_terminal`, fed by a program in a PTY) and through deck, and the two
//! screens compared cell by cell.
//!
//! A case is a `.vt` file: header lines `#! size 80x24`, `#! resize 100x30` and `#! history`
//! (compare the scrollback too), other `#` lines
//! at the top as comments, then the bytes in `printf` syntax (`\e`, `\x1b`, `\r`, `\n`, `\t`,
//! `\\`, `\xNN`). Literal line breaks in the file are ignored, so a long case can be wrapped;
//! write `\n` for a line feed.

use std::fmt::{self, Write as _};
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::NamedColor;

use super::{CellView, Color, Harness};

/// How long both screens must stay unchanged, and the longest wait for that.
const STABLE: Duration = Duration::from_millis(300);
/// The window when the screens differ: long enough for slow output to catch up.
const STABLE_APART: Duration = Duration::from_millis(1500);
const SETTLE_LIMIT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(20);
/// How many differing cells a report lists.
const MAX_LISTED: usize = 20;

/// The flags that change how a cell looks, which are the ones compared. (The leading-spacer
/// flag marks a wide character wrapped to the next row; it draws nothing, so it is left out.)
fn visual() -> Flags {
    Flags::BOLD
        | Flags::DIM
        | Flags::ITALIC
        | Flags::ALL_UNDERLINES
        | Flags::INVERSE
        | Flags::STRIKEOUT
        | Flags::HIDDEN
        | Flags::WIDE_CHAR
        | Flags::WIDE_CHAR_SPACER
}

/// One case of the corpus.
#[derive(Debug, Clone)]
pub struct Case {
    pub name: String,
    pub size: (u16, u16),
    pub resize: Option<(u16, u16)>,
    /// Compare the scrollback too (`#! history`).
    pub history: bool,
    pub bytes: Vec<u8>,
}

impl Case {
    /// Loads `crates/deck/tests/compat/cases/<name>.vt`.
    pub fn named(name: &str) -> Self {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/compat/cases")
            .join(format!("{name}.vt"));
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        Self::parse(name, &text)
    }

    /// Loads a recording, `compat/recordings/<name>.vt` (raw bytes), at the size its scenario
    /// recorded it.
    pub fn recording(name: &str) -> Self {
        let path = super::record::recordings_dir().join(format!("{name}.vt"));
        let bytes = fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        Self {
            name: name.into(),
            size: super::record::Scenario::named(name).size,
            resize: None,
            history: false,
            bytes,
        }
    }

    pub fn parse(name: &str, text: &str) -> Self {
        let mut size = (80, 24);
        let mut resize = None;
        let mut history = false;
        let mut body = String::new();
        let mut header = true;
        for line in text.lines() {
            if header && line.starts_with('#') {
                if let Some(rest) = line.strip_prefix("#!") {
                    let mut words = rest.split_whitespace();
                    match (words.next(), words.next()) {
                        (Some("size"), Some(dims)) => size = dims_of(name, dims),
                        (Some("resize"), Some(dims)) => resize = Some(dims_of(name, dims)),
                        (Some("history"), None) => history = true,
                        _ => panic!("{name}: unknown header line {line:?}"),
                    }
                }
                continue;
            }
            header = false;
            body.push_str(line);
        }
        Self {
            name: name.into(),
            size,
            resize,
            history,
            bytes: unescape(name, &body),
        }
    }
}

fn dims_of(name: &str, text: &str) -> (u16, u16) {
    text.split_once('x')
        .and_then(|(c, r)| Some((c.parse().ok()?, r.parse().ok()?)))
        .unwrap_or_else(|| panic!("{name}: bad size {text:?}"))
}

/// Converts the `printf`-style escapes of a case to bytes.
pub fn unescape(name: &str, text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        let Some(&next) = bytes.get(i + 1) else {
            panic!("{name}: a backslash ends the case");
        };
        i += 2;
        match next {
            b'e' => out.push(0x1b),
            b'r' => out.push(b'\r'),
            b'n' => out.push(b'\n'),
            b't' => out.push(b'\t'),
            b'\\' => out.push(b'\\'),
            b'x' => {
                let hex = text
                    .get(i..i + 2)
                    .and_then(|h| u8::from_str_radix(h, 16).ok())
                    .unwrap_or_else(|| panic!("{name}: bad \\x escape at byte {i}"));
                out.push(hex);
                i += 2;
            }
            other => panic!("{name}: unknown escape \\{}", char::from(other)),
        }
    }
    out
}

/// A colour with the named colours 0 to 15 folded into the indexed ones.
#[derive(Debug, Clone, PartialEq)]
enum Norm {
    DefaultFg,
    DefaultBg,
    Index(u8),
    Rgb(u8, u8, u8),
    Other(String),
}

impl Norm {
    fn of(color: Color) -> Self {
        match color {
            Color::Named(NamedColor::Foreground) => Self::DefaultFg,
            Color::Named(NamedColor::Background) => Self::DefaultBg,
            Color::Named(named) => match u8::try_from(named as usize) {
                Ok(n) if n < 16 => Self::Index(n),
                _ => Self::Other(format!("{named:?}")),
            },
            Color::Indexed(n) => Self::Index(n),
            Color::Spec(rgb) => Self::Rgb(rgb.r, rgb.g, rgb.b),
        }
    }
}

impl fmt::Display for Norm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const NAMES: [&str; 8] = [
            "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
        ];
        match self {
            Self::DefaultFg | Self::DefaultBg => f.write_str("default"),
            Self::Index(n) if *n < 8 => f.write_str(NAMES[usize::from(*n)]),
            Self::Index(n) => write!(f, "#{n}"),
            Self::Rgb(r, g, b) => write!(f, "#{r:02x}{g:02x}{b:02x}"),
            Self::Other(name) => f.write_str(name),
        }
    }
}

/// A screen: every cell and the cursor.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub cols: usize,
    pub rows: usize,
    /// Row-major.
    pub cells: Vec<CellView>,
    pub cursor: (usize, usize),
    pub cursor_shown: bool,
    /// Shape and blinking, e.g. `Block steady`.
    pub cursor_style: String,
}

impl Snapshot {
    fn of(h: &Harness) -> Self {
        let (cols, rows) = h.size();
        let (cols, rows) = (usize::from(cols), usize::from(rows));
        let cells = (0..rows)
            .flat_map(|r| (0..cols).map(move |c| (r, c)))
            .map(|(r, c)| h.cell(r, c))
            .collect();
        let (row, col, cursor_shown) = h.cursor();
        Self {
            cols,
            rows,
            cells,
            cursor: (row, col),
            cursor_shown,
            cursor_style: h.cursor_style(),
        }
    }

    pub fn cell(&self, row: usize, col: usize) -> &CellView {
        &self.cells[row * self.cols + col]
    }

    pub fn cell_mut(&mut self, row: usize, col: usize) -> &mut CellView {
        &mut self.cells[row * self.cols + col]
    }

    /// The screen as text, one line per row, trailing spaces removed.
    pub fn text(&self) -> String {
        let mut lines = Vec::new();
        for row in 0..self.rows {
            let mut line = String::new();
            for col in 0..self.cols {
                let cell = self.cell(row, col);
                if !cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    line.push_str(&shown(cell));
                }
            }
            lines.push(line.trim_end().to_string());
        }
        lines.join("\n")
    }
}

/// What a known difference leaves out of the comparison.
#[derive(Debug, Clone)]
pub enum Ignore {
    /// One or more attribute flags.
    Attr(Flags),
    /// A block of cells: rows and columns.
    Region(Range<usize>, Range<usize>),
    /// The cursor's position and visibility, and its style.
    Cursor,
    /// The cursor's shape and blinking, compared while the cursor is shown.
    CursorStyle,
    /// The scrollback of a `#! history` case.
    History,
    /// The whole case: it must still differ.
    Everything,
}

/// A screen of each side after the bytes (and after the resize, for the second phase).
#[derive(Debug, Clone)]
pub struct Phase {
    pub label: String,
    pub reference: Snapshot,
    pub deck: Snapshot,
}

/// Both sides of a case, captured.
#[derive(Debug, Clone)]
pub struct Run {
    pub case: String,
    pub phases: Vec<Phase>,
    /// The scrollback and screen as text, `(reference, deck)`, for a `#! history` case.
    pub history: Option<(String, String)>,
}

/// A test-only hook that alters deck's captured screen before it is compared.
pub type Tamper<'a> = &'a dyn Fn(&mut Snapshot);

const SCRIPT: &str = "stty raw -echo; cat \"$0\"; exec sleep 100000";

/// Shows the case on both sides and compares them: `Ok` when the screens are equal.
pub fn compare(case: &Case) -> Result<(), Box<Diff>> {
    capture(case, None).diff(&[]).inspect_err(|d| d.save())
}

/// Like [`compare`], leaving out what `ignore` names.
pub fn compare_ignoring(case: &Case, ignore: &[Ignore]) -> Result<(), Box<Diff>> {
    capture(case, None).diff(ignore).inspect_err(|d| d.save())
}

/// Like [`compare`], after `tamper` changed deck's first screen.
pub fn compare_tampered(case: &Case, tamper: Tamper<'_>) -> Result<(), Box<Diff>> {
    capture(case, Some(tamper)).diff(&[])
}

/// Runs the case on the reference and on deck.
pub fn capture(case: &Case, tamper: Option<Tamper<'_>>) -> Run {
    let dir = tempfile::tempdir().expect("temp directory for the case");
    let file = dir.path().join("input.bin");
    fs::write(&file, &case.bytes).expect("write the case bytes");
    let file = file.to_string_lossy().into_owned();

    let mut reference = Harness::spawn(case.size, &["/bin/sh", "-c", SCRIPT, &file]);
    let args = format!("[\"-c\", {SCRIPT:?}, {file:?}]");
    let mut deck = Harness::builder(case.size)
        .config("shell", "\"/bin/sh\"")
        .config("shell_args", &args)
        .deck();

    let mut phases = Vec::new();
    // Deck draws before the shell has run; wait for the first frame so a blank screen is
    // not taken for a stable one.
    let started = Instant::now();
    while (deck.raw_output().is_empty() || reference.raw_output().is_empty())
        && started.elapsed() < SETTLE_LIMIT
    {
        std::thread::sleep(POLL);
    }
    let (r, d) = settle(&reference, &deck);
    let mut deck_first = d;
    if let Some(tamper) = tamper {
        tamper(&mut deck_first);
    }
    phases.push(Phase {
        label: format!("{}x{}", case.size.0, case.size.1),
        reference: r,
        deck: deck_first,
    });
    if let Some((cols, rows)) = case.resize {
        // Both mirrors reflow at once; deck's own redraw arrives later, and settling on the
        // reflow alone would compare the reference with itself. Wait for deck to write.
        let before = deck.raw_output().len();
        reference.resize(cols, rows);
        deck.resize(cols, rows);
        let started = Instant::now();
        while deck.raw_output().len() == before && started.elapsed() < SETTLE_LIMIT {
            std::thread::sleep(POLL);
        }
        let (r, d) = settle(&reference, &deck);
        phases.push(Phase {
            label: format!("{}x{} after resize", cols, rows),
            reference: r,
            deck: d,
        });
    }
    let history = case.history.then(|| {
        // The scrollback settles after the screen; wait until it has stayed unchanged.
        let read = || (reference.history_text(), deck_history(&deck));
        let mut last = read();
        let mut since = Instant::now();
        let deadline = Instant::now() + SETTLE_LIMIT;
        loop {
            let needed = if last.0 == last.1 {
                STABLE
            } else {
                STABLE_APART
            };
            if since.elapsed() >= needed || Instant::now() >= deadline {
                break last;
            }
            std::thread::sleep(POLL);
            let now = read();
            if now != last {
                since = Instant::now();
                last = now;
            }
        }
    });
    Run {
        case: case.name.clone(),
        phases,
        history,
    }
}

/// `deck capture --history` of the only terminal.
fn deck_history(deck: &Harness) -> String {
    let tree = deck.run_deck(&["ls", "--json"]);
    let tree: serde_json::Value =
        serde_json::from_slice(&tree.stdout).expect("deck ls --json prints JSON");
    let id = tree["sessions"][0]["spaces"][0]["panels"][0]["terminals"][0]["id"]
        .as_u64()
        .expect("the terminal's id")
        .to_string();
    let out = deck.run_deck(&["capture", "--terminal", &id, "--history"]);
    String::from_utf8_lossy(&out.stdout).trim_end().to_string()
}

/// Waits until the two screens are equal and both have stayed unchanged for [`STABLE`]; if
/// they differ, until both have stayed unchanged for the longer [`STABLE_APART`]. Under load
/// a side's output can arrive late, so a difference is only reported once it has lasted.
/// Gives up after [`SETTLE_LIMIT`] and returns what it has.
fn settle(reference: &Harness, deck: &Harness) -> (Snapshot, Snapshot) {
    let deadline = Instant::now() + SETTLE_LIMIT;
    let mut last = (Snapshot::of(reference), Snapshot::of(deck));
    let mut since = Instant::now();
    loop {
        let quiet = since.elapsed();
        let equal = differing_cells(&last.0, &last.1, &[]).is_empty()
            && cursor_of(&last.0) == cursor_of(&last.1);
        let needed = if equal { STABLE } else { STABLE_APART };
        if quiet >= needed || Instant::now() >= deadline {
            return last;
        }
        std::thread::sleep(POLL);
        let now = (Snapshot::of(reference), Snapshot::of(deck));
        if !same(&now.0, &last.0, &[]) || !same(&now.1, &last.1, &[]) {
            since = Instant::now();
            last = now;
        }
    }
}

impl Run {
    /// Compares every phase; the first that differs is the error.
    pub fn diff(&self, ignore: &[Ignore]) -> Result<(), Box<Diff>> {
        for phase in &self.phases {
            let cells = differing_cells(&phase.reference, &phase.deck, ignore);
            let cursor = !ignore.iter().any(|i| matches!(i, Ignore::Cursor))
                && cursor_of(&phase.reference) != cursor_of(&phase.deck);
            let style = !ignore
                .iter()
                .any(|i| matches!(i, Ignore::Cursor | Ignore::CursorStyle))
                && style_of(&phase.reference) != style_of(&phase.deck);
            if cells.is_empty() && !cursor && !style && phase.reference.cols == phase.deck.cols {
                continue;
            }
            let diff = Diff {
                case: self.case.clone(),
                phase: phase.clone(),
                cells,
                cursor,
                style,
                history: None,
            };
            return Err(Box::new(diff));
        }
        if let Some((reference, deck)) = &self.history
            && reference != deck
            && !ignore.iter().any(|i| matches!(i, Ignore::History))
        {
            let phase = self.phases.last().expect("a case has a phase");
            return Err(Box::new(Diff {
                case: self.case.clone(),
                phase: phase.clone(),
                cells: Vec::new(),
                cursor: false,
                style: false,
                history: Some((reference.clone(), deck.clone())),
            }));
        }
        Ok(())
    }
}

/// Where the cursor is while it is shown. Deck does not move a hidden cursor: it rests where
/// the last frame's writes ended, which depends on how the output was split into frames.
fn cursor_of(s: &Snapshot) -> Option<(usize, usize)> {
    s.cursor_shown.then_some(s.cursor)
}

/// The cursor's style while it is shown; a hidden cursor has none to compare.
fn style_of(s: &Snapshot) -> Option<&str> {
    s.cursor_shown.then_some(s.cursor_style.as_str())
}

/// Whether two snapshots of the same side are equal, for the settle loop.
fn same(a: &Snapshot, b: &Snapshot, ignore: &[Ignore]) -> bool {
    differing_cells(a, b, ignore).is_empty() && cursor_of(a) == cursor_of(b)
}

type Key = (String, Norm, Norm, Flags, Option<Norm>);

/// What a cell shows. `alacritty_terminal` stores the tab it moved over in the cell, which
/// draws as a blank.
fn shown(cell: &CellView) -> String {
    cell.grapheme()
        .chars()
        .map(|c| if c == '\t' || c == '\0' { ' ' } else { c })
        .collect()
}

fn key(cell: &CellView, mask: Flags) -> Key {
    (
        shown(cell),
        Norm::of(cell.fg),
        Norm::of(cell.bg),
        cell.flags & visual() & !mask,
        cell.underline_color.map(Norm::of),
    )
}

fn differing_cells(a: &Snapshot, b: &Snapshot, ignore: &[Ignore]) -> Vec<(usize, usize)> {
    let mask = ignore.iter().fold(Flags::empty(), |m, i| match i {
        Ignore::Attr(flags) => m | *flags,
        _ => m,
    });
    let mut out = Vec::new();
    for row in 0..a.rows.min(b.rows) {
        for col in 0..a.cols.min(b.cols) {
            let skipped = ignore.iter().any(
                |i| matches!(i, Ignore::Region(rows, cols) if rows.contains(&row) && cols.contains(&col)),
            );
            if !skipped && key(a.cell(row, col), mask) != key(b.cell(row, col), mask) {
                out.push((row, col));
            }
        }
    }
    out
}

/// A report of how deck's screen differs from the reference's.
#[derive(Debug)]
pub struct Diff {
    case: String,
    phase: Phase,
    cells: Vec<(usize, usize)>,
    cursor: bool,
    style: bool,
    /// Set when only the scrollback differs: `(reference, deck)`.
    history: Option<(String, String)>,
}

impl Diff {
    pub fn cells(&self) -> &[(usize, usize)] {
        &self.cells
    }

    fn capture_dir(&self) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/e2e-captures/compat")
            .join(&self.case)
    }

    /// Writes both screens to `target/e2e-captures/compat/<case>/`.
    pub fn save(&self) {
        let dir = self.capture_dir();
        if fs::create_dir_all(&dir).is_ok() {
            let _ = fs::write(
                dir.join("reference.txt"),
                self.phase.reference.text() + "\n",
            );
            let _ = fs::write(dir.join("deck.txt"), self.phase.deck.text() + "\n");
            if let Some((reference, deck)) = &self.history {
                let _ = fs::write(dir.join("reference-history.txt"), reference.clone() + "\n");
                let _ = fs::write(dir.join("deck-history.txt"), deck.clone() + "\n");
            }
        }
    }
}

fn describe(cell: &CellView) -> String {
    let mut out = format!(
        "'{}' {}/{}",
        shown(cell),
        Norm::of(cell.fg),
        Norm::of(cell.bg)
    );
    for (flag, name) in [
        (Flags::BOLD, "bold"),
        (Flags::DIM, "dim"),
        (Flags::ITALIC, "italic"),
        (Flags::UNDERLINE, "underline"),
        (Flags::DOUBLE_UNDERLINE, "double-underline"),
        (Flags::UNDERCURL, "curly-underline"),
        (Flags::DOTTED_UNDERLINE, "dotted-underline"),
        (Flags::DASHED_UNDERLINE, "dashed-underline"),
        (Flags::INVERSE, "inverse"),
        (Flags::STRIKEOUT, "strikeout"),
        (Flags::HIDDEN, "hidden"),
        (Flags::WIDE_CHAR, "wide"),
        (Flags::WIDE_CHAR_SPACER, "spacer"),
    ] {
        if cell.flags.contains(flag) {
            let _ = write!(out, " {name}");
        }
    }
    out
}

impl fmt::Display for Diff {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Phase {
            label,
            reference,
            deck,
        } = &self.phase;
        writeln!(
            f,
            "compat case {:?} differs at {label}: {} cell(s) differ{}",
            self.case,
            self.cells.len(),
            if self.cursor {
                ", the cursor differs"
            } else if self.style {
                ", the cursor style differs"
            } else {
                ""
            }
        )?;
        for &(row, col) in self.cells.iter().take(MAX_LISTED) {
            writeln!(
                f,
                "  ({row}, {col}) ref={}  deck={}",
                describe(reference.cell(row, col)),
                describe(deck.cell(row, col))
            )?;
        }
        if self.cells.len() > MAX_LISTED {
            writeln!(f, "  ... and {} more", self.cells.len() - MAX_LISTED)?;
        }
        if self.cursor {
            writeln!(
                f,
                "  cursor ref=({}, {}) {}  deck=({}, {}) {}",
                reference.cursor.0,
                reference.cursor.1,
                if reference.cursor_shown {
                    "shown"
                } else {
                    "hidden"
                },
                deck.cursor.0,
                deck.cursor.1,
                if deck.cursor_shown { "shown" } else { "hidden" },
            )?;
        }
        if self.style {
            writeln!(
                f,
                "  cursor style ref={}  deck={}",
                reference.cursor_style, deck.cursor_style
            )?;
        }
        let (left, right) = match &self.history {
            Some((r, d)) => {
                writeln!(f, "  the scrollback differs")?;
                (r.clone(), d.clone())
            }
            None => (reference.text(), deck.text()),
        };
        let (left, right): (Vec<_>, Vec<_>) = (left.lines().collect(), right.lines().collect());
        let width = left.iter().map(|l| l.chars().count()).max().unwrap_or(0);
        writeln!(f, "  {:<width$} | deck", "reference")?;
        for i in 0..left.len().max(right.len()) {
            let l = left.get(i).copied().unwrap_or("");
            let r = right.get(i).copied().unwrap_or("");
            writeln!(f, "  {l:<width$} | {r}")?;
        }
        write!(f, "  captures: {}", self.capture_dir().display())
    }
}
