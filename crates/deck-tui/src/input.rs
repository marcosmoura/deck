//! Keys: conversion from crossterm events and the binding table.

use deck_core::config::Config;
use deck_core::keys::{KeyCode, KeyEvent, KeySpec, Modifiers};
use ratatui::crossterm::event as ct;

/// What a bound key does. Later specs add actions here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Detach,
    /// Bound, does nothing until the scroll mode of sub-spec 06.
    Scroll,
}

impl Action {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "detach" => Some(Self::Detach),
            "scroll" => Some(Self::Scroll),
            _ => None,
        }
    }
}

/// The actions that show in the status bar's hints, in order, with their label.
const HINTED: &[(Action, &str)] = &[(Action::Detach, "detach")];

/// The prefix and the keys bound after it.
#[derive(Debug, Clone)]
pub struct Bindings {
    prefix: KeySpec,
    actions: Vec<(Action, Vec<KeySpec>)>,
}

impl Bindings {
    pub fn from_config(config: &Config) -> Self {
        let actions = config
            .keys
            .iter()
            .filter_map(|(name, keys)| Some((Action::from_name(name)?, keys.clone())))
            .collect();
        Self {
            prefix: config.prefix,
            actions,
        }
    }

    pub fn is_prefix(&self, key: &KeyEvent) -> bool {
        self.prefix.matches_prefix(key)
    }

    pub fn lookup(&self, key: &KeyEvent) -> Option<Action> {
        self.actions
            .iter()
            .find(|(_, keys)| keys.iter().any(|k| k.matches(key)))
            .map(|(action, _)| *action)
    }

    /// The hint text of the status bar, e.g. `⌥Space d detach`.
    pub fn hints(&self) -> String {
        let prefix = spec_label(&self.prefix);
        HINTED
            .iter()
            .filter_map(|(action, label)| {
                let keys = &self.actions.iter().find(|(a, _)| a == action)?.1;
                Some(format!("{prefix} {} {label}", spec_label(keys.first()?)))
            })
            .collect::<Vec<_>>()
            .join("  ")
    }
}

/// A key for the status bar: `⌥Space` on macOS, `M-Space` elsewhere.
fn spec_label(spec: &KeySpec) -> String {
    spec.label(cfg!(target_os = "macos"))
}

/// The key as the user would write it in the config, for messages.
pub fn event_label(key: &KeyEvent) -> String {
    let mut mods = key.mods;
    if matches!(key.code, KeyCode::Char(_)) {
        mods.shift = false;
    }
    KeySpec {
        code: key.code,
        mods,
    }
    .to_string()
}

/// Converts a crossterm key event; `None` for releases and keys deck cannot encode.
pub fn convert(key: ct::KeyEvent) -> Option<KeyEvent> {
    if key.kind == ct::KeyEventKind::Release {
        return None;
    }
    let mut mods = Modifiers {
        ctrl: key.modifiers.contains(ct::KeyModifiers::CONTROL),
        alt: key.modifiers.contains(ct::KeyModifiers::ALT),
        shift: key.modifiers.contains(ct::KeyModifiers::SHIFT),
    };
    let code = match key.code {
        ct::KeyCode::Char(c) => KeyCode::Char(c),
        ct::KeyCode::Enter => KeyCode::Enter,
        ct::KeyCode::Tab => KeyCode::Tab,
        ct::KeyCode::BackTab => {
            mods.shift = true;
            KeyCode::Tab
        }
        ct::KeyCode::Esc => KeyCode::Esc,
        ct::KeyCode::Backspace => KeyCode::Backspace,
        ct::KeyCode::Insert => KeyCode::Insert,
        ct::KeyCode::Delete => KeyCode::Delete,
        ct::KeyCode::Up => KeyCode::Up,
        ct::KeyCode::Down => KeyCode::Down,
        ct::KeyCode::Left => KeyCode::Left,
        ct::KeyCode::Right => KeyCode::Right,
        ct::KeyCode::Home => KeyCode::Home,
        ct::KeyCode::End => KeyCode::End,
        ct::KeyCode::PageUp => KeyCode::PageUp,
        ct::KeyCode::PageDown => KeyCode::PageDown,
        ct::KeyCode::F(n @ 1..=12) => KeyCode::F(n),
        _ => return None,
    };
    Some(KeyEvent::new(code, mods))
}

