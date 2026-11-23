//! The floating bar: one row over the bottom of the panel area that shows the modes (prefix,
//! resize, scroll, sidebar) with their hints, and the flashes, only while there is something
//! to show.

use deck_core::theme::Theme;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Clear, Widget};

use crate::app::{App, too_small};
use crate::draw::{color, flash_color, truncate, width};
use crate::input::Action;
use crate::modal::Modal;
use crate::sidebar::SIDEBAR_HINTS;

/// The hints while scroll mode is on, as key and label pairs.
pub const SCROLL_HINTS: &[(&str, &str)] = &[("q", "exit"), ("g", "top"), ("G", "bottom")];

/// The bar as it is on screen: its cells and the text in each.
pub(crate) struct Bar {
    pub rect: Rect,
    segments: Vec<(String, Style)>,
}

/// What a mode puts on the bar after its badge.
#[derive(Default)]
struct Mode {
    badge: String,
    /// `[offset/history]` of a scrolled-back terminal in scroll mode.
    indicator: Option<String>,
    hints: Vec<(String, String)>,
}

fn pairs(hints: &[(&str, &str)]) -> Vec<(String, String)> {
    hints
        .iter()
        .map(|(k, l)| (k.to_string(), l.to_string()))
        .collect()
}

impl App {
    /// The mode the bar shows a badge for, by the table's precedence; none while an overlay
    /// has the screen.
    fn bar_mode(&self) -> Option<Mode> {
        let resizing = self.resize_until.is_some() || self.drag.is_some();
        let plain = self.scroll_mode.is_none() && !self.sidebar_focus;
        if self.sidebar_drag.is_some() {
            return Some(Mode {
                badge: format!(" SIDEBAR {} ", self.sidebar_width),
                ..Mode::default()
            });
        }
        if self.armed {
            let mut hints = vec![("esc".to_string(), "cancel".to_string())];
            if let Some(key) = self.bindings.first_key(Action::Help) {
                hints.push((key, "help".to_string()));
            }
            return Some(Mode {
                badge: " PREFIX ".into(),
                hints,
                ..Mode::default()
            });
        }
        if resizing && plain {
            // A key repeat shows its keys; a border drag shows none.
            let hints = if self.resize_until.is_some() {
                self.bindings.resize_hints()
            } else {
                Vec::new()
            };
            return Some(Mode {
                badge: " RESIZE ".into(),
                hints,
                ..Mode::default()
            });
        }
        if self.scroll_mode.is_some() {
            let indicator = self
                .mirror
                .focused_id()
                .and_then(|id| self.mirror.terminal(id))
                .filter(|t| t.display_offset > 0)
                .map(|t| format!("[{}/{}]", t.display_offset, t.history));
            return Some(Mode {
                badge: " SCROLL ".into(),
                indicator,
                hints: pairs(SCROLL_HINTS),
            });
        }
        if self.sidebar_focus {
            return Some(Mode {
                badge: " SIDEBAR ".into(),
                hints: pairs(SIDEBAR_HINTS),
                ..Mode::default()
            });
        }
        None
    }

    /// The bar for this frame, when it shows.
    pub(crate) fn bar(&self) -> Option<Bar> {
        if too_small(self.size) || matches!(self.modal, Some(Modal::Prompt(_) | Modal::Confirm(_)))
        {
            return None;
        }
        let theme = &self.theme;
        let mode = if self.overlay_open() {
            None
        } else {
            self.bar_mode()
        };
        let flash = self.flash.as_ref().map(|(text, _)| text.as_str());
        if mode.is_none() && flash.is_none() {
            return None;
        }
        let side = self.sidebar_cols();
        let room = self.size.0.saturating_sub(side);
        if room == 0 || self.size.1 == 0 {
            return None;
        }
        let base = Style::new()
            .fg(color(theme.status_fg))
            .bg(color(theme.status_bg));
        let pad = (" ".to_string(), base);
        let mut tail: Vec<(String, Style)> = Vec::new();
        let mut head = vec![pad.clone()];
        let mut badge_w = 0;
        if let Some(mode) = &mode {
            badge_w = width(&mode.badge);
            let badge = base
                .fg(color(theme.warning))
                .add_modifier(Modifier::REVERSED);
            // A badge that does not fit with its padding is cut and nothing follows it
            // (unreachable at `MIN_SIZE`, kept for safety).
            let text = if badge_w + 2 > room {
                truncate(&mode.badge, room.saturating_sub(2))
            } else {
                mode.badge.clone()
            };
            head.push((text, badge));
            if let Some(indicator) = &mode.indicator {
                tail.push((" ".into(), base));
                tail.push((indicator.clone(), base.fg(color(theme.warning))));
            }
            let hinted = flash.is_none() && !mode.hints.is_empty();
            if hinted || flash.is_some() {
                tail.push((
                    if mode.indicator.is_some() { "  " } else { " " }.into(),
                    base,
                ));
            }
            if hinted {
                for (i, (key, label)) in mode.hints.iter().enumerate() {
                    if i > 0 {
                        tail.push(("  ".into(), base));
                    }
                    tail.push((key.clone(), base.add_modifier(Modifier::BOLD)));
                    tail.push((format!(" {label}"), base.fg(color(theme.muted))));
                }
            }
        }
        if let Some(text) = flash {
            tail.push((text.to_string(), base.fg(color(flash_color(self, theme)))));
        }
        let tail_room = room.saturating_sub(2 + badge_w);
        let mut segments = head;
        segments.extend(cut(tail, tail_room));
        segments.push(pad);
        let w: u16 = segments.iter().map(|(t, _)| width(t)).sum();
        let w = w.min(room);
        let rect = Rect::new(side + (room - w) / 2, self.size.1 - 1, w, 1);
        Some(Bar { rect, segments })
    }

    /// Whether a cell of the window is under the bar.
    pub(crate) fn bar_covers(&self, column: u16, row: u16) -> bool {
        self.bar()
            .is_some_and(|bar| bar.rect.contains((column, row).into()))
    }
}

/// `segments` cut to `room` columns, the last one shown ending in `…`.
pub(crate) fn cut(segments: Vec<(String, Style)>, room: u16) -> Vec<(String, Style)> {
    let total: u16 = segments.iter().map(|(t, _)| width(t)).sum();
    if total <= room {
        return segments;
    }
    let mut out: Vec<(String, Style)> = Vec::new();
    let mut left = room;
    for (text, style) in segments {
        let w = width(&text);
        if w <= left {
            left -= w;
            out.push((text, style));
            continue;
        }
        if left > 0 {
            // The text, or what is left of it, with `…` for what is cut.
            out.push((truncate(&text, left), style));
        } else if let Some((last, last_style)) = out.pop() {
            // The segments filled the room exactly: the last one gives a cell to the `…`.
            let w = width(&last);
            out.push((truncate(&format!("{last}…"), w), last_style));
        }
        break;
    }
    out
}

/// Draws the bar, when it shows.
pub(crate) fn draw(buf: &mut Buffer, app: &App, theme: &Theme) {
    let Some(bar) = app.bar() else { return };
    Clear.render(bar.rect, buf);
    buf.set_style(
        bar.rect,
        Style::new()
            .fg(color(theme.status_fg))
            .bg(color(theme.status_bg)),
    );
    let mut x = bar.rect.x;
    let end = bar.rect.x + bar.rect.width;
    for (text, style) in &bar.segments {
        let (next, _) = buf.set_stringn(x, bar.rect.y, text, usize::from(end - x), *style);
        x = next;
    }
}
