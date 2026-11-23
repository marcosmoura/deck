//! Where things go in the outer window, and the automatic grid inside a panel.
//!
//! The grid functions are pure integer geometry (`u32` arithmetic, never floats): see
//! `docs/architecture.md#layout`. Terminal indexes are 0-based everywhere in this module.

use serde::{Deserialize, Serialize};

use crate::config::SIDEBAR_WIDTH_RANGE;

pub mod golden;
#[cfg(test)]
mod tests;

/// The panel area keeps at least this many columns, or the sidebar is hidden.
pub const MIN_PANEL_COLS: u16 = 20;

/// Columns the sidebar and its separator take in a window `cols` wide: none when the user
/// hid it (`shown` false) or when the panel area would be narrower than [`MIN_PANEL_COLS`].
pub fn sidebar_cols(cols: u16, sidebar_width: u16, shown: bool) -> u16 {
    if shown && cols >= sidebar_width + 1 + MIN_PANEL_COLS {
        sidebar_width + 1
    } else {
        0
    }
}

/// `wanted` as a sidebar width in a window `cols` wide: within the `sidebar_width` limits
/// and leaving the panel area its [`MIN_PANEL_COLS`]. Where the two conflict (a window under
/// 41 columns, which hides the sidebar anyway) the lower limit wins.
pub fn clamp_sidebar_width(cols: u16, wanted: u16) -> u16 {
    let (low, high) = (*SIDEBAR_WIDTH_RANGE.start(), *SIDEBAR_WIDTH_RANGE.end());
    let room = cols.saturating_sub(1 + MIN_PANEL_COLS);
    wanted.min(high).min(room).max(low)
}

/// The panel area of an `outer` window (columns, rows): what is left of it after the
/// sidebar; it keeps every row. `sidebar_visible` says the user has not hidden the
/// sidebar; a window too narrow for it hides it anyway.
pub fn panel_area(outer: (u16, u16), sidebar_width: u16, sidebar_visible: bool) -> (u16, u16) {
    let (cols, rows) = outer;
    (
        cols - sidebar_cols(cols, sidebar_width, sidebar_visible),
        rows,
    )
}

/// A rectangle in cell units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl Rect {
    pub const fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    fn left(self) -> u32 {
        u32::from(self.x)
    }

    fn top(self) -> u32 {
        u32::from(self.y)
    }

    fn right(self) -> u32 {
        u32::from(self.x) + u32::from(self.width)
    }

    fn bottom(self) -> u32 {
        u32::from(self.y) + u32::from(self.height)
    }

    fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// A direction of focus or resize.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// The shape of the grid for a terminal count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shape {
    /// Terminal 1 takes the left part at full height (odd counts above 1).
    pub main: bool,
    /// Columns of the grid (the right-hand grid when `main`).
    pub cols: usize,
    /// Rows of the grid.
    pub rows: usize,
}

/// The shape for `n` terminals (`n` below 1 is read as 1).
pub fn shape(n: usize) -> Shape {
    let n = n.max(1);
    if n == 1 {
        return Shape {
            main: false,
            cols: 1,
            rows: 1,
        };
    }
    if n.is_multiple_of(2) {
        let (small, large) = near_square(n);
        Shape {
            main: false,
            cols: large,
            rows: small,
        }
    } else {
        let (small, large) = near_square(n - 1);
        Shape {
            main: true,
            cols: small,
            rows: large,
        }
    }
}

/// The factor pair of `n` whose members differ least, as `(smaller, larger)`.
fn near_square(n: usize) -> (usize, usize) {
    let mut small = 1;
    let mut candidate = 1;
    while candidate * candidate <= n {
        if n.is_multiple_of(candidate) {
            small = candidate;
        }
        candidate += 1;
    }
    (small, n / small)
}

/// Sizes of the tracks of a grid, in per-mille. See [`Ratios::default_for`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ratios {
    /// The left part's share for odd counts above 1.
    pub main: Option<u16>,
    /// Column widths of the grid.
    pub cols: Vec<u16>,
    /// Row heights of the grid.
    pub rows: Vec<u16>,
}

impl Ratios {
    /// The default ratios for `n` terminals: main at half, equal tracks.
    pub fn default_for(n: usize) -> Self {
        let shape = shape(n);
        Self {
            main: shape.main.then_some(500),
            cols: equal_shares(shape.cols),
            rows: equal_shares(shape.rows),
        }
    }

