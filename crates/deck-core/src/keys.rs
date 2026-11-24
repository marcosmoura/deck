//! Key specs (`M-Space`, `C-a`), key events, xterm key encoding and the kitty keyboard
//! protocol's encoding.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::protocol::Modes;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum KeyCode {
    /// A character key; the space bar is `Char(' ')`.
    Char(char),
    Enter,
    Tab,
    Esc,
    Backspace,
    Insert,
    Delete,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    /// `F(1)` to `F(12)`; the keys the kitty protocol adds, `F(13)` to `F(35)`, come from
    /// the outer terminal only (the config has no name for them).
    F(u8),
    /// The keypad's `5` with Num Lock off. Only kitty reports it; the legacy encoding has no
    /// form for it.
    KeypadBegin,
    Media(MediaKey),
    /// A modifier key pressed or released on its own.
    Modifier(ModifierKey),
    Special(SpecialKey),
}

/// The media keys of the kitty protocol, in the order of its key numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MediaKey {
    Play,
    Pause,
    PlayPause,
    Reverse,
    Stop,
    FastForward,
    Rewind,
    TrackNext,
    TrackPrevious,
    Record,
    LowerVolume,
    RaiseVolume,
    MuteVolume,
}

/// The modifier keys of the kitty protocol, in the order of its key numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModifierKey {
    LeftShift,
    LeftControl,
    LeftAlt,
    LeftSuper,
    LeftHyper,
    LeftMeta,
    RightShift,
    RightControl,
    RightAlt,
    RightSuper,
    RightHyper,
    RightMeta,
    IsoLevel3Shift,
    IsoLevel5Shift,
}

/// The other keys of the kitty protocol that make no text, in the order of its key numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SpecialKey {
    CapsLock,
    ScrollLock,
    NumLock,
    PrintScreen,
    Pause,
    Menu,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Super (Cmd, the logo key), Hyper and Meta: only the kitty protocol reports them.
    pub sup: bool,
    pub hyper: bool,
    pub meta: bool,
}

impl Modifiers {
    pub const NONE: Self = Self {
        ctrl: false,
        alt: false,
        shift: false,
        sup: false,
        hyper: false,
        meta: false,
    };
    pub const CTRL: Self = Self {
        ctrl: true,
        alt: false,
        shift: false,
        sup: false,
        hyper: false,
        meta: false,
    };
    pub const ALT: Self = Self {
        ctrl: false,
        alt: true,
        shift: false,
        sup: false,
        hyper: false,
        meta: false,
    };
    pub const SHIFT: Self = Self {
        ctrl: false,
        alt: false,
        shift: true,
        sup: false,
        hyper: false,
        meta: false,
    };
}

/// A key press, as the client converts it from the outer terminal's events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyEvent {
    pub code: KeyCode,
    pub mods: Modifiers,
    /// The key is on the keypad (`code` is what it types: a digit, `Enter`, `Left`). Only the
    /// kitty encoding tells it from the main key.
    pub keypad: bool,
}

impl KeyEvent {
    pub fn new(code: KeyCode, mods: Modifiers) -> Self {
        Self {
            code,
            mods,
            keypad: false,
        }
    }

    /// The same key on the keypad.
    #[must_use]
    pub fn on_keypad(mut self) -> Self {
        self.keypad = true;
        self
    }
}

/// What happened to a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyEventKind {
    Press,
    /// The key is held down.
    Repeat,
    Release,
}

/// A key as written in the config: a code and modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KeySpec {
    pub code: KeyCode,
    pub mods: Modifiers,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid key `{input}`: {reason}")]
pub struct KeyParseError {
    pub input: String,
    pub reason: &'static str,
}

/// The character macOS terminals send for Option+Space when Option is not Alt.
pub const NBSP: char = '\u{a0}';

const NAMED: &[(&str, KeyCode)] = &[
    ("Space", KeyCode::Char(' ')),
    ("Enter", KeyCode::Enter),
    ("Tab", KeyCode::Tab),
    ("Esc", KeyCode::Esc),
    ("Backspace", KeyCode::Backspace),
    ("Insert", KeyCode::Insert),
    ("Delete", KeyCode::Delete),
    ("Up", KeyCode::Up),
    ("Down", KeyCode::Down),
    ("Left", KeyCode::Left),
    ("Right", KeyCode::Right),
    ("Home", KeyCode::Home),
    ("End", KeyCode::End),
    ("PageUp", KeyCode::PageUp),
    ("PageDown", KeyCode::PageDown),
];

impl KeySpec {
    pub fn parse(input: &str) -> Result<Self, KeyParseError> {
        let err = |reason| KeyParseError {
            input: input.to_string(),
            reason,
        };
        let mut rest = input;
        let mut mods = Modifiers::NONE;
        let mut repeated = false;
        while rest.len() > 2 {
            match rest.get(..2) {
                Some("C-") => repeated |= std::mem::replace(&mut mods.ctrl, true),
                Some("M-") => repeated |= std::mem::replace(&mut mods.alt, true),
                Some("S-") => repeated |= std::mem::replace(&mut mods.shift, true),
                _ => break,
            }
            rest = &rest[2..];
        }
        if rest.is_empty() {
            return Err(err("empty key"));
        }
        let mut chars = rest.chars();
        let first = chars.next();
        let code = if let (Some(c), None) = (first, chars.next()) {
            KeyCode::Char(c)
        } else if let Some((_, code)) = NAMED.iter().find(|(name, _)| *name == rest) {
            *code
        } else if let Some(n) = rest.strip_prefix('F').and_then(|n| n.parse::<u8>().ok())
            && (1..=12).contains(&n)
            && !rest[1..].starts_with('+')
        {
            KeyCode::F(n)
        } else {
            return Err(err("unknown key name or modifier"));
        };
        if repeated {
            return Err(err("a modifier is repeated"));
        }
        if let KeyCode::Char(c) = code {
            if mods.shift {
                return Err(err(
                    "S- applies only to named keys; write the character itself",
                ));
            }
            if mods.ctrl && c.is_ascii_uppercase() {
                return Err(err(
                    "C- with an uppercase letter is never reported by terminals; use the lowercase letter",
                ));
            }
        }
        Ok(Self { code, mods })
    }

    /// Whether `event` is this key used as the prefix. On macOS a terminal where Option is
    /// not Alt sends the non-breaking space for Option+Space, so that counts for `M-Space`.
    pub fn matches_prefix(&self, event: &KeyEvent) -> bool {
        let option_space = self.code == KeyCode::Char(' ') && self.mods == Modifiers::ALT;
        self.matches(event)
            || (option_space && event.code == KeyCode::Char(NBSP) && event.mods == Modifiers::NONE)
    }

