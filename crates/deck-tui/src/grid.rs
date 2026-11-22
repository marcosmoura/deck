//! The active panel's grid as the client sees it: the daemon's layout, from the viewport it
//! sized the terminals for, shared by drawing and the mouse.

use deck_core::layout::{self, Boundary, Ratios, Rect};
use deck_core::model::{PanelId, TerminalId};

use crate::app::App;

#[derive(Debug)]
pub(crate) struct Grid {
    pub panel: PanelId,
    /// The panel area of the layout viewport, which the rects fill.
    pub area: Rect,
    /// The ratios the rects were laid out with: the stored ones, clamped to the viewport.
    pub effective: Ratios,
    pub ids: Vec<TerminalId>,
    /// One rect per terminal, in cells of the panel area (the layout viewport).
    pub rects: Vec<Rect>,
    pub focused: Option<usize>,
}

impl Grid {
    /// The grid of the active panel; `None` without one.
    pub fn of(app: &App) -> Option<Self> {
        let panel = app.mirror.active_panel()?;
        let (cols, rows) = app
            .mirror
            .view()
            .and_then(|v| v.layout_viewport)
            .unwrap_or_else(|| app.viewport());
        let n = panel.terminals.len();
        let area = Rect::new(0, 0, cols, rows);
        let effective = layout::fit(
            area,
            n,
            &panel.ratios,
            layout::MIN_INNER.0,
            layout::MIN_INNER.1,
        );
        let rects = layout::layout(area, n, &effective);
        Some(Self {
            panel: panel.id,
            area,
            effective,
            ids: panel.terminals.iter().map(|t| t.id).collect(),
            rects,
            focused: panel
                .terminals
                .iter()
                .position(|t| Some(t.id) == panel.focused_terminal),
        })
    }

    /// With two or more terminals each one has a border; a single one has none.
    pub fn bordered(&self) -> bool {
        self.ids.len() >= 2
    }

    /// The cells of terminal `i` that show its screen: its rect minus the border.
    pub fn inner(&self, i: usize) -> Rect {
        let r = self.rects.get(i).copied().unwrap_or_default();
        if self.bordered() {
            Rect::new(
                r.x + 1,
                r.y + 1,
                r.width.saturating_sub(2),
                r.height.saturating_sub(2),
            )
        } else {
            r
        }
    }

    /// The terminal whose rect holds a cell of the panel area.
    pub fn hit(&self, x: u16, y: u16) -> Option<usize> {
        self.rects.iter().position(|r| {
            x >= r.x
                && y >= r.y
                && u32::from(x) < u32::from(r.x) + u32::from(r.width)
                && u32::from(y) < u32::from(r.y) + u32::from(r.height)
        })
    }

    /// The cells of the border lines `boundary` moves: the facing edges of the terminals on
    /// either side of it.
    pub fn boundary_cells(&self, boundary: Boundary) -> Vec<(u16, u16)> {
        let shape = layout::shape(self.ids.len());
        let off = usize::from(shape.main);
        let mut cells = Vec::new();
        for (i, r) in self.rects.iter().enumerate() {
            let (col, row) = if shape.main && i == 0 {
                (None, None)
            } else {
                let g = i - off;
                (Some(g % shape.cols), Some(g / shape.cols))
            };
            // Whether this rect is before (Some(true)) or after (Some(false)) the boundary.
            let side = match boundary {
                Boundary::Main if i == 0 => Some(true),
                Boundary::Main => (col == Some(0)).then_some(false),
                Boundary::Col(c) => col.and_then(|x| match x {
                    x if x == c => Some(true),
                    x if x == c + 1 => Some(false),
                    _ => None,
                }),
                Boundary::Row(c) => row.and_then(|y| match y {
                    y if y == c => Some(true),
                    y if y == c + 1 => Some(false),
                    _ => None,
                }),
            };
            let Some(before) = side else { continue };
            if r.width == 0 || r.height == 0 {
                continue;
            }
            if matches!(boundary, Boundary::Row(_)) {
                let y = if before { r.y + r.height - 1 } else { r.y };
                cells.extend((r.x..r.x + r.width).map(|x| (x, y)));
            } else {
                let x = if before { r.x + r.width - 1 } else { r.x };
                cells.extend((r.y..r.y + r.height).map(|y| (x, y)));
            }
        }
        cells
    }
}