    /// Whether these ratios describe the grid of `n` terminals.
    pub fn is_valid_for(&self, n: usize) -> bool {
        let shape = shape(n);
        let main_ok = match self.main {
            Some(main) => shape.main && (1..=999).contains(&main),
            None => !shape.main,
        };
        main_ok && shares_valid(&self.cols, shape.cols) && shares_valid(&self.rows, shape.rows)
    }

    /// The ratios for `new_n` terminals that keep the proportions of `old` (valid for
    /// `old_n`), relative to the defaults: untouched defaults adapt to the new defaults.
    #[expect(
        clippy::indexing_slicing,
        reason = "`full` and `d_new_full` have one track per column of the new shape plus the main split, so `j + 1` is in range for odd counts"
    )]
    pub fn adapt(old: &Self, old_n: usize, new_n: usize) -> Self {
        let old = if old.is_valid_for(old_n) {
            old.clone()
        } else {
            Self::default_for(old_n)
        };
        let d_old = Self::default_for(old_n);
        let d_new = Self::default_for(new_n);
        let d_new_full = full_columns(&d_new, new_n);

        let full = remap(
            &full_columns(&old, old_n),
            &full_columns(&d_old, old_n),
            &d_new_full,
        );
        let rows = remap(&widen(&old.rows), &widen(&d_old.rows), &widen(&d_new.rows));

        if shape(new_n).main {
            let main = full[0].clamp(1, 999);
            let grid: Vec<u32> = d_new
                .cols
                .iter()
                .enumerate()
                .map(|(j, &share)| u32::from(share) * u32::from(full[j + 1]) / d_new_full[j + 1])
                .collect();
            Self {
                main: Some(main),
                cols: normalize(&grid, 1000),
                rows,
            }
        } else {
            Self {
                main: None,
                cols: full,
                rows,
            }
        }
    }
}

fn equal_shares(len: usize) -> Vec<u16> {
    let len = len.max(1);
    let share = u16::try_from(1000 / len).unwrap_or(1);
    let mut shares = vec![share; len];
    let used = share * u16::try_from(len).unwrap_or(1);
    if let Some(last) = shares.last_mut() {
        *last += 1000 - used;
    }
    shares
}

fn shares_valid(shares: &[u16], len: usize) -> bool {
    shares.len() == len
        && shares.iter().all(|&share| share >= 1)
        && shares.iter().map(|&share| u32::from(share)).sum::<u32>() == 1000
}

fn widen(shares: &[u16]) -> Vec<u32> {
    shares.iter().map(|&share| u32::from(share)).collect()
}

/// `values` scaled to sum to `total`: floor per track, the remainder on the last one.
fn scale(values: &[u32], total: u32) -> Vec<u32> {
    let sum: u32 = values.iter().sum();
    if sum == 0 {
        let len = values.len().max(1);
        let each = total / u32::try_from(len).unwrap_or(1);
        let mut out = vec![each; len];
        let used: u32 = out.iter().sum();
        if let Some(last) = out.last_mut() {
            *last += total - used;
        }
        return out;
    }
    let mut out: Vec<u32> = values.iter().map(|&v| v * total / sum).collect();
    let used: u32 = out.iter().sum();
    if let Some(last) = out.last_mut() {
        *last += total - used;
    }
    out
}

/// [`scale`] to `total`, then every share raised to at least 1, taken from the largest.
#[expect(
    clippy::indexing_slicing,
    reason = "`i` and `k` come from ranges over `out.len()`"
)]
fn normalize(values: &[u32], total: u32) -> Vec<u16> {
    let mut out = scale(values, total);
    for i in 0..out.len() {
        if out[i] == 0 {
            let largest = (0..out.len())
                .max_by_key(|&k| out[k])
                .filter(|&k| out[k] > 1);
            if let Some(k) = largest {
                out[k] -= 1;
                out[i] = 1;
            }
        }
    }
    out.into_iter()
        .map(|v| u16::try_from(v).unwrap_or(u16::MAX))
        .collect()
}

/// The ratios as full-width columns: `[main, grid columns scaled to the rest]` for odd
/// counts, the columns otherwise.
fn full_columns(ratios: &Ratios, n: usize) -> Vec<u32> {
    let shape = shape(n);
    if n <= 1 {
        vec![1000]
    } else if shape.main {
        let main = u32::from(ratios.main.unwrap_or(500));
        let mut full = vec![main];
        full.extend(scale(&widen(&ratios.cols), 1000 - main));
        full
    } else {
        widen(&ratios.cols)
    }
}

