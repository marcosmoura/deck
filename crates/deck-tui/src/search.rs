//! Searching the history from scroll mode: the prompt (`/` down, `?` up), `n` and `N`, and the
//! highlights of the matches on screen.
//!
//! The daemon finds the next match (`Request::Search`); the client finds the matches of the
//! rows it has, to draw them, with the same smart case and the same units (a character with
//! its combining marks, a wide character in its two cells).

use std::collections::VecDeque;
use std::time::Instant;

use deck_core::keys::{KeyCode, KeyEvent};
use deck_core::model::TerminalId;
use deck_core::protocol::{Pos, Request, ScrollTo};

use crate::app::{App, Out};
use crate::modal::Editor;
use crate::picker::Req;
use crate::select::{Cell, cells};

/// The query prompt in the floating bar.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SearchPrompt {
    /// `?`: toward older rows.
    pub up: bool,
    pub editor: Editor,
}

/// The search of scroll mode.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SearchState {
    pub prompt: Option<SearchPrompt>,
    /// The last query, kept for an empty `Enter` and for `n`.
    pub last: String,
    /// The terminal whose matches are highlighted; set by the first search.
    pub active: Option<TerminalId>,
    /// The direction of the last search.
    pub up: bool,
    /// The match the last search found, both ends included.
    pub current: Option<(Pos, Pos)>,
    /// Searches asked for while one is in flight: they start where it ends.
    queued: VecDeque<(String, bool)>,
    /// Bumped as scroll mode ends: an answer asked in an earlier one is not for this cursor.
    epoch: u32,
}

impl SearchState {
    /// Forgets the highlights and the prompt, as scroll mode ends; the last query stays.
    pub fn clear(&mut self) {
        self.prompt = None;
        self.active = None;
        self.current = None;
        self.queued.clear();
        self.epoch = self.epoch.wrapping_add(1);
    }
}

impl App {
    /// `/` and `?`.
    pub(crate) fn search_open(&mut self, up: bool) {
        self.search.prompt = Some(SearchPrompt {
            up,
            editor: Editor::default(),
        });
    }

    /// A key while the query prompt is open.
    pub(crate) fn search_prompt_key(&mut self, terminal: TerminalId, key: KeyEvent) -> Vec<Out> {
        self.dirty = true;
        let Some(prompt) = self.search.prompt.as_mut() else {
            return Vec::new();
        };
        let plain = !key.mods.ctrl && !key.mods.alt;
        match key.code {
            KeyCode::Esc if plain => {
                self.search.prompt = None;
            }
            KeyCode::Enter => {
                let up = prompt.up;
                let typed = prompt.editor.text();
                self.search.prompt = None;
                let query = if typed.is_empty() {
                    self.search.last.clone()
                } else {
                    typed
                };
                if query.is_empty() {
                    return Vec::new();
                }
                self.search.last.clone_from(&query);
                self.search.active = Some(terminal);
                self.search.up = up;
                return self.search_ask(terminal, query, up);
            }
            _ => {
                prompt.editor.on_key(&key);
            }
        }
        Vec::new()
    }

    pub(crate) fn search_paste(&mut self, text: &str) {
        if let Some(prompt) = self.search.prompt.as_mut() {
            prompt.editor.insert_str(text);
            self.dirty = true;
        }
    }

    /// `n` (`reverse` false) and `N`: the last query again.
    pub(crate) fn search_again(&mut self, terminal: TerminalId, reverse: bool) -> Vec<Out> {
        if self.search.last.is_empty() {
            return Vec::new();
        }
        self.search.active = Some(terminal);
        let query = self.search.last.clone();
        let up = self.search.up != reverse;
        self.search_ask(terminal, query, up)
    }

