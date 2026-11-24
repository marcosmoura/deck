//! `deck_core::keys::encode_kitty` against Ghostty's own encoder (`libghostty-vt`).

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "integration-test helpers fail the test on error"
)]

use deck_core::keys::{
    KITTY_ALL_KEYS, KITTY_DISAMBIGUATE, KITTY_EVENT_TYPES, KeyCode, KeyEvent, KeyEventKind,
    Modifiers, encode_kitty,
};
use ghostty_vt::key::{Action, Encoder, Event, Key, KittyKeyFlags, Mods};

fn ghostty(
    key: Key,
    text: Option<&str>,
    base: char,
    mods: Mods,
    kind: KeyEventKind,
    flags: u8,
) -> Vec<u8> {
    let mut encoder = Encoder::new().unwrap();
    encoder.set_kitty_flags(KittyKeyFlags::from_bits_retain(flags));
    let mut event = Event::new().unwrap();
    event
        .set_key(key)
        .set_mods(mods)
        .set_action(match kind {
            KeyEventKind::Press => Action::Press,
            KeyEventKind::Repeat => Action::Repeat,
            KeyEventKind::Release => Action::Release,
        })
        .set_utf8(text)
        .set_unshifted_codepoint(base);
    let mut out = Vec::new();
    encoder.encode_to_vec(&event, &mut out).unwrap();
    out
}

/// Ghostty writes the press event type (`:1`) once event types are on, and deck leaves it
/// out as kitty does; they mean the same.
fn normal(bytes: &[u8]) -> Vec<u8> {
    let mut text = String::from_utf8_lossy(bytes).replace("[1;1:1", "[");
    for c in "ABCDHFPQSu~".chars() {
        text = text.replace(&format!(":1{c}"), &c.to_string());
    }
    text.replace(";1u", "u").into_bytes()
}

/// A key as Ghostty and deck each name it.
type Case<'a> = (Key, Option<&'a str>, char, Mods, KeyCode, Modifiers);

#[test]
fn deck_encodes_like_ghostty() {
    let none = Mods::empty();
    let ctrl = Mods::CTRL;
    let shift = Mods::SHIFT;
    // (ghostty key, text, base char, ghostty mods, deck key, deck mods)
    let keys: Vec<Case> = vec![
        (
            Key::A,
            Some("a"),
            'a',
            none,
            KeyCode::Char('a'),
            Modifiers::NONE,
        ),
        (Key::A, None, 'a', ctrl, KeyCode::Char('a'), Modifiers::CTRL),
        (
            Key::Enter,
            Some("\r"),
            '\0',
            none,
            KeyCode::Enter,
            Modifiers::NONE,
        ),
        (
            Key::Enter,
            Some("\r"),
            '\0',
            shift,
            KeyCode::Enter,
            Modifiers::SHIFT,
        ),
        (
            Key::Tab,
            Some("\t"),
            '\0',
            ctrl,
            KeyCode::Tab,
            Modifiers::CTRL,
        ),
        (
            Key::Backspace,
            None,
            '\0',
            none,
            KeyCode::Backspace,
            Modifiers::NONE,
        ),
        (Key::Escape, None, '\0', none, KeyCode::Esc, Modifiers::NONE),
        (Key::ArrowUp, None, '\0', none, KeyCode::Up, Modifiers::NONE),
        (
            Key::ArrowUp,
            None,
            '\0',
            shift,
            KeyCode::Up,
            Modifiers::SHIFT,
        ),
        (Key::F1, None, '\0', none, KeyCode::F(1), Modifiers::NONE),
        (Key::F5, None, '\0', ctrl, KeyCode::F(5), Modifiers::CTRL),
        (
            Key::Delete,
            None,
            '\0',
            none,
            KeyCode::Delete,
            Modifiers::NONE,
        ),
    ];
    let mut differences = Vec::new();
    for flags in [
        KITTY_DISAMBIGUATE,
        KITTY_DISAMBIGUATE | KITTY_EVENT_TYPES,
        KITTY_DISAMBIGUATE | KITTY_EVENT_TYPES | KITTY_ALL_KEYS,
    ] {
        for kind in [
            KeyEventKind::Press,
            KeyEventKind::Repeat,
            KeyEventKind::Release,
        ] {
            for (gkey, text, base, gmods, code, mods) in &keys {
                let want = normal(&ghostty(*gkey, *text, *base, *gmods, kind, flags));
                let got =
                    encode_kitty(&KeyEvent::new(*code, *mods), kind, flags).map(|b| normal(&b));
                // `None` is the legacy form, which Ghostty writes too (its text or the
                // sequence); only the escape-code forms are compared here.
                let Some(got) = got else { continue };
                if got != want {
                    differences.push(format!(
                        "{code:?} {mods:?} {kind:?} flags {flags}: deck {:?}, ghostty {:?}",
                        String::from_utf8_lossy(&got),
                        String::from_utf8_lossy(&want)
                    ));
                }
            }
        }
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}
