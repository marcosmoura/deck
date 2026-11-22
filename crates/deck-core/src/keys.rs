//! Key specs (`M-Space`, `C-a`), key events and xterm key encoding.

use std::fmt;
use std::str::FromStr;

use crate::protocol::Modes;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
    /// `F(1)` to `F(12)`.
    F(u8),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Modifiers {
    pub const NONE: Self = Self {
        ctrl: false,
        alt: false,
        shift: false,
    };
    pub const CTRL: Self = Self {
        ctrl: true,
        alt: false,
        shift: false,
    };
    pub const ALT: Self = Self {
        ctrl: false,
        alt: true,
        shift: false,
    };
    pub const SHIFT: Self = Self {
        ctrl: false,
        alt: false,
        shift: true,
    };
}

/// A key press, as the client converts it from the outer terminal's events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyEvent {
    pub code: KeyCode,
    pub mods: Modifiers,
}

impl KeyEvent {
    pub fn new(code: KeyCode, mods: Modifiers) -> Self {
        Self { code, mods }
    }
}

/// A key as written in the config: a code and modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
        KeyEvent {
            code: self.code,
            mods: self.mods,
        }
    }

    /// Whether `event` is this key. For character keys the `Shift` flag is ignored on both
    /// sides (terminals report `X` with `Shift`); `S-` matters only for named keys.
    pub fn matches(&self, event: &KeyEvent) -> bool {
        if self.code != event.code {
            return false;
        }
        let mut a = self.mods;
        let mut b = event.mods;
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
        KeyCode::F(n @ 5..=12) => tilde([15, 17, 18, 19, 20, 21, 23, 24][usize::from(n) - 5]),
        KeyCode::F(_) => Vec::new(),
    }
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
        KeyEvent::new(code, Modifiers { ctrl, alt, shift })
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
                },
                4,
            ),
            (Modifiers::CTRL, 5),
            (
                Modifiers {
                    ctrl: true,
                    alt: false,
                    shift: true,
                },
                6,
            ),
            (
                Modifiers {
                    ctrl: true,
                    alt: true,
                    shift: false,
                },
                7,
            ),
            (
                Modifiers {
                    ctrl: true,
                    alt: true,
                    shift: true,
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
}
