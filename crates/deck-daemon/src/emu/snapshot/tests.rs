use deck_core::model::TerminalId;
use std::time::Duration;

use deck_core::protocol::{CursorShape, Modes, ScrollTo};

use super::*;
use crate::emu::EmuEvent;

const ID: TerminalId = TerminalId(1);

/// A fresh emulator of the source's size that was fed the source's snapshot.
fn copy_of(source: &Emu) -> Emu {
    let (cols, rows) = source.size();
    let mut copy = Emu::new(cols, rows, source.scrollback).expect("emulator");
    let events = copy.feed(&source.snapshot());
    let replies: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, EmuEvent::Reply(_)))
        .collect();
    assert!(
        replies.is_empty(),
        "the snapshot made the emulator reply: {replies:?}"
    );
    copy
}

fn assert_same(source: &mut Emu, copy: &mut Emu, what: &str) {
    let a = source.full_frame(ID);
    let b = copy.full_frame(ID);
    assert_eq!(a, b, "{what}: the screens differ");
}

/// Feeds `input` into an emulator of 30 by 8, copies it, and compares the frames at the
/// bottom and at the top of the history.
fn round_trip(what: &str, input: &[u8]) -> (Emu, Emu) {
    let mut source = Emu::new(30, 8, 100).expect("emulator");
    source.feed(input);
    let mut copy = copy_of(&source);
    assert_same(&mut source, &mut copy, what);
    source.scroll(ScrollTo::Top);
    copy.scroll(ScrollTo::Top);
    assert_same(
        &mut source,
        &mut copy,
        &format!("{what} (top of the history)"),
    );
    source.scroll(ScrollTo::Bottom);
    copy.scroll(ScrollTo::Bottom);
    (source, copy)
}

/// Twenty lines of a shell session, so the first ones are history.
fn shell() -> Vec<u8> {
    let mut out = Vec::new();
    for n in 0..20 {
        out.extend_from_slice(
            format!(
                "\x1b[32muser@host\x1b[0m:\x1b[38;5;{}m~/dir{n}\x1b[0m$ \x1b[38;2;{};{};9mls -la\x1b[0m\r\n",
                16 + n * 9,
                n * 12,
                255 - n * 12
            )
            .as_bytes(),
        );
    }
    out.extend_from_slice(b"\x1b[48;5;236m\x1b[Kprompt $ ");
    out
}

#[test]
fn a_shell_session_with_colours_and_history() {
    round_trip("shell", &shell());
}

#[test]
fn every_style_flag_and_colour() {
    round_trip(
        "styles",
        b"\x1b[1mb\x1b[0;2md\x1b[0;3mi\x1b[0;4mu\x1b[0;4:3mc\x1b[0;7mr\x1b[0;9ms\x1b[0;8mh\x1b[0m \
          \x1b[1;3;4;31;44mall\x1b[0m \x1b[31ma\x1b[93mb\x1b[38;5;200mc\x1b[38;2;1;2;3md\x1b[0;48;5;17me\x1b[48;2;4;5;6mf\x1b[0m\r\n\
          \x1b[41m   \x1b[0m  \x1b[7;32mrev\x1b[0m",
    );
}

#[test]
fn a_soft_wrapped_line_and_a_hard_break() {
    let (mut source, mut copy) = round_trip(
        "wrap",
        format!("{}\r\nnext\r\n{}", "w".repeat(75), "z".repeat(30)).as_bytes(),
    );
    // A later resize reflows the copy as it does the source.
    source.resize(12, 8);
    copy.resize(12, 8);
    assert_same(&mut source, &mut copy, "wrap after a resize");
}

#[test]
fn a_200_column_line_in_80_columns_reflows_in_120() {
    let mut source = Emu::new(80, 24, 1000).expect("emulator");
    let line: String = (0..200)
        .map(|i| char::from(b'a' + (i % 26) as u8))
        .collect();
    source.feed(format!("head\r\n{line}\r\ntail").as_bytes());
    let mut copy = copy_of(&source);
    assert_same(&mut source, &mut copy, "80 columns");
    source.resize(120, 24);
    copy.resize(120, 24);
    assert_same(&mut source, &mut copy, "120 columns");
    source.resize(50, 24);
    copy.resize(50, 24);
    assert_same(&mut source, &mut copy, "50 columns");
}

