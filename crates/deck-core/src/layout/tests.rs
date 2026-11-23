use super::golden::{GOLDEN, SHAPES};
use super::*;

const AREA: Rect = Rect::new(0, 0, 100, 40);

fn rects(area: Rect, n: usize) -> Vec<Rect> {
    layout(area, n, &Ratios::default_for(n))
}

fn ratios(main: Option<u16>, cols: &[u16], rows: &[u16]) -> Ratios {
    Ratios {
        main,
        cols: cols.to_vec(),
        rows: rows.to_vec(),
    }
}

/// A small deterministic generator, so no crate is needed.
fn next(seed: &mut u64) -> u64 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *seed >> 33
}

fn random_shares(len: usize, seed: &mut u64) -> Vec<u16> {
    // Distinct cut points in 1..1000 give shares of at least 1.
    let mut cuts = Vec::new();
    while cuts.len() + 1 < len {
        // Squared, so small cuts (shares of 1) are common.
        let cut = 1 + (next(seed) % 1000 * (next(seed) % 1000) / 1000).min(998) as u16;
        if !cuts.contains(&cut) {
            cuts.push(cut);
        }
    }
    cuts.sort_unstable();
    let mut shares = Vec::new();
    let mut previous = 0;
    for cut in cuts {
        shares.push(cut - previous);
        previous = cut;
    }
    shares.push(1000 - previous);
    shares
}

fn random_ratios(n: usize, seed: &mut u64) -> Ratios {
    let shape = shape(n);
    Ratios {
        main: shape.main.then(|| 1 + (next(seed) % 999) as u16),
        cols: random_shares(shape.cols, seed),
        rows: random_shares(shape.rows, seed),
    }
}

// --- panel area (DECK-08) ---

#[test]
fn the_panel_area_leaves_the_sidebar_and_keeps_every_row() {
    assert_eq!(panel_area((100, 24), 28, true), (71, 24));
    assert_eq!(panel_area((100, 24), 28, false), (100, 24));
    assert_eq!(panel_area((100, 0), 28, false), (100, 0));
}

#[test]
fn a_narrow_window_hides_the_sidebar() {
    // 28 + separator + 20 columns is the narrowest that keeps it.
    assert_eq!(panel_area((49, 24), 28, true), (20, 24));
    assert_eq!(panel_area((48, 24), 28, true), (48, 24));
    assert_eq!(panel_area((100, 24), 40, true), (59, 24));
}

#[test]
fn sidebar_cols_takes_the_width() {
    assert_eq!(sidebar_cols(100, 40, true), 41);
    assert_eq!(sidebar_cols(100, 40, false), 0);
    assert_eq!(sidebar_cols(60, 40, true), 0);
}

#[test]
fn the_sidebar_width_is_clamped_to_the_limits_and_the_panel_area() {
    // (cols, wanted, expected)
    let table = [
        (100, 28, 28),
        (100, 19, 20),
        (100, 0, 20),
        (100, 20, 20),
        (100, 60, 60),
        (100, 61, 60),
        (100, 500, 60),
        // The panel area keeps its 20 columns: cols - 21 at most.
        (60, 60, 39),
        (60, 39, 39),
        (60, 30, 30),
        (41, 60, 20),
        (41, 28, 20),
        // Under 41 columns the lower bound wins.
        (40, 60, 20),
        (40, 28, 20),
        (20, 28, 20),
        (0, 28, 20),
    ];
    for (cols, wanted, expected) in table {
        assert_eq!(
            clamp_sidebar_width(cols, wanted),
            expected,
            "{cols}, {wanted}"
        );
    }
}

// --- shape, layout, golden ---

#[test]
fn shape_matches_the_table() {
    for &(n, expected) in SHAPES {
        assert_eq!(shape(n), expected, "n = {n}");
    }
}

#[test]
fn golden_layouts_match() {
    for case in GOLDEN {
        assert_eq!(
            rects(case.area, case.n),
            case.rects,
            "{:?} with {}",
            case.area,
            case.n
        );
    }
}

