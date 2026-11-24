//! Copy mode through the client's state machine: the cursor and its motions, the keyboard
//! selection, the copy, the shared view and the drag past the screen's edge.

use std::time::{Duration, Instant};

use deck_core::model::TerminalId;
use deck_core::protocol::{ClientMsg, Frame, Pos, Request, Response, ScrollTo, ServerMsg};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event as ct;
use ratatui::style::Modifier;

use crate::app::{App, Out};
use crate::copy::{first_non_blank, last_non_blank, word_back, word_end, word_forward};
use crate::draw::draw;
use crate::mirror::tests::{frame, line, no_sidebar, view};
use crate::select::{Cell, Unit, cells};

const T: TerminalId = TerminalId(4);
const LEFT: ct::MouseButton = ct::MouseButton::Left;
/// The id of the first row of the live screen in these tests.
const LIVE: u64 = 1000;
const ROWS: u64 = 21;

fn screen(top: u64, offset: u32, history: u32, lines: &[&str]) -> Frame {
    let rows = lines
        .iter()
        .enumerate()
        .map(|(i, l)| (u16::try_from(i).unwrap(), line(l)))
        .collect();
    let mut f = frame(true, (78, 21), rows);
    f.top_row = top;
    f.display_offset = offset;
    f.history = history;
    f.cursor.row = 20;
    f.cursor.col = 0;
    f
}

/// A client in scroll mode on a terminal at the bottom of 300 rows of history, the cursor on
/// its last row.
fn scrolling(lines: &[&str]) -> App {
    let mut a = App::new(&no_sidebar(), (80, 24), &[]);
    a.on_server(ServerMsg::SessionView(view(None)));
    a.on_server(ServerMsg::Frame(screen(LIVE, 0, 300, lines)));
    let now = Instant::now();
    a.on_event(prefix(), now);
    a.on_event(ch('['), now);
    assert_eq!(a.scroll_mode, Some(T));
    a
}

fn prefix() -> ct::Event {
    ct::Event::Key(ct::KeyEvent::new(
        ct::KeyCode::Char(' '),
        ct::KeyModifiers::ALT,
    ))
}

fn ch(c: char) -> ct::Event {
    let mods = if c.is_uppercase() || "$^".contains(c) {
        ct::KeyModifiers::SHIFT
    } else {
        ct::KeyModifiers::NONE
    };
    ct::Event::Key(ct::KeyEvent::new(ct::KeyCode::Char(c), mods))
}

fn ctrl(c: char) -> ct::Event {
    ct::Event::Key(ct::KeyEvent::new(
        ct::KeyCode::Char(c),
        ct::KeyModifiers::CONTROL,
    ))
}

fn pressed(code: ct::KeyCode) -> ct::Event {
    ct::Event::Key(ct::KeyEvent::new(code, ct::KeyModifiers::NONE))
}

fn press(a: &mut App, events: &[ct::Event]) -> Vec<Out> {
    let now = Instant::now();
    events
        .iter()
        .flat_map(|e| a.on_event(e.clone(), now))
        .collect()
}

fn scroll(to: ScrollTo) -> Out {
    Out::Send(ClientMsg::Scroll { terminal: T, to })
}

fn text_request(from: (u64, u16), to: (u64, u16), lines: bool) -> Out {
    Out::Send(ClientMsg::Request(Request::Text {
        terminal: T,
        from: Pos {
            row: from.0,
            col: from.1,
        },
        to: Pos {
            row: to.0,
            col: to.1,
        },
        lines,
    }))
}

fn cursor(a: &App) -> (u64, u16) {
    let c = a.copy.as_ref().unwrap().cursor;
    (c.row, c.col)
}

fn answer(a: &mut App, text: &str, truncated: bool) -> Vec<Out> {
    a.on_server(ServerMsg::Response(Response::Text {
        text: text.into(),
        truncated,
    }))
}