    /// The key for display in the UI: `⌥Space d` style on macOS (`mac`), the config's own
    /// spelling (`M-Space`) elsewhere.
    pub fn label(&self, mac: bool) -> String {
        if !mac {
            return self.to_string();
        }
        let mut out = String::new();
        if self.mods.ctrl {
            out.push('⌃');
        }
        if self.mods.alt {
            out.push('⌥');
        }
        if self.mods.shift {
            out.push('⇧');
        }
        out.push_str(
            &KeySpec {
                code: self.code,
                mods: Modifiers::NONE,
            }
            .to_string(),
        );
        out
    }

    pub fn to_event(self) -> KeyEvent {
        KeyEvent::new(self.code, self.mods)
    }

    /// Whether `event` is this key. For character keys the `Shift` flag is ignored on both
    /// sides (terminals report `X` with `Shift`); `S-` matters only for named keys.
    pub fn matches(&self, event: &KeyEvent) -> bool {
        if self.code != event.code {
            return false;
        }
        let mut a = self.mods;
        let mut b = event.mods;
        // Hyper and Meta are reported only by the kitty protocol and no binding names them;
        // Super counts.
        for m in [&mut a, &mut b] {
            m.hyper = false;
            m.meta = false;
        }
        if matches!(self.code, KeyCode::Char(_)) {
            a.shift = false;
            b.shift = false;
        }
        a == b
    }
}

impl From<KeySpec> for KeyEvent {
    fn from(spec: KeySpec) -> Self {
        spec.to_event()
    }
}

impl FromStr for KeySpec {
    type Err = KeyParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl fmt::Display for KeySpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.mods.ctrl {
            f.write_str("C-")?;
        }
        if self.mods.alt {
            f.write_str("M-")?;
        }
        if self.mods.shift {
            f.write_str("S-")?;
        }
        match self.code {
            KeyCode::Char(' ') => f.write_str("Space"),
            KeyCode::Char(c) => write!(f, "{c}"),
            KeyCode::F(n) => write!(f, "F{n}"),
            code => {
                let name = NAMED
                    .iter()
                    .find(|(_, c)| *c == code)
                    .map_or("?", |(name, _)| name);
                f.write_str(name)
            }
        }
    }
}

/// Encodes a key press as the bytes a program in an xterm-compatible terminal receives.
pub fn encode(key: &KeyEvent, modes: Modes) -> Vec<u8> {
    let m = key.mods;
    let param = 1 + u8::from(m.shift) + 2 * u8::from(m.alt) + 4 * u8::from(m.ctrl);
    let app_cursor = modes.has(Modes::APPLICATION_CURSOR);

    // Keys of the form `ESC [ 1 ; m X` / `ESC O X` (arrows, Home, End, F1-F4).
    let letter = |final_byte: u8, ss3_plain: bool| -> Vec<u8> {
        if param > 1 {
            format!("\x1b[1;{param}{}", char::from(final_byte)).into_bytes()
        } else if ss3_plain {
            vec![0x1b, b'O', final_byte]
        } else {
            vec![0x1b, b'[', final_byte]
        }
    };
    // Keys of the form `ESC [ n ; m ~`.
    let tilde = |n: u8| -> Vec<u8> {
        if param > 1 {
            format!("\x1b[{n};{param}~").into_bytes()
        } else {
            format!("\x1b[{n}~").into_bytes()
        }
    };
    let alt_prefix = |mut bytes: Vec<u8>| -> Vec<u8> {
        if m.alt {
            bytes.insert(0, 0x1b);
        }
        bytes
    };

    match key.code {
        KeyCode::Char(c) => alt_prefix(encode_char(c, m.ctrl)),
        KeyCode::Enter => alt_prefix(vec![b'\r']),
        KeyCode::Tab if m.shift => alt_prefix(b"\x1b[Z".to_vec()),
        KeyCode::Tab => alt_prefix(vec![b'\t']),
        KeyCode::Backspace => alt_prefix(vec![0x7f]),
        KeyCode::Esc => alt_prefix(vec![0x1b]),
        KeyCode::Up => letter(b'A', app_cursor),
        KeyCode::Down => letter(b'B', app_cursor),
        KeyCode::Right => letter(b'C', app_cursor),
        KeyCode::Left => letter(b'D', app_cursor),
        KeyCode::Home => letter(b'H', app_cursor),
        KeyCode::End => letter(b'F', app_cursor),
        KeyCode::Insert => tilde(2),
        KeyCode::Delete => tilde(3),
        KeyCode::PageUp => tilde(5),
        KeyCode::PageDown => tilde(6),
        KeyCode::F(n @ 1..=4) => letter(b'O' + n, true),
        KeyCode::F(n @ 5..=12) => [15, 17, 18, 19, 20, 21, 23, 24]
            .get(usize::from(n) - 5)
            .map_or_else(Vec::new, |&code| tilde(code)),
        KeyCode::F(_)
        | KeyCode::KeypadBegin
        | KeyCode::Media(_)
        | KeyCode::Modifier(_)
        | KeyCode::Special(_) => Vec::new(),
    }
}

/// Kitty keyboard flag: keys that the legacy encoding cannot tell apart get escape codes.
pub const KITTY_DISAMBIGUATE: u8 = 1;
/// Kitty keyboard flag: releases and repeats are reported.
pub const KITTY_EVENT_TYPES: u8 = 2;
/// Kitty keyboard flag: a shifted letter carries its shifted key.
pub const KITTY_ALTERNATE_KEYS: u8 = 4;
/// Kitty keyboard flag: every key, text included, is an escape code.
pub const KITTY_ALL_KEYS: u8 = 8;
/// Kitty keyboard flag: the text a key types goes with its escape code.
pub const KITTY_TEXT: u8 = 16;

/// Encodes a key for a terminal in `modes`: with the kitty protocol when the program set
/// flags and the protocol has a form for the key, else as [`encode`]. A release gives nothing
/// unless the program asked for event types.
pub fn encode_key(key: &KeyEvent, kind: KeyEventKind, modes: Modes) -> Vec<u8> {
    let flags = modes.kitty_flags();
    if flags != 0
        && let Some(bytes) = encode_kitty(key, kind, flags)
    {
        return bytes;
    }
    match kind {
        KeyEventKind::Release => Vec::new(),
        _ => encode(key, modes),
    }
}

