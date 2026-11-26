//! The hover tooltip of a sidebar row whose text is cut: shown after `TIP_DELAY`, in full,
//! across the sidebar's edge, and taken down by anything but resting on the row.

use std::time::{Duration, Instant};

use deck_core::agents::{AgentKind, AgentStatus};
use deck_core::config::Config;
use deck_core::model::{PanelId, SessionView, TerminalId};
use deck_core::protocol::ServerMsg;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event as ct;

use crate::app::{App, TIP_DELAY};
use crate::draw::draw;
use crate::icons_tests::agent;
use crate::mirror::tests::{frame, mockup};

const NAME: &str = "This is a long session title that we could not display";
const ACTIVITY: &str = "running some long command in here with arguments";

/// `mockup()` with the `editor` panel (3, row 3; its activity line on row 4) renamed to
/// `name` and running an idle agent (no spinner timer) whose line is `activity`.
fn view(name: &str, activity: &str) -> SessionView {
    let mut view = mockup();
    let panel = view
        .projects
        .iter_mut()
        .flat_map(|p| &mut p.panels)
        .find(|p| p.id == PanelId(3))
        .unwrap();
    panel.name = name.to_string();
    let terminal = panel
        .terminals
        .iter_mut()
        .find(|t| t.id == TerminalId(31))
        .unwrap();
    let mut state = agent(AgentKind::Claude, AgentStatus::Idle);
    state.activity = Some(activity.to_string());
    terminal.agent = Some(state);
    view
}

fn app(view: SessionView) -> App {
    let config = Config {
        sidebar_width: 28,
        ..Config::default()
    };
    let mut app = App::new(&config, (100, 24), &[]);
    app.on_server(ServerMsg::SessionView(view));
    let mut f = frame(true, (40, 10), vec![]);
    f.terminal = TerminalId(31);
    app.on_server(ServerMsg::Frame(f));
    app
}

fn buffer(a: &App) -> Buffer {
    let mut t = Terminal::new(TestBackend::new(a.size.0, a.size.1)).unwrap();
    t.draw(|f| draw(f, a)).unwrap();
    t.backend().buffer().clone()
}

fn line(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
}

/// The column (in cells of one character each) where `needle` starts on `row`.
fn col(row: &str, needle: &str) -> Option<usize> {
    row.find(needle).map(|at| row[..at].chars().count())
}

/// Draws (the sidebar notes which texts it cut, as a real draw before any pointer move
/// does), then moves the pointer onto `row`.
fn hover(a: &mut App, row: u16, now: Instant) {
    buffer(a);
    a.on_event(
        ct::Event::Mouse(ct::MouseEvent {
            kind: ct::MouseEventKind::Moved,
            column: 8,
            row,
            modifiers: ct::KeyModifiers::NONE,
        }),
        now,
    );
}

/// Rests the pointer on `row` and lets `after` pass on the clock.
fn rest(a: &mut App, row: u16, start: Instant, after: Duration) -> Buffer {
    hover(a, row, start);
    a.tick(start + after);
    buffer(a)
}

/// The texts the tooltip shows on `rows`, joined by spaces. With `NAME` (54 cells) and
/// `ACTIVITY` from column 4, the box may be a quarter narrower than 55 cells: 41, so a line
/// holds 40 and the name takes rows 3 and 4, the activity rows 5 and 6. The longest line has
/// 38 cells, so the box ends at column 43: only what is inside it is read.
fn tip(buf: &Buffer, rows: std::ops::Range<u16>) -> String {
    rows.map(|y| {
        line(buf, y)
            .chars()
            .skip(4)
            .take(39)
            .collect::<String>()
            .trim()
            .to_string()
    })
    .filter(|t| !t.is_empty())
    .collect::<Vec<_>>()
    .join(" ")
}

#[test]
fn a_cut_row_is_shown_in_full_after_the_delay() {
    let mut a = app(view(NAME, ACTIVITY));
    let t0 = Instant::now();
    let before = rest(&mut a, 3, t0, TIP_DELAY - Duration::from_millis(1));
    assert!(line(&before, 3).contains('…'), "{}", line(&before, 3));
    assert!(!line(&before, 3).contains(NAME));
    a.tick(t0 + TIP_DELAY);
    let after = buffer(&a);
    assert_eq!(tip(&after, 3..5), NAME);
    // Its activity line expands with it under the name, from the same column as in the
    // sidebar.
    assert_eq!(tip(&after, 5..7), ACTIVITY);
    assert_eq!(col(&line(&after, 5), "running"), Some(4));
    assert_eq!(col(&line(&after, 3), "This"), Some(4));
}