#[test]
fn rects_tile_the_area_exactly() {
    let mut widths: Vec<u16> = (1..=40).collect();
    widths.extend((41..=300).step_by(13));
    widths.push(300);
    let mut heights: Vec<u16> = (1..=20).collect();
    heights.extend((21..=100).step_by(7));
    heights.push(100);
    for n in 1..=16 {
        for &w in &widths {
            for &h in &heights {
                for origin in [(0, 0), (3, 2)] {
                    let area = Rect::new(origin.0, origin.1, w, h);
                    let mut seen = vec![0u8; usize::from(w) * usize::from(h)];
                    for rect in rects(area, n) {
                        assert!(rect.x >= area.x && rect.right() <= area.right());
                        assert!(rect.y >= area.y && rect.bottom() <= area.bottom());
                        for y in rect.y..rect.y + rect.height {
                            for x in rect.x..rect.x + rect.width {
                                let i = usize::from(y - area.y) * usize::from(w)
                                    + usize::from(x - area.x);
                                seen[i] += 1;
                            }
                        }
                    }
                    assert!(seen.iter().all(|&c| c == 1), "n={n} area={area:?}");
                }
            }
        }
    }
}

#[test]
fn large_areas_do_not_overflow() {
    let area = Rect::new(0, 0, u16::MAX, u16::MAX);
    let rects = rects(area, 5);
    assert_eq!(rects[0].width, 32768);
    assert_eq!(u32::from(rects[1].width) + u32::from(rects[2].width), 32767);
}

#[test]
fn inner_depends_on_the_count() {
    let rect = Rect::new(0, 0, 100, 23);
    assert_eq!(inner(rect, 1), (100, 23));
    assert_eq!(inner(rect, 2), (98, 21));
    let one = Rect::new(0, 0, 1, 1);
    assert_eq!(inner(one, 1), (1, 1));
    assert_eq!(inner(one, 2), (1, 1));
}

#[test]
fn validity_and_fallback() {
    assert!(Ratios::default_for(5).is_valid_for(5));
    assert!(!Ratios::default_for(5).is_valid_for(4));
    assert!(!ratios(None, &[500, 500], &[1000]).is_valid_for(3));
    assert!(!ratios(None, &[0, 1000], &[1000]).is_valid_for(2));
    assert!(!ratios(None, &[500, 400], &[1000]).is_valid_for(2));
    assert!(!ratios(Some(0), &[1000], &[500, 500]).is_valid_for(3));
    assert!(!ratios(Some(1000), &[1000], &[500, 500]).is_valid_for(3));
    assert!(!ratios(Some(500), &[500, 500], &[1000]).is_valid_for(2));
    let bad = ratios(None, &[0, 1000], &[1000]);
    assert_eq!(layout(AREA, 2, &bad), rects(AREA, 2));
    assert_eq!(Ratios::default_for(7).cols, vec![500, 500]);
    assert_eq!(Ratios::default_for(9).rows, vec![250; 4]);
    assert_eq!(Ratios::default_for(3).cols, vec![1000]);
    assert_eq!(Ratios::default_for(6).cols, vec![333, 333, 334]);
}

#[test]
fn fits_checks_every_inner_size() {
    let defaults = Ratios::default_for(4);
    assert!(fits(AREA, 4, &defaults, 10, 3));
    assert!(!fits(Rect::new(0, 0, 23, 40), 4, &defaults, 10, 3));
    assert!(fits(Rect::new(0, 0, 24, 10), 4, &defaults, 10, 3));
}

// --- neighbor ---

#[test]
fn neighbor_in_five_terminals() {
    let r = rects(AREA, 5);
    assert_eq!(neighbor(&r, 0, Direction::Right), Some(1));
    assert_eq!(neighbor(&r, 3, Direction::Left), Some(0));
    assert_eq!(neighbor(&r, 1, Direction::Down), Some(3));
    assert_eq!(neighbor(&r, 2, Direction::Up), None);
    assert_eq!(neighbor(&r, 0, Direction::Up), None);
    assert_eq!(neighbor(&r, 2, Direction::Right), None);
    assert_eq!(neighbor(&r, 4, Direction::Up), Some(2));
    assert_eq!(neighbor(&r, 9, Direction::Up), None);
}

// --- boundary_for ---