/// What an unmodified key does while the program has not asked for every key as an escape
/// code.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Plain {
    /// A text key: its text, also with Shift.
    Text,
    /// `Enter`, `Tab` and `Backspace`: the legacy byte, and no release.
    Enter,
    /// The arrows, `Home`, `End`, `Insert`, `Delete`, `PageUp`, `PageDown`, `F1` to `F12`:
    /// the legacy sequence.
    Legacy,
    /// Always an escape code.
    Always,
    /// Only reported when every key is (the modifier and lock keys).
    AllOnly,
}

/// The shape of a key's escape code.
#[derive(Clone, Copy)]
enum Form {
    /// `CSI number u`.
    U(u32),
    /// `CSI 1 ; mods final`.
    Letter(char),
    /// `CSI number ; mods ~`.
    Tilde(u8),
}

/// The kitty key number of a keypad key, and the rule for it.
fn keypad_key(code: KeyCode) -> Option<(u32, Plain)> {
    Some(match code {
        KeyCode::Char(c @ '0'..='9') => (57399 + u32::from(c) - u32::from('0'), Plain::Text),
        KeyCode::Char('.') => (57409, Plain::Text),
        KeyCode::Char('/') => (57410, Plain::Text),
        KeyCode::Char('*') => (57411, Plain::Text),
        KeyCode::Char('-') => (57412, Plain::Text),
        KeyCode::Char('+') => (57413, Plain::Text),
        KeyCode::Enter => (57414, Plain::Always),
        KeyCode::Char('=') => (57415, Plain::Text),
        KeyCode::Char(',') => (57416, Plain::Text),
        KeyCode::Left => (57417, Plain::Always),
        KeyCode::Right => (57418, Plain::Always),
        KeyCode::Up => (57419, Plain::Always),
        KeyCode::Down => (57420, Plain::Always),
        KeyCode::PageUp => (57421, Plain::Always),
        KeyCode::PageDown => (57422, Plain::Always),
        KeyCode::Home => (57423, Plain::Always),
        KeyCode::End => (57424, Plain::Always),
        KeyCode::Insert => (57425, Plain::Always),
        KeyCode::Delete => (57426, Plain::Always),
        _ => return None,
    })
}

/// The kitty key number of a key, its shape and its rule.
fn kitty_form(key: &KeyEvent) -> Option<(Form, Plain)> {
    if key.keypad
        && let Some((number, plain)) = keypad_key(key.code)
    {
        return Some((Form::U(number), plain));
    }
    let offset = |base: u32, index: usize| base + u32::try_from(index).unwrap_or(0);
    Some(match key.code {
        KeyCode::Char(c) => {
            // The key number is the unshifted character.
            let base = if c.is_uppercase() {
                c.to_lowercase().next().unwrap_or(c)
            } else {
                c
            };
            (Form::U(u32::from(base)), Plain::Text)
        }
        KeyCode::Enter => (Form::U(13), Plain::Enter),
        KeyCode::Tab => (Form::U(9), Plain::Enter),
        KeyCode::Backspace => (Form::U(127), Plain::Enter),
        KeyCode::Esc => (Form::U(27), Plain::Always),
        KeyCode::Up => (Form::Letter('A'), Plain::Legacy),
        KeyCode::Down => (Form::Letter('B'), Plain::Legacy),
        KeyCode::Right => (Form::Letter('C'), Plain::Legacy),
        KeyCode::Left => (Form::Letter('D'), Plain::Legacy),
        KeyCode::Home => (Form::Letter('H'), Plain::Legacy),
        KeyCode::End => (Form::Letter('F'), Plain::Legacy),
        KeyCode::Insert => (Form::Tilde(2), Plain::Legacy),
        KeyCode::Delete => (Form::Tilde(3), Plain::Legacy),
        KeyCode::PageUp => (Form::Tilde(5), Plain::Legacy),
        KeyCode::PageDown => (Form::Tilde(6), Plain::Legacy),
        KeyCode::F(1) => (Form::Letter('P'), Plain::Legacy),
        KeyCode::F(2) => (Form::Letter('Q'), Plain::Legacy),
        KeyCode::F(3) => (Form::Tilde(13), Plain::Legacy),
        KeyCode::F(4) => (Form::Letter('S'), Plain::Legacy),
        KeyCode::F(n @ 5..=12) => (
            Form::Tilde(*[15, 17, 18, 19, 20, 21, 23, 24].get(usize::from(n) - 5)?),
            Plain::Legacy,
        ),
        KeyCode::F(n @ 13..=35) => (Form::U(57376 + u32::from(n) - 13), Plain::Always),
        KeyCode::F(_) => return None,
        KeyCode::KeypadBegin => (Form::U(57427), Plain::Always),
        KeyCode::Media(k) => (Form::U(offset(57428, k as usize)), Plain::Always),
        KeyCode::Modifier(k) => (Form::U(offset(57441, k as usize)), Plain::AllOnly),
        KeyCode::Special(k) => {
            let plain = match k {
                SpecialKey::CapsLock | SpecialKey::ScrollLock | SpecialKey::NumLock => {
                    Plain::AllOnly
                }
                _ => Plain::Always,
            };
            (Form::U(offset(57358, k as usize)), plain)
        }
    })
}

/// The modifier bit a modifier key stands for, as in the kitty protocol's `modifiers` field
/// minus one.
fn modifier_key_bit(key: ModifierKey) -> u8 {
    match key {
        ModifierKey::LeftShift | ModifierKey::RightShift => 1,
        ModifierKey::LeftAlt | ModifierKey::RightAlt => 2,
        ModifierKey::LeftControl | ModifierKey::RightControl => 4,
        ModifierKey::LeftSuper | ModifierKey::RightSuper => 8,
        ModifierKey::LeftHyper | ModifierKey::RightHyper => 16,
        ModifierKey::LeftMeta | ModifierKey::RightMeta => 32,
        ModifierKey::IsoLevel3Shift | ModifierKey::IsoLevel5Shift => 0,
    }
}