/// Track `i` of the new list: `old[i] × d_new[i] / d_old[i]`, or `d_new[i]` for an added
/// track; normalized to 1000.
#[expect(
    clippy::indexing_slicing,
    reason = "`old.get(i)` is `Some` only for `i < old.len() == d_old.len()`"
)]
fn remap(old: &[u32], d_old: &[u32], d_new: &[u32]) -> Vec<u16> {
    let mapped: Vec<u32> = d_new
        .iter()
        .enumerate()
        .map(|(i, &new)| match old.get(i) {
            Some(&share) => share * new / d_old[i],
            None => new,
        })
        .collect();
    normalize(&mapped, 1000)
}

/// The ratios to use: the given ones, or the defaults when they are invalid for `n`.
fn effective(ratios: &Ratios, n: usize) -> Ratios {
    if ratios.is_valid_for(n) {
        ratios.clone()
    } else {
        Ratios::default_for(n)
    }
}

/// Cell sizes of the tracks of a span `total` cells long: cumulative boundaries.
fn split(total: u32, shares: &[u16]) -> Vec<u32> {
    let mut out = Vec::with_capacity(shares.len());
    let mut previous = 0;
    let mut cumulative = 0;
    for (k, &share) in shares.iter().enumerate() {
        cumulative += u32::from(share);
        let end = if k + 1 == shares.len() {
            total
        } else {
            total * cumulative / 1000
        };
        out.push(end - previous);
        previous = end;
    }
    out
}

/// Cells the main split gives the left terminal.
fn main_width(width: u32, main: u16) -> u32 {
    (width * u32::from(main) + 500) / 1000
}

/// Cell sizes of the tracks along one axis: for the horizontal axis of an odd layout,
/// the main width followed by the grid columns; otherwise the columns or rows.
fn axis_tracks(area: Rect, n: usize, ratios: &Ratios, horizontal: bool) -> Vec<u32> {
    let shape = shape(n);
    if !horizontal {
        return split(u32::from(area.height), &ratios.rows);
    }
    let width = u32::from(area.width);
    if shape.main {
        let main = main_width(width, ratios.main.unwrap_or(500));
        let mut tracks = vec![main];
        tracks.extend(split(width - main, &ratios.cols));
        tracks
    } else {
        split(width, &ratios.cols)
    }
}

/// One rect per terminal, in order. Invalid `ratios` are replaced by the defaults.
#[expect(
    clippy::indexing_slicing,
    reason = "`xs` has the main track plus `shape.cols` tracks (or just the columns) and `ys` has `shape.rows`; `col < cols` and `row < rows` by the division and modulo of `g`"
)]
pub fn layout(area: Rect, n: usize, ratios: &Ratios) -> Vec<Rect> {
    if n == 0 {
        return Vec::new();
    }
    let ratios = effective(ratios, n);
    let shape = shape(n);
    let xs = axis_tracks(area, n, &ratios, true);
    let ys = axis_tracks(area, n, &ratios, false);
    let cell = |v: u32| u16::try_from(v).unwrap_or(u16::MAX);

    let mut rects = Vec::with_capacity(n);
    let mut x0 = area.left();
    let mut grid_xs = xs.as_slice();
    if shape.main {
        rects.push(Rect::new(area.x, area.y, cell(xs[0]), area.height));
        x0 += xs[0];
        grid_xs = &xs[1..];
    }
    for g in 0..shape.cols * shape.rows {
        let (col, row) = (g % shape.cols, g / shape.cols);
        let x = x0 + grid_xs[..col].iter().sum::<u32>();
        let y = area.top() + ys[..row].iter().sum::<u32>();
        rects.push(Rect::new(
            cell(x),
            cell(y),
            cell(grid_xs[col]),
            cell(ys[row]),
        ));
    }
    rects
}

/// The inner size (what the PTY gets) of a terminal in a rect, for `n` terminals: the
/// whole rect for one terminal (no border), else the rect minus its border. At least 1×1.
pub fn inner(rect: Rect, n: usize) -> (u16, u16) {
    let border = if n >= 2 { 2 } else { 0 };
    (
        rect.width.saturating_sub(border).max(1),
        rect.height.saturating_sub(border).max(1),
    )
}