fn clipboard(text: &str) -> Out {
    let b64 = deck_core::base64::encode(text.as_bytes());
    Out::Clipboard(format!("\x1b]52;c;{b64}\x07").into_bytes())
}

fn row(text: &str) -> Vec<Cell> {
    cells(Some(&line(text)), 40)
}

#[test]
fn word_motions_follow_the_mouse_word_rule() {
    // Words are runs of anything but blanks and brackets: `src/main.rs` is one.
    let rows = vec![row("cat src/main.rs | less"), row(""), row("(a) b")];
    assert_eq!(word_forward(&rows, (0, 0)), (0, 4));
    // The bar is no word: `less` is the next one.
    assert_eq!(word_forward(&rows, (0, 5)), (0, 18));
    // Over an empty row to the next row's first word.
    assert_eq!(word_forward(&rows, (0, 18)), (2, 1));
    assert_eq!(word_forward(&rows, (2, 1)), (2, 4));
    // Past the last word: below the screen.
    assert_eq!(word_forward(&rows, (2, 4)), (3, 0));

    assert_eq!(word_back(&rows, (0, 18)), (0, 4));
    assert_eq!(word_back(&rows, (0, 17)), (0, 4));
    assert_eq!(word_back(&rows, (0, 5)), (0, 4));
    assert_eq!(word_back(&rows, (2, 1)), (0, 18));
    assert_eq!(word_back(&rows, (0, 0)), (-1, 0));

    assert_eq!(word_end(&rows, (0, 0)), (0, 2));
    assert_eq!(word_end(&rows, (0, 2)), (0, 14));
    assert_eq!(word_end(&rows, (0, 14)), (0, 21));
    assert_eq!(word_end(&rows, (0, 21)), (2, 1));
    assert_eq!(word_end(&rows, (2, 4)), (3, 0));
}

#[test]
fn line_motions_find_the_ends_of_the_text() {
    let r = row("   ls -la   ");
    assert_eq!(first_non_blank(&r), 3);
    assert_eq!(last_non_blank(&r), 8);
    assert_eq!(first_non_blank(&row("")), 0);
    assert_eq!(last_non_blank(&row("")), 0);
    // A wide character ends on its first cell.
    assert_eq!(last_non_blank(&row("a界")), 1);
}

#[test]
fn entering_scroll_mode_puts_the_cursor_on_the_terminals_cursor() {
    let a = scrolling(&[]);
    assert_eq!(cursor(&a), (LIVE + 20, 0));
}

#[test]
fn horizontal_and_screen_motions_move_the_cursor_without_scrolling() {
    let mut a = scrolling(&[
        "one two", "", "", "", "", "", "", "", "", "", "", "", "", "", "", "", "", "", "", "", "",
    ]);
    assert!(press(&mut a, &[ch('H')]).is_empty());
    assert_eq!(cursor(&a), (LIVE, 0));
    assert!(press(&mut a, &[ch('l'), ch('l'), ch('l')]).is_empty());
    assert_eq!(cursor(&a), (LIVE, 3));
    assert!(press(&mut a, &[ch('h'), pressed(ct::KeyCode::Left)]).is_empty());
    assert_eq!(cursor(&a), (LIVE, 1));
    press(&mut a, &[ch('w')]);
    assert_eq!(cursor(&a), (LIVE, 4));
    press(&mut a, &[ch('e')]);
    assert_eq!(cursor(&a), (LIVE, 6));
    press(&mut a, &[ch('b'), ch('b')]);
    assert_eq!(cursor(&a), (LIVE, 0));
    press(&mut a, &[ch('$')]);
    assert_eq!(cursor(&a), (LIVE, 6));
    press(&mut a, &[ch('0')]);
    assert_eq!(cursor(&a), (LIVE, 0));
    press(&mut a, &[ch('M')]);
    assert_eq!(cursor(&a).0, LIVE + 10);
    press(&mut a, &[ch('L')]);
    assert_eq!(cursor(&a).0, LIVE + 20);
}