/// Encodes a key with the kitty keyboard protocol for the `flags` the program set
/// (`KITTY_*`). `None` when the legacy encoding applies (text, `Enter`, the arrows... without
/// modifiers); `Some` of nothing when the key is not reported (a release the program did not
/// ask for). The shifted key (flag 4) is the character given, so a terminal has to report it;
/// the base layout key is never sent.
pub fn encode_kitty(key: &KeyEvent, kind: KeyEventKind, flags: u8) -> Option<Vec<u8>> {
    let all = flags & KITTY_ALL_KEYS != 0;
    let events = flags & KITTY_EVENT_TYPES != 0;
    if flags & (KITTY_DISAMBIGUATE | KITTY_ALL_KEYS) == 0 {
        return (kind == KeyEventKind::Release).then(Vec::new);
    }
    if kind == KeyEventKind::Release && !events {
        return Some(Vec::new());
    }
    // Without event types a repeat is a press.
    let kind = if events { kind } else { KeyEventKind::Press };
    let (form, plain) = kitty_form(key)?;

    let mut bits = [
        key.mods.shift,
        key.mods.alt,
        key.mods.ctrl,
        key.mods.sup,
        key.mods.hyper,
        key.mods.meta,
    ]
    .iter()
    .enumerate()
    .fold(0u8, |acc, (i, held)| acc | (u8::from(*held) << i));
    if let (KeyCode::Modifier(m), KeyEventKind::Release) = (key.code, kind) {
        bits &= !modifier_key_bit(m);
    }
    // A character that is uppercase was shifted, even when the terminal left Shift out of
    // the modifiers (flag 4 reports the shifted key instead).
    let mut shifted = None;
    if let KeyCode::Char(c) = key.code
        && !key.keypad
        && c.is_uppercase()
        && c.to_lowercase().next() != Some(c)
    {
        bits |= 1;
        shifted = Some(c);
    }
    let text_mods = bits & !1 == 0;

    if plain == Plain::AllOnly && !all {
        return Some(Vec::new());
    }
    if !all && kind != KeyEventKind::Release {
        let legacy = match plain {
            Plain::Text => text_mods,
            Plain::Enter => bits == 0,
            Plain::Legacy => bits == 0 && kind == KeyEventKind::Press,
            Plain::Always | Plain::AllOnly => false,
        };
        if legacy {
            return None;
        }
    }
    if kind == KeyEventKind::Release && !all && plain == Plain::Enter {
        return Some(Vec::new());
    }

    let event = match kind {
        KeyEventKind::Press => 1,
        KeyEventKind::Repeat => 2,
        KeyEventKind::Release => 3,
    };
    let modifiers = if bits != 0 || event != 1 {
        let mut text = (1 + u32::from(bits)).to_string();
        if event != 1 {
            text.push_str(&format!(":{event}"));
        }
        text
    } else {
        String::new()
    };

    let out = match form {
        Form::Letter(final_byte) => {
            if modifiers.is_empty() {
                format!("\x1b[{final_byte}")
            } else {
                format!("\x1b[1;{modifiers}{final_byte}")
            }
        }
        Form::Tilde(number) => {
            if modifiers.is_empty() {
                format!("\x1b[{number}~")
            } else {
                format!("\x1b[{number};{modifiers}~")
            }
        }
        Form::U(number) => {
            let mut code = number.to_string();
            if flags & KITTY_ALTERNATE_KEYS != 0
                && let Some(shifted) = shifted
            {
                code.push_str(&format!(":{}", u32::from(shifted)));
            }
            let text = (flags & KITTY_TEXT != 0 && all && kind != KeyEventKind::Release)
                .then(|| associated_text(key, bits, shifted))
                .flatten();
            match text {
                Some(text) => format!("\x1b[{code};{modifiers};{text}u"),
                None if modifiers.is_empty() => format!("\x1b[{code}u"),
                None => format!("\x1b[{code};{modifiers}u"),
            }
        }
    };
    Some(out.into_bytes())
}

/// The text a key types, as the kitty protocol's code points (`97`, `65:66`): only for text
/// keys with at most Shift held.
fn associated_text(key: &KeyEvent, bits: u8, shifted: Option<char>) -> Option<String> {
    let KeyCode::Char(c) = key.code else {
        return None;
    };
    if bits & !1 != 0 {
        return None;
    }
    let c = match shifted {
        Some(shifted) => shifted,
        None if bits & 1 != 0 => {
            let mut upper = c.to_uppercase();
            match (upper.next(), upper.next()) {
                (Some(u), None) => u,
                _ => c,
            }
        }
        None => c,
    };
    Some(u32::from(c).to_string())
}