/// Whether every inner size of the layout is at least `min_cols × min_rows`.
pub fn fits(area: Rect, n: usize, ratios: &Ratios, min_cols: u16, min_rows: u16) -> bool {
    layout(area, n, ratios).into_iter().all(|rect| {
        let (cols, rows) = inner(rect, n);
        cols >= min_cols && rows >= min_rows
    })
}

/// The smallest inner size every terminal of a panel may get when one is added.
pub const MIN_INNER: (u16, u16) = (10, 3);

/// The rects of the `n` terminals of a panel with the stored `ratios` in `area`: the grid
/// drawn with [`fit`] applied, which is what the PTYs are sized from.
pub fn panel_rects(area: Rect, n: usize, ratios: &Ratios) -> Vec<Rect> {
    let effective = fit(area, n, ratios, MIN_INNER.0, MIN_INNER.1);
    layout(area, n, &effective)
}

/// Whether a panel of `n - 1` terminals has room for the `n`-th: the default ratios for `n`
/// give every terminal at least [`MIN_INNER`].
pub fn has_room_for(area: Rect, n: usize) -> bool {
    fits(area, n, &Ratios::default_for(n), MIN_INNER.0, MIN_INNER.1)
}

/// The effective ratios for one viewport: the stored ones when they fit, else per axis
/// each boundary moved by the fewest cells that gives every terminal the minimum inner
/// size, else (axis too short) that axis's defaults. Never changes the stored ratios.
#[expect(
    clippy::indexing_slicing,
    reason = "`positions` holds both ends plus one entry per boundary, so the inner slices and `positions[1]` (odd layouts have at least two tracks) are in range"
)]
pub fn fit(area: Rect, n: usize, ratios: &Ratios, min_cols: u16, min_rows: u16) -> Ratios {
    let base = effective(ratios, n);
    if n < 2 {
        return base;
    }
    let defaults = Ratios::default_for(n);
    let shape = shape(n);
    let mut out = base.clone();

    // Horizontal axis.
    let m = u32::from(min_cols) + 2;
    let tracks = axis_tracks(area, n, &base, true);
    if tracks.iter().any(|&t| t < m) {
        let width = u32::from(area.width);
        let count = u32::try_from(tracks.len()).unwrap_or(u32::MAX);
        if width < count * m {
            out.main = defaults.main;
            out.cols = defaults.cols.clone();
        } else {
            let mut positions = boundaries(&tracks);
            project(&mut positions, m);
            if shape.main {
                let main = ((positions[1] * 1000 + width / 2) / width).clamp(1, 999);
                let main = u16::try_from(main).unwrap_or(999);
                let grid_start = main_width(width, main);
                let span = width - grid_start;
                let inner_bounds: Vec<u32> = positions[2..positions.len() - 1]
                    .iter()
                    .map(|&p| ceil_share(p.saturating_sub(grid_start), span))
                    .collect();
                out.main = Some(main);
                out.cols = shares_from_cumulative(&inner_bounds, shape.cols);
            } else {
                let inner_bounds: Vec<u32> = positions[1..positions.len() - 1]
                    .iter()
                    .map(|&p| ceil_share(p, width))
                    .collect();
                out.cols = shares_from_cumulative(&inner_bounds, shape.cols);
            }
        }
    }

    // Vertical axis.
    let m = u32::from(min_rows) + 2;
    let tracks = axis_tracks(area, n, &base, false);
    if tracks.iter().any(|&t| t < m) {
        let height = u32::from(area.height);
        let count = u32::try_from(tracks.len()).unwrap_or(u32::MAX);
        if height < count * m {
            out.rows = defaults.rows;
        } else {
            let mut positions = boundaries(&tracks);
            project(&mut positions, m);
            let inner_bounds: Vec<u32> = positions[1..positions.len() - 1]
                .iter()
                .map(|&p| ceil_share(p, height))
                .collect();
            out.rows = shares_from_cumulative(&inner_bounds, shape.rows);
        }
    }
    out
}

/// Positions of the track boundaries including both ends: `[0, …, total]`.
fn boundaries(tracks: &[u32]) -> Vec<u32> {
    let mut positions = vec![0];
    let mut sum = 0;
    for &track in tracks {
        sum += track;
        positions.push(sum);
    }
    positions
}