#[test]
fn k_at_the_top_edge_scrolls_exactly_one_line() {
    let mut a = scrolling(&[]);
    press(&mut a, &[ch('H')]);
    assert_eq!(press(&mut a, &[ch('k')]), [scroll(ScrollTo::Lines(1))]);
    assert_eq!(cursor(&a).0, LIVE - 1);
    // Without waiting for the frame, the next one scrolls one more and the cursor goes on.
    assert_eq!(press(&mut a, &[ch('k')]), [scroll(ScrollTo::Lines(1))]);
    assert_eq!(cursor(&a).0, LIVE - 2);
    // The frame brings the view to the cursor: it is on the top row.
    a.on_server(ServerMsg::Frame(screen(LIVE - 2, 2, 300, &[])));
    assert_eq!(press(&mut a, &[ch('k')]), [scroll(ScrollTo::Lines(1))]);
    assert_eq!(cursor(&a).0, LIVE - 3);
    // `j` moves the cursor first: from the top row it is inside the screen.
    a.on_server(ServerMsg::Frame(screen(LIVE - 3, 3, 300, &[])));
    assert!(press(&mut a, &[ch('j')]).is_empty());
    assert_eq!(cursor(&a).0, LIVE - 2);
}

#[test]
fn j_at_the_bottom_edge_of_a_scrolled_view_scrolls_one_line_down() {
    let mut a = scrolling(&[]);
    a.on_server(ServerMsg::Frame(screen(LIVE - 40, 40, 300, &[])));
    // The cursor is off screen below: it comes back to the last row, then `j` crosses.
    assert_eq!(press(&mut a, &[ch('j')]), [scroll(ScrollTo::Lines(-1))]);
    assert_eq!(cursor(&a).0, LIVE - 40 + ROWS);
}

#[test]
fn the_cursor_never_leaves_the_history() {
    let mut a = scrolling(&[]);
    a.on_server(ServerMsg::Frame(screen(LIVE - 300, 300, 300, &[])));
    press(&mut a, &[ch('H')]);
    assert!(press(&mut a, &[ch('k')]).is_empty());
    assert_eq!(cursor(&a).0, LIVE - 300);
    press(&mut a, &[ch('L'), ch('G')]);
    assert_eq!(cursor(&a).0, LIVE + 20);
}

#[test]
fn g_and_capital_g_jump_by_row_id() {
    let mut a = scrolling(&[]);
    assert_eq!(
        press(&mut a, &[ch('g')]),
        [scroll(ScrollTo::Row(LIVE - 300))]
    );
    assert_eq!(cursor(&a), (LIVE - 300, 0));
    assert_eq!(
        press(&mut a, &[ch('G')]),
        [scroll(ScrollTo::Row(LIVE + 20))]
    );
    assert_eq!(cursor(&a), (LIVE + 20, 0));
    // Its own scroll will reach the bottom: scroll mode stays when it does.
    a.on_server(ServerMsg::Frame(screen(LIVE - 40, 40, 300, &[])));
    press(&mut a, &[ch('G')]);
    a.on_server(ServerMsg::Frame(screen(LIVE, 0, 300, &[])));
    assert_eq!(a.scroll_mode, Some(T));
}

#[test]
fn half_pages_and_pages_scroll_at_once_and_carry_the_cursor() {
    let mut a = scrolling(&[]);
    press(&mut a, &[ch('M')]);
    assert_eq!(cursor(&a).0, LIVE + 10);
    assert_eq!(press(&mut a, &[ctrl('u')]), [scroll(ScrollTo::Lines(10))]);
    assert_eq!(cursor(&a).0, LIVE);
    assert_eq!(
        press(&mut a, &[pressed(ct::KeyCode::PageUp)]),
        [scroll(ScrollTo::Pages(1))]
    );
    assert_eq!(cursor(&a).0, LIVE - 21);
    assert_eq!(press(&mut a, &[ctrl('d')]), [scroll(ScrollTo::Lines(-10))]);
    assert_eq!(cursor(&a).0, LIVE - 11);
}

