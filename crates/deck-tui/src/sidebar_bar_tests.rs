//! The sidebar's header and bottom bar: the `New space` chip at the top, the `New` and
//! `Restore` chips at the bottom, the hover state and the redraw rule.

use std::path::PathBuf;
use std::time::Instant;

use deck_core::config::Config;
use deck_core::model::{PanelId, ProjectId, SessionView};
use deck_core::protocol::{ClientMsg, Modes, ServerMsg};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event as ct;
use ratatui::style::Modifier;

use crate::app::{App, Out};
use crate::draw::{color, draw};
use crate::mirror::tests::{frame, mockup, panel, project};
use crate::modal::Modal;
use crate::sidebar::Hover;

fn app_at(size: (u16, u16), sidebar_width: u16, view: SessionView) -> App {
    let config = Config {
        sidebar_width,
        ..Config::default()
    };
    let mut app = App::new(&config, size, &[])
        .with_dirs(PathBuf::from("/work/proj"), Some(PathBuf::from("/home/me")));
    app.on_server(ServerMsg::SessionView(view));
    if let Some(terminal) = app.mirror.focused_id() {
        let mut f = frame(true, (40, 10), vec![]);
        f.terminal = terminal;
        app.on_server(ServerMsg::Frame(f));
    }
    app
}

fn app() -> App {
    app_at((100, 24), 28, mockup())
}

fn mouse(kind: ct::MouseEventKind, column: u16, row: u16) -> ct::Event {
    ct::Event::Mouse(ct::MouseEvent {
        kind,
        column,
        row,
        modifiers: ct::KeyModifiers::NONE,
    })
}

const LEFT: ct::MouseButton = ct::MouseButton::Left;

fn moved(a: &mut App, column: u16, row: u16) -> Vec<Out> {
    a.on_event(
        mouse(ct::MouseEventKind::Moved, column, row),
        Instant::now(),
    )
}

fn click(a: &mut App, column: u16, row: u16) -> Vec<Out> {
    let now = Instant::now();
    let mut outs = a.on_event(mouse(ct::MouseEventKind::Down(LEFT), column, row), now);
    outs.extend(a.on_event(mouse(ct::MouseEventKind::Up(LEFT), column, row), now));
    outs
}

fn draw_app(a: &App) -> Buffer {
    let mut t = Terminal::new(TestBackend::new(a.size.0, a.size.1)).unwrap();
    t.draw(|f| draw(f, a)).unwrap();
    t.backend().buffer().clone()
}

fn row_text(buf: &Buffer, y: u16, width: u16) -> String {
    (0..width).map(|x| buf[(x, y)].symbol()).collect()
}

/// The bar row of a 24-row window: the sidebar area is 23 rows tall.
const BAR: u16 = 22;
/// A column inside the header's `New space` chip of a 28-column sidebar (it takes 16..27).
const CHIP: u16 = 17;

#[test]
fn the_header_reads_spaces_and_holds_the_new_space_chip_at_the_right() {
    // (sidebar width, the left part as shown, the chip's glyph text)
    for (width, left, chip) in [
        (40, " Spaces", "+ New space"),
        (30, " Spaces", "+ New space"),
        (28, " Spaces", "+ New space"),
        (26, " Spaces", "+ New space"),
        (25, " Spaces", "+"),
        (20, " Spaces", "+"),
    ] {
        let a = app_at((100, 24), width, mockup());
        let buf = draw_app(&a);
        let text = row_text(&buf, 0, width);
        // The chip ends one cell before the separator: the margin.
        let at = usize::from(width) - 1 - chip.chars().count();
        let want = format!("{left:<at$}{chip} ");
        assert_eq!(text, want, "{width}");
    }
    let a = app();
    let buf = draw_app(&a);
    assert_eq!(buf[(1, 0)].fg, color(a.theme.accent));
    assert!(buf[(1, 0)].modifier.contains(Modifier::BOLD));
    assert_eq!(buf[(CHIP, 0)].fg, color(a.theme.muted));
}

#[test]
fn the_bar_shows_the_chip_at_the_left_and_the_restore_chip_at_the_right() {
    for (cols, width, label) in [
        (100, 20, " + "),
        (100, 25, " + "),
        (100, 26, " + New "),
        (100, 28, " + New "),
        (100, 60, " + New "),
    ] {
        let a = app_at((cols, 24), width, mockup());
        let buf = draw_app(&a);
        let text = row_text(&buf, BAR, width);
        let restore = if width < 26 { " ↻ " } else { " Restore " };
        let gap = usize::from(width) - 2 - label.len() - restore.chars().count();
        let expected = format!(" {label}{}{restore} ", " ".repeat(gap));
        assert_eq!(text, expected, "width {width}");
        // The spacer above is blank.
        assert_eq!(
            row_text(&buf, BAR - 1, width),
            " ".repeat(usize::from(width))
        );
    }
}