/// Moves the inner boundaries so consecutive ones are `m` apart (the ends stay put), with
/// the fewest cells moved in total. The span must hold `m` per track.
///
/// With `y[k] = p[k] - k * m` the constraint is `y` non-decreasing within `0..=W - c * m`:
/// an L1 isotonic regression, solved by pooling adjacent violators with a median per block.
#[expect(
    clippy::indexing_slicing,
    reason = "`last` is `len - 1`, and `k` runs over `1..=last` or the already-built block lengths, which sum to `last - 1`"
)]
fn project(positions: &mut [u32], m: u32) {
    let last = positions.len() - 1;
    if last < 2 {
        return;
    }
    let top = positions[last].saturating_sub(m * u32::try_from(last).unwrap_or(0));
    let mut blocks: Vec<Vec<u32>> = Vec::new();
    for (k, &p) in positions.iter().enumerate().take(last).skip(1) {
        let y = p.saturating_sub(m * u32::try_from(k).unwrap_or(0));
        blocks.push(vec![y]);
        while blocks.len() > 1 {
            let (a, b) = (blocks.len() - 2, blocks.len() - 1);
            if median(&blocks[a]) <= median(&blocks[b]) {
                break;
            }
            let merged = blocks.pop().unwrap_or_default();
            blocks[a].extend(merged);
        }
    }
    let mut k = 1;
    for block in &blocks {
        let value = median(block).min(top);
        for _ in block {
            positions[k] = value + m * u32::try_from(k).unwrap_or(0);
            k += 1;
        }
    }
}

/// The lower median of a block of values.
#[expect(
    clippy::indexing_slicing,
    reason = "a block is never empty, so `(len - 1) / 2` is in range"
)]
fn median(values: &[u32]) -> u32 {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted[(sorted.len() - 1) / 2]
}

/// The cumulative share whose boundary lands on cell `pos` of a span `len` long.
fn ceil_share(pos: u32, len: u32) -> u32 {
    if len == 0 {
        0
    } else {
        (pos * 1000).div_ceil(len)
    }
}

/// Shares from the cumulative values of the inner boundaries, each track at least 1.
fn shares_from_cumulative(cumulative: &[u32], len: usize) -> Vec<u16> {
    let len_u32 = u32::try_from(len).unwrap_or(1);
    let mut shares = Vec::with_capacity(len);
    let mut previous = 0;
    for (k, &value) in cumulative.iter().enumerate() {
        let remaining = len_u32 - u32::try_from(k).unwrap_or(0) - 1;
        let value = value.clamp(previous + 1, 1000 - remaining);
        shares.push(u16::try_from(value - previous).unwrap_or(1));
        previous = value;
    }
    shares.push(u16::try_from(1000 - previous).unwrap_or(1));
    shares
}

/// The terminal to focus when moving from `from` in `dir`: among the rects overlapping
/// its span on the other axis and lying beyond its edge, the nearest; ties go to the one
/// nearest the top (left/right moves) or the left (up/down moves). No wrap-around.
pub fn neighbor(rects: &[Rect], from: usize, dir: Direction) -> Option<usize> {
    let origin = *rects.get(from)?;
    if origin.is_empty() {
        return None;
    }
    let overlap = |a0: u32, a1: u32, b0: u32, b1: u32| a0 < b1 && b0 < a1;
    rects
        .iter()
        .enumerate()
        .filter(|&(i, rect)| i != from && !rect.is_empty())
        .filter_map(|(i, &rect)| {
            // (gap to the edge, tie-break coordinate)
            let key = match dir {
                Direction::Right => (rect.left() >= origin.right()
                    && overlap(rect.top(), rect.bottom(), origin.top(), origin.bottom()))
                .then(|| (rect.left() - origin.right(), rect.top())),
                Direction::Left => (rect.right() <= origin.left()
                    && overlap(rect.top(), rect.bottom(), origin.top(), origin.bottom()))
                .then(|| (origin.left() - rect.right(), rect.top())),
                Direction::Down => (rect.top() >= origin.bottom()
                    && overlap(rect.left(), rect.right(), origin.left(), origin.right()))
                .then(|| (rect.top() - origin.bottom(), rect.left())),
                Direction::Up => (rect.bottom() <= origin.top()
                    && overlap(rect.left(), rect.right(), origin.left(), origin.right()))
                .then(|| (origin.top() - rect.bottom(), rect.left())),
            };
            key.map(|key| (key, i))
        })
        .min()
        .map(|(_, i)| i)
}

/// A movable border of the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Boundary {
    /// Between terminal 1 and the right-hand grid (odd counts).
    Main,
    /// Between grid column `i` and `i + 1`.
    Col(usize),
    /// Between grid row `i` and `i + 1`.
    Row(usize),
}

