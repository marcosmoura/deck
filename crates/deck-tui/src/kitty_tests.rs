//! The kitty keyboard protocol in the client: the encoding a program that set flags gets, the
//! prefix, releases, and the flags pushed on the outer terminal.

use std::time::Instant;

use deck_core::model::TerminalId;
use deck_core::protocol::{ClientMsg, Modes, ServerMsg};
use ratatui::crossterm::event as ct;

use crate::app::{App, Out};
use crate::mirror::tests::{frame, no_sidebar, view};

fn app_with_flags(flags: u8, outer_kitty: bool) -> App {
    let mut app = App::new(&no_sidebar(), (80, 24), &[]);
    app.outer_kitty = outer_kitty;
    app.on_server(ServerMsg::SessionView(view(None)));
    let mut f = frame(true, (78, 21), vec![]);
    f.modes = Modes::default().with_kitty_flags(flags);
    app.on_server(ServerMsg::Frame(f));
    app
}

fn key(code: ct::KeyCode, mods: ct::KeyModifiers, kind: ct::KeyEventKind) -> ct::Event {
    ct::Event::Key(ct::KeyEvent::new_with_kind(code, mods, kind))
}

fn press(code: ct::KeyCode, mods: ct::KeyModifiers) -> ct::Event {
    key(code, mods, ct::KeyEventKind::Press)
}

fn input(bytes: &[u8]) -> Vec<Out> {
    vec![Out::Send(ClientMsg::Input {
        terminal: TerminalId(4),
        bytes: bytes.to_vec(),
    })]
}

#[test]
fn shift_enter_is_an_escape_code_for_a_program_with_flags_and_a_return_without() {
    let now = Instant::now();
    let shift_enter = || press(ct::KeyCode::Enter, ct::KeyModifiers::SHIFT);
    assert_eq!(
        app_with_flags(1, true).on_event(shift_enter(), now),
        input(b"\x1b[13;2u")
    );
    // The outer terminal does not matter for the encoding: what arrived is what is encoded.
    assert_eq!(
        app_with_flags(1, false).on_event(shift_enter(), now),
        input(b"\x1b[13;2u")
    );
    assert_eq!(
        app_with_flags(0, true).on_event(shift_enter(), now),
        input(b"\r")
    );
    // Ctrl+Backspace, Ctrl+Tab and Ctrl+I are told apart from Backspace, Tab and Tab.
    let mut a = app_with_flags(1, true);
    let ctrl = ct::KeyModifiers::CONTROL;
    assert_eq!(
        a.on_event(press(ct::KeyCode::Backspace, ctrl), now),
        input(b"\x1b[127;5u")
    );
    assert_eq!(
        a.on_event(press(ct::KeyCode::Tab, ctrl), now),
        input(b"\x1b[9;5u")
    );
    assert_eq!(
        a.on_event(press(ct::KeyCode::Char('i'), ctrl), now),
        input(b"\x1b[105;5u")
    );
    assert_eq!(
        a.on_event(press(ct::KeyCode::Tab, ct::KeyModifiers::NONE), now),
        input(b"\t")
    );
}

#[test]
fn releases_and_repeats_reach_a_program_only_when_it_asked() {
    let now = Instant::now();
    let a_key = |kind| key(ct::KeyCode::Char('a'), ct::KeyModifiers::NONE, kind);
    let mut a = app_with_flags(3, true);
    assert_eq!(a.on_event(a_key(ct::KeyEventKind::Press), now), input(b"a"));
    assert_eq!(
        a.on_event(a_key(ct::KeyEventKind::Repeat), now),
        input(b"a")
    );
    // A modified key's repeat is an event.
    let ctrl_a = |kind| key(ct::KeyCode::Char('a'), ct::KeyModifiers::CONTROL, kind);
    assert_eq!(
        a.on_event(ctrl_a(ct::KeyEventKind::Repeat), now),
        input(b"\x1b[97;5:2u")
    );
    assert_eq!(
        a.on_event(a_key(ct::KeyEventKind::Release), now),
        input(b"\x1b[97;1:3u")
    );
    // A release of a key whose press went nowhere is nobody's.
    assert!(a.on_event(a_key(ct::KeyEventKind::Release), now).is_empty());
    // Flag 1 alone: no release.
    let mut a = app_with_flags(1, true);
    a.on_event(a_key(ct::KeyEventKind::Press), now);
    assert!(a.on_event(a_key(ct::KeyEventKind::Release), now).is_empty());
    // No flags: the repeat is a press, the release nothing.
    let mut a = app_with_flags(0, true);
    assert_eq!(
        a.on_event(a_key(ct::KeyEventKind::Repeat), now),
        input(b"a")
    );
    assert!(a.on_event(a_key(ct::KeyEventKind::Release), now).is_empty());
}

#[test]
fn the_prefix_works_with_flags_and_twice_it_goes_to_the_program_in_its_encoding() {
    let now = Instant::now();
    let prefix = || press(ct::KeyCode::Char(' '), ct::KeyModifiers::ALT);
    for (flags, want) in [(0, &b"\x1b "[..]), (1, b"\x1b[32;3u"), (9, b"\x1b[32;3u")] {
        let mut a = app_with_flags(flags, true);
        assert!(a.on_event(prefix(), now).is_empty(), "{flags}");
        assert!(a.armed);
        // A second one sends the key and disarms.
        assert_eq!(a.on_event(prefix(), now), input(want), "{flags}");
        assert!(!a.armed);
        // A binding after the prefix still runs (the flash for a missing one shows it did).
        a.on_event(prefix(), now);
        a.on_event(press(ct::KeyCode::Char('!'), ct::KeyModifiers::NONE), now);
        assert!(!a.armed);
    }
}