#[test]
fn v_selects_characters_and_y_asks_the_daemon_for_the_text() {
    let mut a = scrolling(&[]);
    press(&mut a, &[ch('H'), ch('v')]);
    let sel = a.text_sel.as_ref().unwrap();
    assert_eq!((sel.unit, sel.anchor.row), (Unit::Cell, LIVE));
    // Thirty `k` past the top edge, on a view that follows.
    for _ in 0..30 {
        press(&mut a, &[ch('k')]);
    }
    assert_eq!(cursor(&a).0, LIVE - 30);
    let sel = a.text_sel.as_ref().unwrap();
    assert_eq!((sel.anchor.row, sel.head.row), (LIVE, LIVE - 30));
    let outs = press(&mut a, &[ch('y')]);
    assert_eq!(outs, [text_request((LIVE - 30, 0), (LIVE, 0), false)]);
    // The selection is cleared at once; scroll mode stays.
    assert_eq!(a.text_sel, None);
    assert_eq!(a.scroll_mode, Some(T));
}

#[test]
fn the_answer_to_a_copy_goes_to_the_clipboard_and_the_bar_says_how_much() {
    let mut a = scrolling(&[]);
    press(&mut a, &[ch('v'), ch('y')]);
    assert_eq!(
        answer(&mut a, "one\ntwo\nthree", false),
        [clipboard("one\ntwo\nthree")]
    );
    assert_eq!(a.flash.as_ref().unwrap().0, "Copied 3 lines");
    assert!(!a.flash_error);
    press(&mut a, &[ch('y')]);
    answer(&mut a, "word", false);
    assert_eq!(a.flash.as_ref().unwrap().0, "Copied 4 characters");
    press(&mut a, &[ch('y')]);
    let outs = answer(&mut a, "x", true);
    assert_eq!(outs, [clipboard("x")]);
    assert_eq!(a.flash.as_ref().unwrap().0, "Copied the first 8 MiB");
    // An error is flashed.
    press(&mut a, &[ch('y')]);
    a.on_server(ServerMsg::Response(Response::Error(
        "the selection is no longer in the history".into(),
    )));
    assert_eq!(
        a.flash.as_ref().unwrap().0,
        "the selection is no longer in the history"
    );
    assert!(a.flash_error);
}

#[test]
fn capital_v_selects_lines_and_o_grows_the_other_end() {
    let mut a = scrolling(&[]);
    press(&mut a, &[ch('H'), ch('V')]);
    assert_eq!(a.text_sel.as_ref().unwrap().unit, Unit::Line);
    // Down to the bottom of the history.
    press(&mut a, &[ch('G')]);
    assert_eq!(
        press(&mut a, &[ch('y')]),
        [text_request((LIVE, 0), (LIVE + 20, u16::MAX), true)]
    );
    // `o` puts the cursor on the other end, `k` grows it from there.
    let mut a = scrolling(&[]);
    press(
        &mut a,
        &[ch('H'), ch('j'), ch('V'), ch('j'), ch('j'), ch('o')],
    );
    assert_eq!(cursor(&a).0, LIVE + 1);
    press(&mut a, &[ch('k')]);
    let outs = press(&mut a, &[ch('y')]);
    assert_eq!(outs, [text_request((LIVE, 0), (LIVE + 3, u16::MAX), true)]);
}

#[test]
fn o_scrolls_to_an_end_that_is_off_screen() {
    let mut a = scrolling(&[]);
    press(&mut a, &[ch('v'), ch('g')]);
    a.on_server(ServerMsg::Frame(screen(LIVE - 300, 300, 300, &[])));
    assert_eq!(
        press(&mut a, &[ch('o')]),
        [scroll(ScrollTo::Row(LIVE + 20))]
    );
    assert_eq!(cursor(&a).0, LIVE + 20);
}

