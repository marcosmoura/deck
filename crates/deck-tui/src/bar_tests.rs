//! The floating bar: when it shows, what it shows, where it sits and what it covers, and
//! the sidebar header's session name and exited note.

use std::time::{Duration, Instant};

use deck_core::config::Config;
use deck_core::model::{SessionView, TerminalId};
use deck_core::protocol::{ClientMsg, Modes, NoticeLevel, ServerMsg};
use ratatui::crossterm::event as ct;
use ratatui::layout::Position;
use ratatui::style::Modifier;

use crate::app::{App, Out};
use crate::bar::{SCROLL_HINTS, cut};
use crate::draw::{color, width};
use crate::grid_tests::{app_with, ch, mouse, prefix, render, run};
use crate::mirror::tests::{frame, mockup};

const LEFT: ct::MouseButton = ct::MouseButton::Left;
/// The window's last row.
const LAST: usize = 23;

/// The 100×24 window of the grid tests: no sidebar, so the panel area is the window.
fn plain() -> App {
    app_with(1, 1)
}

fn flash(a: &mut App, text: &str, level: NoticeLevel) {
    a.on_server(ServerMsg::Notice {
        level,
        text: text.into(),
    });
}

#[test]
fn no_bar_when_nothing_is_going_on() {
    let a = plain();
    assert!(a.bar().is_none());
    let (_, _, rows) = render(&a);
    assert!(!rows[LAST].contains("PREFIX"));
}

#[test]
fn the_prefix_bar_is_centered_and_shows_the_cancel_and_help_hints() {
    let mut a = plain();
    run(&mut a, &[prefix()]);
    let (_, buf, rows) = render(&a);
    // Padding, ` PREFIX `, a space, `esc cancel  ? help`, padding: 29 cells.
    let text = "  PREFIX  esc cancel  ? help ";
    assert_eq!(width(text), 29);
    assert_eq!(a.bar().unwrap().rect.width, 29);
    // 100 - 29 = 71: 35 cells on the left, 36 on the right.
    let x: u16 = 35;
    assert_eq!(a.bar().unwrap().rect.x, x);
    let cells: String = rows[LAST].chars().skip(usize::from(x)).take(29).collect();
    assert_eq!(cells, text);
    // The badge: warning, reversed. The key: bold. The label: muted.
    let badge = &buf[(x + 1, 23)];
    assert_eq!(badge.fg, color(a.theme.warning));
    assert!(badge.modifier.contains(Modifier::REVERSED));
    let key = &buf[(x + 10, 23)];
    assert_eq!(key.symbol(), "e");
    assert!(key.modifier.contains(Modifier::BOLD));
    assert_eq!(key.fg, color(a.theme.status_fg));
    assert_eq!(buf[(x + 14, 23)].fg, color(a.theme.muted));
    assert_eq!(buf[(x, 23)].bg, color(a.theme.status_bg));
    // Escape ends it: the bar is gone on the next frame.
    run(
        &mut a,
        &[ct::Event::Key(ct::KeyEvent::new(
            ct::KeyCode::Esc,
            ct::KeyModifiers::NONE,
        ))],
    );
    assert!(a.bar().is_none());
    let (_, _, rows) = render(&a);
    assert!(!rows[LAST].contains("PREFIX"), "{}", rows[LAST]);
}

#[test]
fn the_prefix_bar_leaves_the_help_hint_out_when_help_is_unbound() {
    let mut config = Config::default();
    config.keys.remove("help");
    let mut a = plain();
    a.apply_config(config);
    run(&mut a, &[prefix()]);
    let (_, _, rows) = render(&a);
    assert!(
        rows[LAST].contains(" PREFIX  esc cancel "),
        "{}",
        rows[LAST]
    );
    assert!(!rows[LAST].contains("help"), "{}", rows[LAST]);
}

