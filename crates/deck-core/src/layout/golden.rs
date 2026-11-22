//! The golden layouts: a plain `pub` fixture so the daemon and TUI tests can assert against
//! the same values as the layout engine's own tests. Every case uses the default ratios.

use super::{Rect, Shape};

/// A layout with its expected rects, for [`super::Ratios::default_for`].
#[derive(Debug, Clone, Copy)]
pub struct Golden {
    pub area: Rect,
    pub n: usize,
    pub rects: &'static [Rect],
}

const fn r(x: u16, y: u16, width: u16, height: u16) -> Rect {
    Rect::new(x, y, width, height)
}

const fn s(main: bool, cols: usize, rows: usize) -> Shape {
    Shape { main, cols, rows }
}

/// Terminal count and its shape.
pub const SHAPES: &[(usize, Shape)] = &[
    (1, s(false, 1, 1)),
    (2, s(false, 2, 1)),
    (3, s(true, 1, 2)),
    (4, s(false, 2, 2)),
    (5, s(true, 2, 2)),
    (6, s(false, 3, 2)),
    (7, s(true, 2, 3)),
    (8, s(false, 4, 2)),
    (9, s(true, 2, 4)),
    (10, s(false, 5, 2)),
    (12, s(false, 4, 3)),
];

/// Areas and terminal counts with their rects.
pub const GOLDEN: &[Golden] = &[
    Golden {
        area: r(0, 0, 100, 40),
        n: 3,
        rects: &[r(0, 0, 50, 40), r(50, 0, 50, 20), r(50, 20, 50, 20)],
    },
    Golden {
        area: r(0, 0, 100, 40),
        n: 5,
        rects: &[
            r(0, 0, 50, 40),
            r(50, 0, 25, 20),
            r(75, 0, 25, 20),
            r(50, 20, 25, 20),
            r(75, 20, 25, 20),
        ],
    },
    Golden {
        area: r(0, 0, 100, 40),
        n: 4,
        rects: &[
            r(0, 0, 50, 20),
            r(50, 0, 50, 20),
            r(0, 20, 50, 20),
            r(50, 20, 50, 20),
        ],
    },
    Golden {
        area: r(0, 0, 101, 41),
        n: 3,
        rects: &[r(0, 0, 51, 41), r(51, 0, 50, 20), r(51, 20, 50, 21)],
    },
    Golden {
        area: r(0, 0, 7, 3),
        n: 5,
        rects: &[
            r(0, 0, 4, 3),
            r(4, 0, 1, 1),
            r(5, 0, 2, 1),
            r(4, 1, 1, 2),
            r(5, 1, 2, 2),
        ],
    },
    Golden {
        area: r(0, 0, 1, 3),
        n: 3,
        rects: &[r(0, 0, 1, 3), r(1, 0, 0, 1), r(1, 1, 0, 2)],
    },
];
