//! Keys: conversion from crossterm events and the binding table.

use std::collections::BTreeMap;

use deck_core::config::{Config, DEFAULT_BINDINGS, HelpGroup};
use deck_core::keys::{
    KITTY_ALL_KEYS, KITTY_DISAMBIGUATE, KITTY_EVENT_TYPES, KITTY_TEXT, KeyCode, KeyEvent,
    KeyEventKind, KeySpec, MediaKey, ModifierKey, Modifiers, SpecialKey,
};
use deck_core::layout::Direction;
use ratatui::crossterm::event as ct;

/// What a bound key does. Later specs add actions here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Detach,
    Sessions,
    RenameSession,
    /// Opens the Restore dialog.
    Restore,
    Scroll,
    NewTerminal,
    CloseTerminal,
    /// Moves focus to the terminal in that direction.
    FocusTerminal(Direction),
    /// Moves the border of the focused terminal's edge in that direction.
    Resize(Direction),
    ResetLayout,
    NewPanel,
    NextPanel,
    PrevPanel,
    /// The N-th panel of the active project, from 1.
    GotoPanel(usize),
    RenamePanel,
    ClosePanel,
    NewProject,
    /// Opens the New menu for the active project.
    NewMenu,
    RenameProject,
    CloseProject,
    ToggleSidebar,
    /// Collapses the sidebar to its narrow strip, or expands it.
    CollapseSidebar,
    FocusSidebar,
    /// Jumps to the next agent that needs the user.
    NextAttention,
    Help,
}

impl Action {
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "detach" => Self::Detach,
            "sessions" => Self::Sessions,
            "rename_session" => Self::RenameSession,
            "restore" => Self::Restore,
            "scroll" => Self::Scroll,
            "new_terminal" => Self::NewTerminal,
            "close_terminal" => Self::CloseTerminal,
            "focus_left" => Self::FocusTerminal(Direction::Left),
            "focus_right" => Self::FocusTerminal(Direction::Right),
            "focus_up" => Self::FocusTerminal(Direction::Up),
            "focus_down" => Self::FocusTerminal(Direction::Down),
            "resize_left" => Self::Resize(Direction::Left),
            "resize_right" => Self::Resize(Direction::Right),
            "resize_up" => Self::Resize(Direction::Up),
            "resize_down" => Self::Resize(Direction::Down),
            "reset_layout" => Self::ResetLayout,
            "new_panel" => Self::NewPanel,
            "next_panel" => Self::NextPanel,
            "prev_panel" => Self::PrevPanel,
            "rename_panel" => Self::RenamePanel,
            "close_panel" => Self::ClosePanel,
            "new_space" => Self::NewProject,
            "new_menu" => Self::NewMenu,
            "rename_space" => Self::RenameProject,
            "close_space" => Self::CloseProject,
            "toggle_sidebar" => Self::ToggleSidebar,
            "collapse_sidebar" => Self::CollapseSidebar,
            "focus_sidebar" => Self::FocusSidebar,
            "next_attention" => Self::NextAttention,
            "help" => Self::Help,
            other => {
                let n: usize = other.strip_prefix("panel_")?.parse().ok()?;
                (1..=9).contains(&n).then_some(Self::GotoPanel(n))?
            }
        })
    }
}