#[test]
fn each_state_has_its_badge_and_hints_and_the_precedence_holds() {
    let text = |a: &App| render(a).2[LAST].clone();
    let mut a = plain();
    // Resize repeat.
    a.resize_until = Some(Instant::now() + Duration::from_secs(1));
    assert!(
        text(&a).contains(" RESIZE  H J K L resize  = reset "),
        "{}",
        text(&a)
    );
    // Scroll mode over the repeat.
    a.scroll_mode = Some(TerminalId(31));
    assert!(
        text(&a).contains(" SCROLL  q exit  g top  G bottom "),
        "{}",
        text(&a)
    );
    assert!(!text(&a).contains("RESIZE"));
    // The sidebar focus over scroll mode? No: scroll mode comes first.
    a.sidebar_focus = true;
    assert!(text(&a).contains(" SCROLL "), "{}", text(&a));
    a.scroll_mode = None;
    assert!(text(&a).contains(" SIDEBAR  ↑↓ move"), "{}", text(&a));
    assert!(!text(&a).contains("RESIZE"));
    // The prefix over everything but the separator drag.
    a.armed = true;
    assert!(text(&a).contains(" PREFIX  esc cancel"), "{}", text(&a));
    a.armed = false;
    a.sidebar_focus = false;
    a.resize_until = None;
    assert!(a.bar().is_none());
}

#[test]
fn a_scrolled_back_terminal_shows_its_offset_in_warning() {
    let mut a = plain();
    let mut f = frame(true, (98, 22), vec![]);
    f.terminal = TerminalId(31);
    f.display_offset = 120;
    f.history = 4096;
    a.on_server(ServerMsg::Frame(f));
    // The wheel alone shows nothing.
    assert!(a.bar().is_none());
    a.scroll_mode = Some(TerminalId(31));
    let (_, buf, rows) = render(&a);
    assert!(
        rows[LAST].contains(" SCROLL  [120/4096]  q exit  g top  G bottom "),
        "{}",
        rows[LAST]
    );
    let at = rows[LAST].chars().position(|c| c == '[').unwrap();
    assert_eq!(
        buf[(u16::try_from(at).unwrap(), 23)].fg,
        color(a.theme.warning)
    );
}

#[test]
fn a_flash_replaces_the_hints_and_shows_alone_without_a_mode() {
    let mut a = plain();
    flash(&mut a, "config reloaded", NoticeLevel::Info);
    // Alone: no badge, centered, in the info colour.
    let (_, buf, rows) = render(&a);
    let w = width("config reloaded") + 2;
    let x = (100 - w) / 2;
    assert_eq!(a.bar().unwrap().rect.x, x);
    assert!(rows[LAST].contains(" config reloaded "), "{}", rows[LAST]);
    assert_eq!(buf[(x + 1, 23)].fg, color(a.theme.muted));
    // With a mode: the badge stays, the hints give way.
    run(&mut a, &[prefix()]);
    let (_, _, rows) = render(&a);
    assert!(
        rows[LAST].contains(" PREFIX  config reloaded "),
        "{}",
        rows[LAST]
    );
    assert!(!rows[LAST].contains("esc cancel"), "{}", rows[LAST]);
    // Each level has its colour.
    for (level, want) in [
        (NoticeLevel::Warning, a.theme.warning),
        (NoticeLevel::Error, a.theme.error),
    ] {
        a.flash = None;
        a.flash_level = None;
        flash(&mut a, "x", level);
        let (_, buf, _) = render(&a);
        let bar = a.bar().unwrap().rect;
        assert_eq!(
            buf[(bar.x + bar.width - 2, 23)].fg,
            color(want),
            "{level:?}"
        );
    }
}

#[test]
fn the_bar_is_centered_with_the_odd_remainder_on_the_right() {
    for (text, left) in [("ab", 48), ("abc", 47), ("abcd", 47), ("abcde", 46)] {
        let mut a = plain();
        flash(&mut a, text, NoticeLevel::Info);
        let rect = a.bar().unwrap().rect;
        // 100 columns: `(100 - width) / 2` on the left.
        assert_eq!(rect.x, left, "{text}");
        assert_eq!(rect.width, width(text) + 2);
        assert_eq!(rect.y, 23);
    }
}

