//! Resizing: the keys and their repeat (with an injected clock), the reset, the drag state
//! machine and the `RESIZE` badge.

use std::time::{Duration, Instant};

use deck_core::action::{Action as Act, ActionError};
use deck_core::config::Config;
use deck_core::keys::{KeyCode, KeyEvent, Modifiers};
use deck_core::layout::{self, Direction, Ratios, Rect};
use deck_core::model::{PanelId, TerminalId};
use deck_core::protocol::{ClientMsg, Modes, ServerMsg};
use deck_core::theme::Theme;
use ratatui::crossterm::event as ct;

use crate::app::{App, Out};
use crate::draw::color;
use crate::grid_tests::{SIZE, action, app_with, ch, left, mouse, prefix, render};
use crate::input::{Action, Bindings};
use crate::mirror::tests::frame;

fn shifted(c: char) -> ct::Event {
    ct::Event::Key(ct::KeyEvent::new(
        ct::KeyCode::Char(c),
        ct::KeyModifiers::SHIFT,
    ))
}

/// Events at one instant.
fn at(a: &mut App, now: Instant, events: &[ct::Event]) -> Vec<Out> {
    events
        .iter()
        .flat_map(|e| a.on_event(e.clone(), now))
        .collect()
}

fn set(panel: u64, n: usize, ratios: Ratios) -> Out {
    action(Act::SetRatios {
        panel: PanelId(panel),
        n,
        ratios,
    })
}

fn ratios(main: Option<u16>, cols: &[u16], rows: &[u16]) -> Ratios {
    Ratios {
        main,
        cols: cols.to_vec(),
        rows: rows.to_vec(),
    }
}

/// The app with the panel's stored ratios replaced, as a `SessionView` would.
fn with_ratios(mut a: App, r: Ratios) -> App {
    let mut view = a.mirror.view().unwrap().clone();
    view.projects[0].panels[0].ratios = r;
    a.on_server(ServerMsg::SessionView(view));
    a
}

fn input(terminal: u64, bytes: &[u8]) -> Out {
    Out::Send(ClientMsg::Input {
        terminal: TerminalId(terminal),
        bytes: bytes.to_vec(),
    })
}

const MS: fn(u64) -> Duration = Duration::from_millis;

#[test]
fn h_and_h_are_different_actions() {
    let b = Bindings::from_config(&Config::default());
    let plain = |c| KeyEvent::new(KeyCode::Char(c), Modifiers::NONE);
    let shift = |c| KeyEvent::new(KeyCode::Char(c), Modifiers::SHIFT);
    for (small, big, dir) in [
        ('h', 'H', Direction::Left),
        ('l', 'L', Direction::Right),
        ('k', 'K', Direction::Up),
        ('j', 'J', Direction::Down),
    ] {
        assert_eq!(b.lookup(&plain(small)), Some(Action::FocusTerminal(dir)));
        assert_eq!(b.lookup(&shift(big)), Some(Action::Resize(dir)));
    }
    assert_eq!(b.lookup(&plain('=')), Some(Action::ResetLayout));
    assert_eq!(b.resize_hints(), "H J K L resize  = reset");
}

/// The ratios of the one `SetRatios` in `out`.
fn sent(out: &[Out]) -> Ratios {
    let [Out::Send(ClientMsg::Action(Act::SetRatios { ratios, .. }))] = out else {
        panic!("{out:?}");
    };
    ratios.clone()
}

#[test]
fn a_resize_key_after_the_prefix_moves_a_border_by_five_percent() {
    let mut a = app_with(3, 1);
    let out = at(&mut a, Instant::now(), &[prefix(), shifted('L')]);
    assert_eq!(out, [set(3, 3, ratios(Some(550), &[1000], &[500, 500]))]);
    // Terminal 2: its left edge is the main border, so L moves it right (terminal 2 shrinks).
    let mut a = app_with(3, 2);
    let out = at(&mut a, Instant::now(), &[prefix(), shifted('L')]);
    assert_eq!(out, [set(3, 3, ratios(Some(550), &[1000], &[500, 500]))]);
    // J on terminal 2 moves the border between 2 and 3 down (the grid is 23 rows high: a
    // step of 5% is one row).
    let r = sent(&at(&mut a, Instant::now(), &[prefix(), shifted('J')]));
    assert_eq!(r.main, Some(500));
    assert!(r.rows[0] > 500, "{r:?}");
    // J on terminal 3: its top edge moves down, terminal 3 shrinks.
    let mut a = app_with(3, 3);
    let r = sent(&at(&mut a, Instant::now(), &[prefix(), shifted('J')]));
    assert!(r.rows[0] > 500, "{r:?}");
    // K and H go the other way.
    let r = sent(&at(&mut a, Instant::now(), &[prefix(), shifted('K')]));
    assert!(r.rows[0] < 500, "{r:?}");
    let mut a = app_with(3, 2);
    let out = at(&mut a, Instant::now(), &[prefix(), shifted('H')]);
    assert_eq!(out, [set(3, 3, ratios(Some(450), &[1000], &[500, 500]))]);
}