#[test]
fn the_wake_deadline_is_the_tooltips_while_it_waits() {
    let mut a = app(view(NAME, ACTIVITY));
    let t0 = Instant::now();
    hover(&mut a, 3, t0);
    assert_eq!(a.wake_deadline(), Some(t0 + TIP_DELAY));
    a.tick(t0 + TIP_DELAY);
    assert!(a.wake_deadline().is_none_or(|at| at != t0 + TIP_DELAY));
}

#[test]
fn a_row_that_fits_has_no_tooltip() {
    let mut a = app(view("editor", "thinking"));
    let t0 = Instant::now();
    let plain = buffer(&a);
    hover(&mut a, 3, t0);
    // No timer runs for it: nothing to redraw.
    assert_eq!(a.wake_deadline(), None);
    let shown = rest(&mut a, 3, t0, TIP_DELAY);
    // Only the hover background differs from a draw without the pointer: the panel area
    // past the sidebar is untouched.
    let past = |b: &Buffer| line(b, 3).chars().skip(30).collect::<String>();
    assert_eq!(past(&shown), past(&plain));
}

#[test]
fn the_tooltip_follows_the_pointer_and_any_other_input_takes_it_down() {
    let mut a = app(view(NAME, ACTIVITY));
    let t0 = Instant::now();
    rest(&mut a, 3, t0, TIP_DELAY);
    assert_eq!(tip(&buffer(&a), 3..5), NAME);
    // Moving within the same row keeps it.
    hover(&mut a, 4, t0 + TIP_DELAY);
    assert_eq!(tip(&buffer(&a), 3..5), NAME);
    // A key takes it down; resting again on the row starts the delay over.
    a.on_event(
        ct::Event::Key(ct::KeyEvent::new(
            ct::KeyCode::Char('x'),
            ct::KeyModifiers::NONE,
        )),
        t0 + TIP_DELAY,
    );
    assert!(!line(&buffer(&a), 3).contains(NAME));
    // Another row: the delay starts over there and the first row's tooltip stays down.
    let t1 = t0 + TIP_DELAY * 2;
    rest(&mut a, 5, t1, TIP_DELAY / 2);
    assert!(!line(&buffer(&a), 3).contains(NAME));
}

#[test]
fn leaving_the_row_before_the_delay_shows_nothing() {
    let mut a = app(view(NAME, ACTIVITY));
    let t0 = Instant::now();
    hover(&mut a, 3, t0);
    buffer(&a);
    hover(&mut a, 20, t0 + TIP_DELAY / 2);
    a.tick(t0 + TIP_DELAY);
    assert!(!line(&buffer(&a), 3).contains(NAME));
}

/// The words of `row` from column `from` on, trimmed.
fn text_from(row: &str, from: usize) -> String {
    row.chars()
        .skip(from)
        .collect::<String>()
        .trim()
        .to_string()
}

#[test]
fn a_long_text_wraps_at_words_within_sixty_columns() {
    // Texts from column 4: the box is 60 columns at most, so a line holds 59.
    let long = "word ".repeat(30);
    let long = long.trim();
    let mut a = app(view(long, ACTIVITY));
    let shown = rest(&mut a, 3, Instant::now(), TIP_DELAY);
    let mut lines = Vec::new();
    for y in 3..6 {
        let row = line(&shown, y);
        let text = text_from(&row, 4);
        assert!(text.len() <= 59, "{y}: {row}");
        assert!(text.split(' ').all(|w| w == "word"), "{y}: {row}");
        lines.push(text);
    }
    assert_eq!(lines.join(" "), long);
    // The activity line follows the name's last line, from its own column.
    assert_eq!(
        col(&line(&shown, 6), ACTIVITY),
        Some(4),
        "{}",
        line(&shown, 6)
    );
    // Past the box the panels show again.
    assert!(line(&shown, 3).chars().skip(64).all(|c| c != 'w'));
}

