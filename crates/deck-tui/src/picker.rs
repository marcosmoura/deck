//! The session picker: its state, keys, the requests it sends and its drawing.

use std::time::Instant;

use deck_core::keys::{KeyCode, KeyEvent};
use deck_core::protocol::{ClientMsg, Request, Response, SessionSummary};
use deck_core::theme::Theme;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, Clear, Widget};

use crate::app::{App, Out};
use crate::draw::{border_type, color, selection, width};
use crate::modal::{Confirm, Editor, Modal, PromptKind, Retry, Target};

/// The picker's hints, on its bottom row.
pub const HINTS: &str = "⏎ switch  ^o new  ^r rename  ^x kill  esc close";

/// What a request sent to the daemon was for, so its answer can be used. Answers come back in
/// the order of the requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Req {
    List,
    /// A new session; the prompt to reopen when the daemon refuses.
    Create(Option<Retry>),
    Rename(Retry),
    Kill,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Picker {
    pub filter: Editor,
    /// Most recently attached first.
    sessions: Vec<SessionSummary>,
    /// Index into [`Picker::visible`].
    pub selected: usize,
    /// The first list has arrived.
    loaded: bool,
    /// Keys typed before it did, replayed once it has: they act on the list.
    held: Vec<KeyEvent>,
}

impl Picker {
    fn set_sessions(&mut self, mut sessions: Vec<SessionSummary>) {
        let keep = self.selected_session().map(|s| s.name.clone());
        sessions.sort_by(|a, b| {
            b.last_attached
                .cmp(&a.last_attached)
                .then_with(|| a.name.cmp(&b.name))
        });
        self.sessions = sessions;
        self.loaded = true;
        let (found, len) = {
            let visible = self.visible();
            let found = keep.and_then(|name| visible.iter().position(|s| s.name == name));
            (found, visible.len())
        };
        self.selected = found.unwrap_or_else(|| self.selected.min(len.saturating_sub(1)));
    }

    /// The sessions the filter lets through (a case-insensitive substring of the name).
    pub fn visible(&self) -> Vec<&SessionSummary> {
        let needle = self.filter.text().to_lowercase();
        self.sessions
            .iter()
            .filter(|s| s.name.to_lowercase().contains(&needle))
            .collect()
    }

    pub fn selected_session(&self) -> Option<&SessionSummary> {
        self.visible().get(self.selected).copied()
    }

    fn move_by(&mut self, delta: isize) {
        let last = self.visible().len().saturating_sub(1);
        self.selected = self.selected.saturating_add_signed(delta).min(last);
    }

    /// Applies an editing key to the filter; the selection returns to the top when the text
    /// changed.
    fn edit(&mut self, key: &KeyEvent) {
        let before = self.filter.text();
        self.filter.on_key(key);
        if self.filter.text() != before {
            self.selected = 0;
        }
    }

    pub(crate) fn paste(&mut self, text: &str) {
        self.filter.insert_str(text);
        self.selected = 0;
    }
}

impl App {
    /// Sends a request; its answer comes back in order, with `req` saying what it was for.
    pub(crate) fn request(&mut self, request: Request, req: Req) -> Out {
        self.requests.push_back(req);
        Out::Send(ClientMsg::Request(request))
    }

    pub(crate) fn open_picker(&mut self) -> Vec<Out> {
        self.picker = Some(Picker::default());
        vec![self.request(Request::ListSessions, Req::List)]
    }