#[test]
fn a_single_terminal_has_nothing_to_resize() {
    let mut a = app_with(1, 1);
    assert!(at(&mut a, Instant::now(), &[prefix(), shifted('L')]).is_empty());
    assert!(a.resize_until.is_none());
    // The key is text, not a swallowed resize key.
    assert_eq!(
        at(&mut a, Instant::now(), &[shifted('L')]),
        [input(31, b"L")]
    );
}

#[test]
fn resize_keys_repeat_without_the_prefix_for_a_second() {
    let mut a = app_with(3, 1);
    let t0 = Instant::now();
    assert_eq!(at(&mut a, t0, &[prefix(), shifted('L')]).len(), 1);
    // Each resize key extends the repeat.
    assert_eq!(at(&mut a, t0 + MS(900), &[shifted('L')]).len(), 1);
    assert_eq!(at(&mut a, t0 + MS(1800), &[shifted('K')]).len(), 0);
    let _ = at(&mut a, t0 + MS(1800), &[shifted('L')]);
    // A second without a resize key ends it: the key is typed into the terminal.
    assert_eq!(
        at(&mut a, t0 + MS(2900), &[shifted('L')]),
        [input(31, b"L")]
    );
    assert_eq!(
        at(&mut a, t0 + MS(2901), &[shifted('L')]),
        [input(31, b"L")]
    );
}

#[test]
fn the_badge_shows_while_repeating_and_goes_with_the_deadline() {
    let mut a = app_with(3, 1);
    let t0 = Instant::now();
    at(&mut a, t0, &[prefix(), shifted('L')]);
    let (_, buf, rows) = render(&a);
    let theme = Theme::default();
    assert!(
        rows[23].starts_with(" RESIZE  terminal-multiplexer › ❯ tests"),
        "{}",
        rows[23]
    );
    assert!(
        rows[23].ends_with("H J K L resize  = reset"),
        "{}",
        rows[23]
    );
    assert_eq!(buf[(1, 23)].fg, color(theme.warning));
    assert!(
        buf[(1, 23)]
            .modifier
            .contains(ratatui::style::Modifier::REVERSED)
    );
    assert_eq!(a.wake_deadline(), Some(t0 + Duration::from_secs(1)));
    a.tick(t0 + MS(999));
    assert!(a.resize_until.is_some());
    a.tick(t0 + MS(1000));
    assert!(a.resize_until.is_none() && a.is_dirty());
    let (_, _, rows) = render(&a);
    assert!(rows[23].starts_with(" main  "), "{}", rows[23]);
}

#[test]
fn another_key_a_paste_or_a_focus_event_ends_the_repeat() {
    let t0 = Instant::now();
    for end in [
        ch('x'),
        ct::Event::Paste("pasted".into()),
        ct::Event::FocusLost,
        prefix(),
    ] {
        let mut a = app_with(3, 1);
        at(&mut a, t0, &[prefix(), shifted('L')]);
        let out = at(&mut a, t0 + MS(100), std::slice::from_ref(&end));
        assert!(a.resize_until.is_none(), "{end:?}");
        // The event itself is handled normally.
        match &end {
            ct::Event::Key(_) if a.armed => {}
            ct::Event::Key(_) => assert_eq!(out, [input(31, b"x")]),
            ct::Event::Paste(_) => assert_eq!(out, [input(31, b"\x1b[200~pasted\x1b[201~")]),
            // A focus event is told to the daemon only.
            _ => assert_eq!(out, [Out::Send(ClientMsg::Focus(false))]),
        }
        a.armed = false;
        assert_eq!(at(&mut a, t0 + MS(200), &[shifted('L')]), [input(31, b"L")]);
    }
}