#[test]
fn boundary_for_three_terminals() {
    use Boundary::*;
    use Direction::*;
    let cases = [
        (0, Right, Some((Main, 1))),
        (0, Left, Some((Main, -1))),
        (0, Up, None),
        (0, Down, None),
        (1, Left, Some((Main, -1))),
        (1, Right, Some((Main, 1))),
        (1, Down, Some((Row(0), 1))),
        (1, Up, Some((Row(0), -1))),
        (2, Up, Some((Row(0), -1))),
        (2, Down, Some((Row(0), 1))),
    ];
    for (from, dir, expected) in cases {
        assert_eq!(boundary_for(3, from, dir), expected, "{from} {dir:?}");
    }
}

#[test]
fn boundary_for_other_counts() {
    use Boundary::*;
    use Direction::*;
    assert_eq!(boundary_for(5, 1, Left), Some((Main, -1)));
    assert_eq!(boundary_for(5, 2, Right), Some((Col(0), 1)));
    assert_eq!(boundary_for(5, 2, Left), Some((Col(0), -1)));
    assert_eq!(boundary_for(1, 0, Right), None);
    assert_eq!(boundary_for(2, 0, Right), Some((Col(0), 1)));
    assert_eq!(boundary_for(2, 0, Left), Some((Col(0), -1)));
    assert_eq!(boundary_for(2, 1, Right), Some((Col(0), 1)));
    assert_eq!(boundary_for(2, 1, Left), Some((Col(0), -1)));
    assert_eq!(boundary_for(2, 0, Up), None);
    assert_eq!(boundary_for(2, 1, Down), None);
    assert_eq!(boundary_for(4, 0, Right), Some((Col(0), 1)));
    assert_eq!(boundary_for(4, 0, Down), Some((Row(0), 1)));
    assert_eq!(boundary_for(4, 3, Right), Some((Col(0), 1)));
    assert_eq!(boundary_for(4, 3, Down), Some((Row(0), 1)));
    assert_eq!(boundary_for(4, 3, Left), Some((Col(0), -1)));
    assert_eq!(boundary_for(4, 3, Up), Some((Row(0), -1)));
}

// --- resize ---

#[test]
fn resize_main_by_a_step() {
    let r = Ratios::default_for(3);
    let out = resize(3, &r, Boundary::Main, 50, AREA, 1, 1);
    assert_eq!(layout(AREA, 3, &out)[0].width, 55);
    let out = resize(3, &r, Boundary::Main, -50, AREA, 1, 1);
    assert_eq!(layout(AREA, 3, &out)[0].width, 45);
}

#[test]
fn a_step_under_one_cell_moves_one_cell() {
    let area = Rect::new(0, 0, 15, 40);
    let r = Ratios::default_for(3);
    let before = layout(area, 3, &r)[0].width;
    let out = resize(3, &r, Boundary::Main, 50, area, 1, 1);
    assert_eq!(layout(area, 3, &out)[0].width, before + 1);
    let out = resize(3, &r, Boundary::Main, -50, area, 1, 1);
    assert_eq!(layout(area, 3, &out)[0].width, before - 1);
}

#[test]
fn resize_stops_at_the_minimum() {
    let mut r = Ratios::default_for(3);
    for _ in 0..40 {
        r = resize(3, &r, Boundary::Main, 50, AREA, 10, 3);
    }
    let l = layout(AREA, 3, &r);
    assert_eq!((l[0].width, l[1].width), (88, 12));
    assert_eq!(resize(3, &r, Boundary::Main, 50, AREA, 10, 3), r);
    // Rows stop at 5 outer rows (3 inner).
    let mut r = Ratios::default_for(3);
    for _ in 0..40 {
        r = resize(3, &r, Boundary::Row(0), 50, AREA, 10, 3);
    }
    let l = layout(AREA, 3, &r);
    assert_eq!((l[1].height, l[2].height), (35, 5));
    // Resizing only touches the two tracks around the boundary.
    let mut r = Ratios::default_for(6);
    r = resize(6, &r, Boundary::Col(0), 50, AREA, 10, 3);
    assert_eq!(r.cols[2], 334);
    assert_eq!(r.rows, vec![500, 500]);
}

