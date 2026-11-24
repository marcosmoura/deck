//! Search in scroll mode through the client's state machine: the prompt, the requests, the
//! answers, `n` and `N`, and the highlights.

use std::time::Instant;

use deck_core::model::TerminalId;
use deck_core::protocol::{ClientMsg, Frame, Pos, Request, Response, ScrollTo, ServerMsg};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event as ct;
use ratatui::style::Modifier;

use crate::app::{App, Out};
use crate::draw::{color, draw};
use crate::mirror::tests::{frame, line, no_sidebar, view};
use crate::search::find;
use crate::select::cells;

const T: TerminalId = TerminalId(4);
const LIVE: u64 = 1000;

fn screen(lines: &[&str]) -> Frame {
    let rows = lines
        .iter()
        .enumerate()
        .map(|(i, l)| (u16::try_from(i).unwrap(), line(l)))
        .collect();
    let mut f = frame(true, (78, 21), rows);
    f.top_row = LIVE;
    f.history = 300;
    f.cursor.row = 20;
    f.cursor.col = 0;
    f
}

fn scrolling(lines: &[&str]) -> App {
    let mut a = App::new(&no_sidebar(), (80, 24), &[]);
    a.on_server(ServerMsg::SessionView(view(None)));
    a.on_server(ServerMsg::Frame(screen(lines)));
    let now = Instant::now();
    a.on_event(
        ct::Event::Key(ct::KeyEvent::new(
            ct::KeyCode::Char(' '),
            ct::KeyModifiers::ALT,
        )),
        now,
    );
    a.on_event(key('['), now);
    assert_eq!(a.scroll_mode, Some(T));
    a
}

fn key(c: char) -> ct::Event {
    let mods = if c.is_uppercase() || "?$^".contains(c) {
        ct::KeyModifiers::SHIFT
    } else {
        ct::KeyModifiers::NONE
    };
    ct::Event::Key(ct::KeyEvent::new(ct::KeyCode::Char(c), mods))
}

fn code(code: ct::KeyCode) -> ct::Event {
    ct::Event::Key(ct::KeyEvent::new(code, ct::KeyModifiers::NONE))
}

fn ctrl(c: char) -> ct::Event {
    ct::Event::Key(ct::KeyEvent::new(
        ct::KeyCode::Char(c),
        ct::KeyModifiers::CONTROL,
    ))
}

fn press(a: &mut App, events: &[ct::Event]) -> Vec<Out> {
    let now = Instant::now();
    events
        .iter()
        .flat_map(|e| a.on_event(e.clone(), now))
        .collect()
}

fn typed(a: &mut App, text: &str) -> Vec<Out> {
    let events: Vec<ct::Event> = text.chars().map(key).collect();
    press(a, &events)
}

fn pos(row: u64, col: u16) -> Pos {
    Pos { row, col }
}

fn search_request(query: &str, from: Pos, up: bool) -> Out {
    Out::Send(ClientMsg::Request(Request::Search {
        terminal: T,
        query: query.into(),
        from,
        up,
    }))
}

fn found(a: &mut App, start: Pos, end: Pos, wrapped: bool) -> Vec<Out> {
    a.on_server(ServerMsg::Response(Response::Found {
        start,
        end,
        wrapped,
    }))
}

fn flash(a: &App) -> Option<&str> {
    a.flash.as_ref().map(|(t, _)| t.as_str())
}

fn render(a: &App) -> ratatui::buffer::Buffer {
    let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
    t.draw(|f| draw(f, a)).unwrap();
    t.backend().buffer().clone()
}

fn bar(a: &App) -> String {
    let buf = render(a);
    (0..80).map(|x| buf[(x, 23)].symbol().to_string()).collect()
}