#[test]
fn a_bar_too_wide_is_cut_with_an_ellipsis_and_stays_in_the_panel_area() {
    let mut a = crate::app::App::new(&Config::default(), (50, 10), &[]);
    a.on_server(ServerMsg::SessionView(mockup()));
    // The sidebar takes 29 columns: the panel area is 21 wide.
    a.scroll_mode = Some(TerminalId(4));
    let bar = a.bar().unwrap();
    assert_eq!(bar.rect.width, 21);
    assert_eq!(bar.rect.x, 29);
    let (_, _, rows) = {
        let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(50, 10)).unwrap();
        t.draw(|f| crate::draw::draw(f, &a)).unwrap();
        let buf = t.backend().buffer().clone();
        let rows: Vec<String> = (0..10)
            .map(|y| (0..50).map(|x| buf[(x, y)].symbol()).collect())
            .collect();
        ((), (), rows)
    };
    let last: String = rows[9].chars().skip(29).collect();
    assert!(last.starts_with("  SCROLL  q exit"), "{last}");
    assert!(last.ends_with("… "), "{last:?}");
    assert_eq!(width(&last), 21);
}

#[test]
fn cutting_keeps_the_ellipsis_in_the_last_cell() {
    let style = ratatui::style::Style::new();
    let segments = vec![("abc".to_string(), style), ("defg".to_string(), style)];
    let text = |segments: Vec<(String, ratatui::style::Style)>| -> String {
        segments.into_iter().map(|(t, _)| t).collect()
    };
    assert_eq!(text(cut(segments.clone(), 7)), "abcdefg");
    assert_eq!(text(cut(segments.clone(), 6)), "abcde…");
    // The room ends where a segment does: the last cell gives way.
    assert_eq!(text(cut(segments.clone(), 3)), "ab…");
    assert_eq!(text(cut(segments.clone(), 0)), "");
    assert_eq!(SCROLL_HINTS.len(), 3);
}

#[test]
fn the_bar_is_not_drawn_under_a_dialog_but_a_flash_is_over_other_overlays() {
    // A dialog shows the flash itself: no bar even with a mode and a flash.
    let mut a = plain();
    run(&mut a, &[prefix(), ch('.')]);
    assert!(
        matches!(a.modal, Some(crate::modal::Modal::Prompt(_))),
        "{:?}",
        a.modal
    );
    flash(&mut a, "oops", NoticeLevel::Warning);
    a.armed = true;
    assert!(a.bar().is_none());

    // The help overlay: the mode's bar is gone, a flash is drawn alone.
    let mut a = plain();
    run(&mut a, &[prefix(), ch('?')]);
    assert!(a.overlay_open());
    a.armed = true;
    a.scroll_mode = Some(TerminalId(31));
    assert!(a.bar().is_none());
    flash(&mut a, "hello", NoticeLevel::Info);
    let (_, _, rows) = render(&a);
    assert!(rows[LAST].contains(" hello "), "{}", rows[LAST]);
    assert!(!rows[LAST].contains("PREFIX"), "{}", rows[LAST]);

    // The session picker and the New menu are overlays too.
    let mut a = plain();
    a.armed = true;
    a.open_picker();
    assert!(a.bar().is_none());
    let mut a = plain();
    a.armed = true;
    a.open_new_menu(deck_core::model::ProjectId(1));
    assert!(a.bar().is_none());
}

#[test]
fn the_cursor_is_hidden_under_the_bar() {
    let mut a = plain();
    let mut f = frame(true, (98, 22), vec![]);
    f.terminal = TerminalId(31);
    f.cursor.row = 23;
    f.cursor.col = 50;
    f.cursor.visible = true;
    a.on_server(ServerMsg::Frame(f));
    let (mut t, _, _) = render(&a);
    t.backend_mut()
        .assert_cursor_position(Position::new(50, 23));
    flash(&mut a, "cover the cursor", NoticeLevel::Info);
    let (mut t, _, _) = render(&a);
    t.backend_mut().assert_cursor_position(Position::new(0, 0));
}