#[test]
fn the_chip_is_muted_when_idle_and_in_the_selection_colours_under_the_pointer() {
    let mut a = app();
    let idle = draw_app(&a);
    assert_eq!(idle[(2, BAR)].fg, color(a.theme.muted));
    assert_eq!(idle[(2, BAR)].bg, color(a.theme.sidebar_bg));
    moved(&mut a, 5, BAR);
    assert_eq!(a.hover, Some(Hover::New));
    let hot = draw_app(&a);
    for x in 1..8 {
        let cell = &hot[(x, BAR)];
        let reversed = cell.modifier.contains(Modifier::REVERSED);
        let themed =
            cell.bg == color(a.theme.selection_bg) && cell.fg == color(a.theme.selection_fg);
        assert!(reversed || themed, "column {x}: {cell:?}");
    }
    assert_ne!(hot[(2, BAR)], idle[(2, BAR)]);
    // The cell after the chip is untouched.
    assert_eq!(hot[(8, BAR)], idle[(8, BAR)]);
    moved(&mut a, 10, BAR);
    assert_eq!(a.hover, None);
    assert_eq!(draw_app(&a)[(2, BAR)], idle[(2, BAR)]);
}

#[test]
fn a_short_sidebar_has_no_bar_and_the_list_uses_the_full_height() {
    // 8 rows: the sidebar area is 7 tall, under the minimum of 8.
    let mut view = mockup();
    view.projects = vec![project(
        1,
        "big",
        false,
        (1..=30).map(|n| panel(n, &format!("p{n}"), 1)).collect(),
    )];
    view.active_project = Some(ProjectId(1));
    view.active_panel = Some(PanelId(1));
    let a = app_at((100, 8), 28, view.clone());
    let buf = draw_app(&a);
    assert!(!(0..7).any(|y| row_text(&buf, y, 28).contains("Restore")));
    assert!(
        row_text(&buf, 6, 28).contains("p4"),
        "{}",
        row_text(&buf, 6, 28)
    );
    assert_eq!(a.hover_at(5, 6), None);
    // 9 rows: area 8 tall, the bar appears and the list loses two rows.
    let a = app_at((100, 9), 28, view);
    let buf = draw_app(&a);
    assert!(row_text(&buf, 7, 28).contains("+ New"));
    assert!(
        row_text(&buf, 5, 28).contains("p3"),
        "{}",
        row_text(&buf, 5, 28)
    );
    assert_eq!(row_text(&buf, 6, 28), " ".repeat(28));
}

#[test]
fn the_list_scrolls_inside_a_list_area_two_rows_shorter() {
    let mut view = mockup();
    view.projects = (1..=30)
        .map(|n| project(n, &format!("proj{n}"), true, vec![]))
        .collect();
    view.active_project = Some(ProjectId(1));
    view.active_panel = None;
    let mut a = app_at((100, 24), 28, view);
    let buf = draw_app(&a);
    // The list shows rows 2..=20 (19 rows); the bar stays at the bottom.
    assert!(row_text(&buf, 20, 28).contains("proj19"));
    assert!(!row_text(&buf, 21, 28).contains("proj"));
    assert!(row_text(&buf, BAR, 28).contains("+ New"));
    for _ in 0..20 {
        a.on_event(mouse(ct::MouseEventKind::ScrollDown, 5, 5), Instant::now());
    }
    assert_eq!(a.sidebar_offset, 30 - 19);
    let buf = draw_app(&a);
    assert!(row_text(&buf, 20, 28).contains("proj30"));
    assert!(row_text(&buf, BAR, 28).contains("+ New"));
}

#[test]
fn clicking_the_header_chip_starts_the_new_project_flow() {
    let mut a = app();
    assert!(click(&mut a, CHIP, 0).is_empty());
    let Some(Modal::Prompt(p)) = &a.modal else {
        panic!("no prompt")
    };
    assert_eq!(
        (p.label(), p.editor.text()),
        ("Directory:", "/work/proj".to_string())
    );
    assert_eq!(a.hover, None);
    // A click on the header outside the chip, or on the blank row below, does nothing.
    let mut a = app();
    assert!(click(&mut a, 5, 0).is_empty());
    assert!(click(&mut a, CHIP, 1).is_empty());
    assert!(a.modal.is_none());
    // A press on the chip released elsewhere does nothing.
    let now = Instant::now();
    a.on_event(mouse(ct::MouseEventKind::Down(LEFT), CHIP, 0), now);
    a.on_event(mouse(ct::MouseEventKind::Up(LEFT), 40, 3), now);
    a.on_event(mouse(ct::MouseEventKind::Up(LEFT), CHIP, 0), now);
    assert!(a.modal.is_none());
}

