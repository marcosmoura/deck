//! Resizing a panel's terminals: the resize keys and their repeat, the reset, and dragging a
//! border with the mouse. Pure, like the rest of the client: it returns what to send.

use std::time::{Duration, Instant};

use deck_core::action::Action as Act;
use deck_core::layout::{self, Boundary, Direction, Ratios};
use deck_core::model::PanelId;
use ratatui::crossterm::event as ct;

use crate::app::{App, Out};
use crate::grid::Grid;

/// How long after a resize key the next ones act without the prefix.
pub const REPEAT_FOR: Duration = Duration::from_secs(1);
/// A key's step, in per-mille of the boundary's span.
const STEP: i32 = 50;
/// The least time between two `SetRatios` sent while dragging.
pub const DRAG_EVERY: Duration = Duration::from_millis(33);

/// A border being dragged.
#[derive(Debug)]
pub(crate) struct Drag {
    pub panel: PanelId,
    /// The terminal count the drag started with.
    pub n: usize,
    pub boundary: Boundary,
    /// Where the pointer grabbed the border, relative to the border's position.
    grab: i32,
    /// The ratios of the last `SetRatios` sent, and when.
    sent: Option<(Ratios, Instant)>,
}

/// The sidebar's separator being dragged.
#[derive(Debug)]
pub(crate) struct SidebarDrag {
    /// When the width last changed, for the throttle.
    applied: Option<Instant>,
}

/// The position of `boundary` along its axis, in cells of the panel area, and where its span
/// starts.
fn edge(grid: &Grid, boundary: Boundary) -> (i32, i32) {
    let shape = layout::shape(grid.ids.len());
    let off = usize::from(shape.main);
    let at = |i: usize| grid.rects.get(i).copied().unwrap_or_default();
    let right = |i: usize| i32::from(at(i).x) + i32::from(at(i).width);
    match boundary {
        Boundary::Main => (right(0), 0),
        Boundary::Col(i) => (right(off + i), if shape.main { right(0) } else { 0 }),
        Boundary::Row(i) => (i32::from(at(off + (i + 1) * shape.cols).y), 0),
    }
}

impl App {
    /// A resize key, after the prefix or in the repeat: moves the boundary the focused
    /// terminal's edge in that direction is on, and keeps the repeat going.
    pub(crate) fn resize_key(&mut self, dir: Direction, now: Instant) -> Vec<Out> {
        let Some(grid) = Grid::of(self).filter(Grid::bordered) else {
            return Vec::new();
        };
        // The repeat starts even when nothing moves, so a key held at the limit never
        // reaches the terminal.
        self.resize_until = Some(now + REPEAT_FOR);
        let n = grid.ids.len();
        let Some((boundary, sign)) = grid
            .focused
            .and_then(|from| layout::boundary_for(n, from, dir))
        else {
            return Vec::new();
        };
        let next = layout::resize(
            n,
            &grid.effective,
            boundary,
            sign * STEP,
            grid.area,
            layout::MIN_INNER.0,
            layout::MIN_INNER.1,
        );
        if next == grid.effective {
            return Vec::new();
        }
        vec![self.set_ratios(grid.panel, n, next)]
    }

    /// `reset_layout`: the default ratios for the panel's count.
    pub(crate) fn reset_layout(&mut self) -> Vec<Out> {
        self.resize_until = None;
        let Some(panel) = self.mirror.active_panel() else {
            return Vec::new();
        };
        let n = panel.terminals.len();
        let default = Ratios::default_for(n);
        if n < 2 || panel.ratios == default {
            return Vec::new();
        }
        vec![self.set_ratios(panel.id, n, default)]
    }

    /// `SetRatios`, whose refusal shows nothing: the next view puts things right.
    fn set_ratios(&mut self, panel: PanelId, n: usize, ratios: Ratios) -> Out {
        self.act_silent(Act::SetRatios { panel, n, ratios })
    }

    /// Whether a sidebar drag is on, or this event starts one on the separator; `Some` with
    /// what to send when the event is the drag's and must go no further.
    pub(crate) fn sidebar_drag_mouse(
        &mut self,
        event: ct::MouseEvent,
        now: Instant,
    ) -> Option<Vec<Out>> {
        use ct::MouseButton::Left;
        use ct::MouseEventKind as Kind;
        if self.sidebar_drag.is_some() {
            match event.kind {
                Kind::Drag(Left) => return Some(self.sidebar_drag_to(event.column, now, false)),
                Kind::Up(Left) => {
                    let outs = self.sidebar_drag_to(event.column, now, true);
                    self.sidebar_drag = None;
                    self.dirty = true;
                    return Some(outs);
                }
                // The release was lost: the drag is over, and this event is handled as usual.
                Kind::Moved | Kind::Down(_) => {
                    self.sidebar_drag = None;
                    self.dirty = true;
                }
                _ => return Some(Vec::new()),
            }
        }
        let on_separator = self.sidebar_visible()
            && !self.sidebar_collapsed
            && event.column == self.sidebar_width
            && event.row < self.size.1;
        if event.kind == Kind::Down(Left) && on_separator {
            self.sidebar_drag = Some(SidebarDrag { applied: None });
            self.dirty = true;
            return Some(Vec::new());
        }
        None
    }