    /// A key while the picker is open and nothing is on top of it.
    pub(crate) fn picker_key(&mut self, key: KeyEvent) -> Vec<Out> {
        let Some(mut picker) = self.picker.take() else {
            return Vec::new();
        };
        let (ctrl, alt) = (key.mods.ctrl, key.mods.alt);
        if !picker.loaded && !(key.code == KeyCode::Esc && !ctrl && !alt) {
            // Filter text shows at once; keys that act on the list wait for it, and so does
            // everything typed after them, to keep the order.
            let action = matches!(key.code, KeyCode::Enter | KeyCode::Up | KeyCode::Down)
                || (ctrl && !alt && matches!(key.code, KeyCode::Char('p' | 'n' | 'o' | 'r' | 'x')));
            if action || !picker.held.is_empty() {
                picker.held.push(key);
            } else {
                picker.edit(&key);
            }
            self.picker = Some(picker);
            return Vec::new();
        }
        let current = self.mirror.session_name().map(str::to_string);
        let selected = picker
            .selected_session()
            .map(|s| (s.name.clone(), s.terminals));
        let mut outs = Vec::new();
        match key.code {
            KeyCode::Esc if !ctrl && !alt => return Vec::new(),
            KeyCode::Up => picker.move_by(-1),
            KeyCode::Down => picker.move_by(1),
            KeyCode::Char('p') if ctrl && !alt => picker.move_by(-1),
            KeyCode::Char('n') if ctrl && !alt => picker.move_by(1),
            KeyCode::Enter => match selected {
                Some((name, _)) if current.as_deref() == Some(name.as_str()) => return outs,
                Some((name, _)) => return vec![Out::Switch(name)],
                None => {}
            },
            KeyCode::Char('o') if ctrl && !alt => {
                let name = picker.filter.text().trim().to_string();
                if name.is_empty() {
                    self.open_prompt(PromptKind::NewSession, "", None);
                } else {
                    let request = Request::NewSession {
                        name: Some(name),
                        cwd: self.cwd.clone(),
                    };
                    outs.push(self.request(request, Req::Create(None)));
                }
            }
            KeyCode::Char('r') if ctrl && !alt => {
                if let Some((name, _)) = selected {
                    let kind = PromptKind::RenameSession {
                        current: current.as_deref() == Some(name.as_str()),
                        from: name.clone(),
                    };
                    self.open_prompt(kind, &name, None);
                }
            }
            KeyCode::Char('x') if ctrl && !alt => {
                if let Some((name, terminals)) = selected {
                    let question = format!(
                        "Kill session \"{name}\" and its {terminals} {}? (y/n)",
                        if terminals == 1 {
                            "terminal"
                        } else {
                            "terminals"
                        }
                    );
                    self.modal = Some(Modal::Confirm(Confirm {
                        target: Target::Session(name),
                        question,
                    }));
                }
            }
            _ => picker.edit(&key),
        }
        self.picker = Some(picker);
        outs
    }