/// The help lines of the keys typed in scroll mode, after the `scroll` binding's keys.
const SCROLL_MODE_HELP: &[(&str, &str)] = &[
    ("h j k l", "Scroll mode: move the copy cursor"),
    ("w b e 0 ^ $", "Scroll mode: move by word and in the line"),
    (
        "H M L g G",
        "Scroll mode: top, middle, bottom of the screen, of the history",
    ),
    ("C-u C-d C-b C-f", "Scroll mode: half and whole pages"),
    (
        "v V o y Enter",
        "Scroll mode: select, select lines, other end, copy",
    ),
    (
        "/ ? n N",
        "Scroll mode: search down, up, next, the other way",
    ),
    ("q Esc", "Scroll mode: clear the selection, then leave"),
];

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

    /// The first key of an action, without the prefix, e.g. `?` for `help`.
    pub fn first_key(&self, action: Action) -> Option<String> {
        let keys = &self.actions.iter().find(|(a, _)| *a == action)?.1;
        Some(spec_label(keys.first()?))
    }

    /// The hints while the resize repeat is on as key and label pairs, e.g. `H J K L resize`
    /// and `= reset`.
    pub fn resize_hints(&self) -> Vec<(String, String)> {
        let first = |action| {
            let keys = &self.actions.iter().find(|(a, _)| *a == action)?.1;
            Some(spec_label(keys.first()?))
        };
        let moves: Vec<String> = [
            Direction::Left,
            Direction::Down,
            Direction::Up,
            Direction::Right,
        ]
        .into_iter()
        .filter_map(|d| first(Action::Resize(d)))
        .collect();
        let mut parts = Vec::new();
        if !moves.is_empty() {
            parts.push((moves.join(" "), "resize".to_string()));
        }
        if let Some(reset) = first(Action::ResetLayout) {
            parts.push((reset, "reset".to_string()));
        }
        parts
    }

    /// Every action with its keys, in the order of the binding table, for the help overlay.
    /// `panel_1` to `panel_9` share one line, and so do the four `focus_*` actions and the
    /// four `resize_*` ones.
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
            } else if def.name == "focus_left" {
                let all: Vec<String> = ["left", "right", "up", "down"]
                    .iter()
                    .map(|d| labels(&format!("focus_{d}")).join("/"))
                    .collect();
                (all.join(" "), "Focus the terminal by direction".to_string())
            } else if def.name == "resize_left" {
                let all: Vec<String> = ["left", "right", "up", "down"]
                    .iter()
                    .map(|d| labels(&format!("resize_{d}")).join("/"))
                    .collect();
                (
                    all.join(" "),
                    "Resize the terminal by direction".to_string(),
                )
            } else if def.name.starts_with("panel_")
                || matches!(
                    def.name,
                    "focus_right"
                        | "focus_up"
                        | "focus_down"
                        | "resize_right"
                        | "resize_up"
                        | "resize_down"
                )
            {
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
        // The keys of scroll mode are fixed; they follow the `scroll` binding, and none
        // are listed while it is unbound.
        if let Some(enter) = self.key_label(Action::Scroll) {
            for (keys, text) in SCROLL_MODE_HELP {
                rows.push(HelpRow {
                    group: HelpGroup::Terminal,
                    keys: format!("{enter} {keys}"),
                    text: (*text).to_string(),
                });
            }
        }
        rows
    }
}

/// A key for the UI: `⌥Space` on macOS, `M-Space` elsewhere.
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

/// Converts a crossterm key event; `None` for keys deck cannot encode. Releases convert like
/// presses; [`kind`] tells them apart.
pub fn convert(key: ct::KeyEvent) -> Option<KeyEvent> {
    let mut mods = Modifiers {
        ctrl: key.modifiers.contains(ct::KeyModifiers::CONTROL),
        alt: key.modifiers.contains(ct::KeyModifiers::ALT),
        shift: key.modifiers.contains(ct::KeyModifiers::SHIFT),
        sup: key.modifiers.contains(ct::KeyModifiers::SUPER),
        hyper: key.modifiers.contains(ct::KeyModifiers::HYPER),
        meta: key.modifiers.contains(ct::KeyModifiers::META),
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
        ct::KeyCode::F(n @ 1..=35) => KeyCode::F(n),
        ct::KeyCode::KeypadBegin => KeyCode::KeypadBegin,
        ct::KeyCode::CapsLock => KeyCode::Special(SpecialKey::CapsLock),
        ct::KeyCode::ScrollLock => KeyCode::Special(SpecialKey::ScrollLock),
        ct::KeyCode::NumLock => KeyCode::Special(SpecialKey::NumLock),
        ct::KeyCode::PrintScreen => KeyCode::Special(SpecialKey::PrintScreen),
        ct::KeyCode::Pause => KeyCode::Special(SpecialKey::Pause),
        ct::KeyCode::Menu => KeyCode::Special(SpecialKey::Menu),
        ct::KeyCode::Media(media) => KeyCode::Media(media_key(media)),
        ct::KeyCode::Modifier(modifier) => KeyCode::Modifier(modifier_key(modifier)),
        _ => return None,
    };
    let event = KeyEvent::new(code, mods);
    Some(if key.state.contains(ct::KeyEventState::KEYPAD) {
        event.on_keypad()
    } else {
        event
    })
}