#[test]
fn the_prefix_comes_through_when_the_outer_terminal_sends_it_as_an_escape_code() {
    // With every key reported, the Option key is its own event first, then Option+Space.
    let now = Instant::now();
    let mut a = app_with_flags(8, true);
    let option = ct::KeyCode::Modifier(ct::ModifierKeyCode::LeftAlt);
    assert_eq!(
        a.on_event(press(option, ct::KeyModifiers::ALT), now),
        input(b"\x1b[57443;3u")
    );
    assert!(
        a.on_event(press(ct::KeyCode::Char(' '), ct::KeyModifiers::ALT), now)
            .is_empty()
    );
    assert!(a.armed);
    // A modifier key while armed leaves the prefix armed and goes nowhere.
    assert!(
        a.on_event(
            press(
                ct::KeyCode::Modifier(ct::ModifierKeyCode::LeftShift),
                ct::KeyModifiers::SHIFT
            ),
            now
        )
        .is_empty()
    );
    assert!(a.armed);
}

#[test]
fn keys_only_kitty_reports_go_to_the_program() {
    let now = Instant::now();
    let mut a = app_with_flags(1, true);
    assert_eq!(
        a.on_event(press(ct::KeyCode::F(13), ct::KeyModifiers::NONE), now),
        input(b"\x1b[57376u")
    );
    assert_eq!(
        a.on_event(
            press(
                ct::KeyCode::Media(ct::MediaKeyCode::PlayPause),
                ct::KeyModifiers::NONE
            ),
            now
        ),
        input(b"\x1b[57430u")
    );
    let mut keypad = ct::KeyEvent::new(ct::KeyCode::Enter, ct::KeyModifiers::NONE);
    keypad.state = ct::KeyEventState::KEYPAD;
    assert_eq!(
        a.on_event(ct::Event::Key(keypad), now),
        input(b"\x1b[57414u")
    );
    // The program with no flags gets what the legacy encoding has, and nothing for the rest.
    let mut a = app_with_flags(0, true);
    assert!(
        a.on_event(press(ct::KeyCode::F(13), ct::KeyModifiers::NONE), now)
            .is_empty()
    );
    assert_eq!(a.on_event(ct::Event::Key(keypad), now), input(b"\r"));
}

#[test]
fn a_shifted_letter_the_outer_terminal_reports_as_the_shifted_key_keeps_its_modifier() {
    // Flag 4 on the outer terminal gives the shifted character without Shift.
    let now = Instant::now();
    let mut a = app_with_flags(8 | 4, true);
    assert_eq!(
        a.on_event(press(ct::KeyCode::Char('A'), ct::KeyModifiers::NONE), now),
        input(b"\x1b[97:65;2u")
    );
}

#[test]
fn the_outer_flags_follow_the_focused_terminal_and_the_overlays() {
    let now = Instant::now();
    let mut a = app_with_flags(1, true);
    assert_eq!(a.outer_keyboard(), 1);
    // The program moves to the events and every key.
    let mut f = frame(false, (78, 21), vec![]);
    f.modes = Modes::default().with_kitty_flags(2 | 8 | 16);
    a.on_server(ServerMsg::Frame(f));
    assert_eq!(a.outer_keyboard(), 1 | 2 | 4);
    // A terminal without flags: nothing pushed.
    let mut f = frame(false, (78, 21), vec![]);
    f.modes = Modes::default();
    a.on_server(ServerMsg::Frame(f));
    assert_eq!(a.outer_keyboard(), 0);
    // An overlay takes the keyboard.
    let mut a = app_with_flags(1, true);
    a.on_event(press(ct::KeyCode::Char(' '), ct::KeyModifiers::ALT), now);
    a.on_event(press(ct::KeyCode::Char('?'), ct::KeyModifiers::NONE), now);
    assert!(a.overlay_open());
    assert_eq!(a.outer_keyboard(), 0);
    // An outer terminal without the protocol gets nothing, whatever the program does.
    assert_eq!(app_with_flags(31, false).outer_keyboard(), 0);
}

#[test]
fn a_release_matches_its_press_without_case_and_dies_with_the_session() {
    let now = Instant::now();
    let mut a = app_with_flags(3, true);
    a.on_event(press(ct::KeyCode::Char('A'), ct::KeyModifiers::SHIFT), now);
    // The release reports the key without the Shift.
    let out = a.on_event(
        key(
            ct::KeyCode::Char('a'),
            ct::KeyModifiers::NONE,
            ct::KeyEventKind::Release,
        ),
        now,
    );
    assert_eq!(out, input(b"\x1b[97;1:3u"));
    // After a session reset a release is nobody's.
    let mut a = app_with_flags(3, true);
    a.on_event(press(ct::KeyCode::Char('a'), ct::KeyModifiers::NONE), now);
    a.reset_session();
    a.on_server(ServerMsg::SessionView(view(None)));
    let mut f = frame(true, (78, 21), vec![]);
    f.modes = Modes::default().with_kitty_flags(3);
    a.on_server(ServerMsg::Frame(f));
    let out = a.on_event(
        key(
            ct::KeyCode::Char('a'),
            ct::KeyModifiers::NONE,
            ct::KeyEventKind::Release,
        ),
        now,
    );
    assert!(out.is_empty());
}