#[test]
fn the_tooltip_shows_five_lines_at_most_and_ends_with_an_ellipsis() {
    let long = "word ".repeat(100);
    let mut a = app(view(long.trim(), ACTIVITY));
    let shown = rest(&mut a, 3, Instant::now(), TIP_DELAY);
    for y in 3..7 {
        assert!(!line(&shown, y).contains('…'), "{y}: {}", line(&shown, y));
    }
    let last = text_from(&line(&shown, 7), 4);
    assert!(last.ends_with("word…"), "{last}");
    // The activity line has no room left.
    assert!(!line(&shown, 8).contains(ACTIVITY));
}

#[test]
fn a_tooltip_near_the_bottom_is_moved_up_into_the_window() {
    let long = "word ".repeat(30);
    let mut a = app(view(long.trim(), ACTIVITY));
    rest(&mut a, 3, Instant::now(), TIP_DELAY);
    // A window 6 rows tall: the 4 lines from row 3 would end past it, so they start at row 2.
    let mut buf = Buffer::empty(ratatui::layout::Rect::new(0, 0, 100, 6));
    crate::sidebar::draw_tip(&mut buf, &a);
    assert!(line(&buf, 2).contains("word word"), "{}", line(&buf, 2));
    assert_eq!(col(&line(&buf, 5), ACTIVITY), Some(4), "{}", line(&buf, 5));
}

#[test]
fn the_tooltip_blanks_what_was_under_it() {
    let mut a = app(view(NAME, ACTIVITY));
    rest(&mut a, 3, Instant::now(), TIP_DELAY);
    // Whatever the panels drew under the box (text, image placeholders) must not show
    // through: draw the tooltip over a screen full of `Z`.
    let mut buf = Buffer::empty(ratatui::layout::Rect::new(0, 0, a.size.0, a.size.1));
    for y in 0..a.size.1 {
        buf.set_string(
            0,
            y,
            "Z".repeat(usize::from(a.size.0)),
            ratatui::style::Style::new(),
        );
    }
    crate::sidebar::draw_tip(&mut buf, &a);
    // The box runs a cell past the longest line, "running some long command in here with"
    // (38 cells), on all four lines. The hovered row keeps what it drew before its text; the
    // rows under it are blanked from the window's left.
    let right = 4 + 38 + 1;
    for y in 3..7 {
        let row = line(&buf, y);
        let from = if y == 3 { 4 } else { 0 };
        let inside: String = row.chars().skip(from).take(right - from).collect();
        assert!(!inside.contains('Z'), "{y}: {row}");
        assert_eq!(row.chars().nth(right), Some('Z'), "{y}: {row}");
    }
    assert!(line(&buf, 3).starts_with("ZZZZThis"), "{}", line(&buf, 3));
    assert_eq!(tip(&buf, 3..5), NAME);
    assert_eq!(tip(&buf, 5..7), ACTIVITY);
}

#[test]
fn a_name_deck_cut_when_it_made_it_is_shown_in_full() {
    // The daemon names the panel after its agent's title, cut to 32 cells.
    let title = "Homepage header glass tiles and the pinned rail layout";
    let kept: String = title.chars().take(31).collect();
    let mut v = view(&format!("{kept}…"), ACTIVITY);
    let panel = v
        .projects
        .iter_mut()
        .flat_map(|p| &mut p.panels)
        .find(|p| p.id == PanelId(3))
        .unwrap();
    panel.name_source = deck_core::model::NameSource::Auto;
    panel.terminals[0].title = Some(format!("✳ {title}"));
    let mut a = app(v);
    let shown = rest(&mut a, 3, Instant::now(), TIP_DELAY);
    assert_eq!(tip(&shown, 3..5), title);
    // A name the user gave stays as given.
    let mut v = view(&format!("{kept}…"), ACTIVITY);
    let panel = v
        .projects
        .iter_mut()
        .flat_map(|p| &mut p.panels)
        .find(|p| p.id == PanelId(3))
        .unwrap();
    panel.name_source = deck_core::model::NameSource::Manual;
    panel.terminals[0].title = Some(title.to_string());
    let mut a = app(v);
    let shown = rest(&mut a, 3, Instant::now(), TIP_DELAY);
    assert!(!line(&shown, 3).contains(title));
    assert!(line(&shown, 3).contains(&format!("{kept}…")));
}