/// Whether a crossterm key event is a press, a repeat or a release.
pub fn kind(key: &ct::KeyEvent) -> KeyEventKind {
    match key.kind {
        ct::KeyEventKind::Press => KeyEventKind::Press,
        ct::KeyEventKind::Repeat => KeyEventKind::Repeat,
        ct::KeyEventKind::Release => KeyEventKind::Release,
    }
}

fn media_key(key: ct::MediaKeyCode) -> MediaKey {
    match key {
        ct::MediaKeyCode::Play => MediaKey::Play,
        ct::MediaKeyCode::Pause => MediaKey::Pause,
        ct::MediaKeyCode::PlayPause => MediaKey::PlayPause,
        ct::MediaKeyCode::Reverse => MediaKey::Reverse,
        ct::MediaKeyCode::Stop => MediaKey::Stop,
        ct::MediaKeyCode::FastForward => MediaKey::FastForward,
        ct::MediaKeyCode::Rewind => MediaKey::Rewind,
        ct::MediaKeyCode::TrackNext => MediaKey::TrackNext,
        ct::MediaKeyCode::TrackPrevious => MediaKey::TrackPrevious,
        ct::MediaKeyCode::Record => MediaKey::Record,
        ct::MediaKeyCode::LowerVolume => MediaKey::LowerVolume,
        ct::MediaKeyCode::RaiseVolume => MediaKey::RaiseVolume,
        ct::MediaKeyCode::MuteVolume => MediaKey::MuteVolume,
    }
}

fn modifier_key(key: ct::ModifierKeyCode) -> ModifierKey {
    match key {
        ct::ModifierKeyCode::LeftShift => ModifierKey::LeftShift,
        ct::ModifierKeyCode::LeftControl => ModifierKey::LeftControl,
        ct::ModifierKeyCode::LeftAlt => ModifierKey::LeftAlt,
        ct::ModifierKeyCode::LeftSuper => ModifierKey::LeftSuper,
        ct::ModifierKeyCode::LeftHyper => ModifierKey::LeftHyper,
        ct::ModifierKeyCode::LeftMeta => ModifierKey::LeftMeta,
        ct::ModifierKeyCode::RightShift => ModifierKey::RightShift,
        ct::ModifierKeyCode::RightControl => ModifierKey::RightControl,
        ct::ModifierKeyCode::RightAlt => ModifierKey::RightAlt,
        ct::ModifierKeyCode::RightSuper => ModifierKey::RightSuper,
        ct::ModifierKeyCode::RightHyper => ModifierKey::RightHyper,
        ct::ModifierKeyCode::RightMeta => ModifierKey::RightMeta,
        ct::ModifierKeyCode::IsoLevel3Shift => ModifierKey::IsoLevel3Shift,
        ct::ModifierKeyCode::IsoLevel5Shift => ModifierKey::IsoLevel5Shift,
    }
}