#[test]
fn wide_characters_and_combining_marks() {
    round_trip(
        "wide",
        "日本語 🙂 e\u{301}x a\u{308}\u{323}\r\nカタカナ\r\n🙂🙂🙂🙂🙂🙂🙂🙂🙂🙂🙂🙂🙂🙂🙂🙂🙂\r\n\
         0123456789012345678901234567日本"
            .as_bytes(),
    );
    // A wide character that does not fit at the end of a line.
    round_trip(
        "wide at the edge",
        "1234567890123456789012345678日\r\nx".as_bytes(),
    );
}

#[test]
fn less_on_the_alternate_screen() {
    let mut input = shell();
    input.extend_from_slice(b"\x1b[3;7H\x1b7\x1b[?1049h\x1b[H\x1b[2J");
    for n in 1..=7 {
        input.extend_from_slice(format!("\x1b[{n};1Hfile line {n}\x1b[K").as_bytes());
    }
    input.extend_from_slice(b"\x1b[8;1H\x1b[7m(END)\x1b[0m\x1b[?25l");
    let (mut source, mut copy) = round_trip("alternate screen", &input);
    assert!(source.full_frame(ID).modes.has(Modes::ALTERNATE_SCREEN));
    // Leaving the alternate screen puts the primary screen and its cursor back.
    source.feed(b"\x1b[?1049l");
    copy.feed(b"\x1b[?1049l");
    assert_same(&mut source, &mut copy, "after leaving the alternate screen");
}

#[test]
fn the_primary_cursor_survives_the_alternate_screen() {
    let mut source = Emu::new(30, 8, 100).expect("emulator");
    source.feed(b"one\r\ntwo\r\n   three\x1b[?1049h\x1b[3;3Halt");
    let mut copy = copy_of(&source);
    source.feed(b"\x1b[?1049l");
    copy.feed(b"\x1b[?1049l");
    let (a, b) = (source.full_frame(ID).cursor, copy.full_frame(ID).cursor);
    assert_eq!((a.row, a.col), (2, 8));
    assert_eq!(a, b);
}

#[test]
fn a_cursor_moved_and_hidden_and_a_bar_shape() {
    let (mut source, _) = round_trip("cursor", b"abc\r\ndef\x1b[2;2H\x1b[?25l\x1b[6 q");
    let cursor = source.full_frame(ID).cursor;
    assert!(!cursor.visible);
    assert_eq!(cursor.shape, CursorShape::Bar);
    for (code, shape) in [
        (1, CursorShape::Block),
        (3, CursorShape::Underline),
        (4, CursorShape::Underline),
        (5, CursorShape::Bar),
    ] {
        round_trip("shape", format!("x\x1b[{code} q").as_bytes());
        let mut e = Emu::new(30, 8, 100).expect("emulator");
        e.feed(format!("x\x1b[{code} q").as_bytes());
        assert_eq!(e.full_frame(ID).cursor.shape, shape);
    }
    round_trip("default shape", b"x");
}

#[test]
fn modes_and_the_title() {
    let (mut source, _) = round_trip(
        "modes",
        b"\x1b]2;my title\x07\x1b[?1h\x1b[?2004h\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1004h\x1b[?1006h\x1b[?1007lhello",
    );
    let modes = source.full_frame(ID).modes;
    for bit in [
        Modes::APPLICATION_CURSOR,
        Modes::BRACKETED_PASTE,
        Modes::MOUSE_CLICK,
        Modes::MOUSE_DRAG,
        Modes::MOUSE_MOTION,
        Modes::FOCUS_REPORTING,
        Modes::SGR_MOUSE,
    ] {
        assert!(modes.has(bit), "{bit}");
    }
    assert!(!modes.has(Modes::ALTERNATE_SCROLL));
    let copy = copy_of(&source);
    assert_eq!(copy.term.title().unwrap(), "my title");
}