#[test]
fn the_chip_works_without_sidebar_focus_and_not_while_an_overlay_is_open() {
    let mut a = app();
    assert!(!a.sidebar_focus);
    click(&mut a, CHIP, 0);
    assert!(matches!(a.modal, Some(Modal::Prompt(_))));
    // With the prompt open a click does nothing and the prompt is untouched.
    let before = a.modal.as_ref().map(|m| format!("{m:?}"));
    assert!(click(&mut a, CHIP, 0).is_empty());
    assert_eq!(a.modal.as_ref().map(|m| format!("{m:?}")), before);
    // Nor does it with a confirmation.
    let mut a = app();
    a.confirm_close_project(ProjectId(1));
    assert!(matches!(a.modal, Some(Modal::Confirm(_))));
    assert!(click(&mut a, CHIP, 0).is_empty());
    assert!(click(&mut a, 5, BAR).is_empty());
    assert!(matches!(a.modal, Some(Modal::Confirm(_))));
}

#[test]
fn a_move_that_keeps_the_hover_target_does_not_mark_the_screen_dirty() {
    let mut a = app();
    a.take_dirty();
    // Off the chip, over the list and the blank rows: no target, no redraw.
    for (x, y) in [(5, 3), (6, 3), (10, 15), (27, 21)] {
        assert!(moved(&mut a, x, y).is_empty());
        assert!(!a.take_dirty(), "({x}, {y})");
    }
    // Onto the chip: one redraw; further moves over it: none.
    moved(&mut a, CHIP, 0);
    assert!(a.take_dirty());
    for x in 16..27 {
        moved(&mut a, x, 0);
        assert!(!a.take_dirty(), "column {x}");
    }
    // Off it: one redraw.
    moved(&mut a, 8, 0);
    assert!(a.take_dirty());
    assert!(!{
        moved(&mut a, 7, 0);
        a.take_dirty()
    });
    // The `New` chip of the bar is a target of its own.
    moved(&mut a, 3, BAR);
    assert!(a.take_dirty());
    moved(&mut a, 4, BAR);
    assert!(!a.take_dirty());
    moved(&mut a, 60, 5);
    a.take_dirty();
    // Motion over the panel area with no hover to clear does not redraw either.
    moved(&mut a, 60, 5);
    assert!(!a.take_dirty());
}

#[test]
fn the_hover_clears_on_motion_elsewhere_focus_loss_overlays_clicks_and_wheels() {
    let now = Instant::now();
    let hover = |a: &mut App| {
        moved(a, CHIP, 0);
        assert_eq!(a.hover, Some(Hover::NewProject));
    };
    let mut a = app();
    hover(&mut a);
    moved(&mut a, 60, 5);
    assert_eq!(a.hover, None, "motion over the panel area");
    hover(&mut a);
    a.on_event(ct::Event::FocusLost, now);
    assert_eq!(a.hover, None, "focus loss");
    hover(&mut a);
    // A prompt opened by the keyboard.
    for key in [
        ct::KeyEvent::new(ct::KeyCode::Char(' '), ct::KeyModifiers::ALT),
        ct::KeyEvent::new(ct::KeyCode::Char('P'), ct::KeyModifiers::SHIFT),
    ] {
        a.on_event(ct::Event::Key(key), now);
    }
    assert!(a.overlay_open());
    assert_eq!(a.hover, None, "a prompt opening");
    let mut a = app();
    hover(&mut a);
    a.on_event(mouse(ct::MouseEventKind::Down(LEFT), 10, 3), now);
    assert_eq!(a.hover, None, "a click on a row");
    hover(&mut a);
    a.on_event(mouse(ct::MouseEventKind::ScrollDown, 80, 3), now);
    assert_eq!(a.hover, None, "a wheel over the panel area");
}