#[test]
fn the_same_key_ends_a_selection_and_the_other_switches_its_kind() {
    let mut a = scrolling(&[]);
    press(&mut a, &[ch('v')]);
    assert!(a.text_sel.is_some());
    press(&mut a, &[ch('V')]);
    assert_eq!(a.text_sel.as_ref().unwrap().unit, Unit::Line);
    press(&mut a, &[ch('v')]);
    assert_eq!(a.text_sel.as_ref().unwrap().unit, Unit::Cell);
    press(&mut a, &[ch('v')]);
    assert_eq!(a.text_sel, None);
}

#[test]
fn y_without_a_selection_copies_the_cursors_line_and_enter_needs_one() {
    let mut a = scrolling(&[]);
    press(&mut a, &[ch('H'), ch('j')]);
    assert_eq!(
        press(&mut a, &[ch('y')]),
        [text_request((LIVE + 1, 0), (LIVE + 1, 0), true)]
    );
    assert!(press(&mut a, &[pressed(ct::KeyCode::Enter)]).is_empty());
    press(&mut a, &[ch('v'), ch('l')]);
    assert_eq!(
        press(&mut a, &[pressed(ct::KeyCode::Enter)]),
        [text_request((LIVE + 1, 0), (LIVE + 1, 1), false)]
    );
}

#[test]
fn escape_clears_a_selection_then_leaves() {
    let mut a = scrolling(&[]);
    press(&mut a, &[ch('v'), ch('h')]);
    assert!(press(&mut a, &[pressed(ct::KeyCode::Esc)]).is_empty());
    assert_eq!(a.text_sel, None);
    assert_eq!(a.scroll_mode, Some(T));
    assert_eq!(
        press(&mut a, &[pressed(ct::KeyCode::Esc)]),
        [scroll(ScrollTo::Bottom)]
    );
    assert_eq!(a.scroll_mode, None);
    assert_eq!(a.copy, None);
}

#[test]
fn another_clients_scroll_clamps_the_cursor_and_its_g_returns_to_the_bottom() {
    let mut a = scrolling(&[]);
    press(&mut a, &[ch('H'), ch('j')]);
    assert_eq!(cursor(&a).0, LIVE + 1);
    // Another client scrolls up 50 lines: the view is at the history's depth 50.
    a.on_server(ServerMsg::Frame(screen(LIVE - 50, 50, 300, &[])));
    // Still in scroll mode, the cursor keeps its id and is drawn on the nearest edge row.
    assert_eq!(a.scroll_mode, Some(T));
    assert_eq!(cursor(&a).0, LIVE + 1);
    assert_eq!(crate::copy::cursor_cell(&a, T), Some((20, 0)));
    // The next motion starts from that edge row.
    assert!(press(&mut a, &[ch('k')]).is_empty());
    assert_eq!(cursor(&a).0, LIVE - 50 + 19);
    // `G` returns to the bottom without leaving scroll mode.
    assert_eq!(
        press(&mut a, &[ch('G')]),
        [scroll(ScrollTo::Row(LIVE + 20))]
    );
    a.on_server(ServerMsg::Frame(screen(LIVE, 0, 300, &[])));
    assert_eq!(a.scroll_mode, Some(T));
}

#[test]
fn new_output_moves_the_rows_up_and_the_cursor_keeps_its_row() {
    let mut a = scrolling(&[]);
    press(&mut a, &[ch('H')]);
    a.on_server(ServerMsg::Frame(screen(LIVE + 5, 0, 305, &[])));
    assert_eq!(cursor(&a).0, LIVE);
    assert_eq!(crate::copy::cursor_cell(&a, T), Some((0, 0)));
}