#[test]
fn autowrap_insert_and_the_scroll_region() {
    round_trip(
        "no autowrap",
        b"\x1b[?7labcdefghijklmnopqrstuvwxyz0123456789ABCDEF\r\nx",
    );
    round_trip("insert", b"abcdef\x1b[4h\x1b[1;3HXY");
    // The region scrolls in the copy as it does in the source.
    let mut input = Vec::new();
    for n in 1..=8 {
        input.extend_from_slice(format!("\x1b[{n};1Hrow {n}").as_bytes());
    }
    input.extend_from_slice(b"\x1b[3;6r\x1b[6;1H\r\nnew");
    let (mut source, mut copy) = round_trip("scroll region", &input);
    source.feed(b"\r\nmore\r\nmore");
    copy.feed(b"\r\nmore\r\nmore");
    assert_same(&mut source, &mut copy, "scrolling inside the region");
}

#[test]
fn the_pending_wrap_at_the_last_column() {
    let (mut source, mut copy) =
        round_trip("pending wrap", b"line\r\n123456789012345678901234567890");
    assert!(source.term.is_cursor_pending_wrap().unwrap());
    assert!(copy.term.is_cursor_pending_wrap().unwrap());
    source.feed(b"Z");
    copy.feed(b"Z");
    assert_same(&mut source, &mut copy, "after the character that wraps");
}

#[test]
fn an_empty_and_a_scrolled_terminal() {
    round_trip("empty", b"");
    // Output while the display is scrolled back: the copy shows the live screen.
    let mut source = Emu::new(30, 8, 100).expect("emulator");
    source.feed(&shell());
    source.scroll(ScrollTo::Lines(5));
    let mut copy = copy_of(&source);
    source.scroll(ScrollTo::Bottom);
    assert_same(&mut source, &mut copy, "scrolled source");
}

#[test]
fn a_hold_of_synchronized_output_is_not_replayed() {
    let mut source = Emu::new(30, 8, 100).expect("emulator");
    source.feed(b"\x1b[?2026hheld");
    let snapshot = source.snapshot();
    assert!(!String::from_utf8_lossy(&snapshot).contains("2026"));
    let mut copy = copy_of(&source);
    assert!(
        copy.damage_frame(ID, Duration::ZERO).is_some(),
        "the copy holds nothing"
    );
}

#[test]
fn the_snapshot_holds_no_query() {
    let mut source = Emu::new(30, 8, 100).expect("emulator");
    source.feed(shell().as_slice());
    source.feed(b"\x1b]2;t\x07\x1b[?1049h\x1b[?1h\x1b[5 qx");
    let text = source.snapshot();
    for query in [
        &b"\x1b[c"[..],
        b"\x1b[>c",
        b"\x1b[5n",
        b"\x1b[6n",
        b"$p",
        b"\x1b[>q",
        b"?\x07",
        b"?\x1b\\",
    ] {
        assert!(
            !text.windows(query.len()).any(|w| w == query),
            "{:?} in the snapshot",
            String::from_utf8_lossy(query)
        );
    }
    let _ = copy_of(&source);
}

#[test]
fn history_above_a_mostly_blank_screen() {
    let mut input = shell();
    // `clear` keeps the history and leaves one line on a blank screen.
    input.extend_from_slice(b"\x1b[H\x1b[2Jtop\x1b[5;3Hmid");
    let (mut source, _) = round_trip("blank screen", &input);
    assert!(source.full_frame(ID).history > 10);
}

#[test]
fn origin_mode_and_its_cursor() {
    let (mut source, mut copy) = round_trip(
        "origin mode",
        b"\x1b[1;1Hone\x1b[3;6r\x1b[?6h\x1b[2;4Hxy\x1b[0;0H",
    );
    // The cursor is where the source has it, and addresses relative to the region.
    source.feed(b"\x1b[2;1Hz");
    copy.feed(b"\x1b[2;1Hz");
    assert_same(&mut source, &mut copy, "after addressing inside the region");
    let (mut source, mut copy) = round_trip("origin mode, whole screen", b"abc\x1b[?6h\x1b[3;3H");
    source.feed(b"q");
    copy.feed(b"q");
    assert_same(&mut source, &mut copy, "origin without a region");
}