/// The boundary a resize of terminal `from` in `dir` moves, and the sign of the move
/// (the key's direction: `+1` for right/down, `-1` for left/up). The terminal's edge on
/// the `dir` side when it is a boundary (the terminal grows), else its opposite edge
/// (it shrinks), else `None`.
pub fn boundary_for(n: usize, from: usize, dir: Direction) -> Option<(Boundary, i32)> {
    let shape = shape(n);
    if n < 2 || from >= n {
        return None;
    }
    let sign = match dir {
        Direction::Right | Direction::Down => 1,
        Direction::Left | Direction::Up => -1,
    };
    // Edges of the terminal along each axis: (before, after).
    let (before_x, after_x, before_y, after_y);
    if shape.main && from == 0 {
        before_x = None;
        after_x = Some(Boundary::Main);
        before_y = None;
        after_y = None;
    } else {
        let g = from - usize::from(shape.main);
        let (col, row) = (g % shape.cols, g / shape.cols);
        before_x = if col > 0 {
            Some(Boundary::Col(col - 1))
        } else if shape.main {
            Some(Boundary::Main)
        } else {
            None
        };
        after_x = (col + 1 < shape.cols).then_some(Boundary::Col(col));
        before_y = (row > 0).then(|| Boundary::Row(row - 1));
        after_y = (row + 1 < shape.rows).then_some(Boundary::Row(row));
    }
    let (toward, away) = match dir {
        Direction::Right => (after_x, before_x),
        Direction::Left => (before_x, after_x),
        Direction::Down => (after_y, before_y),
        Direction::Up => (before_y, after_y),
    };
    toward.or(away).map(|boundary| (boundary, sign))
}

/// `(horizontal, span length, boundary position)` of a boundary, in cells from the start
/// of its span; `None` when the layout of `n` has no such boundary.
#[expect(
    clippy::indexing_slicing,
    reason = "the match guards check the boundary index against the shape, and `axis_tracks` has one track per column (plus main) or row"
)]
fn boundary_span(
    area: Rect,
    n: usize,
    ratios: &Ratios,
    boundary: Boundary,
) -> Option<(bool, u32, u32)> {
    let shape = shape(n);
    match boundary {
        Boundary::Main if shape.main => {
            let tracks = axis_tracks(area, n, ratios, true);
            Some((true, u32::from(area.width), tracks[0]))
        }
        Boundary::Col(i) if i + 1 < shape.cols => {
            let tracks = axis_tracks(area, n, ratios, true);
            let off = usize::from(shape.main);
            let len = tracks[off..].iter().sum();
            let pos = tracks[off..=off + i].iter().sum();
            Some((true, len, pos))
        }
        Boundary::Row(i) if i + 1 < shape.rows => {
            let tracks = axis_tracks(area, n, ratios, false);
            Some((false, u32::from(area.height), tracks[..=i].iter().sum()))
        }
        _ => None,
    }
}

/// `ratios` with `boundary` at cell `pos` of its span `len` cells long; only the two
/// tracks on either side change.
fn set_boundary(ratios: &Ratios, boundary: Boundary, len: u32, pos: u32) -> Ratios {
    let mut out = ratios.clone();
    match boundary {
        Boundary::Main => {
            let main = ((pos * 1000 + len / 2) / len).clamp(1, 999);
            out.main = Some(u16::try_from(main).unwrap_or(999));
        }
        Boundary::Col(i) => set_pair(&mut out.cols, i, pos, len),
        Boundary::Row(i) => set_pair(&mut out.rows, i, pos, len),
    }
    out
}

#[expect(
    clippy::indexing_slicing,
    reason = "callers pass a boundary index checked by `boundary_span`, so `i + 1` is in range"
)]
fn set_pair(shares: &mut [u16], i: usize, pos: u32, len: u32) {
    let before: u32 = shares[..i].iter().map(|&s| u32::from(s)).sum();
    let pair = u32::from(shares[i]) + u32::from(shares[i + 1]);
    let first = ceil_share(pos, len)
        .saturating_sub(before)
        .clamp(1, pair - 1);
    shares[i] = u16::try_from(first).unwrap_or(1);
    shares[i + 1] = u16::try_from(pair - first).unwrap_or(1);
}