#[test]
fn the_repeat_holds_at_the_limit_so_the_key_never_reaches_the_terminal() {
    // The main terminal is already as wide as the right-hand ones allow.
    let mut a = with_ratios(app_with(3, 1), ratios(Some(880), &[1000], &[500, 500]));
    let t0 = Instant::now();
    assert!(at(&mut a, t0, &[prefix(), shifted('L')]).is_empty());
    assert!(a.resize_until.is_some());
    assert!(at(&mut a, t0 + MS(100), &[shifted('L')]).is_empty());
    // The other way still works.
    assert_eq!(at(&mut a, t0 + MS(200), &[shifted('H')]).len(), 1);
}

#[test]
fn the_reset_sends_the_defaults_and_ends_the_repeat() {
    let mut a = with_ratios(app_with(3, 1), ratios(Some(700), &[1000], &[300, 700]));
    let t0 = Instant::now();
    assert_eq!(
        at(&mut a, t0, &[prefix(), ch('=')]),
        [set(3, 3, Ratios::default_for(3))]
    );
    assert!(a.resize_until.is_none());
    // Already the defaults: nothing to send.
    let mut a = app_with(3, 1);
    assert!(at(&mut a, t0, &[prefix(), ch('=')]).is_empty());
    // `=` also works inside the repeat.
    let mut a = with_ratios(app_with(3, 1), ratios(Some(700), &[1000], &[300, 700]));
    at(&mut a, t0, &[prefix(), shifted('L')]);
    assert_eq!(
        at(&mut a, t0 + MS(100), &[ch('=')]),
        [set(3, 3, Ratios::default_for(3))]
    );
}

#[test]
fn a_refused_set_ratios_shows_nothing_but_other_refusals_do() {
    let mut a = app_with(3, 1);
    at(&mut a, Instant::now(), &[prefix(), shifted('L')]);
    a.on_server(ServerMsg::ActionResult(Err(ActionError::LayoutChanged)));
    assert!(a.flash.is_none());
    assert!(a.pending.is_empty());
    at(&mut a, Instant::now(), &[prefix(), ch('t')]);
    a.on_server(ServerMsg::ActionResult(Err(ActionError::NoRoom)));
    assert!(a.flash.is_some());
}

// Drag. The 3-terminal grid in 100x23: terminal 1 is 0..50, the others 50..100, and the row
// border on the right is at y 11/12.

#[test]
fn dragging_the_main_border_follows_the_pointer_and_never_reaches_a_program() {
    let mut a = app_with(3, 2);
    // Terminal 1's program asked for the mouse.
    let mut f = frame(false, (48, 21), vec![]);
    f.terminal = TerminalId(31);
    f.modes = Modes(Modes::MOUSE_CLICK | Modes::MOUSE_DRAG | Modes::SGR_MOUSE);
    a.on_server(ServerMsg::Frame(f));
    let t0 = Instant::now();
    let (down, drag, up) = (
        ct::MouseEventKind::Down,
        ct::MouseEventKind::Drag,
        ct::MouseEventKind::Up,
    );
    // Press on terminal 1's own right border column.
    assert!(at(&mut a, t0, &[left(down, 49, 5)]).is_empty());
    assert!(a.drag.is_some());
    // The border is drawn in the accent while dragging.
    let theme = Theme::default();
    let (_, buf, _) = render(&a);
    assert_eq!(buf[(49, 5)].fg, color(theme.accent));
    assert_eq!(buf[(49, 17)].fg, color(theme.accent));
    // Moving by 10 cells moves the border by 10 cells (no jump on the grab).
    assert_eq!(
        at(&mut a, t0, &[left(drag, 59, 5)]),
        [set(3, 3, ratios(Some(600), &[1000], &[500, 500]))]
    );
    // Faster than every 33 ms: held back.
    assert!(at(&mut a, t0 + MS(10), &[left(drag, 69, 5)]).is_empty());
    assert_eq!(
        at(&mut a, t0 + MS(40), &[left(drag, 69, 5)]),
        [set(3, 3, ratios(Some(700), &[1000], &[500, 500]))]
    );
    // The release sends where the pointer ended, even inside the interval.
    assert_eq!(
        at(&mut a, t0 + MS(45), &[left(up, 74, 5)]),
        [set(3, 3, ratios(Some(750), &[1000], &[500, 500]))]
    );
    assert!(a.drag.is_none());
    // Nothing of it went to the program; a later press is a normal click again.
    let (_, buf, _) = render(&a);
    assert_ne!(buf[(49, 5)].fg, color(theme.accent));
}

