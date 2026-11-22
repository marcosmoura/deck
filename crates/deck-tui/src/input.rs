//! Keys: conversion from crossterm events and the binding table.

use std::collections::BTreeMap;

use deck_core::config::{Config, DEFAULT_BINDINGS, HelpGroup};
use deck_core::keys::{KeyCode, KeyEvent, KeySpec, Modifiers};
use ratatui::crossterm::event as ct;

/// What a bound key does. Later specs add actions here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Detach,
    Sessions,
    RenameSession,
    Scroll,
    NewPanel,
    NextPanel,
    PrevPanel,
    /// The N-th panel of the active project, from 1.
    GotoPanel(usize),
    RenamePanel,
    ClosePanel,
    NewProject,
    RenameProject,
    CloseProject,
    ToggleSidebar,
    FocusSidebar,
    Help,
}

impl Action {
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "detach" => Self::Detach,
            "sessions" => Self::Sessions,
            "rename_session" => Self::RenameSession,
            "scroll" => Self::Scroll,
            "new_panel" => Self::NewPanel,
            "next_panel" => Self::NextPanel,
            "prev_panel" => Self::PrevPanel,
            "rename_panel" => Self::RenamePanel,
            "close_panel" => Self::ClosePanel,
            "new_project" => Self::NewProject,
            "rename_project" => Self::RenameProject,
            "close_project" => Self::CloseProject,
            "toggle_sidebar" => Self::ToggleSidebar,
            "focus_sidebar" => Self::FocusSidebar,
            "help" => Self::Help,
            other => {
                let n: usize = other.strip_prefix("panel_")?.parse().ok()?;
                (1..=9).contains(&n).then_some(Self::GotoPanel(n))?
            }
        })
    }
}

/// The actions that show in the status bar's hints, in order, with their label.
const HINTED: &[(Action, &str)] = &[(Action::Help, "help"), (Action::Detach, "detach")];

/// One line of the help overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpRow {
    pub group: HelpGroup,
    /// The prefix and the key(s), e.g. `⌥Space c`.
    pub keys: String,
    pub text: String,
}

/// The prefix and the keys bound after it.
#[derive(Debug, Clone)]
pub struct Bindings {
    prefix: KeySpec,
    actions: Vec<(Action, Vec<KeySpec>)>,
    table: BTreeMap<String, Vec<KeySpec>>,
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
            table: config.keys.clone(),
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

    /// The prefix as shown in the UI.
    pub fn prefix_label(&self) -> String {
        spec_label(&self.prefix)
    }

    /// The prefix and the first key of an action, e.g. `⌥Space c`.
    pub fn key_label(&self, action: Action) -> Option<String> {
        let keys = &self.actions.iter().find(|(a, _)| *a == action)?.1;
        Some(format!(
            "{} {}",
            self.prefix_label(),
            spec_label(keys.first()?)
        ))
    }

    /// The hint text of the status bar, e.g. `⌥Space ? help  d detach`.
    pub fn hints(&self) -> String {
        let items: Vec<String> = HINTED
            .iter()
            .filter_map(|(action, label)| {
                let keys = &self.actions.iter().find(|(a, _)| a == action)?.1;
                Some(format!("{} {label}", spec_label(keys.first()?)))
            })
            .collect();
        if items.is_empty() {
            return String::new();
        }
        format!("{} {}", self.prefix_label(), items.join("  "))
    }

    /// Every action with its keys, in the order of the binding table, for the help overlay.
    /// `panel_1` to `panel_9` share one line.
    pub fn help_rows(&self) -> Vec<HelpRow> {
        let prefix = self.prefix_label();
        let labels = |name: &str| -> Vec<String> {
            self.table
                .get(name)
                .map(|keys| keys.iter().map(spec_label).collect())
                .unwrap_or_default()
        };
        let mut rows = Vec::new();
        for def in DEFAULT_BINDINGS {
            let (keys, text) = if def.name == "panel_1" {
                let all: Vec<Vec<String>> =
                    (1..=9).map(|n| labels(&format!("panel_{n}"))).collect();
                let default = all
                    .iter()
                    .enumerate()
                    .all(|(i, l)| *l == [(i + 1).to_string()]);
                let keys = if default {
                    "1-9".to_string()
                } else {
                    all.iter()
                        .map(|l| l.join("/"))
                        .collect::<Vec<_>>()
                        .join(" ")
                };
                (keys, "Focus panel N".to_string())
            } else if def.name.starts_with("panel_") {
                continue;
            } else {
                (labels(def.name).join("/"), def.help.to_string())
            };
            // An action the user unbound (`detach = []`) stays listed, so it is visible.
            let keys = if keys.trim().is_empty() {
                "unbound".to_string()
            } else {
                format!("{prefix} {keys}")
            };
            rows.push(HelpRow {
                group: def.group,
                keys,
                text,
            });
        }
        rows
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
        let config = Config::parse("[keys]\ndetach = \"Z\"").unwrap().config;
        let b = Bindings::from_config(&config);
        let shifted = convert(press(ct::KeyCode::Char('Z'), ct::KeyModifiers::SHIFT)).unwrap();
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
        assert_eq!(b.hints(), format!("{prefix} ? help  d detach"));
    }

    #[test]
    fn the_new_actions_are_bound_and_the_help_lists_them_grouped() {
        let b = Bindings::from_config(&Config::default());
        let key = |c: char| KeyEvent::new(KeyCode::Char(c), Modifiers::NONE);
        assert_eq!(b.lookup(&key('c')), Some(Action::NewPanel));
        assert_eq!(b.lookup(&key('n')), Some(Action::NextPanel));
        assert_eq!(b.lookup(&key('p')), Some(Action::PrevPanel));
        assert_eq!(b.lookup(&key('3')), Some(Action::GotoPanel(3)));
        assert_eq!(b.lookup(&key('0')), None);
        assert_eq!(b.lookup(&key(',')), Some(Action::RenamePanel));
        assert_eq!(
            b.lookup(&KeyEvent::new(KeyCode::Char('X'), Modifiers::SHIFT)),
            Some(Action::ClosePanel)
        );
        assert_eq!(b.lookup(&key('?')), Some(Action::Help));
        assert_eq!(b.lookup(&key('s')), Some(Action::Sessions));
        assert_eq!(b.lookup(&key('$')), Some(Action::RenameSession));
        assert_eq!(
            b.lookup(&KeyEvent::new(KeyCode::Tab, Modifiers::NONE)),
            Some(Action::FocusSidebar)
        );
        let rows = b.help_rows();
        let p = b.prefix_label();
        assert!(rows.iter().any(|r| r.keys == format!("{p} 1-9")));
        assert!(rows.iter().all(|r| !r.text.is_empty()));
        assert!(!rows.iter().any(|r| r.keys.ends_with(" 2")));
        assert_eq!(rows.len(), DEFAULT_BINDINGS.len() - 8);
        // An unbound action stays listed as unbound.
        let config = Config::parse("[keys]\ndetach = []").unwrap().config;
        let rows = Bindings::from_config(&config).help_rows();
        assert!(
            rows.iter()
                .any(|r| r.keys == "unbound" && r.text == "Detach from the session")
        );
        // An override shows its new key.
        let config = Config::parse("[keys]\nnew_panel = [\"t\", \"C-n\"]")
            .unwrap()
            .config;
        let rows = Bindings::from_config(&config).help_rows();
        assert!(
            rows.iter()
                .any(|r| r.keys.starts_with(&format!("{p} t/")) && r.text == "New panel")
        );
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