#[test]
fn the_finder_uses_smart_case() {
    let row = cells(Some(&line("Error: an error, ERROR")), 40);
    assert_eq!(find(&row, "error"), [(0, 4), (10, 14), (17, 21)]);
    assert_eq!(find(&row, "Error"), [(0, 4)]);
    assert_eq!(find(&row, "eRror"), Vec::<(u16, u16)>::new());
    assert_eq!(find(&row, ""), Vec::<(u16, u16)>::new());
}

#[test]
fn the_finder_counts_wide_cells_and_keeps_combining_marks_with_their_base() {
    let row = cells(Some(&line("日本語 の e\u{301}clair")), 40);
    // Each wide character covers two columns; a match ends on the last of them.
    assert_eq!(find(&row, "本語"), [(2, 5)]);
    // `e` alone is not a match inside `é`, `é` is.
    assert_eq!(find(&row, "e"), Vec::<(u16, u16)>::new());
    assert_eq!(find(&row, "e\u{301}c"), [(10, 11)]);
    // Blanks inside the row count; the trailing ones do not.
    let row = cells(Some(&line("a b   ")), 40);
    assert_eq!(find(&row, "a b"), [(0, 2)]);
    assert_eq!(find(&row, "b "), Vec::<(u16, u16)>::new());
}

#[test]
fn slash_and_question_mark_open_the_prompt_in_the_bar() {
    let mut a = scrolling(&[]);
    press(&mut a, &[key('?')]);
    typed(&mut a, "error");
    let b = bar(&a);
    assert!(b.contains("?error█"), "{b}");
    assert!(!b.contains("SCROLL"), "{b}");
    press(&mut a, &[code(ct::KeyCode::Esc)]);
    assert!(a.search.prompt.is_none());
    assert_eq!(a.scroll_mode, Some(T), "Esc cancels the prompt only");
    press(&mut a, &[key('/')]);
    typed(&mut a, "x");
    assert!(bar(&a).contains("/x█"), "{}", bar(&a));
}

#[test]
fn the_prompt_edits_like_the_other_prompts() {
    let mut a = scrolling(&[]);
    press(&mut a, &[key('/')]);
    typed(&mut a, "foo bar");
    press(&mut a, &[ctrl('w')]);
    assert_eq!(a.search.prompt.as_ref().unwrap().editor.text(), "foo ");
    press(
        &mut a,
        &[code(ct::KeyCode::Backspace), code(ct::KeyCode::Left)],
    );
    typed(&mut a, "X");
    assert_eq!(a.search.prompt.as_ref().unwrap().editor.text(), "foXo");
    press(&mut a, &[ctrl('u')]);
    assert_eq!(a.search.prompt.as_ref().unwrap().editor.text(), "");
    // Keys do not reach the terminal or the prefix's bindings.
    let outs = press(&mut a, &[key('q')]);
    assert!(outs.is_empty());
    assert_eq!(a.scroll_mode, Some(T));
    a.on_event(ct::Event::Paste("a\nb".into()), Instant::now());
    assert_eq!(a.search.prompt.as_ref().unwrap().editor.text(), "qab");
}

#[test]
fn enter_asks_the_daemon_from_the_copy_cursor() {
    let mut a = scrolling(&[]);
    let at = a.copy.as_ref().unwrap().cursor;
    press(&mut a, &[key('?')]);
    typed(&mut a, "error");
    let outs = press(&mut a, &[code(ct::KeyCode::Enter)]);
    assert_eq!(outs, [search_request("error", at, true)]);
    assert!(a.search.prompt.is_none());
    // Down with `/`.
    let mut b = scrolling(&[]);
    press(&mut b, &[key('/')]);
    typed(&mut b, "Error");
    let outs = press(&mut b, &[code(ct::KeyCode::Enter)]);
    assert_eq!(outs, [search_request("Error", at, false)]);
}

