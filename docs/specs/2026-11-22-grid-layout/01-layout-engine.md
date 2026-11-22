---
type: foundation
status: draft
---

# 01 — Layout engine

## Scope

The pure geometry of the grid in `deck-core::layout`, used by the daemon to size PTYs and by the client to draw, focus, resize and hit-test. Nothing on screen yet; sub-specs 02 and 03 use it. Owning crate: `deck-core`.

## Requirements

1. **Types**: `Rect { x, y, width, height }` (`u16`, cell units, `Copy`, serde); `Direction { Left, Right, Up, Down }`; `Ratios { main: Option<u16>, cols: Vec<u16>, rows: Vec<u16> }` (per-mille, serde); `Shape { main: bool, cols: usize, rows: usize }`.
2. **`shape(n) -> Shape`** for `n ≥ 1` as in [architecture.md#layout](../../architecture.md#layout): `n = 1` → no main, 1×1; even `n` → `cols × rows = n`, `cols ≥ rows`, `cols − rows` minimal; odd `n > 1` → main plus a grid of `n − 1` with `rows ≥ cols`, `rows − cols` minimal.
3. **`Ratios::default_for(n)`**: `main = Some(500)` for odd `n > 1`, else `None`; `cols` and `rows` equal shares that sum to 1000 (remainder on the last track). **`Ratios::adapt(&old, old_n, new_n) -> Ratios`**: the count-change rule of [architecture.md#layout](../../architecture.md#layout) (full-width columns and rows, equal share for an added track, proportional give-back for a removed one); it always returns ratios valid for `new_n`. Integer rounding: an added track gets `1000 / count` (floored), existing shares are scaled with `floor(share × rest / 1000)`, and any remainder goes to the last track of the list, the same rule as `layout`. **`Ratios::is_valid_for(n)`**: lengths match `shape(n)`, every share ≥ 1, each list sums to 1000, `main` in 1..=999.
4. **`layout(area, n, &ratios) -> Vec<Rect>`**: one rect per terminal in order. All arithmetic is integer, in `u32` (never `f32`, never `u16` products, which overflow at 65535 × 1000). The main split gives terminal 1 the left `(width × main + 500) / 1000` columns at full height (rounding half up); the grid fills the rest left to right, then top to bottom. Each track but the last is `len × share / 1000` (integer division), and the last track takes the remainder, so the rects tile the area exactly with no gaps or overlaps. With invalid ratios, `layout` uses the defaults. A rect may be 0 wide or high when the area is too small; callers clamp inner sizes to at least 1×1 ([architecture.md#layout](../../architecture.md#layout)).
5. **`inner(rect) -> (cols, rows)`**: `(width − 2, height − 2)`, saturating, at least `(1, 1)`.
6. **`fits(area, n, &ratios, min_cols, min_rows) -> bool`**: every inner size is at least the minimum.
7. **`neighbor(rects, from, dir) -> Option<usize>`**: the rule in the overview's decisions (span overlap, nearest edge, tie to top or left, no wrap).
8. **Boundaries**: `Boundary { Main, Col(usize), Row(usize) }`. `Col(i)` is between grid column `i` and `i + 1`, and `Row(i)` between grid row `i` and `i + 1`; in an odd layout they index the right-hand grid only, and the left edge of its first column is `Main`. `boundary_for(n, from, dir) -> Option<(Boundary, i32)>` returns the boundary a resize of terminal `from` in `dir` moves and the sign of the move, which is always the key's direction (`+1` for `Right`/`Down`, `-1` for `Left`/`Up`):
   - the terminal's edge on the `dir` side, when that edge is a boundary (the terminal grows);
   - else its opposite edge on the same axis, when that is a boundary (the boundary still moves in `dir`, so the terminal shrinks);
   - else `None`: `n = 1`, `Up`/`Down` in a single-row grid, and `Up`/`Down` on terminal 1 of an odd layout (which spans the full height).

   So `n = 2` (`Col(0)` only): 1 `Right` → `(Col(0), +1)`, 1 `Left` → `(Col(0), -1)`, 2 `Right` → `(Col(0), +1)`, 2 `Left` → `(Col(0), -1)`, `Up`/`Down` → `None`. `n = 4`: terminal 1 `Right`/`Down` → `Col(0)`/`Row(0)` growing it, terminal 4 `Right`/`Down` → `Col(0)`/`Row(0)` shrinking it. Odd `n`: terminal 1 `Right` and `Left` → `Main`; a terminal in the first grid column `Left` → `Main`; a terminal in the last grid column `Right` → its left edge (`Col(c − 2)`, or `Main` when the grid has one column).
9. **`resize(n, &ratios, boundary, delta_permille, area, min_cols, min_rows) -> Ratios`**: moves the boundary by `delta` (positive = right or down), changing only the two tracks (or `main` and the grid) on either side of it. A step that would move the boundary by 0 cells is enlarged to move it by exactly 1 cell. The result is clamped: a terminal at or above the minimum inner size never goes below it, and a terminal already below it (after the window shrank) never gets smaller; moves that grow a cramped terminal are allowed. Returns the ratios unchanged when no move is possible.
10. **`hit_boundary(area, n, &ratios, x, y) -> Option<Boundary>`**: the boundary whose two border columns (or rows) contain the point, for mouse drags. `Main` spans the full height; `Col` boundaries span their grid's height and `Row` boundaries their grid's width only (the right half in an odd layout). Where a `Col` and a `Row` boundary cross, `Col` wins. **`ratio_at(area, n, &ratios, boundary, pos) -> Ratios`** puts that boundary at cell `pos` (measured from the start of the boundary's span): it sets the cumulative share to `ceil(pos × 1000 / len)`, so that integer division in `layout` lands the boundary exactly on `pos` for spans up to 1000 cells, changing only the two tracks on either side, with the same clamping as `resize`.

## Acceptance criteria

- [ ] `shape` matches this table: 1 → 1×1; 2 → 2×1; 3 → main + 1×2; 4 → 2×2; 5 → main + 2×2; 6 → 3×2; 7 → main + 2×3; 8 → 4×2; 9 → main + 2×4; 10 → 5×2; 12 → 4×3.
- [ ] In a 100×40 area: 3 terminals give `(0,0,50,40)`, `(50,0,50,20)`, `(50,20,50,20)`; 5 give `(0,0,50,40)`, `(50,0,25,20)`, `(75,0,25,20)`, `(50,20,25,20)`, `(75,20,25,20)`; 4 give four 50×20 quarters.
- [ ] Golden rounding cases: 101×41 with 3 terminals gives `(0,0,51,41)`, `(51,0,50,20)`, `(51,20,50,21)`; 7×3 with 5 gives `(0,0,4,3)`, `(4,0,1,1)`, `(5,0,2,1)`, `(4,1,1,2)`, `(5,1,2,2)`; a 1-column area with 3 terminals gives terminal 1 the column. The golden table is a `pub` test fixture in `deck-core` so later daemon and TUI tests can assert against the same values.
- [ ] For every `n` in 1..=16 and every area from 1×1 to 300×100 (sampled), the rects tile the area exactly (property test written as a loop; no new crate).
- [ ] `neighbor` in the 5-terminal layout: from 1 `Right` → 2; from 4 `Left` → 1; from 2 `Down` → 4; from 3 `Up` → none; from 1 `Up` → none.
- [ ] `boundary_for` in the 3-terminal layout: 1 `Right` → `(Main, +1)` (grows 1); 1 `Left` → `(Main, -1)` (shrinks 1); 1 `Up`/`Down` → `None`; 2 `Left` → `(Main, -1)` (grows 2); 2 `Right` → `(Main, +1)` (shrinks 2); 2 `Down` → `(Row(0), +1)`; 2 `Up` → `(Row(0), -1)` (shrinks 2); 3 `Up` → `(Row(0), -1)` (grows 3); 3 `Down` → `(Row(0), +1)` (shrinks 3). In the 5-terminal layout: 2 `Left` → `(Main, -1)`; 3 `Right` → `(Col(0), +1)` (shrinks 3); 3 `Left` → `(Col(0), -1)`. The `n = 2` and `n = 4` cases of requirement 8 hold.
- [ ] `resize` with +50‰ on `Main` in 100×40 grows terminal 1 from 50 to 55 columns; in a 15-column area the step still moves the boundary by 1 cell; resizing stops at the 10×3 minimum; with an area already too small, a move that grows the cramped terminal is applied and one that shrinks it is refused.
- [ ] `ratio_at` on `Main` at every `pos` of a 100-column and a 997-column area puts the boundary exactly on `pos` (within the clamp).
- [ ] `hit_boundary` finds `Main` on both border columns between terminal 1 and the right half at any height, `Col(0)` at the crossing with `Row(0)` in a 4-terminal layout, `None` for a point on the left half's row line in an odd layout (rows span the right half only), and `None` inside a terminal.
- [ ] `Ratios::adapt`: 2 → 3 with cols `[600, 400]` gives `main = 600` and right rows `[500, 500]`; 3 → 4 with `main = 600`, rows `[300, 700]` gives cols `[600, 400]`, rows `[300, 700]`; 4 → 5 with cols `[600, 400]` gives full columns `[400, 266, 334]` (main 400, right cols `[266, 334]` rescaled to sum 1000); 5 → 4 gives back two columns in the old proportion; 1 → 2 and 2 → 1 work; for every `old_n`, `new_n` in 1..=16 (`|old_n − new_n| = 1`) and sampled valid ratios, the result is valid for `new_n`.
- [ ] `Ratios::is_valid_for` rejects wrong lengths, zero shares and sums other than 1000; `layout` falls back to defaults for them.

## Tests

- Unit tests in `crates/deck-core/src/layout.rs` (or `layout/tests.rs`) for every criterion above.