#[test]
fn a_new_numbering_drops_the_selection_and_replaces_the_cursor() {
    let mut a = scrolling(&[]);
    press(&mut a, &[ch('H'), ch('v')]);
    let mut f = screen(5, 0, 0, &[]);
    f.numbering = 1;
    f.cursor.row = 3;
    a.on_server(ServerMsg::Frame(f));
    assert_eq!(a.text_sel, None);
    assert_eq!(cursor(&a), (8, 0));
    assert_eq!(a.scroll_mode, Some(T));
}

#[test]
fn the_cursor_is_drawn_in_the_accent_colour_over_a_selection() {
    let mut a = scrolling(&["hello world"]);
    press(&mut a, &[ch('H'), ch('v'), ch('l'), ch('l')]);
    let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
    t.draw(|f| draw(f, &a)).unwrap();
    let buf = t.backend().buffer();
    let accent = crate::draw::color(a.theme.accent);
    assert_eq!(buf[(2, 0)].bg, accent);
    assert!(!buf[(2, 0)].modifier.contains(Modifier::REVERSED));
    // The rest of the selection is reversed.
    assert!(buf[(0, 0)].modifier.contains(Modifier::REVERSED));
    assert!(buf[(1, 0)].modifier.contains(Modifier::REVERSED));
    assert!(!buf[(3, 0)].modifier.contains(Modifier::REVERSED));
}

#[test]
fn the_bar_shows_the_select_badge_with_the_size_of_the_selection() {
    let mut a = scrolling(&[]);
    let bar = |a: &App| {
        let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
        t.draw(|f| draw(f, a)).unwrap();
        let buf = t.backend().buffer().clone();
        (0..80)
            .map(|x| buf[(x, 23)].symbol().to_string())
            .collect::<String>()
    };
    assert!(
        bar(&a).contains(" SCROLL  q exit  v select  / search  y copy "),
        "{}",
        bar(&a)
    );
    press(&mut a, &[ch('H'), ch('v'), ch('j'), ch('j')]);
    let b = bar(&a);
    assert!(
        b.contains(" SELECT  3 lines  y copy  o other end  Esc clear "),
        "{b}"
    );
    press(&mut a, &[ch('k'), ch('k')]);
    assert!(bar(&a).contains(" SELECT  1 line  "), "{}", bar(&a));
}

/// Presses at the window cell, drags and releases it.
fn mouse(a: &mut App, kind: ct::MouseEventKind, at: (u16, u16), now: Instant) -> Vec<Out> {
    a.on_event(
        ct::Event::Mouse(ct::MouseEvent {
            kind,
            column: at.0,
            row: at.1,
            modifiers: ct::KeyModifiers::NONE,
        }),
        now,
    )
}

#[test]
fn a_mouse_selection_follows_its_rows_through_a_scroll_and_copies_the_same_text() {
    let mut a = scrolling(&["hello world", "second line"]);
    let now = Instant::now();
    mouse(&mut a, ct::MouseEventKind::Down(LEFT), (6, 0), now);
    mouse(&mut a, ct::MouseEventKind::Drag(LEFT), (5, 1), now);
    let outs = mouse(&mut a, ct::MouseEventKind::Up(LEFT), (5, 1), now);
    assert_eq!(outs, [clipboard("world\nsecond")]);
    // The wheel moves the view three lines up: the selection moves down with its rows.
    a.on_server(ServerMsg::Frame(screen(
        LIVE - 3,
        3,
        300,
        &["x", "y", "z", "hello world", "second line"],
    )));
    assert!(a.text_sel.is_some());
    let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
    t.draw(|f| draw(f, &a)).unwrap();
    let buf = t.backend().buffer();
    assert!(buf[(6, 3)].modifier.contains(Modifier::REVERSED));
    assert!(!buf[(6, 0)].modifier.contains(Modifier::REVERSED));
    // Back to where it was.
    let outs = mouse(&mut a, ct::MouseEventKind::ScrollDown, (5, 5), now);
    assert_eq!(outs, [scroll(ScrollTo::Lines(-3))]);
    a.on_server(ServerMsg::Frame(screen(
        LIVE,
        0,
        300,
        &["hello world", "second line"],
    )));
    assert!(a.text_sel.is_some());
    // `y` copies it through the daemon.
    assert_eq!(
        press(&mut a, &[ch('y')]),
        [text_request((LIVE, 6), (LIVE + 1, 5), false)]
    );
}