#[test]
fn an_empty_enter_repeats_the_last_query_in_the_new_direction() {
    let mut a = scrolling(&[]);
    let at = a.copy.as_ref().unwrap().cursor;
    // Nothing to repeat yet.
    press(&mut a, &[key('/')]);
    assert!(press(&mut a, &[code(ct::KeyCode::Enter)]).is_empty());
    press(&mut a, &[key('?')]);
    typed(&mut a, "foo");
    press(&mut a, &[code(ct::KeyCode::Enter)]);
    found(&mut a, pos(5, 2), pos(5, 4), false);
    let outs = press(&mut a, &[key('/'), code(ct::KeyCode::Enter)]);
    assert_eq!(outs, [search_request("foo", pos(5, 2), false)]);
    assert_ne!(at, pos(5, 2));
}

#[test]
fn a_found_match_moves_the_cursor_and_the_view_and_a_wrap_is_flashed() {
    let mut a = scrolling(&[]);
    press(&mut a, &[key('?')]);
    typed(&mut a, "foo");
    press(&mut a, &[code(ct::KeyCode::Enter)]);
    let outs = found(&mut a, pos(500, 3), pos(500, 5), false);
    assert_eq!(
        outs,
        [Out::Send(ClientMsg::Scroll {
            terminal: T,
            to: ScrollTo::Row(500)
        })]
    );
    let c = a.copy.as_ref().unwrap();
    assert_eq!(c.cursor, pos(500, 3));
    assert_eq!(flash(&a), None);
    press(&mut a, &[key('n')]);
    found(&mut a, pos(100, 0), pos(100, 2), true);
    assert_eq!(flash(&a), Some("Search wrapped"));
    assert_eq!(a.copy.as_ref().unwrap().cursor, pos(100, 0));
}

#[test]
fn a_missing_word_is_flashed() {
    let mut a = scrolling(&[]);
    press(&mut a, &[key('/')]);
    typed(&mut a, "zzz");
    press(&mut a, &[code(ct::KeyCode::Enter)]);
    let cursor = a.copy.as_ref().unwrap().cursor;
    let outs = a.on_server(ServerMsg::Response(Response::NotFound));
    assert!(outs.is_empty());
    assert_eq!(flash(&a), Some("Not found: zzz"));
    assert_eq!(a.copy.as_ref().unwrap().cursor, cursor);
}

#[test]
fn n_keeps_the_direction_and_capital_n_reverses_it() {
    let mut a = scrolling(&[]);
    let at = a.copy.as_ref().unwrap().cursor;
    // `n` before any search does nothing.
    assert!(press(&mut a, &[key('n'), key('N')]).is_empty());
    press(&mut a, &[key('?')]);
    typed(&mut a, "foo");
    press(&mut a, &[code(ct::KeyCode::Enter)]);
    found(&mut a, pos(500, 3), pos(500, 5), false);
    assert_eq!(
        press(&mut a, &[key('n')]),
        [search_request("foo", pos(500, 3), true)]
    );
    found(&mut a, pos(400, 0), pos(400, 2), false);
    assert_eq!(
        press(&mut a, &[key('N')]),
        [search_request("foo", pos(400, 0), false)]
    );
    found(&mut a, pos(500, 3), pos(500, 5), false);
    // `N` did not change the direction.
    assert_eq!(
        press(&mut a, &[key('n')]),
        [search_request("foo", pos(500, 3), true)]
    );
    assert_ne!(at, pos(500, 3));
}

#[test]
fn searches_typed_ahead_wait_for_the_answer_that_moves_the_cursor() {
    let mut a = scrolling(&[]);
    press(&mut a, &[key('?')]);
    typed(&mut a, "foo");
    let first = press(&mut a, &[code(ct::KeyCode::Enter)]);
    assert_eq!(first.len(), 1);
    assert!(press(&mut a, &[key('n')]).is_empty());
    let outs = found(&mut a, pos(500, 3), pos(500, 5), false);
    assert_eq!(outs.len(), 2);
    assert_eq!(outs[1], search_request("foo", pos(500, 3), true));
}