#[test]
fn the_hover_is_recomputed_after_a_wheel_scroll_a_resize_and_a_view() {
    let mut a = app();
    moved(&mut a, 3, BAR);
    a.take_dirty();
    // A wheel scroll of the sidebar keeps the hover under the still pointer.
    a.on_event(
        mouse(ct::MouseEventKind::ScrollDown, 3, BAR),
        Instant::now(),
    );
    assert_eq!(a.hover, Some(Hover::New));
    // A resize that moves the bar row away from the pointer clears it.
    a.take_dirty();
    a.on_event(ct::Event::Resize(100, 20), Instant::now());
    assert_eq!(a.hover, None);
    assert!(a.take_dirty());
    // A resize that moves the bar under the pointer sets it.
    moved(&mut a, 3, 18);
    assert_eq!(a.hover, Some(Hover::New));
    a.on_event(ct::Event::Resize(100, 24), Instant::now());
    assert_eq!(a.hover, None);
    // A view update recomputes from the pointer too.
    moved(&mut a, 3, BAR);
    a.take_dirty();
    a.on_server(ServerMsg::SessionView(mockup()));
    assert_eq!(a.hover, Some(Hover::New));
    // Rows that vanish under the pointer end it.
    let mut view = mockup();
    view.projects.clear();
    a.on_server(ServerMsg::SessionView(view));
    assert_eq!(
        a.hover,
        Some(Hover::New),
        "the bar does not move with the rows"
    );
}

#[test]
fn a_press_that_drifts_on_the_chip_still_clicks_and_one_released_off_it_does_not() {
    let now = Instant::now();
    let ev = |a: &mut App, kind, x, y| a.on_event(mouse(kind, x, y), now);
    let mut a = app();
    ev(&mut a, ct::MouseEventKind::Down(LEFT), CHIP, 0);
    ev(&mut a, ct::MouseEventKind::Drag(LEFT), 16, 0);
    ev(&mut a, ct::MouseEventKind::Up(LEFT), 16, 0);
    assert!(matches!(a.modal, Some(Modal::Prompt(_))));
    // Dragged off the chip and released off it: nothing.
    let mut a = app();
    ev(&mut a, ct::MouseEventKind::Down(LEFT), CHIP, 0);
    ev(&mut a, ct::MouseEventKind::Drag(LEFT), 5, 0);
    ev(&mut a, ct::MouseEventKind::Up(LEFT), 5, 0);
    assert!(a.modal.is_none());
    // A motion event between press and release ends the press.
    ev(&mut a, ct::MouseEventKind::Down(LEFT), CHIP, 0);
    ev(&mut a, ct::MouseEventKind::Moved, 14, 0);
    ev(&mut a, ct::MouseEventKind::Up(LEFT), 14, 0);
    assert!(a.modal.is_none());
}

#[test]
fn a_prompt_reopened_by_an_answer_ends_the_hover() {
    let mut a = app();
    a.open_new_project();
    a.hover = Some(Hover::NewProject);
    a.on_server(ServerMsg::Notice {
        level: deck_core::protocol::NoticeLevel::Info,
        text: "x".into(),
    });
    assert_eq!(a.hover, None);
}

#[test]
fn a_config_reload_that_narrows_the_sidebar_recomputes_the_hover() {
    let mut a = app();
    moved(&mut a, 16, 0);
    assert_eq!(a.hover, Some(Hover::NewProject));
    let config = Config {
        sidebar_width: 20,
        ..Config::default()
    };
    a.apply_config(config);
    assert_eq!(a.hover, None, "the chip is three cells now, at the right");
}

#[test]
fn motion_over_the_sidebar_never_reaches_a_program_but_motion_over_it_does() {
    let mut a = app();
    let mut f = frame(true, (40, 10), vec![]);
    f.terminal = a.mirror.focused_id().unwrap();
    f.modes = Modes(Modes::MOUSE_CLICK | Modes::MOUSE_MOTION | Modes::SGR_MOUSE);
    a.on_server(ServerMsg::Frame(f));
    for (x, y) in [(5, 3), (3, BAR), (CHIP, 0), (28, 3)] {
        assert!(moved(&mut a, x, y).is_empty(), "({x}, {y})");
    }
    let outs = moved(&mut a, 30, 2);
    assert!(
        matches!(outs.as_slice(), [Out::Send(ClientMsg::Input { .. })]),
        "{outs:?}"
    );
    assert_eq!(a.hover, None);
}

#[test]
fn the_chips_are_hit_where_they_are_drawn_at_any_width() {
    // (sidebar width, a column on the header chip, the column before it)
    for (width, on, before) in [(20, 18, 17), (28, 16, 15), (40, 28, 27)] {
        let mut a = app_at((100, 24), width, mockup());
        moved(&mut a, on, 0);
        assert_eq!(a.hover, Some(Hover::NewProject), "width {width}");
        moved(&mut a, before, 0);
        assert_eq!(a.hover, None, "width {width}");
    }
    // `+ New` takes columns 1..4 at 20 columns (` + `).
    let mut a = app_at((100, 24), 20, mockup());
    moved(&mut a, 3, BAR);
    assert_eq!(a.hover, Some(Hover::New));
    moved(&mut a, 4, BAR);
    assert_eq!(a.hover, None);
}