#[test]
fn a_mouse_selection_with_rows_off_screen_copies_through_the_daemon() {
    let mut a = scrolling(&["a", "b"]);
    let now = Instant::now();
    mouse(&mut a, ct::MouseEventKind::Down(LEFT), (0, 1), now);
    mouse(&mut a, ct::MouseEventKind::Drag(LEFT), (3, 1), now);
    // The view scrolls (the wheel) before the release; row 1's id is above the screen then.
    a.on_server(ServerMsg::Frame(screen(LIVE + 30, 0, 330, &["c"])));
    // A frame that moves the view keeps the selection as the rows are not on screen.
    assert!(a.text_sel.is_some());
    let outs = mouse(&mut a, ct::MouseEventKind::Up(LEFT), (3, 1), now);
    assert_eq!(outs, [text_request((LIVE + 1, 0), (LIVE + 1, 3), false)]);
    answer(&mut a, "abc", false);
    assert_eq!(a.flash.as_ref().unwrap().0, "Copied 3 characters");
}

#[test]
fn dragging_past_the_top_edge_scrolls_and_extends_the_selection() {
    let mut a = scrolling(&["abc"]);
    a.on_server(ServerMsg::Frame(screen(LIVE - 10, 10, 300, &["abc"])));
    let t0 = Instant::now();
    mouse(&mut a, ct::MouseEventKind::Down(LEFT), (1, 5), t0);
    assert_eq!(a.text_sel.as_ref().unwrap().anchor.row, LIVE - 5);
    // The pointer goes to the top row of the screen: the view scrolls one row up now.
    let outs = mouse(&mut a, ct::MouseEventKind::Drag(LEFT), (2, 0), t0);
    assert_eq!(outs, [scroll(ScrollTo::Lines(1))]);
    let head = a.text_sel.as_ref().unwrap().head;
    assert_eq!((head.row, head.col), (LIVE - 11, 2));
    // Held there it goes on every 50 ms, not sooner.
    assert!(a.tick(t0 + Duration::from_millis(49)).is_empty());
    assert_eq!(
        a.tick(t0 + Duration::from_millis(50)),
        [scroll(ScrollTo::Lines(1))]
    );
    assert_eq!(a.text_sel.as_ref().unwrap().head.row, LIVE - 12);
    assert_eq!(a.wake_deadline(), Some(t0 + Duration::from_millis(100)));
    // The release ends it and copies through the daemon: its rows are off screen.
    let outs = mouse(
        &mut a,
        ct::MouseEventKind::Up(LEFT),
        (2, 0),
        t0 + Duration::from_millis(60),
    );
    assert_eq!(outs, [text_request((LIVE - 12, 2), (LIVE - 5, 1), false)]);
    assert!(a.tick(t0 + Duration::from_millis(100)).is_empty());
}

#[test]
fn dragging_past_the_bottom_edge_scrolls_down() {
    let mut a = scrolling(&["abc"]);
    a.on_server(ServerMsg::Frame(screen(LIVE - 10, 10, 300, &["abc"])));
    let t0 = Instant::now();
    mouse(&mut a, ct::MouseEventKind::Down(LEFT), (1, 5), t0);
    let outs = mouse(&mut a, ct::MouseEventKind::Drag(LEFT), (1, 23), t0);
    assert_eq!(outs, [scroll(ScrollTo::Lines(-1))]);
    assert_eq!(a.text_sel.as_ref().unwrap().head.row, LIVE - 10 + ROWS);
    // Back inside the screen: no more scrolling.
    mouse(&mut a, ct::MouseEventKind::Drag(LEFT), (1, 8), t0);
    assert!(a.tick(t0 + Duration::from_secs(1)).is_empty());
}