#[test]
fn a_search_answer_after_scroll_mode_ended_is_dropped() {
    let mut a = scrolling(&[]);
    press(&mut a, &[key('/')]);
    typed(&mut a, "foo");
    press(&mut a, &[code(ct::KeyCode::Enter), key('q')]);
    assert!(found(&mut a, pos(500, 3), pos(500, 5), false).is_empty());
    assert_eq!(a.scroll_mode, None);
}

#[test]
fn v_after_a_search_starts_at_the_match_and_a_selection_extends_to_it() {
    let mut a = scrolling(&[]);
    press(&mut a, &[key('?')]);
    typed(&mut a, "foo");
    press(&mut a, &[code(ct::KeyCode::Enter)]);
    found(&mut a, pos(500, 3), pos(500, 5), false);
    // The view arrives, with the row on screen.
    let mut f = screen(&[]);
    f.top_row = 490;
    f.display_offset = 510;
    f.history = 600;
    a.on_server(ServerMsg::Frame(f));
    press(&mut a, &[key('v')]);
    let sel = a.text_sel.as_ref().unwrap();
    assert_eq!((sel.anchor, sel.head), (pos(500, 3), pos(500, 3)));
    // A selection started before the search extends to the next match.
    press(&mut a, &[key('n')]);
    found(&mut a, pos(450, 1), pos(450, 3), false);
    let sel = a.text_sel.as_ref().unwrap();
    assert_eq!((sel.anchor, sel.head), (pos(500, 3), pos(450, 1)));
}

#[test]
fn escape_clears_a_selection_then_the_highlights_then_leaves() {
    let mut a = scrolling(&["foo bar"]);
    press(&mut a, &[key('/')]);
    typed(&mut a, "foo");
    press(&mut a, &[code(ct::KeyCode::Enter)]);
    found(&mut a, pos(LIVE, 0), pos(LIVE, 2), false);
    press(&mut a, &[key('v'), key('l')]);
    let esc = code(ct::KeyCode::Esc);
    press(&mut a, std::slice::from_ref(&esc));
    assert!(a.text_sel.is_none());
    assert!(a.search.active.is_some());
    assert_eq!(a.scroll_mode, Some(T));
    press(&mut a, std::slice::from_ref(&esc));
    assert!(a.search.active.is_none());
    assert_eq!(a.scroll_mode, Some(T));
    press(&mut a, &[esc]);
    assert_eq!(a.scroll_mode, None);
}

#[test]
fn the_matches_are_drawn_in_the_search_colours_and_the_current_one_bold() {
    let mut a = scrolling(&["error one", "an Error, error"]);
    press(&mut a, &[key('/')]);
    typed(&mut a, "error");
    press(&mut a, &[code(ct::KeyCode::Enter)]);
    found(&mut a, pos(LIVE, 0), pos(LIVE, 4), false);
    let buf = render(&a);
    let t = &a.theme;
    let plain = (color(t.search_match_fg), color(t.search_match_bg));
    let current = (color(t.search_current_fg), color(t.search_current_bg));
    let style = |x, y| (buf[(x, y)].fg, buf[(x, y)].bg);
    // Row 0: the current match; row 1: `Error` and `error`, smart case finds both.
    // (The copy cursor, in the accent colour, is on the first cell.)
    for x in 1..5 {
        assert_eq!(style(x, 0), current, "x {x}");
        assert!(buf[(x, 0)].modifier.contains(Modifier::BOLD));
    }
    assert_ne!(style(5, 0), current);
    for x in (3..8).chain(10..15) {
        assert_eq!(style(x, 1), plain, "x {x}");
        assert!(!buf[(x, 1)].modifier.contains(Modifier::BOLD));
    }
    assert_ne!(style(8, 1), plain);
}