fn reporting(a: &mut App) {
    let mut f = frame(true, (100, 24), vec![]);
    f.terminal = TerminalId(31);
    f.modes = Modes(Modes::MOUSE_CLICK | Modes::MOUSE_MOTION | Modes::SGR_MOUSE);
    a.on_server(ServerMsg::Frame(f));
}

#[test]
fn mouse_events_on_the_bar_reach_no_program_and_start_nothing() {
    let mut a = plain();
    reporting(&mut a);
    let now = Instant::now();
    // Without a bar the press goes to the program.
    let outs = a.on_event(mouse(ct::MouseEventKind::Down(LEFT), 50, 23), now);
    assert!(
        matches!(outs.as_slice(), [Out::Send(ClientMsg::Input { .. })]),
        "{outs:?}"
    );
    a.on_event(mouse(ct::MouseEventKind::Up(LEFT), 50, 23), now);
    flash(&mut a, "a message that covers cells", NoticeLevel::Info);
    let rect = a.bar().unwrap().rect;
    let (x, y) = (rect.x + 3, rect.y);
    for kind in [
        ct::MouseEventKind::Down(LEFT),
        ct::MouseEventKind::Moved,
        ct::MouseEventKind::ScrollUp,
        ct::MouseEventKind::ScrollDown,
    ] {
        assert!(a.on_event(mouse(kind, x, y), now).is_empty(), "{kind:?}");
    }
    // Beside the bar the program still hears the press.
    let outs = a.on_event(mouse(ct::MouseEventKind::Down(LEFT), rect.x - 1, y), now);
    assert!(
        matches!(outs.as_slice(), [Out::Send(ClientMsg::Input { .. })]),
        "{outs:?}"
    );
    // A release on the bar still reaches the program that took the press.
    let outs = a.on_event(mouse(ct::MouseEventKind::Up(LEFT), x, y), now);
    assert!(
        matches!(outs.as_slice(), [Out::Send(ClientMsg::Input { .. })]),
        "{outs:?}"
    );

    // A program with no mouse reporting would get a selection: not on the bar.
    let mut a = plain();
    flash(&mut a, "a message that covers cells", NoticeLevel::Info);
    let rect = a.bar().unwrap().rect;
    a.on_event(mouse(ct::MouseEventKind::Down(LEFT), rect.x + 3, 23), now);
    assert!(a.text_sel.is_none());
    a.on_event(mouse(ct::MouseEventKind::Down(LEFT), rect.x - 1, 23), now);
    assert!(a.text_sel.is_some());
}

#[test]
fn a_selection_in_progress_continues_across_the_bar() {
    let mut a = plain();
    let now = Instant::now();
    a.on_event(mouse(ct::MouseEventKind::Down(LEFT), 10, 5), now);
    assert!(a.text_sel.is_some());
    flash(&mut a, "a message that covers cells", NoticeLevel::Info);
    let rect = a.bar().unwrap().rect;
    a.on_event(mouse(ct::MouseEventKind::Drag(LEFT), rect.x + 3, 23), now);
    let sel = a.text_sel.as_ref().unwrap();
    assert_eq!(sel.head, (rect.x + 3, 23));
}

#[test]
fn a_border_drag_shows_the_resize_bar_without_hints_and_crosses_the_bar() {
    let mut a = app_with(3, 1);
    let now = Instant::now();
    // The border between the big terminal and the other two.
    a.on_event(mouse(ct::MouseEventKind::Down(LEFT), 49, 5), now);
    assert!(a.drag.is_some());
    let (_, _, rows) = render(&a);
    assert!(rows[LAST].contains("  RESIZE "), "{}", rows[LAST]);
    assert!(!rows[LAST].contains("resize"), "{}", rows[LAST]);
    assert_eq!(a.bar().unwrap().rect.width, 10);
    // The drag goes on over the bar's row and ends with the release.
    let rect = a.bar().unwrap().rect;
    a.on_event(mouse(ct::MouseEventKind::Drag(LEFT), rect.x + 2, 23), now);
    assert!(a.drag.is_some());
    a.on_event(mouse(ct::MouseEventKind::Up(LEFT), rect.x + 2, 23), now);
    assert!(a.drag.is_none());
    assert!(a.bar().is_none());
}