#[test]
fn a_copied_wrapped_row_is_the_same_on_screen_and_off() {
    // Two rows that wrap into one line, then a third.
    let mut first = line("abcdef ");
    first.wrapped = true;
    let mut a = scrolling(&[]);
    let mut f = screen(LIVE, 0, 300, &[]);
    f.lines = vec![(0, first), (1, line("ghi  ")), (2, line("next"))];
    f.size = (7, 21);
    a.on_server(ServerMsg::Frame(f));
    let now = Instant::now();
    mouse(&mut a, ct::MouseEventKind::Down(LEFT), (0, 0), now);
    mouse(&mut a, ct::MouseEventKind::Drag(LEFT), (3, 2), now);
    let on_screen = mouse(&mut a, ct::MouseEventKind::Up(LEFT), (3, 2), now);
    // The wrapped row keeps its trailing blank and joins the next without a newline.
    assert_eq!(on_screen, [clipboard("abcdef ghi\nnext")]);
}

#[test]
fn quick_wheel_notches_back_to_the_bottom_keep_scroll_mode() {
    let mut a = scrolling(&[]);
    a.on_server(ServerMsg::Frame(screen(LIVE - 9, 9, 300, &[])));
    let now = Instant::now();
    for _ in 0..3 {
        let outs = mouse(&mut a, ct::MouseEventKind::ScrollDown, (5, 5), now);
        assert_eq!(outs, [scroll(ScrollTo::Lines(-3))]);
    }
    // The frames come one by one: the first two are still scrolled back.
    for (top, offset) in [(LIVE - 6, 6), (LIVE - 3, 3), (LIVE, 0)] {
        a.on_server(ServerMsg::Frame(screen(top, offset, 300, &[])));
        assert_eq!(a.scroll_mode, Some(T), "at offset {offset}");
    }
}

#[test]
fn an_edge_drag_stops_when_its_selection_is_gone() {
    let drag = |a: &mut App, t0: Instant| {
        mouse(a, ct::MouseEventKind::Down(LEFT), (1, 5), t0);
        mouse(a, ct::MouseEventKind::Drag(LEFT), (2, 0), t0);
        assert!(a.edge.is_some());
    };
    let t0 = Instant::now();
    let later = t0 + Duration::from_millis(500);
    // A frame that changes a selected row clears the selection and the drag.
    let mut a = scrolling(&["abc"]);
    a.on_server(ServerMsg::Frame(screen(LIVE - 10, 10, 300, &["abc"])));
    drag(&mut a, t0);
    let mut f = screen(LIVE - 10, 10, 300, &[]);
    f.full = false;
    f.lines = (0..21).map(|r| (r, line("changed"))).collect();
    a.on_server(ServerMsg::Frame(f));
    assert_eq!(a.text_sel, None);
    assert!(a.tick(later).is_empty());
    assert_eq!(a.edge, None);
    // So does Esc.
    let mut a = scrolling(&["abc"]);
    a.on_server(ServerMsg::Frame(screen(LIVE - 10, 10, 300, &["abc"])));
    drag(&mut a, t0);
    press(&mut a, &[pressed(ct::KeyCode::Esc)]);
    assert!(a.tick(later).is_empty());
    assert_eq!(a.edge, None);
}

#[test]
fn a_screen_row_is_an_edge_only_when_no_row_lies_beyond_it() {
    let mut a = scrolling(&["abc"]);
    a.on_server(ServerMsg::Frame(screen(LIVE - 10, 10, 300, &["abc"])));
    // The terminal touches the window's top: its first row is the edge.
    let t0 = Instant::now();
    mouse(&mut a, ct::MouseEventKind::Down(LEFT), (1, 5), t0);
    mouse(&mut a, ct::MouseEventKind::Drag(LEFT), (1, 0), t0);
    assert!(a.edge.is_some());
}