    /// The daemon's answer to the oldest request sent.
    pub(crate) fn response(&mut self, response: Response, now: Instant) -> Vec<Out> {
        // A session listing arrives when it is done, out of order, and is routed by its kind:
        // it is never the answer of the oldest request. The restore dialog takes it, or
        // drops it when the dialog is closed.
        if let Response::AgentSessions {
            kind,
            sessions,
            error,
        } = response
        {
            self.restore_response(kind, sessions, error);
            return Vec::new();
        }
        let req = self.requests.pop_front();
        match (req, response) {
            (_, Response::Sessions(list)) => {
                let Some(picker) = &mut self.picker else {
                    return Vec::new();
                };
                picker.set_sessions(list);
                let held = std::mem::take(&mut picker.held);
                let mut outs = Vec::new();
                for key in held {
                    // A key may close the picker or open a dialog; the rest follow the rules.
                    outs.extend(if self.modal.is_some() {
                        self.modal_key(key)
                    } else if self.picker.is_some() {
                        self.picker_key(key)
                    } else {
                        Vec::new()
                    });
                }
                outs
            }
            (Some(Req::Create(_)), Response::Session { name, .. }) => vec![Out::Switch(name)],
            (Some(Req::Rename(_) | Req::Kill), Response::Ok) if self.picker.is_some() => {
                vec![self.request(Request::ListSessions, Req::List)]
            }
            (req, Response::Error(text)) => {
                // A list that cannot be had must not leave the picker waiting.
                if let (Some(Req::List), Some(picker)) = (&req, &mut self.picker) {
                    picker.loaded = true;
                    picker.held.clear();
                }
                self.set_flash(text, now, true);
                let retry = match req {
                    Some(Req::Create(retry)) => retry,
                    Some(Req::Rename(retry)) => Some(retry),
                    _ => None,
                };
                if let (Some(retry), None) = (retry, &self.modal) {
                    self.modal = Some(Modal::Prompt(retry.prompt));
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Asks for the list again when the picker is open, after something changed.
    pub(crate) fn refresh_picker(&mut self) -> Vec<Out> {
        if self.picker.is_some() {
            vec![self.request(Request::ListSessions, Req::List)]
        } else {
            Vec::new()
        }
    }
}

fn count(n: u32, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The picker: a rounded box in the middle of `area`, 60% wide (at least 40 columns) and at
/// most 70% high.
pub fn draw(buf: &mut Buffer, area: Rect, app: &App, picker: &Picker, theme: &Theme) {
    let visible = picker.visible();
    let current = app.mirror.session_name();
    // Borders, the filter, a blank row above and below the list, the hints.
    const CHROME: u16 = 6;
    let w = (area.width * 6 / 10).max(40).min(area.width);
    let max_h = (area.height * 7 / 10).max(CHROME + 1).min(area.height);
    let want = u16::try_from(visible.len().max(1)).unwrap_or(u16::MAX);
    let list_rows = want.min(max_h.saturating_sub(CHROME)).max(1);
    let h = (list_rows + CHROME).min(area.height);
    // Too short for the blank rows and the hints: only the filter and the list stay.
    let compact = h < CHROME + 1;
    let top = if compact { 1 } else { 2 };
    let list_rows = if compact {
        list_rows
            .min((h.saturating_sub(2)).saturating_sub(1))
            .max(1)
    } else {
        list_rows
    };
    let rect = Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    );
    Clear.render(rect, buf);
    let base = Style::new().bg(color(theme.overlay_bg));
    buf.set_style(rect, base);
    Block::bordered()
        .border_type(border_type(theme.border_style))
        .border_style(base.fg(color(theme.accent)))
        .title("─ Sessions ")
        .render(rect, buf);
    let inner = Rect::new(rect.x + 1, rect.y + 1, w - 2, h.saturating_sub(2));
    let line = |row: u16| inner.y + row;

    // The filter, with its cursor.
    let (before, after) = picker.filter.split();
    let text = format!(" > {before}█{after}");
    buf.set_stringn(inner.x, line(0), &text, usize::from(inner.width), base);

    // The rows, scrolled so the selection is in sight.
    let pw = visible
        .iter()
        .map(|s| width(&count(s.projects, "space", "spaces")))
        .max()
        .unwrap_or(0);
    let tw = visible
        .iter()
        .map(|s| width(&count(s.terminals, "terminal", "terminals")))
        .max()
        .unwrap_or(0);
    let nw = visible.iter().map(|s| width(&s.name)).max().unwrap_or(0);
    let first = (picker.selected + 1).saturating_sub(usize::from(list_rows));
    if visible.is_empty() {
        buf.set_stringn(
            inner.x,
            line(top),
            "   no sessions match",
            usize::from(inner.width),
            base.fg(color(theme.muted)),
        );
    }
    for (i, s) in visible
        .iter()
        .skip(first)
        .take(usize::from(list_rows))
        .enumerate()
    {
        let mark = if current == Some(s.name.as_str()) {
            "●"
        } else {
            " "
        };
        // Our own client is attached to the current session.
        let own = u32::from(current == Some(s.name.as_str()));
        let attached = if s.attached_clients > own {
            "attached"
        } else {
            ""
        };
        let pad = |text: String, to: u16| {
            let fill = usize::from(to.saturating_sub(width(&text)));
            format!("{text}{}", " ".repeat(fill))
        };
        let row = format!(
            " {mark} {}  {}  {}  {attached}",
            pad(s.name.clone(), nw),
            pad(count(s.projects, "space", "spaces"), pw),
            pad(count(s.terminals, "terminal", "terminals"), tw),
        );
        let y = line(top + u16::try_from(i).unwrap_or(0));
        let selected = first + i == picker.selected;
        let style = if selected {
            selection(theme, base)
        } else {
            base
        };
        let row_rect = Rect::new(inner.x, y, inner.width, 1);
        buf.set_style(row_rect, style);
        buf.set_stringn(inner.x, y, &row, usize::from(inner.width), style);
    }
    if !compact {
        buf.set_stringn(
            inner.x,
            line(inner.height.saturating_sub(1)),
            format!(" {HINTS}"),
            usize::from(inner.width),
            base.fg(color(theme.muted)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mirror::tests::{frame, no_sidebar, view};
    use deck_core::action::Action as Act;
    use deck_core::model::SessionId;
    use deck_core::protocol::ServerMsg;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event as ct;

    fn summary(
        name: &str,
        projects: u32,
        terminals: u32,
        attached: u32,
        last: u64,
    ) -> SessionSummary {
        SessionSummary {
            id: SessionId(last + 1),
            name: name.into(),
            attached_clients: attached,
            terminals,
            projects,
            last_attached: last,
        }
    }

    fn sessions() -> Vec<SessionSummary> {
        vec![
            summary("main", 3, 7, 1, 5),
            summary("Workshop", 1, 1, 1, 3),
            summary("work", 1, 2, 0, 4),
            summary("never", 1, 1, 0, 0),
        ]
    }

    fn app() -> App {
        let mut app = App::new(&no_sidebar(), (100, 30), &[]);
        app.on_server(ServerMsg::SessionView(view(None)));
        app.on_server(ServerMsg::Frame(frame(true, (98, 27), vec![])));
        app
    }

    fn open(app: &mut App) {
        let out = app.open_picker();
        assert_eq!(out, [Out::Send(ClientMsg::Request(Request::ListSessions))]);
        app.on_server(ServerMsg::Response(Response::Sessions(sessions())));
    }

    fn press(app: &mut App, code: ct::KeyCode, mods: ct::KeyModifiers) -> Vec<Out> {
        app.on_event(
            ct::Event::Key(ct::KeyEvent::new(code, mods)),
            Instant::now(),
        )
    }

    fn typed(app: &mut App, text: &str) {
        for c in text.chars() {
            press(app, ct::KeyCode::Char(c), ct::KeyModifiers::NONE);
        }
    }

    fn ctrl(app: &mut App, c: char) -> Vec<Out> {
        press(app, ct::KeyCode::Char(c), ct::KeyModifiers::CONTROL)
    }

    fn names(app: &App) -> Vec<String> {
        let picker = app.picker.as_ref().unwrap();
        picker.visible().iter().map(|s| s.name.clone()).collect()
    }

    fn render(app: &App) -> Vec<String> {
        let mut t = Terminal::new(TestBackend::new(app.size.0, app.size.1)).unwrap();
        t.draw(|f| crate::draw::draw(f, app)).unwrap();
        let buf = t.backend().buffer();
        (0..buf.area.height)
            .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect())
            .collect()
    }

    #[test]
    fn sessions_are_ordered_by_recent_attach_and_filtered_by_substring_ignoring_case() {
        let mut a = app();
        open(&mut a);
        assert_eq!(names(&a), ["main", "work", "Workshop", "never"]);
        typed(&mut a, "WORK");
        assert_eq!(names(&a), ["work", "Workshop"]);
        typed(&mut a, "s");
        assert_eq!(names(&a), ["Workshop"]);
        ctrl(&mut a, 'u');
        assert_eq!(names(&a).len(), 4);
    }

    #[test]
    fn the_selection_moves_with_arrows_and_ctrl_p_n_and_returns_to_the_top_when_filtering() {
        let mut a = app();
        open(&mut a);
        let sel = |a: &App| a.picker.as_ref().unwrap().selected;
        press(&mut a, ct::KeyCode::Down, ct::KeyModifiers::NONE);
        ctrl(&mut a, 'n');
        assert_eq!(sel(&a), 2);
        ctrl(&mut a, 'p');
        press(&mut a, ct::KeyCode::Up, ct::KeyModifiers::NONE);
        press(&mut a, ct::KeyCode::Up, ct::KeyModifiers::NONE);
        assert_eq!(sel(&a), 0);
        for _ in 0..10 {
            press(&mut a, ct::KeyCode::Down, ct::KeyModifiers::NONE);
        }
        assert_eq!(sel(&a), 3);
        typed(&mut a, "w");
        assert_eq!(sel(&a), 0);
    }

    #[test]
    fn enter_switches_to_the_selected_session_and_closes_on_the_current_one() {
        let mut a = app();
        open(&mut a);
        // The first row is the current session: nothing to switch to.
        assert!(press(&mut a, ct::KeyCode::Enter, ct::KeyModifiers::NONE).is_empty());
        assert!(a.picker.is_none());
        open(&mut a);
        typed(&mut a, "shop");
        assert_eq!(
            press(&mut a, ct::KeyCode::Enter, ct::KeyModifiers::NONE),
            [Out::Switch("Workshop".into())]
        );
        assert!(a.picker.is_none());
        open(&mut a);
        press(&mut a, ct::KeyCode::Esc, ct::KeyModifiers::NONE);
        assert!(a.picker.is_none());
    }

    #[test]
    fn keys_in_the_picker_never_reach_the_terminal_or_the_prefix() {
        let mut a = app();
        open(&mut a);
        assert!(press(&mut a, ct::KeyCode::Char(' '), ct::KeyModifiers::ALT).is_empty());
        assert!(!a.armed);
        assert!(typed_out(&mut a).is_empty());
        ctrl(&mut a, 'u');
        a.on_event(ct::Event::Paste("wor\nk".into()), Instant::now());
        assert_eq!(names(&a), ["work", "Workshop"]);
    }

    fn typed_out(a: &mut App) -> Vec<Out> {
        press(a, ct::KeyCode::Char('x'), ct::KeyModifiers::NONE)
    }

    #[test]
    fn ctrl_o_creates_from_the_filter_or_asks_for_a_name() {
        let mut a = app();
        a = a.with_dirs("/work".into(), None);
        open(&mut a);
        typed(&mut a, "third");
        let new = |name: Option<&str>| {
            Out::Send(ClientMsg::Request(Request::NewSession {
                name: name.map(str::to_string),
                cwd: "/work".into(),
            }))
        };
        assert_eq!(ctrl(&mut a, 'o'), [new(Some("third"))]);
        assert_eq!(
            a.on_server(ServerMsg::Response(Response::Session {
                id: SessionId(9),
                name: "third".into()
            })),
            [Out::Switch("third".into())]
        );

        // An empty filter prompts.
        let mut a = app().with_dirs("/work".into(), None);
        open(&mut a);
        assert!(ctrl(&mut a, 'o').is_empty());
        let Some(Modal::Prompt(p)) = &a.modal else {
            panic!("no prompt")
        };
        assert_eq!(p.label(), "New session:");
        typed(&mut a, "fresh");
        let out = press(&mut a, ct::KeyCode::Enter, ct::KeyModifiers::NONE);
        assert_eq!(out, [new(Some("fresh"))]);
        // A refusal reopens the prompt with the error.
        a.on_server(ServerMsg::Response(Response::Error(
            "a session named \"fresh\" already exists".into(),
        )));
        assert!(matches!(a.modal, Some(Modal::Prompt(_))));
        assert!(a.flash_error && a.flash.is_some());
        assert!(a.picker.is_some());
    }

    #[test]
    fn ctrl_x_kills_another_session_by_request_and_the_current_one_by_action() {
        let mut a = app();
        open(&mut a);
        press(&mut a, ct::KeyCode::Down, ct::KeyModifiers::NONE);
        ctrl(&mut a, 'x');
        let Some(Modal::Confirm(c)) = &a.modal else {
            panic!("no confirmation")
        };
        assert_eq!(
            c.question,
            "Kill session \"work\" and its 2 terminals? (y/n)"
        );
        assert!(typed_out(&mut a).is_empty());
        assert!(a.modal.is_some());
        assert_eq!(
            typed_y(&mut a),
            [Out::Send(ClientMsg::Request(Request::KillSession {
                name: "work".into()
            }))]
        );
        // The list is asked again once it is done.
        assert_eq!(
            a.on_server(ServerMsg::Response(Response::Ok)),
            [Out::Send(ClientMsg::Request(Request::ListSessions))]
        );

        let mut a = app();
        open(&mut a);
        ctrl(&mut a, 'x');
        assert_eq!(
            typed_y(&mut a),
            [Out::Send(ClientMsg::Action(Act::KillSession))]
        );
        // `n` and Escape keep the session and the picker.
        let mut a = app();
        open(&mut a);
        ctrl(&mut a, 'x');
        typed(&mut a, "n");
        assert!(a.modal.is_none() && a.picker.is_some());
    }

    fn typed_y(a: &mut App) -> Vec<Out> {
        press(a, ct::KeyCode::Char('y'), ct::KeyModifiers::NONE)
    }

    #[test]
    fn ctrl_r_renames_another_session_by_request_and_a_refusal_reopens_the_prompt() {
        let mut a = app();
        open(&mut a);
        press(&mut a, ct::KeyCode::Down, ct::KeyModifiers::NONE);
        ctrl(&mut a, 'r');
        ctrl(&mut a, 'u');
        typed(&mut a, "renamed");
        assert_eq!(
            press(&mut a, ct::KeyCode::Enter, ct::KeyModifiers::NONE),
            [Out::Send(ClientMsg::Request(Request::RenameSession {
                from: "work".into(),
                to: "renamed".into()
            }))]
        );
        assert!(a.modal.is_none());
        a.on_server(ServerMsg::Response(Response::Error("no".into())));
        let Some(Modal::Prompt(p)) = &a.modal else {
            panic!("no prompt")
        };
        assert_eq!(p.editor.text(), "renamed");

        // The current session is renamed by an action, which refreshes the list.
        let mut a = app();
        open(&mut a);
        ctrl(&mut a, 'r');
        typed(&mut a, "2");
        assert_eq!(
            press(&mut a, ct::KeyCode::Enter, ct::KeyModifiers::NONE),
            [Out::Send(ClientMsg::Action(Act::RenameSession {
                name: "main2".into()
            }))]
        );
        assert_eq!(
            a.on_server(ServerMsg::ActionResult(Ok(()))),
            [Out::Send(ClientMsg::Request(Request::ListSessions))]
        );
    }

    #[test]
    fn the_overlay_is_drawn_with_the_current_session_marked_and_the_selection_reversed() {
        let mut a = app();
        open(&mut a);
        typed(&mut a, "wo");
        let text = render(&a);
        let joined = text.join("\n");
        assert!(joined.contains("╭─ Sessions"), "{joined}");
        assert!(joined.contains("> wo█"), "{joined}");
        assert!(joined.contains(HINTS), "{joined}");
        assert!(
            text.iter()
                .any(|r| r.contains("  work      1 space  2 terminals")),
            "{joined}"
        );
        assert!(
            text.iter()
                .any(|r| r.contains("Workshop") && r.contains("attached")),
            "{joined}"
        );
        // Overlay is 60 columns wide in a 100-column window.
        let top = text.iter().find(|r| r.contains("Sessions")).unwrap();
        let start = top.chars().position(|c| c == '╭').unwrap();
        let end = top.chars().position(|c| c == '╮').unwrap();
        assert_eq!(end - start + 1, 60);

        let mut a = app();
        open(&mut a);
        let mut t = Terminal::new(TestBackend::new(100, 30)).unwrap();
        t.draw(|f| crate::draw::draw(f, &a)).unwrap();
        let rows = render(&a);
        let y = rows.iter().position(|r| r.contains("● main")).unwrap();
        let x = rows[y].chars().position(|c| c == '●').unwrap();
        let buf = t.backend().buffer();
        assert!(
            buf[(u16::try_from(x).unwrap(), u16::try_from(y).unwrap())]
                .modifier
                .contains(ratatui::style::Modifier::REVERSED)
        );
        // The cursor of the terminal is hidden while the overlay is up.
        assert!(a.overlay_open());
    }

    #[test]
    fn a_long_list_scrolls_to_keep_the_selection_in_sight() {
        let mut a = App::new(&no_sidebar(), (80, 14), &[]);
        a.on_server(ServerMsg::SessionView(view(None)));
        a.open_picker();
        let many: Vec<SessionSummary> = (0..20)
            .map(|i| summary(&format!("s{i:02}"), 1, 1, 0, 100 - i))
            .collect();
        a.on_server(ServerMsg::Response(Response::Sessions(many)));
        for _ in 0..12 {
            press(&mut a, ct::KeyCode::Down, ct::KeyModifiers::NONE);
        }
        let text = render(&a);
        assert!(text.iter().any(|r| r.contains("s12")), "{text:#?}");
        assert!(!text.iter().any(|r| r.contains("s00")), "{text:#?}");
        // At most 70% of the height: 13 rows of panel area give 9.
        let boxed = text
            .iter()
            .filter(|r| r.contains('│') && r.contains("s"))
            .count();
        assert!(boxed <= 9);
    }

    #[test]
    fn a_five_row_window_keeps_the_list_row_and_drops_the_hints() {
        let mut a = App::new(&no_sidebar(), (40, 5), &[]);
        a.on_server(ServerMsg::SessionView(view(None)));
        a.open_picker();
        a.on_server(ServerMsg::Response(Response::Sessions(sessions())));
        let text = render(&a);
        let joined = text.join("\n");
        assert!(joined.contains("● main"), "{joined}");
        assert!(joined.contains("> █"), "{joined}");
        assert!(!joined.contains("switch"), "{joined}");
    }

    #[test]
    fn keys_typed_before_the_list_arrives_are_replayed_on_it() {
        let mut a = app();
        a.open_picker();
        assert!(ctrl(&mut a, 'x').is_empty());
        assert!(a.modal.is_none());
        a.on_server(ServerMsg::Response(Response::Sessions(sessions())));
        let Some(Modal::Confirm(c)) = &a.modal else {
            panic!("no confirmation")
        };
        assert!(c.question.starts_with("Kill session \"main\""));
        // Escape does not wait.
        let mut a = app();
        a.open_picker();
        press(&mut a, ct::KeyCode::Esc, ct::KeyModifiers::NONE);
        assert!(a.picker.is_none());
    }

    #[test]
    fn filter_text_typed_while_loading_shows_at_once_and_keeps_its_order_with_actions() {
        let mut a = app();
        a.open_picker();
        typed(&mut a, "wor");
        assert_eq!(a.picker.as_ref().unwrap().filter.text(), "wor");
        // `m`, `^x`, `y`: the filter gets `m`, then the kill is asked of the first match
        // and `y` answers it instead of going into the filter.
        typed(&mut a, "k");
        ctrl(&mut a, 'x');
        let out_y = typed_y(&mut a);
        assert!(out_y.is_empty());
        assert_eq!(a.picker.as_ref().unwrap().filter.text(), "work");
        let out = a.on_server(ServerMsg::Response(Response::Sessions(sessions())));
        assert_eq!(
            out,
            [Out::Send(ClientMsg::Request(Request::KillSession {
                name: "work".into()
            }))]
        );
        assert!(a.modal.is_none());
        assert_eq!(a.picker.as_ref().unwrap().filter.text(), "work");
    }

    #[test]
    fn an_agent_session_listing_never_takes_the_place_of_a_pending_request() {
        let mut a = app();
        a.open_picker();
        // The listing of another request arrives before the picker's own answer.
        a.on_server(ServerMsg::Response(Response::AgentSessions {
            kind: deck_core::agents::AgentKind::Codex,
            sessions: Vec::new(),
            error: Some("timed out".into()),
        }));
        assert_eq!(a.requests.len(), 1, "the picker's request is still pending");
        assert!(!a.flash_error);
        a.on_server(ServerMsg::Response(Response::Sessions(sessions())));
        assert!(a.requests.is_empty());
        assert_eq!(a.picker.as_ref().unwrap().visible().len(), 4);
    }

    #[test]
    fn a_failed_list_request_does_not_leave_the_picker_waiting() {
        let mut a = app();
        a.open_picker();
        ctrl(&mut a, 'x');
        a.on_server(ServerMsg::Response(Response::Error("no list".into())));
        assert!(a.modal.is_none() && a.flash_error);
        typed(&mut a, "w");
        assert_eq!(a.picker.as_ref().unwrap().filter.text(), "w");
    }

    #[test]
    fn switching_forgets_what_belongs_to_the_old_session() {
        let mut a = app();
        open(&mut a);
        a.reset_session();
        assert!(a.picker.is_none() && a.mirror.view().is_none());
        assert!(a.pending.is_empty() && a.requests.is_empty());
    }
}