#[test]
fn a_drag_stops_at_the_minimum_and_a_row_border_drags_vertically() {
    let mut a = app_with(3, 1);
    let t0 = Instant::now();
    let (down, drag, up) = (
        ct::MouseEventKind::Down,
        ct::MouseEventKind::Drag,
        ct::MouseEventKind::Up,
    );
    at(&mut a, t0, &[left(down, 50, 5)]);
    let out = at(&mut a, t0, &[left(drag, 99, 5)]);
    assert_eq!(out, [set(3, 3, ratios(Some(880), &[1000], &[500, 500]))]);
    at(&mut a, t0, &[left(up, 99, 5)]);

    let area = Rect::new(0, 0, SIZE.0, SIZE.1 - 1);
    let rects = layout::panel_rects(area, 3, &Ratios::default_for(3));
    let y = rects[2].y;
    at(&mut a, t0, &[left(down, 70, y)]);
    let ratios = sent(&at(&mut a, t0, &[left(drag, 70, y + 4)]));
    assert!(ratios.rows[0] > 500, "{ratios:?}");
    assert_eq!(ratios.main, Some(500));
}

#[test]
fn only_a_left_press_on_a_border_starts_a_drag() {
    let mut a = app_with(1, 1);
    let t0 = Instant::now();
    let down = ct::MouseEventKind::Down;
    at(&mut a, t0, &[left(down, 49, 5)]);
    assert!(a.drag.is_none(), "one terminal has no border");

    let mut a = app_with(3, 1);
    at(
        &mut a,
        t0,
        &[mouse(
            ct::MouseEventKind::Down(ct::MouseButton::Right),
            49,
            5,
        )],
    );
    assert!(a.drag.is_none());
    // Inside a terminal's screen: a click, not a drag.
    at(&mut a, t0, &[left(down, 20, 5)]);
    assert!(a.drag.is_none());
}

#[test]
fn a_lost_release_ends_the_drag_on_motion_a_press_or_an_overlay() {
    let t0 = Instant::now();
    let (down, drag) = (ct::MouseEventKind::Down, ct::MouseEventKind::Drag);
    // Hovering (no button) after a lost release.
    let mut a = app_with(3, 2);
    at(&mut a, t0, &[left(down, 49, 5)]);
    assert!(a.drag.is_some());
    assert!(at(&mut a, t0, &[mouse(ct::MouseEventKind::Moved, 60, 5)]).is_empty());
    assert!(a.drag.is_none());
    assert!(at(&mut a, t0 + MS(100), &[left(drag, 70, 5)]).is_empty());
    // A new press is handled, not eaten: here it focuses terminal 1.
    let mut a = app_with(3, 2);
    at(&mut a, t0, &[left(down, 49, 5)]);
    assert_eq!(
        at(&mut a, t0, &[left(down, 20, 5)]),
        [action(Act::FocusTerminal {
            terminal: TerminalId(31)
        })]
    );
    assert!(a.drag.is_none());
    // An overlay ends it.
    let mut a = app_with(3, 2);
    at(&mut a, t0, &[left(down, 49, 5)]);
    at(&mut a, t0, &[prefix(), ch('?')]);
    assert!(a.drag.is_none());
}

#[test]
fn the_main_border_highlight_leaves_the_grids_own_borders_alone() {
    let mut a = app_with(5, 5);
    let t0 = Instant::now();
    // The main border of 5 terminals is at column 50.
    at(&mut a, t0, &[left(ct::MouseEventKind::Down, 49, 5)]);
    assert!(a.drag.is_some());
    let theme = Theme::default();
    let (_, buf, _) = render(&a);
    assert_eq!(buf[(49, 5)].fg, color(theme.accent));
    // The border between columns 2 and 3 (x 74/75) keeps its colour.
    assert_ne!(buf[(74, 5)].fg, color(theme.accent));
    assert_ne!(buf[(75, 5)].fg, color(theme.accent));
}