#[test]
fn the_separator_drag_shows_the_width_in_the_badge() {
    let mut a = App::new(&Config::default(), (100, 24), &[]);
    a.on_server(ServerMsg::SessionView(mockup()));
    let now = Instant::now();
    a.on_event(mouse(ct::MouseEventKind::Down(LEFT), 28, 5), now);
    a.on_event(mouse(ct::MouseEventKind::Drag(LEFT), 40, 5), now);
    let (_, _, rows) = render(&a);
    assert!(rows[LAST].contains(" SIDEBAR 40 "), "{}", rows[LAST]);
    assert!(!rows[LAST].contains("esc"), "{}", rows[LAST]);
    a.on_event(mouse(ct::MouseEventKind::Up(LEFT), 40, 5), now);
    assert!(a.bar().is_none());
}

fn sidebar_app(name: &str, view: SessionView) -> App {
    let mut a = App::new(&Config::default(), (100, 24), &[]);
    let mut view = view;
    view.name = name.into();
    a.on_server(ServerMsg::SessionView(view));
    a
}

#[test]
fn the_header_shows_the_session_name_and_a_rename_updates_it_at_once() {
    let mut a = sidebar_app("work", mockup());
    let (_, buf, rows) = render(&a);
    assert!(rows[0].starts_with(" work "), "{}", rows[0]);
    assert_eq!(buf[(1, 0)].fg, color(a.theme.accent));
    assert!(buf[(1, 0)].modifier.contains(Modifier::BOLD));
    assert!(rows[0].contains("+ New space"));
    let mut view = mockup();
    view.name = "play".into();
    a.on_server(ServerMsg::SessionView(view));
    let (_, _, rows) = render(&a);
    assert!(rows[0].starts_with(" play "), "{}", rows[0]);
    // A long name is cut before the chip.
    let a = sidebar_app(
        "a-session-name-that-is-far-too-long-for-the-header",
        mockup(),
    );
    let (_, _, rows) = render(&a);
    assert!(rows[0].contains("… "), "{}", rows[0]);
    assert!(rows[0].contains("+ New space"), "{}", rows[0]);
    // Before the first session view the title is empty.
    let a = App::new(&Config::default(), (100, 24), &[]);
    let (_, _, rows) = render(&a);
    assert!(!rows[0].contains("Spaces"), "{}", rows[0]);
}

#[test]
fn a_kept_exited_terminal_alone_in_its_panel_notes_its_code_in_the_sidebar_row() {
    let mut view = mockup();
    view.projects[0].panels[0].terminals[0].keep = true;
    view.projects[0].panels[0].terminals[0].exit_status = Some(143);
    let a = sidebar_app("main", view);
    let (_, buf, rows) = render(&a);
    let row = rows.iter().find(|r| r.contains("editor")).unwrap();
    assert!(row.contains("editor · exited 143"), "{row}");
    let y = rows.iter().position(|r| r.contains("exited 143")).unwrap();
    let at = rows[y].chars().position(|c| c == '·').unwrap();
    let cell = &buf[(u16::try_from(at).unwrap(), u16::try_from(y).unwrap())];
    // The active row has the reversed band: the note keeps the row's text colour.
    assert_eq!(cell.fg, color(a.theme.sidebar_fg));
    // The name is cut first when the room is short.
    let mut view = mockup();
    view.projects[0].panels[0].name = "a-long-panel-name-for-the-row".into();
    view.projects[0].panels[0].terminals[0].keep = true;
    view.projects[0].panels[0].terminals[0].exit_status = Some(1);
    let a = sidebar_app("main", view);
    let (_, _, rows) = render(&a);
    let row = rows.iter().find(|r| r.contains("exited 1")).unwrap();
    assert!(row.contains("… · exited 1"), "{row}");
    // A panel of two terminals, or a running one, has no note.
    let a = sidebar_app("main", mockup());
    let (_, _, rows) = render(&a);
    assert!(!rows.iter().any(|r| r.contains("exited")));
}