    /// Asks the daemon for the next match from the copy cursor, or waits for the search in
    /// flight, which moves the cursor.
    fn search_ask(&mut self, terminal: TerminalId, query: String, up: bool) -> Vec<Out> {
        self.dirty = true;
        let in_flight = self
            .requests
            .iter()
            .any(|r| matches!(r, Req::Search { .. }));
        if in_flight {
            self.search.queued.push_back((query, up));
            return Vec::new();
        }
        let Some(from) = self.copy.as_ref().map(|c| c.cursor) else {
            return Vec::new();
        };
        vec![self.request(
            Request::Search {
                terminal,
                query: query.clone(),
                from,
                up,
            },
            Req::Search {
                terminal,
                query,
                epoch: self.search.epoch,
            },
        )]
    }

    /// Whether a search answer is still for the copy cursor it was asked for.
    fn search_wanted(&self, terminal: TerminalId, epoch: u32) -> bool {
        self.search.epoch == epoch
            && self.scroll_mode == Some(terminal)
            && self.copy.as_ref().is_some_and(|c| c.terminal == terminal)
    }

    /// The daemon found a match: the cursor goes to its start and the view to it.
    pub(crate) fn search_found(
        &mut self,
        terminal: TerminalId,
        epoch: u32,
        (start, end): (Pos, Pos),
        wrapped: bool,
        now: Instant,
    ) -> Vec<Out> {
        if !self.search_wanted(terminal, epoch) {
            return Vec::new();
        }
        self.search.current = Some((start, end));
        self.set_cursor(terminal, start);
        let mut outs = vec![self.scroll(terminal, ScrollTo::Row(start.row))];
        if wrapped {
            self.set_flash("Search wrapped".into(), now, false);
        }
        self.dirty = true;
        outs.extend(self.search_next_queued(terminal));
        outs
    }

    pub(crate) fn search_missing(
        &mut self,
        terminal: TerminalId,
        epoch: u32,
        query: &str,
        now: Instant,
    ) -> Vec<Out> {
        if !self.search_wanted(terminal, epoch) {
            return Vec::new();
        }
        self.search.current = None;
        self.set_flash(format!("Not found: {query}"), now, false);
        self.search.queued.clear();
        Vec::new()
    }

    /// A search that failed: the ones behind it are dropped.
    pub(crate) fn search_failed(&mut self, epoch: u32) {
        if self.search.epoch == epoch {
            self.search.queued.clear();
        }
    }

    fn search_next_queued(&mut self, terminal: TerminalId) -> Vec<Out> {
        match self.search.queued.pop_front() {
            Some((query, up)) => self.search_ask(terminal, query, up),
            None => Vec::new(),
        }
    }
}

/// The columns (first, last) where `query` occurs in a row, by the rule the daemon's search
/// uses (`deck_core::search`), over the cells of the client's mirror.
pub(crate) fn find(row: &[Cell], query: &str) -> Vec<(u16, u16)> {
    let units = row.iter().enumerate().filter_map(|(col, cell)| {
        let col = u16::try_from(col).unwrap_or(u16::MAX);
        let text = match cell {
            Cell::Spacer => return None,
            Cell::Blank => " ",
            Cell::Text(t) => t.as_str(),
        };
        let wide = row.get(usize::from(col) + 1) == Some(&Cell::Spacer);
        Some((col, if wide { col.saturating_add(1) } else { col }, text))
    });
    deck_core::search::find(units, query)
}

/// A match on a terminal's screen: its row index, its columns and whether it is the one the
/// last search found.
pub(crate) struct Visible {
    pub row: u16,
    pub from: u16,
    pub to: u16,
    pub current: bool,
}

/// The matches of the active query on the rows `id` shows; none unless scroll mode is on `id`
/// with a search made.
pub(crate) fn visible(app: &App, id: TerminalId) -> Vec<Visible> {
    if app.search.active != Some(id) || app.scroll_mode != Some(id) || app.search.last.is_empty() {
        return Vec::new();
    }
    let Some(term) = app.mirror.terminal(id) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for row in 0..term.size.1 {
        let cells = cells(term.lines.get(usize::from(row)), term.size.0);
        let at = term.top_row + u64::from(row);
        for (from, to) in find(&cells, &app.search.last) {
            let current = app
                .search
                .current
                .is_some_and(|(s, _)| s.row == at && s.col == from);
            out.push(Visible {
                row,
                from,
                to,
                current,
            });
        }
    }
    out
}