/// The kitty keyboard flags to push on the outer terminal for a program that set `program`
/// (0: push nothing). The outer terminal gets what the client needs to build the program's
/// encoding: flag 1 always, 2 for the events, 4 (the shifted key) when the program wants
/// every key or its text. Never 8 (crossterm would then report Caps Lock in a state deck
/// drops, and AltGr characters as their base key, so typing would come out wrong; the cost is
/// that a program gets no modifier-key presses) and never 16 (the client computes the text).
pub fn outer_flags(program: u8) -> u8 {
    if program == 0 {
        return 0;
    }
    let wants_text_keys = program & (KITTY_ALL_KEYS | KITTY_TEXT) != 0;
    KITTY_DISAMBIGUATE | (program & KITTY_EVENT_TYPES) | if wants_text_keys { 4 } else { 0 }
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
                    ..Modifiers::NONE
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
            convert(press(ct::KeyCode::F(36), ct::KeyModifiers::NONE)),
            None
        );
        assert_eq!(
            convert(press(ct::KeyCode::Null, ct::KeyModifiers::NONE)),
            None
        );
    }

    #[test]
    fn back_tab_is_shift_tab_and_releases_are_told_apart() {
        let k = convert(press(ct::KeyCode::BackTab, ct::KeyModifiers::SHIFT)).unwrap();
        assert_eq!(k, KeyEvent::new(KeyCode::Tab, Modifiers::SHIFT));
        let mut release = press(ct::KeyCode::Char('a'), ct::KeyModifiers::NONE);
        assert_eq!(kind(&release), KeyEventKind::Press);
        release.kind = ct::KeyEventKind::Release;
        assert_eq!(kind(&release), KeyEventKind::Release);
        release.kind = ct::KeyEventKind::Repeat;
        assert_eq!(kind(&release), KeyEventKind::Repeat);
    }

    #[test]
    fn the_keys_kitty_reports_convert() {
        let k = |code, mods| convert(press(code, mods)).unwrap();
        assert_eq!(
            k(ct::KeyCode::F(35), ct::KeyModifiers::NONE).code,
            KeyCode::F(35)
        );
        assert_eq!(
            k(
                ct::KeyCode::Media(ct::MediaKeyCode::MuteVolume),
                ct::KeyModifiers::NONE
            )
            .code,
            KeyCode::Media(MediaKey::MuteVolume)
        );
        assert_eq!(
            k(
                ct::KeyCode::Modifier(ct::ModifierKeyCode::RightControl),
                ct::KeyModifiers::CONTROL
            ),
            KeyEvent::new(
                KeyCode::Modifier(ModifierKey::RightControl),
                Modifiers::CTRL
            )
        );
        assert_eq!(
            k(ct::KeyCode::CapsLock, ct::KeyModifiers::NONE).code,
            KeyCode::Special(SpecialKey::CapsLock)
        );
        let mods = k(
            ct::KeyCode::Char('a'),
            ct::KeyModifiers::SUPER | ct::KeyModifiers::HYPER | ct::KeyModifiers::META,
        )
        .mods;
        assert!(mods.sup && mods.hyper && mods.meta && !mods.ctrl);
        // The keypad's keys are the same keys, marked.
        let mut pad = press(ct::KeyCode::Char('5'), ct::KeyModifiers::NONE);
        pad.state = ct::KeyEventState::KEYPAD;
        let pad = convert(pad).unwrap();
        assert_eq!(pad.code, KeyCode::Char('5'));
        assert!(pad.keypad);
        assert!(!k(ct::KeyCode::Char('5'), ct::KeyModifiers::NONE).keypad);
    }

    #[test]
    fn the_outer_flags_follow_the_programs() {
        for (program, outer) in [
            (0, 0),
            (1, 1),
            (2, 3),
            (3, 3),
            (8, 1 | 4),
            (9, 1 | 4),
            (16, 1 | 4),
            (31, 1 | 2 | 4),
        ] {
            assert_eq!(outer_flags(program), outer, "{program}");
            // Never the associated text.
            assert_eq!(outer_flags(program) & 16, 0);
        }
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
        assert_eq!(b.lookup(&key('t')), Some(Action::NewTerminal));
        assert_eq!(b.lookup(&key('x')), Some(Action::CloseTerminal));
        for (c, code, dir) in [
            ('h', KeyCode::Left, Direction::Left),
            ('l', KeyCode::Right, Direction::Right),
            ('k', KeyCode::Up, Direction::Up),
            ('j', KeyCode::Down, Direction::Down),
        ] {
            let want = Some(Action::FocusTerminal(dir));
            assert_eq!(b.lookup(&key(c)), want);
            assert_eq!(b.lookup(&KeyEvent::new(code, Modifiers::NONE)), want);
        }
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
        assert_eq!(
            rows.len(),
            DEFAULT_BINDINGS.len() - 14 + SCROLL_MODE_HELP.len()
        );
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