#[test]
fn a_cramped_terminal_may_grow_but_never_shrink() {
    let area = Rect::new(0, 0, 20, 40);
    let r = ratios(None, &[800, 200], &[1000]);
    // Left 16, right 4 (below 12): growing the right one is allowed, shrinking it is not.
    assert_eq!(layout(area, 2, &r)[1].width, 4);
    let grown = resize(2, &r, Boundary::Col(0), -50, area, 10, 3);
    assert_eq!(layout(area, 2, &grown)[1].width, 5);
    assert_eq!(resize(2, &r, Boundary::Col(0), 50, area, 10, 3), r);
    // Both below the minimum: nothing moves.
    let tiny = Rect::new(0, 0, 18, 40);
    let even = Ratios::default_for(2);
    assert_eq!(resize(2, &even, Boundary::Col(0), 50, tiny, 10, 3), even);
    assert_eq!(resize(2, &even, Boundary::Col(0), -50, tiny, 10, 3), even);
}

#[test]
fn resize_without_a_boundary_changes_nothing() {
    let r = Ratios::default_for(4);
    assert_eq!(resize(4, &r, Boundary::Main, 50, AREA, 10, 3), r);
    assert_eq!(resize(4, &r, Boundary::Col(1), 50, AREA, 10, 3), r);
    assert_eq!(
        resize(1, &Ratios::default_for(1), Boundary::Col(0), 50, AREA, 1, 1).cols,
        vec![1000]
    );
}

// --- ratio_at / hit_boundary ---

#[test]
fn ratio_at_lands_on_the_cell() {
    for width in [100u16, 997] {
        let area = Rect::new(0, 0, width, 40);
        let r = Ratios::default_for(3);
        for pos in 3..=width - 3 {
            let out = ratio_at(area, 3, &r, Boundary::Main, pos, 1, 1);
            assert_eq!(layout(area, 3, &out)[0].width, pos, "width {width}");
        }
    }
    let area = Rect::new(0, 0, 997, 40);
    let r = Ratios::default_for(6);
    let l = layout(area, 6, &r);
    let (lo, hi) = (l[1].x + 3, l[2].x + l[2].width - 3);
    for pos in lo..=hi {
        let out = ratio_at(area, 6, &r, Boundary::Col(1), pos, 1, 1);
        let l = layout(area, 6, &out);
        assert_eq!(l[1].x + l[1].width, pos);
        assert_eq!(l[0], layout(area, 6, &r)[0]);
        assert!(out.is_valid_for(6));
    }
    // Past the clamp the boundary stops at it.
    let out = ratio_at(area, 6, &r, Boundary::Col(1), 996, 1, 1);
    assert_eq!(
        layout(area, 6, &out)[1].x + layout(area, 6, &out)[1].width,
        hi
    );
}

#[test]
fn hit_boundary_cases() {
    let r3 = Ratios::default_for(3);
    for y in [0, 17, 39] {
        assert_eq!(hit_boundary(AREA, 3, &r3, 49, y), Some(Boundary::Main));
        assert_eq!(hit_boundary(AREA, 3, &r3, 50, y), Some(Boundary::Main));
    }
    assert_eq!(hit_boundary(AREA, 3, &r3, 48, 10), None);
    assert_eq!(hit_boundary(AREA, 3, &r3, 51, 10), None);
    // The row line spans the right half only.
    assert_eq!(hit_boundary(AREA, 3, &r3, 10, 20), None);
    assert_eq!(hit_boundary(AREA, 3, &r3, 70, 20), Some(Boundary::Row(0)));
    assert_eq!(hit_boundary(AREA, 3, &r3, 70, 19), Some(Boundary::Row(0)));
    assert_eq!(hit_boundary(AREA, 3, &r3, 70, 10), None);
    let r4 = Ratios::default_for(4);
    assert_eq!(hit_boundary(AREA, 4, &r4, 50, 20), Some(Boundary::Col(0)));
    assert_eq!(hit_boundary(AREA, 4, &r4, 10, 20), Some(Boundary::Row(0)));
    assert_eq!(hit_boundary(AREA, 4, &r4, 10, 10), None);
    assert_eq!(hit_boundary(AREA, 4, &r4, 100, 10), None);
    assert_eq!(hit_boundary(AREA, 1, &Ratios::default_for(1), 50, 20), None);
}

// --- adapt ---