/// Keys from bytes read before the event reader started. Plain keys and `ESC` plus a
/// character (Alt+key, the prefix among them); any other escape sequence ends the recovery.
pub fn keys_from_bytes(bytes: &[u8]) -> Vec<KeyEvent> {
    let mut keys = Vec::new();
    let text = String::from_utf8_lossy(bytes);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let mut alt = false;
        let mut c = c;
        if c == '\x1b' {
            match chars.peek() {
                Some(&next) if !matches!(next, '[' | 'O' | '\x1b') && !next.is_control() => {
                    alt = true;
                    c = next;
                    chars.next();
                }
                _ => break,
            }
        }
        let (code, mut mods) = match c {
            '\0' => (KeyCode::Char(' '), Modifiers::CTRL),
            '\r' | '\n' => (KeyCode::Enter, Modifiers::NONE),
            '\t' => (KeyCode::Tab, Modifiers::NONE),
            '\x7f' => (KeyCode::Backspace, Modifiers::NONE),
            '\x01'..='\x1a' => (
                KeyCode::Char(char::from(c as u8 - 1 + b'a')),
                Modifiers::CTRL,
            ),
            '\x1c'..='\x1f' => (
                KeyCode::Char(char::from(c as u8 - 0x1c + b'4')),
                Modifiers::CTRL,
            ),
            c if c.is_control() => continue,
            c if c.is_uppercase() => (KeyCode::Char(c), Modifiers::SHIFT),
            c => (KeyCode::Char(c), Modifiers::NONE),
        };
        mods.alt = alt;
        keys.push(KeyEvent::new(code, mods));
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;
    use deck_core::keys::encode;
    use deck_core::protocol::Modes;

    fn press(code: ct::KeyCode, mods: ct::KeyModifiers) -> ct::KeyEvent {
        ct::KeyEvent::new(code, mods)
    }

    #[test]
    fn plain_keys_convert() {
        let k = convert(press(ct::KeyCode::Char('a'), ct::KeyModifiers::NONE)).unwrap();
        assert_eq!(k, KeyEvent::new(KeyCode::Char('a'), Modifiers::NONE));
        let k = convert(press(
            ct::KeyCode::Up,
            ct::KeyModifiers::CONTROL | ct::KeyModifiers::ALT,
        ))
        .unwrap();
        assert_eq!(
            k,
            KeyEvent::new(
                KeyCode::Up,
                Modifiers {
                    ctrl: true,
                    alt: true,
                    shift: false
                }
            )
        );
        assert_eq!(
            convert(press(ct::KeyCode::F(5), ct::KeyModifiers::NONE))
                .unwrap()
                .code,
            KeyCode::F(5)
        );
        assert_eq!(
            convert(press(ct::KeyCode::F(13), ct::KeyModifiers::NONE)),
            None
        );
        assert_eq!(
            convert(press(ct::KeyCode::Null, ct::KeyModifiers::NONE)),
            None
        );
    }

    #[test]
    fn back_tab_is_shift_tab_and_releases_are_dropped() {
        let k = convert(press(ct::KeyCode::BackTab, ct::KeyModifiers::SHIFT)).unwrap();
        assert_eq!(k, KeyEvent::new(KeyCode::Tab, Modifiers::SHIFT));
        let mut release = press(ct::KeyCode::Char('a'), ct::KeyModifiers::NONE);
        release.kind = ct::KeyEventKind::Release;
        assert_eq!(convert(release), None);
    }

    #[test]
    fn a_shifted_character_matches_its_binding_without_shift() {
        let config = Config::parse("[keys]\ndetach = \"X\"").unwrap().config;
        let b = Bindings::from_config(&config);
        let shifted = convert(press(ct::KeyCode::Char('X'), ct::KeyModifiers::SHIFT)).unwrap();
        assert_eq!(b.lookup(&shifted), Some(Action::Detach));
        let plain = convert(press(ct::KeyCode::Char('d'), ct::KeyModifiers::NONE)).unwrap();
        assert_eq!(b.lookup(&plain), None);
    }

    #[test]
    fn the_default_table_binds_detach_and_scroll_after_the_prefix() {
        let b = Bindings::from_config(&Config::default());
        let alt_space = convert(press(ct::KeyCode::Char(' '), ct::KeyModifiers::ALT)).unwrap();
        assert!(b.is_prefix(&alt_space));
        // Option+Space as the non-breaking space, for terminals where Option is not Alt.
        let nbsp = convert(press(ct::KeyCode::Char('\u{a0}'), ct::KeyModifiers::NONE)).unwrap();
        assert!(b.is_prefix(&nbsp));
        let ctrl_space = convert(press(ct::KeyCode::Char(' '), ct::KeyModifiers::CONTROL)).unwrap();
        assert!(!b.is_prefix(&ctrl_space));
        let d = KeyEvent::new(KeyCode::Char('d'), Modifiers::NONE);
        assert_eq!(b.lookup(&d), Some(Action::Detach));
        let bracket = KeyEvent::new(KeyCode::Char('['), Modifiers::NONE);
        assert_eq!(b.lookup(&bracket), Some(Action::Scroll));
        let prefix = if cfg!(target_os = "macos") {
            "⌥Space"
        } else {
            "M-Space"
        };
        assert_eq!(b.hints(), format!("{prefix} d detach"));
    }

    #[test]
    fn typed_ahead_bytes_become_keys() {
        let keys = keys_from_bytes(b"aB\0\r\x03\x1b \xc2\xa0\x1b[Axyz");
        assert_eq!(
            keys,
            [
                KeyEvent::new(KeyCode::Char('a'), Modifiers::NONE),
                KeyEvent::new(KeyCode::Char('B'), Modifiers::SHIFT),
                KeyEvent::new(KeyCode::Char(' '), Modifiers::CTRL),
                KeyEvent::new(KeyCode::Enter, Modifiers::NONE),
                KeyEvent::new(KeyCode::Char('c'), Modifiers::CTRL),
                KeyEvent::new(KeyCode::Char(' '), Modifiers::ALT),
                KeyEvent::new(KeyCode::Char('\u{a0}'), Modifiers::NONE),
            ]
        );
    }

    #[test]
    fn labels() {
        let x = KeyEvent::new(KeyCode::Char('X'), Modifiers::SHIFT);
        assert_eq!(event_label(&x), "X");
        let up = KeyEvent::new(KeyCode::Up, Modifiers::SHIFT);
        assert_eq!(event_label(&up), "S-Up");
    }

    /// Every byte a terminal can send for a key, one at a time, survives the round trip
    /// through crossterm's parser, our conversion and `encode`.
    #[test]
    fn crossterm_events_encode_back_to_their_bytes() {
        for byte in 0x01u8..=0x7f {
            if byte == 0x1b {
                continue;
            }
            let ev = match byte {
                b'\r' => press(ct::KeyCode::Enter, ct::KeyModifiers::NONE),
                b'\t' => press(ct::KeyCode::Tab, ct::KeyModifiers::NONE),
                0x7f => press(ct::KeyCode::Backspace, ct::KeyModifiers::NONE),
                0x1c..=0x1f => press(
                    ct::KeyCode::Char(char::from(byte - 0x1c + b'4')),
                    ct::KeyModifiers::CONTROL,
                ),
                0x01..=0x1a => press(
                    ct::KeyCode::Char(char::from(byte - 1 + b'a')),
                    ct::KeyModifiers::CONTROL,
                ),
                _ => {
                    let c = char::from(byte);
                    let mods = if c.is_uppercase() {
                        ct::KeyModifiers::SHIFT
                    } else {
                        ct::KeyModifiers::NONE
                    };
                    press(ct::KeyCode::Char(c), mods)
                }
            };
            let key = convert(ev).unwrap();
            assert_eq!(encode(&key, Modes::default()), [byte], "byte {byte:#04x}");
        }
        let nul = convert(press(ct::KeyCode::Char(' '), ct::KeyModifiers::CONTROL)).unwrap();
        assert_eq!(encode(&nul, Modes::default()), [0]);
    }
}