#[test]
fn a_capital_in_the_query_matches_only_that_case() {
    let mut a = scrolling(&["an Error, error"]);
    press(&mut a, &[key('/')]);
    typed(&mut a, "Error");
    press(&mut a, &[code(ct::KeyCode::Enter)]);
    found(&mut a, pos(LIVE, 3), pos(LIVE, 7), false);
    let buf = render(&a);
    let plain = color(a.theme.search_match_bg);
    let current = color(a.theme.search_current_bg);
    assert_eq!(buf[(4, 0)].bg, current);
    assert_ne!(buf[(10, 0)].bg, plain);
    assert_ne!(buf[(10, 0)].bg, current);
}

#[test]
fn a_selections_colours_win_over_a_match() {
    let mut a = scrolling(&["foo foo"]);
    press(&mut a, &[key('/')]);
    typed(&mut a, "foo");
    press(&mut a, &[code(ct::KeyCode::Enter)]);
    found(&mut a, pos(LIVE, 0), pos(LIVE, 2), false);
    // Select the first two cells of the match.
    press(&mut a, &[key('v'), key('l')]);
    let buf = render(&a);
    let bg = color(a.theme.search_current_bg);
    assert!(buf[(0, 0)].modifier.contains(Modifier::REVERSED));
    assert_ne!(buf[(0, 0)].bg, bg);
    assert_ne!(buf[(1, 0)].bg, bg);
    // The rest of the match keeps its colours.
    assert_eq!(buf[(2, 0)].bg, bg);
    // The second `foo` is a plain match.
    assert_eq!(buf[(4, 0)].bg, color(a.theme.search_match_bg));
}

#[test]
fn nothing_is_highlighted_outside_scroll_mode_or_before_a_search() {
    let mut a = scrolling(&["foo"]);
    let buf = render(&a);
    assert_ne!(buf[(0, 0)].bg, color(a.theme.search_match_bg));
    press(&mut a, &[key('/')]);
    typed(&mut a, "foo");
    press(&mut a, &[code(ct::KeyCode::Enter), key('q')]);
    let buf = render(&a);
    assert_ne!(buf[(0, 0)].bg, color(a.theme.search_match_bg));
    assert!(a.search.active.is_none());
}

#[test]
fn the_help_lists_the_scroll_mode_keys_and_a_stale_answer_is_dropped() {
    let a = App::new(&no_sidebar(), (80, 24), &[]);
    let lines = crate::draw::help_lines(&a, &a.theme, 100);
    let text: String = lines.iter().map(|(l, _)| format!("{l}\n")).collect();
    let p = a.bindings.prefix_label();
    assert!(text.contains(&format!("{p} [ / ? n N")), "{text}");
    assert!(text.contains(&format!("{p} [ v V o y Enter")), "{text}");
    // The rows follow the binding, and are left out when it is unbound.
    let config = deck_core::config::Config::parse("[keys]\nscroll = \"S\"")
        .unwrap()
        .config;
    let b = crate::input::Bindings::from_config(&config);
    assert!(
        b.help_rows()
            .iter()
            .any(|r| r.keys == format!("{p} S h j k l"))
    );
    let config = deck_core::config::Config::parse("[keys]\nscroll = []")
        .unwrap()
        .config;
    let b = crate::input::Bindings::from_config(&config);
    assert!(
        !b.help_rows()
            .iter()
            .any(|r| r.text.starts_with("Scroll mode"))
    );

    // An answer that arrives after scroll mode ended and began again is not for the new cursor.
    let mut a = scrolling(&[]);
    press(&mut a, &[key('/')]);
    typed(&mut a, "foo");
    press(&mut a, &[code(ct::KeyCode::Enter), key('q')]);
    let now = Instant::now();
    a.on_event(
        ct::Event::Key(ct::KeyEvent::new(
            ct::KeyCode::Char(' '),
            ct::KeyModifiers::ALT,
        )),
        now,
    );
    a.on_event(key('['), now);
    assert_eq!(a.scroll_mode, Some(T));
    let cursor = a.copy.as_ref().unwrap().cursor;
    assert!(found(&mut a, pos(500, 3), pos(500, 5), false).is_empty());
    assert_eq!(a.copy.as_ref().unwrap().cursor, cursor);
}