#[test]
fn adapt_keeps_the_defaults() {
    for a in 1..=16 {
        for b in 1..=16 {
            assert_eq!(
                Ratios::adapt(&Ratios::default_for(a), a, b),
                Ratios::default_for(b),
                "{a} -> {b}"
            );
        }
    }
}

#[test]
fn adapt_examples() {
    let out = Ratios::adapt(&ratios(None, &[600, 400], &[1000]), 2, 3);
    assert_eq!(out, ratios(Some(600), &[1000], &[500, 500]));

    let out = Ratios::adapt(&ratios(Some(600), &[1000], &[300, 700]), 3, 4);
    assert_eq!(out, ratios(None, &[600, 400], &[300, 700]));

    let out = Ratios::adapt(&ratios(None, &[600, 400], &[500, 500]), 4, 5);
    assert_eq!(out, ratios(Some(571), &[442, 558], &[500, 500]));
    assert_eq!(full_columns(&out, 5), vec![571, 189, 240]);
    let back = Ratios::adapt(&out, 5, 4);
    assert_eq!(back, ratios(None, &[601, 399], &[500, 500]));

    assert_eq!(
        Ratios::adapt(&Ratios::default_for(1), 1, 2),
        Ratios::default_for(2)
    );
    assert_eq!(
        Ratios::adapt(&Ratios::default_for(2), 2, 1).cols,
        vec![1000]
    );
}

#[test]
fn adapt_always_gives_valid_ratios() {
    let mut seed = 7;
    for a in 1..=16usize {
        for b in [a.saturating_sub(1).max(1), a + 1] {
            for _ in 0..200 {
                let old = random_ratios(a, &mut seed);
                assert!(old.is_valid_for(a));
                let out = Ratios::adapt(&old, a, b);
                assert!(out.is_valid_for(b), "{a} -> {b}: {old:?} gave {out:?}");
            }
        }
    }
    // Extreme shares.
    let extreme = ratios(Some(999), &[1, 999], &[1, 999]);
    assert!(Ratios::adapt(&extreme, 5, 6).is_valid_for(6));
    assert!(Ratios::adapt(&extreme, 5, 3).is_valid_for(3));
}

// --- fit ---

#[test]
fn fit_keeps_ratios_that_fit() {
    let r = ratios(None, &[600, 400], &[500, 500]);
    assert_eq!(fit(AREA, 4, &r, 10, 3), r);
    let one = Ratios::default_for(1);
    assert_eq!(fit(AREA, 1, &one, 10, 3), one);
}

#[test]
fn fit_moves_boundaries_the_fewest_cells() {
    let area = Rect::new(0, 0, 40, 11);
    let stored = ratios(None, &[900, 100], &[500, 500]);
    let out = fit(area, 4, &stored, 10, 3);
    assert_eq!(out, ratios(None, &[700, 300], &[500, 500]));
    assert_eq!(layout(area, 4, &out)[1].width, 12);
    assert!(fits(area, 4, &out, 10, 3));
    // The stored ratios are a separate value and apply again when the window grows.
    assert_eq!(stored.cols, vec![900, 100]);
    assert_eq!(fit(Rect::new(0, 0, 200, 40), 4, &stored, 10, 3), stored);
}

#[test]
fn fit_with_a_main_split() {
    let area = Rect::new(0, 0, 40, 40);
    let stored = ratios(Some(900), &[500, 500], &[500, 500]);
    let out = fit(area, 5, &stored, 10, 3);
    assert!(out.is_valid_for(5));
    assert!(fits(area, 5, &out, 10, 3), "{out:?}");
    let narrow = ratios(Some(100), &[500, 500], &[500, 500]);
    assert!(fits(area, 5, &fit(area, 5, &narrow, 10, 3), 10, 3));
}

#[test]
fn an_axis_too_short_uses_its_defaults() {
    let area = Rect::new(0, 0, 40, 7);
    let stored = ratios(None, &[500, 500], &[900, 100]);
    let out = fit(area, 4, &stored, 10, 3);
    assert_eq!(out.rows, vec![500, 500]);
    assert_eq!(out.cols, vec![500, 500]);
    let narrow = Rect::new(0, 0, 20, 40);
    let out = fit(narrow, 4, &ratios(None, &[900, 100], &[800, 200]), 10, 3);
    assert_eq!(out.cols, vec![500, 500]);
    assert_eq!(out.rows, vec![800, 200]);
}