fn encode_char(c: char, ctrl: bool) -> Vec<u8> {
    if ctrl {
        let byte = match c {
            'a'..='z' | 'A'..='Z' => Some(c as u8 & 0x1f),
            ' ' | '@' => Some(0),
            '[' => Some(0x1b),
            '\\' | '4' => Some(0x1c),
            ']' | '5' => Some(0x1d),
            '^' | '6' => Some(0x1e),
            '_' | '7' => Some(0x1f),
            _ => None,
        };
        if let Some(byte) = byte {
            return vec![byte];
        }
    }
    let mut buf = [0u8; 4];
    c.encode_utf8(&mut buf).as_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(s: &str) -> KeySpec {
        KeySpec::parse(s).unwrap()
    }

    fn ev(code: KeyCode, ctrl: bool, alt: bool, shift: bool) -> KeyEvent {
        KeyEvent::new(
            code,
            Modifiers {
                ctrl,
                alt,
                shift,
                ..Modifiers::NONE
            },
        )
    }

    #[test]
    fn parses_and_round_trips() {
        let mut inputs: Vec<String> = [
            "a",
            "X",
            "-",
            "C-a",
            "M-x",
            "C-M-x",
            "C-M-S-Left",
            "S-Tab",
            "C-Space",
            "Space",
            "Enter",
            "Tab",
            "Esc",
            "Backspace",
            "Insert",
            "Delete",
            "Up",
            "Down",
            "Left",
            "Right",
            "Home",
            "End",
            "PageUp",
            "PageDown",
            "[",
            "C--",
            "M-F5",
        ]
        .map(String::from)
        .to_vec();
        inputs.extend((1..=12).map(|n| format!("F{n}")));
        for input in inputs {
            let parsed = spec(&input);
            assert_eq!(parsed.to_string(), input);
            assert_eq!(spec(&parsed.to_string()), parsed);
        }
        assert_eq!(spec("M-C-x").to_string(), "C-M-x");
        assert_eq!(spec("C-Space").code, KeyCode::Char(' '));
        assert!(spec("C-Space").mods.ctrl);
        assert_eq!(spec("F12").code, KeyCode::F(12));
        assert_eq!(spec("C--").code, KeyCode::Char('-'));
    }

    #[test]
    fn rejects_bad_input() {
        for input in [
            "C-", "X-a", "", "F0", "F13", "F+1", "Foo", "C-C-", "ab", "C-C-a", "M-M-x", "S-S-Tab",
            "S-a", "S-Space", "C-A", "C-M-Z",
        ] {
            let err = KeySpec::parse(input).unwrap_err();
            assert_eq!(err.input, input);
            assert!(err.to_string().contains(&format!("`{input}`")), "{err}");
        }
    }

    #[test]
    fn matching() {
        let x = spec("X");
        assert!(x.matches(&ev(KeyCode::Char('X'), false, false, true)));
        assert!(x.matches(&ev(KeyCode::Char('X'), false, false, false)));
        assert!(!x.matches(&ev(KeyCode::Char('X'), true, false, false)));
        assert!(!x.matches(&ev(KeyCode::Char('x'), false, false, false)));
        let s_tab = spec("S-Tab");
        assert!(!s_tab.matches(&ev(KeyCode::Tab, false, false, false)));
        assert!(s_tab.matches(&ev(KeyCode::Tab, false, false, true)));
        assert!(!spec("Tab").matches(&ev(KeyCode::Tab, false, false, true)));
        assert!(spec("C-Space").matches(&ev(KeyCode::Char(' '), true, false, false)));
        assert!(spec("d").to_event() == ev(KeyCode::Char('d'), false, false, false));
        // Hyper and Meta are ignored, Super is not.
        let mut held = ev(KeyCode::Char('d'), false, false, false);
        held.mods.hyper = true;
        held.mods.meta = true;
        assert!(spec("d").matches(&held));
        held.mods.sup = true;
        assert!(!spec("d").matches(&held));
    }

    fn enc(code: KeyCode, mods: Modifiers, app: bool) -> Vec<u8> {
        let modes = if app {
            Modes(Modes::APPLICATION_CURSOR)
        } else {
            Modes::default()
        };
        encode(&KeyEvent::new(code, mods), modes)
    }

    fn plain(code: KeyCode) -> Vec<u8> {
        enc(code, Modifiers::NONE, false)
    }

    #[test]
    fn encodes_plain_keys() {
        assert_eq!(plain(KeyCode::Char('a')), b"a");
        assert_eq!(plain(KeyCode::Char('é')), "é".as_bytes());
        assert_eq!(plain(KeyCode::Char('日')), "日".as_bytes());
        assert_eq!(enc(KeyCode::Char('A'), Modifiers::SHIFT, false), b"A");
        assert_eq!(plain(KeyCode::Enter), b"\r");
        assert_eq!(plain(KeyCode::Tab), b"\t");
        assert_eq!(enc(KeyCode::Tab, Modifiers::SHIFT, false), b"\x1b[Z");
        assert_eq!(plain(KeyCode::Backspace), [0x7f]);
        assert_eq!(plain(KeyCode::Esc), [0x1b]);
        assert_eq!(plain(KeyCode::Insert), b"\x1b[2~");
        assert_eq!(plain(KeyCode::Delete), b"\x1b[3~");
        assert_eq!(plain(KeyCode::PageUp), b"\x1b[5~");
        assert_eq!(plain(KeyCode::PageDown), b"\x1b[6~");
    }

    #[test]
    fn encodes_cursor_keys_in_both_modes() {
        for (code, normal, app) in [
            (KeyCode::Up, "\x1b[A", "\x1bOA"),
            (KeyCode::Down, "\x1b[B", "\x1bOB"),
            (KeyCode::Right, "\x1b[C", "\x1bOC"),
            (KeyCode::Left, "\x1b[D", "\x1bOD"),
            (KeyCode::Home, "\x1b[H", "\x1bOH"),
            (KeyCode::End, "\x1b[F", "\x1bOF"),
        ] {
            assert_eq!(enc(code, Modifiers::NONE, false), normal.as_bytes());
            assert_eq!(enc(code, Modifiers::NONE, true), app.as_bytes());
        }
        // Keys that application mode does not change.
        assert_eq!(enc(KeyCode::Delete, Modifiers::NONE, true), b"\x1b[3~");
        assert_eq!(enc(KeyCode::F(5), Modifiers::NONE, true), b"\x1b[15~");
        assert_eq!(enc(KeyCode::F(1), Modifiers::NONE, true), b"\x1bOP");
    }

    #[test]
    fn encodes_function_keys() {
        let want = [
            "\x1bOP", "\x1bOQ", "\x1bOR", "\x1bOS", "\x1b[15~", "\x1b[17~", "\x1b[18~", "\x1b[19~",
            "\x1b[20~", "\x1b[21~", "\x1b[23~", "\x1b[24~",
        ];
        for (i, want) in want.iter().enumerate() {
            let n = i as u8 + 1;
            assert_eq!(plain(KeyCode::F(n)), want.as_bytes(), "F{n}");
        }
    }

    #[test]
    fn encodes_control_keys() {
        for c in 'a'..='z' {
            assert_eq!(
                enc(KeyCode::Char(c), Modifiers::CTRL, false),
                [c as u8 - b'a' + 1]
            );
        }
        for (c, byte) in [
            (' ', 0u8),
            ('@', 0),
            ('[', 0x1b),
            ('\\', 0x1c),
            (']', 0x1d),
            ('^', 0x1e),
            ('_', 0x1f),
            ('4', 0x1c),
            ('5', 0x1d),
            ('6', 0x1e),
            ('7', 0x1f),
        ] {
            assert_eq!(
                enc(KeyCode::Char(c), Modifiers::CTRL, false),
                [byte],
                "C-{c}"
            );
            assert_eq!(
                enc(KeyCode::Char(c), Modifiers::CTRL, true),
                [byte],
                "C-{c} app"
            );
        }
        // No control mapping: the character itself.
        assert_eq!(enc(KeyCode::Char('1'), Modifiers::CTRL, false), b"1");
    }

    #[test]
    fn encodes_alt() {
        let alt = Modifiers::ALT;
        assert_eq!(enc(KeyCode::Char('x'), alt, false), b"\x1bx");
        assert_eq!(enc(KeyCode::Enter, alt, false), b"\x1b\r");
        assert_eq!(enc(KeyCode::Backspace, alt, false), b"\x1b\x7f");
        assert_eq!(enc(KeyCode::Esc, alt, false), b"\x1b\x1b");
        assert_eq!(enc(KeyCode::Tab, alt, false), b"\x1b\t");
        let ctrl_alt = Modifiers {
            ctrl: true,
            alt: true,
            shift: false,
            ..Modifiers::NONE
        };
        assert_eq!(enc(KeyCode::Char('a'), ctrl_alt, false), b"\x1b\x01");
    }

    #[test]
    fn encodes_modified_named_keys() {
        // m = 1 + shift + 2 alt + 4 ctrl
        let combos = [
            (Modifiers::SHIFT, 2),
            (Modifiers::ALT, 3),
            (
                Modifiers {
                    ctrl: false,
                    alt: true,
                    shift: true,
                    ..Modifiers::NONE
                },
                4,
            ),
            (Modifiers::CTRL, 5),
            (
                Modifiers {
                    ctrl: true,
                    alt: false,
                    shift: true,
                    ..Modifiers::NONE
                },
                6,
            ),
            (
                Modifiers {
                    ctrl: true,
                    alt: true,
                    shift: false,
                    ..Modifiers::NONE
                },
                7,
            ),
            (
                Modifiers {
                    ctrl: true,
                    alt: true,
                    shift: true,
                    ..Modifiers::NONE
                },
                8,
            ),
        ];
        for (mods, m) in combos {
            for app in [false, true] {
                for (code, fin) in [
                    (KeyCode::Up, 'A'),
                    (KeyCode::Down, 'B'),
                    (KeyCode::Right, 'C'),
                    (KeyCode::Left, 'D'),
                    (KeyCode::Home, 'H'),
                    (KeyCode::End, 'F'),
                    (KeyCode::F(1), 'P'),
                    (KeyCode::F(4), 'S'),
                ] {
                    assert_eq!(
                        enc(code, mods, app),
                        format!("\x1b[1;{m}{fin}").into_bytes(),
                        "{code:?} {mods:?} app={app}"
                    );
                }
                for (code, n) in [
                    (KeyCode::Insert, 2),
                    (KeyCode::Delete, 3),
                    (KeyCode::PageUp, 5),
                    (KeyCode::PageDown, 6),
                    (KeyCode::F(5), 15),
                    (KeyCode::F(8), 19),
                    (KeyCode::F(12), 24),
                ] {
                    assert_eq!(enc(code, mods, app), format!("\x1b[{n};{m}~").into_bytes());
                }
            }
        }
    }

    #[test]
    fn option_space_is_also_the_non_breaking_space_but_only_as_the_prefix() {
        let alt_space = KeyEvent::new(KeyCode::Char(' '), Modifiers::ALT);
        let nbsp = KeyEvent::new(KeyCode::Char(NBSP), Modifiers::NONE);
        let prefix = spec("M-Space");
        assert!(prefix.matches_prefix(&alt_space));
        assert!(prefix.matches_prefix(&nbsp));
        assert!(!prefix.matches(&nbsp));
        assert!(!prefix.matches_prefix(&KeyEvent::new(KeyCode::Char(' '), Modifiers::NONE)));
        assert!(!prefix.matches_prefix(&KeyEvent::new(KeyCode::Char(NBSP), Modifiers::ALT)));
        // Other prefixes do not take the alias.
        assert!(!spec("C-Space").matches_prefix(&nbsp));
        assert!(!spec("M-a").matches_prefix(&nbsp));
        // Alt+Space encodes as ESC and a space, the NBSP as its UTF-8 bytes.
        assert_eq!(encode(&alt_space, Modes::default()), b"\x1b ");
        assert_eq!(encode(&nbsp, Modes::default()), "\u{a0}".as_bytes());
    }

    #[test]
    fn labels_use_symbols_on_macos_and_the_config_spelling_elsewhere() {
        assert_eq!(spec("M-Space").label(true), "⌥Space");
        assert_eq!(spec("M-Space").label(false), "M-Space");
        assert_eq!(spec("C-b").label(true), "⌃b");
        assert_eq!(spec("C-b").label(false), "C-b");
        assert_eq!(spec("d").label(true), "d");
        assert_eq!(spec("C-M-S-Up").label(true), "⌃⌥⇧Up");
    }

    #[test]
    fn keyspec_converts_to_encodable_event() {
        let ev = KeySpec::parse("C-Space").unwrap().to_event();
        assert_eq!(encode(&ev, Modes::default()), [0]);
        let ev: KeyEvent = KeySpec::parse("S-Tab").unwrap().into();
        assert_eq!(encode(&ev, Modes::default()), b"\x1b[Z");
    }

    fn kitty(code: KeyCode, mods: Modifiers, kind: KeyEventKind, flags: u8) -> Option<String> {
        encode_kitty(&KeyEvent::new(code, mods), kind, flags)
            .map(|bytes| String::from_utf8(bytes).unwrap().replace('\x1b', "<"))
    }

    fn press(code: KeyCode, mods: Modifiers, flags: u8) -> Option<String> {
        kitty(code, mods, KeyEventKind::Press, flags)
    }

    const CTRL_SHIFT: Modifiers = Modifiers {
        ctrl: true,
        shift: true,
        ..Modifiers::NONE
    };

    /// The protocol's own examples for flag 1.
    #[test]
    fn kitty_disambiguate() {
        let f = KITTY_DISAMBIGUATE;
        let some = |s: &str| Some(s.to_string());
        // Esc is always an escape code.
        assert_eq!(press(KeyCode::Esc, Modifiers::NONE, f), some("<[27u"));
        // Shift+Enter, Ctrl+Enter, Ctrl+Tab, Ctrl+Backspace, Ctrl+I versus Tab.
        assert_eq!(press(KeyCode::Enter, Modifiers::SHIFT, f), some("<[13;2u"));
        assert_eq!(press(KeyCode::Enter, Modifiers::CTRL, f), some("<[13;5u"));
        assert_eq!(press(KeyCode::Tab, Modifiers::CTRL, f), some("<[9;5u"));
        assert_eq!(press(KeyCode::Tab, Modifiers::SHIFT, f), some("<[9;2u"));
        assert_eq!(
            press(KeyCode::Backspace, Modifiers::CTRL, f),
            some("<[127;5u")
        );
        assert_eq!(
            press(KeyCode::Char('i'), Modifiers::CTRL, f),
            some("<[105;5u")
        );
        // Plain keys keep their legacy form.
        assert_eq!(press(KeyCode::Enter, Modifiers::NONE, f), None);
        assert_eq!(press(KeyCode::Tab, Modifiers::NONE, f), None);
        assert_eq!(press(KeyCode::Backspace, Modifiers::NONE, f), None);
        assert_eq!(press(KeyCode::Char('a'), Modifiers::NONE, f), None);
        assert_eq!(press(KeyCode::Char('A'), Modifiers::SHIFT, f), None);
        assert_eq!(press(KeyCode::Up, Modifiers::NONE, f), None);
        assert_eq!(press(KeyCode::F(5), Modifiers::NONE, f), None);
        // Modified text and functional keys.
        assert_eq!(
            press(KeyCode::Char('a'), Modifiers::CTRL, f),
            some("<[97;5u")
        );
        assert_eq!(
            press(KeyCode::Char('a'), Modifiers::ALT, f),
            some("<[97;3u")
        );
        assert_eq!(
            press(KeyCode::Char(' '), Modifiers::ALT, f),
            some("<[32;3u")
        );
        assert_eq!(press(KeyCode::Char('i'), CTRL_SHIFT, f), some("<[105;6u"));
        assert_eq!(press(KeyCode::Up, Modifiers::SHIFT, f), some("<[1;2A"));
        assert_eq!(press(KeyCode::F(1), Modifiers::CTRL, f), some("<[1;5P"));
        assert_eq!(press(KeyCode::F(3), Modifiers::ALT, f), some("<[13;3~"));
        assert_eq!(press(KeyCode::Delete, Modifiers::CTRL, f), some("<[3;5~"));
        assert_eq!(press(KeyCode::F(12), Modifiers::SHIFT, f), some("<[24;2~"));
        let sup = Modifiers {
            sup: true,
            ..Modifiers::NONE
        };
        let all = Modifiers {
            ctrl: true,
            alt: true,
            shift: true,
            sup: true,
            hyper: true,
            meta: true,
        };
        assert_eq!(press(KeyCode::Char('a'), sup, f), some("<[97;9u"));
        assert_eq!(press(KeyCode::Char('a'), all, f), some("<[97;64u"));
        // Keys with no legacy form.
        assert_eq!(press(KeyCode::F(13), Modifiers::NONE, f), some("<[57376u"));
        assert_eq!(press(KeyCode::F(35), Modifiers::NONE, f), some("<[57398u"));
        assert_eq!(
            press(KeyCode::Media(MediaKey::MuteVolume), Modifiers::NONE, f),
            some("<[57440u")
        );
        assert_eq!(
            press(KeyCode::KeypadBegin, Modifiers::NONE, f),
            some("<[57427u")
        );
        assert_eq!(
            press(KeyCode::Special(SpecialKey::Menu), Modifiers::NONE, f),
            some("<[57363u")
        );
        // Modifier and lock keys only with flag 8.
        assert_eq!(
            press(
                KeyCode::Modifier(ModifierKey::LeftShift),
                Modifiers::SHIFT,
                f
            ),
            some("")
        );
    }

    #[test]
    fn kitty_keypad_keys() {
        let f = KITTY_DISAMBIGUATE;
        let pad = |code, mods, flags| {
            encode_kitty(
                &KeyEvent::new(code, mods).on_keypad(),
                KeyEventKind::Press,
                flags,
            )
            .map(|b| String::from_utf8(b).unwrap().replace('\x1b', "<"))
        };
        assert_eq!(
            pad(KeyCode::Enter, Modifiers::NONE, f),
            Some("<[57414u".into())
        );
        assert_eq!(
            pad(KeyCode::Left, Modifiers::NONE, f),
            Some("<[57417u".into())
        );
        assert_eq!(
            pad(KeyCode::Delete, Modifiers::CTRL, f),
            Some("<[57426;5u".into())
        );
        // A keypad digit is text until every key is an escape code.
        assert_eq!(pad(KeyCode::Char('5'), Modifiers::NONE, f), None);
        assert_eq!(
            pad(KeyCode::Char('5'), Modifiers::NONE, KITTY_ALL_KEYS),
            Some("<[57404u".into())
        );
        assert_eq!(
            pad(KeyCode::Char('+'), Modifiers::NONE, KITTY_ALL_KEYS),
            Some("<[57413u".into())
        );
    }

    #[test]
    fn kitty_event_types() {
        let f = KITTY_DISAMBIGUATE | KITTY_EVENT_TYPES;
        let some = |s: &str| Some(s.to_string());
        let on = |code, mods, kind| kitty(code, mods, kind, f);
        // Releases and repeats of text keys, functional keys and modified keys.
        assert_eq!(
            on(KeyCode::Char('a'), Modifiers::NONE, KeyEventKind::Release),
            some("<[97;1:3u")
        );
        // A repeat of a text key is its text, as in Ghostty.
        assert_eq!(
            on(KeyCode::Char('a'), Modifiers::NONE, KeyEventKind::Repeat),
            None
        );
        assert_eq!(
            on(KeyCode::Char('a'), Modifiers::CTRL, KeyEventKind::Repeat),
            some("<[97;5:2u")
        );
        assert_eq!(
            on(KeyCode::Char('a'), Modifiers::NONE, KeyEventKind::Press),
            None
        );
        assert_eq!(
            on(KeyCode::Up, Modifiers::NONE, KeyEventKind::Release),
            some("<[1;1:3A")
        );
        assert_eq!(
            on(KeyCode::Delete, Modifiers::CTRL, KeyEventKind::Release),
            some("<[3;5:3~")
        );
        assert_eq!(
            on(KeyCode::Esc, Modifiers::NONE, KeyEventKind::Release),
            some("<[27;1:3u")
        );
        assert_eq!(
            on(KeyCode::Enter, Modifiers::SHIFT, KeyEventKind::Release),
            some("")
        );
        // Enter, Tab and Backspace have no release without flag 8.
        for code in [KeyCode::Enter, KeyCode::Tab, KeyCode::Backspace] {
            assert_eq!(on(code, Modifiers::NONE, KeyEventKind::Release), some(""));
            assert_eq!(on(code, Modifiers::NONE, KeyEventKind::Repeat), None);
        }
        // Without flag 2 a release is nothing and a repeat is a press.
        assert_eq!(
            kitty(
                KeyCode::Char('a'),
                Modifiers::CTRL,
                KeyEventKind::Release,
                KITTY_DISAMBIGUATE
            ),
            some("")
        );
        assert_eq!(
            kitty(
                KeyCode::Char('a'),
                Modifiers::CTRL,
                KeyEventKind::Repeat,
                KITTY_DISAMBIGUATE
            ),
            some("<[97;5u")
        );
    }

    #[test]
    fn kitty_alternate_keys() {
        let f = KITTY_DISAMBIGUATE | KITTY_ALTERNATE_KEYS;
        // The terminal reports the shifted key (and drops Shift); the key number is the base.
        assert_eq!(
            press(KeyCode::Char('A'), Modifiers::NONE, f | KITTY_ALL_KEYS),
            Some("<[97:65;2u".into())
        );
        assert_eq!(
            press(KeyCode::Char('A'), Modifiers::SHIFT, f | KITTY_ALL_KEYS),
            Some("<[97:65;2u".into())
        );
        assert_eq!(
            press(KeyCode::Char('A'), Modifiers::CTRL, f),
            Some("<[97:65;6u".into())
        );
        // Without the flag, no shifted key.
        assert_eq!(
            press(KeyCode::Char('A'), Modifiers::CTRL, KITTY_DISAMBIGUATE),
            Some("<[97;6u".into())
        );
        // A key with no shifted form gets none.
        assert_eq!(
            press(KeyCode::Char('a'), Modifiers::CTRL, f),
            Some("<[97;5u".into())
        );
    }

    #[test]
    fn kitty_all_keys() {
        let f = KITTY_DISAMBIGUATE | KITTY_ALL_KEYS;
        let some = |s: &str| Some(s.to_string());
        assert_eq!(press(KeyCode::Char('a'), Modifiers::NONE, f), some("<[97u"));
        assert_eq!(
            press(KeyCode::Char('a'), Modifiers::SHIFT, f),
            some("<[97;2u")
        );
        assert_eq!(
            press(KeyCode::Char('A'), Modifiers::SHIFT, f),
            some("<[97;2u")
        );
        assert_eq!(press(KeyCode::Char(' '), Modifiers::NONE, f), some("<[32u"));
        assert_eq!(press(KeyCode::Enter, Modifiers::NONE, f), some("<[13u"));
        assert_eq!(press(KeyCode::Tab, Modifiers::NONE, f), some("<[9u"));
        assert_eq!(
            press(KeyCode::Backspace, Modifiers::NONE, f),
            some("<[127u")
        );
        assert_eq!(press(KeyCode::Up, Modifiers::NONE, f), some("<[A"));
        assert_eq!(press(KeyCode::F(1), Modifiers::NONE, f), some("<[P"));
        assert_eq!(press(KeyCode::Insert, Modifiers::NONE, f), some("<[2~"));
        assert_eq!(
            press(
                KeyCode::Modifier(ModifierKey::LeftShift),
                Modifiers::SHIFT,
                f
            ),
            some("<[57441;2u")
        );
        assert_eq!(
            press(
                KeyCode::Modifier(ModifierKey::RightControl),
                Modifiers::CTRL,
                f
            ),
            some("<[57448;5u")
        );
        // A modifier key's release no longer holds the modifier.
        assert_eq!(
            kitty(
                KeyCode::Modifier(ModifierKey::LeftShift),
                Modifiers::SHIFT,
                KeyEventKind::Release,
                f | KITTY_EVENT_TYPES
            ),
            some("<[57441;1:3u")
        );
        assert_eq!(
            press(KeyCode::Special(SpecialKey::CapsLock), Modifiers::NONE, f),
            some("<[57358u")
        );
        // Release of Enter is reported with flag 8.
        assert_eq!(
            kitty(
                KeyCode::Enter,
                Modifiers::NONE,
                KeyEventKind::Release,
                f | KITTY_EVENT_TYPES
            ),
            some("<[13;1:3u")
        );
    }

    #[test]
    fn kitty_associated_text() {
        let f = KITTY_DISAMBIGUATE | KITTY_ALL_KEYS | KITTY_TEXT;
        let some = |s: &str| Some(s.to_string());
        assert_eq!(
            press(KeyCode::Char('a'), Modifiers::NONE, f),
            some("<[97;;97u")
        );
        assert_eq!(
            press(KeyCode::Char('a'), Modifiers::SHIFT, f),
            some("<[97;2;65u")
        );
        assert_eq!(
            press(KeyCode::Char('A'), Modifiers::NONE, f),
            some("<[97;2;65u")
        );
        assert_eq!(
            press(KeyCode::Char('é'), Modifiers::NONE, f),
            some("<[233;;233u")
        );
        // Not for keys that type nothing, with Ctrl or Alt held, or for a release.
        assert_eq!(press(KeyCode::Enter, Modifiers::NONE, f), some("<[13u"));
        assert_eq!(
            press(KeyCode::Char('a'), Modifiers::CTRL, f),
            some("<[97;5u")
        );
        assert_eq!(
            press(KeyCode::Char('a'), Modifiers::ALT, f),
            some("<[97;3u")
        );
        assert_eq!(
            kitty(
                KeyCode::Char('a'),
                Modifiers::NONE,
                KeyEventKind::Release,
                f | KITTY_EVENT_TYPES
            ),
            some("<[97;1:3u")
        );
        // Text goes with the shifted key too.
        assert_eq!(
            press(
                KeyCode::Char('A'),
                Modifiers::NONE,
                f | KITTY_ALTERNATE_KEYS
            ),
            some("<[97:65;2;65u")
        );
    }

    #[test]
    fn encode_key_falls_back_to_the_legacy_encoding() {
        let flags = |f| Modes::default().with_kitty_flags(f);
        let enter = KeyEvent::new(KeyCode::Enter, Modifiers::SHIFT);
        let press = KeyEventKind::Press;
        assert_eq!(encode_key(&enter, press, flags(1)), b"\x1b[13;2u");
        // No flags: the legacy form, and no release.
        assert_eq!(encode_key(&enter, press, flags(0)), b"\r");
        assert_eq!(encode_key(&enter, KeyEventKind::Release, flags(0)), b"");
        let a = KeyEvent::new(KeyCode::Char('a'), Modifiers::NONE);
        assert_eq!(encode_key(&a, press, flags(1)), b"a");
        assert_eq!(encode_key(&a, KeyEventKind::Release, flags(1)), b"");
        assert_eq!(
            encode_key(&a, KeyEventKind::Release, flags(3)),
            b"\x1b[97;1:3u"
        );
        // The modes' other bits still count for the legacy form.
        let up = KeyEvent::new(KeyCode::Up, Modifiers::NONE);
        let app = Modes(Modes::APPLICATION_CURSOR).with_kitty_flags(1);
        assert_eq!(encode_key(&up, press, app), b"\x1bOA");
        // Keys the legacy encoding cannot write give nothing.
        let f13 = KeyEvent::new(KeyCode::F(13), Modifiers::NONE);
        assert_eq!(encode_key(&f13, press, flags(0)), b"");
        assert_eq!(encode_key(&f13, press, flags(1)), b"\x1b[57376u");
    }

    #[test]
    fn the_kitty_flags_live_in_the_modes() {
        let modes = Modes(Modes::ALTERNATE_SCROLL | Modes::BRACKETED_PASTE);
        assert_eq!(modes.kitty_flags(), 0);
        let with = modes.with_kitty_flags(0b10101);
        assert_eq!(with.kitty_flags(), 0b10101);
        assert!(with.has(Modes::ALTERNATE_SCROLL) && with.has(Modes::BRACKETED_PASTE));
        assert_eq!(with.with_kitty_flags(0), modes);
        // Only the five defined bits.
        assert_eq!(modes.with_kitty_flags(0xff).kitty_flags(), 31);
    }
}