/// Moves `boundary` toward cell `target`, stopping at the nearest position where no
/// terminal on the axis is below the minimum, unless it already was (then it just must
/// not get smaller).
fn move_boundary(
    n: usize,
    ratios: &Ratios,
    boundary: Boundary,
    area: Rect,
    (min_cols, min_rows): (u16, u16),
    target: i64,
) -> Ratios {
    let base = effective(ratios, n);
    let Some((horizontal, len, pos)) = boundary_span(area, n, &base, boundary) else {
        return base;
    };
    if len == 0 {
        return base;
    }
    let minimum = u32::from(if horizontal { min_cols } else { min_rows }) + 2;
    let old = axis_tracks(area, n, &base, horizontal);
    let current = i64::from(pos);
    let mut q = target.clamp(0, i64::from(len));
    let toward = if q > current { -1 } else { 1 };
    while q != current {
        let candidate = set_boundary(&base, boundary, len, u32::try_from(q).unwrap_or(0));
        let tracks = axis_tracks(area, n, &candidate, horizontal);
        if tracks
            .iter()
            .zip(&old)
            .all(|(&new, &before)| new >= minimum.min(before))
        {
            return candidate;
        }
        q += toward;
    }
    base
}

/// Moves `boundary` by `delta_permille` of its span (positive = right or down), by at
/// least one cell, clamped so no terminal goes below the minimum inner size (a terminal
/// already below it never gets smaller). Returns the ratios unchanged when no move is
/// possible. `ratios` should be the effective ones (see [`fit`]).
///
/// Per-mille shares cannot address every cell of a span over 1000 cells, so a boundary
/// lands exactly on the requested cell only for spans up to 1000 cells.
pub fn resize(
    n: usize,
    ratios: &Ratios,
    boundary: Boundary,
    delta_permille: i32,
    area: Rect,
    min_cols: u16,
    min_rows: u16,
) -> Ratios {
    let base = effective(ratios, n);
    let Some((_, len, pos)) = boundary_span(area, n, &base, boundary) else {
        return base;
    };
    if delta_permille == 0 {
        return base;
    }
    let mut step = i64::from(delta_permille) * i64::from(len) / 1000;
    if step == 0 {
        step = i64::from(delta_permille.signum());
    }
    move_boundary(
        n,
        &base,
        boundary,
        area,
        (min_cols, min_rows),
        i64::from(pos) + step,
    )
}

/// The boundary whose two border columns (or rows) contain the point, for mouse drags.
/// `Main` spans the full height, `Col` boundaries the grid's height, `Row` boundaries
/// the grid's width only; where a `Col` and a `Row` cross, `Col` wins.
#[expect(
    clippy::indexing_slicing,
    reason = "`layout` returns one rect per terminal, and the indexes follow the shape: `rects[1]` exists with a main split, `off + i` and `off + (i + 1) * cols` are grid terminals"
)]
pub fn hit_boundary(area: Rect, n: usize, ratios: &Ratios, x: u16, y: u16) -> Option<Boundary> {
    if n < 2 {
        return None;
    }
    let (px, py) = (u32::from(x), u32::from(y));
    if px < area.left() || px >= area.right() || py < area.top() || py >= area.bottom() {
        return None;
    }
    let rects = layout(area, n, ratios);
    let shape = shape(n);
    let off = usize::from(shape.main);
    let near = |edge: u32, v: u32| v == edge || v + 1 == edge;

    if shape.main && near(rects[1].left(), px) {
        return Some(Boundary::Main);
    }
    for i in 0..shape.cols - 1 {
        if near(rects[off + i].right(), px) {
            return Some(Boundary::Col(i));
        }
    }
    if px >= rects[off].left() {
        for i in 0..shape.rows - 1 {
            if near(rects[off + (i + 1) * shape.cols].top(), py) {
                return Some(Boundary::Row(i));
            }
        }
    }
    None
}

/// `ratios` with `boundary` put at cell `pos` of its span (from the span's start), with
/// the clamping of [`resize`]. Used to follow the pointer while dragging. Exact landing on
/// `pos` holds for spans up to 1000 cells (per-mille cannot address more).
pub fn ratio_at(
    area: Rect,
    n: usize,
    ratios: &Ratios,
    boundary: Boundary,
    pos: u16,
    min_cols: u16,
    min_rows: u16,
) -> Ratios {
    move_boundary(
        n,
        ratios,
        boundary,
        area,
        (min_cols, min_rows),
        i64::from(pos),
    )
}