#[test]
fn fit_is_always_valid() {
    let mut seed = 11;
    for n in 2..=12 {
        for (w, h) in [(30, 12), (60, 20), (120, 50), (200, 30)] {
            let area = Rect::new(0, 0, w, h);
            for _ in 0..30 {
                let stored = random_ratios(n, &mut seed);
                let out = fit(area, n, &stored, 10, 3);
                assert!(out.is_valid_for(n), "{n} {area:?} {stored:?}");
            }
        }
    }
}

#[test]
fn project_moves_the_fewest_cells() {
    let m = 3u32;
    for tracks in 3..=4u32 {
        for width in tracks * m..=tracks * m + 5 {
            // Every non-decreasing start, and the brute-force minimum over valid layouts.
            let inner = tracks as usize - 1;
            let mut starts = vec![vec![]];
            for _ in 0..inner {
                starts = starts
                    .into_iter()
                    .flat_map(|v| {
                        let low = v.last().copied().unwrap_or(0);
                        (low..=width).map(move |p| [v.clone(), vec![p]].concat())
                    })
                    .collect();
            }
            let valid: Vec<&Vec<u32>> = starts
                .iter()
                .filter(|v| {
                    let mut previous = 0;
                    v.iter().chain([&width]).all(|&p| {
                        let ok = p >= previous + m;
                        previous = p;
                        ok
                    })
                })
                .collect();
            assert!(!valid.is_empty());
            for start in &starts {
                let cost = |other: &[u32]| -> u32 {
                    start.iter().zip(other).map(|(a, b)| a.abs_diff(*b)).sum()
                };
                let best = valid.iter().map(|v| cost(v)).min().unwrap();
                let mut positions = [vec![0], start.clone(), vec![width]].concat();
                project(&mut positions, m);
                let result = &positions[1..positions.len() - 1];
                assert!(
                    positions.windows(2).all(|w| w[1] >= w[0] + m),
                    "{start:?} in {width}: {positions:?}"
                );
                assert_eq!(cost(result), best, "{start:?} in {width}: {positions:?}");
            }
        }
    }
}

#[test]
fn fit_with_three_and_four_columns() {
    // n = 6 (3 columns) and n = 8 (4 columns), 60 wide, 12 cells per column minimum.
    let area = Rect::new(0, 0, 60, 40);
    let stored = ratios(None, &[408, 1, 541], &[500, 500]);
    let out = fit(area, 6, &stored, 10, 3);
    assert!(fits(area, 6, &out, 10, 3), "{out:?}");
    let stored = ratios(None, &[408, 1, 50, 541], &[500, 500]);
    let out = fit(area, 8, &stored, 10, 3);
    assert!(fits(area, 8, &out, 10, 3), "{out:?}");
    // Boundaries 24, 24, 27 move 21 cells in total, to 12, 24, 36.
    let l = layout(area, 8, &out);
    assert_eq!(
        (l[0].width, l[1].width, l[2].width, l[3].width),
        (12, 12, 12, 24)
    );
}

#[test]
fn panel_rects_apply_fit_and_room_is_checked_on_the_default_ratios() {
    // Defaults that fit: the plain layout.
    let area = Rect::new(0, 0, 100, 23);
    let five = Ratios::default_for(5);
    assert_eq!(panel_rects(area, 5, &five), layout(area, 5, &five));
    // Stored ratios that do not fit are clamped for the call, never changed.
    let stored = ratios(None, &[950, 50], &[1000]);
    let drawn = panel_rects(area, 2, &stored);
    assert!(fits(area, 2, &fit(area, 2, &stored, 10, 3), 10, 3));
    assert_eq!(drawn[0].width + drawn[1].width, 100);
    assert!(drawn[1].width >= 12);
    assert_eq!(stored, ratios(None, &[950, 50], &[1000]));
    // The 40x11 panel area of a 40x12 window: four terminals fit, five do not.
    let small = Rect::new(0, 0, 40, 11);
    assert!(has_room_for(small, 4));
    assert!(!has_room_for(small, 5));
}