    /// Moves the separator to the pointer, at most every [`DRAG_EVERY`] except on release,
    /// and tells the daemon the new panel area.
    fn sidebar_drag_to(&mut self, column: u16, now: Instant, release: bool) -> Vec<Out> {
        let width = layout::clamp_sidebar_width(self.size.0, column);
        let Some(drag) = self.sidebar_drag.as_mut() else {
            return Vec::new();
        };
        let due = release
            || drag
                .applied
                .is_none_or(|at| now.saturating_duration_since(at) >= DRAG_EVERY);
        if width == self.sidebar_width || !due {
            return Vec::new();
        }
        drag.applied = Some(now);
        self.sidebar_width = width;
        self.dirty = true;
        vec![self.viewport_msg()]
    }

    /// Whether a drag is on, or this event starts one; `Some` with what to send when the
    /// event is the drag's and must go no further.
    pub(crate) fn drag_mouse(&mut self, event: ct::MouseEvent, now: Instant) -> Option<Vec<Out>> {
        use ct::MouseButton::Left;
        use ct::MouseEventKind as Kind;
        if self.drag.is_some() {
            match event.kind {
                Kind::Drag(Left) => return Some(self.drag_to(event, now, false)),
                Kind::Up(Left) => {
                    let outs = self.drag_to(event, now, true);
                    self.drag = None;
                    self.dirty = true;
                    return Some(outs);
                }
                // The release was lost (outside the window, or under an overlay): the drag
                // is over, and this event is handled as usual.
                Kind::Moved | Kind::Down(_) => {
                    self.drag = None;
                    self.dirty = true;
                }
                _ => return Some(Vec::new()),
            }
        }
        if event.kind != Kind::Down(Left) {
            return None;
        }
        let grid = Grid::of(self).filter(Grid::bordered)?;
        let x = event.column.checked_sub(self.sidebar_cols())?;
        if event.column >= self.size.0
            || event.row >= self.size.1
            || self.bar_covers(event.column, event.row)
        {
            return None;
        }
        let boundary =
            layout::hit_boundary(grid.area, grid.ids.len(), &grid.effective, x, event.row)?;
        let (at, _) = edge(&grid, boundary);
        let pointer = if matches!(boundary, Boundary::Row(_)) {
            i32::from(event.row)
        } else {
            i32::from(x)
        };
        self.drag = Some(Drag {
            panel: grid.panel,
            n: grid.ids.len(),
            boundary,
            grab: pointer - at,
            sent: None,
        });
        self.dirty = true;
        Some(Vec::new())
    }

    /// Follows the pointer: the border goes where it is, at most every [`DRAG_EVERY`]
    /// except on release.
    fn drag_to(&mut self, event: ct::MouseEvent, now: Instant, release: bool) -> Vec<Out> {
        let Some(drag) = &self.drag else {
            return Vec::new();
        };
        let (panel, n, boundary, grab) = (drag.panel, drag.n, drag.boundary, drag.grab);
        let grid = Grid::of(self).filter(|g| g.panel == panel && g.ids.len() == n);
        let Some(grid) = grid else {
            // The panel changed under the drag.
            self.drag = None;
            self.dirty = true;
            return Vec::new();
        };
        let column = i32::from(event.column) - i32::from(self.sidebar_cols());
        let pointer = if matches!(boundary, Boundary::Row(_)) {
            i32::from(event.row)
        } else {
            column
        };
        let (_, start) = edge(&grid, boundary);
        let pos = u16::try_from((pointer - grab - start).max(0)).unwrap_or(u16::MAX);
        let next = layout::ratio_at(
            grid.area,
            n,
            &grid.effective,
            boundary,
            pos,
            layout::MIN_INNER.0,
            layout::MIN_INNER.1,
        );
        let Some(drag) = self.drag.as_mut() else {
            return Vec::new();
        };
        let (last, since) = match &drag.sent {
            Some((r, at)) => (r, Some(*at)),
            None => (&grid.effective, None),
        };
        let due = release || since.is_none_or(|at| now.saturating_duration_since(at) >= DRAG_EVERY);
        if next == *last || !due {
            return Vec::new();
        }
        drag.sent = Some((next.clone(), now));
        vec![self.set_ratios(panel, n, next)]
    }
}
