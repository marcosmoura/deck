//! Mouse reporting: the bytes a terminal program expects for a mouse event, in SGR or the
//! legacy X10 encoding.

use crate::keys::Modifiers;
use crate::protocol::Modes;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

impl MouseButton {
    fn code(self) -> u16 {
        match self {
            Self::Left => 0,
            Self::Middle => 1,
            Self::Right => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseKind {
    Press(MouseButton),
    Release(MouseButton),
    /// Motion with a button held.
    Drag(MouseButton),
    /// Motion with no button held.
    Move,
    WheelUp,
    WheelDown,
}

/// A mouse event at a cell of the terminal's inner area, counted from 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseEvent {
    pub kind: MouseKind,
    pub col: u16,
    pub row: u16,
    pub mods: Modifiers,
}

/// X10 can name columns and rows up to 223 (a byte, offset by 32).
const X10_LIMIT: u16 = 223;

/// Whether the terminal asked for any mouse reporting.
pub fn reporting(modes: Modes) -> bool {
    modes.0 & (Modes::MOUSE_CLICK | Modes::MOUSE_DRAG | Modes::MOUSE_MOTION) != 0
}

/// The bytes for `event`, with 1-based coordinates. `None` when the modes do not report this
/// kind of event, or when X10 cannot name the cell.
pub fn encode(event: &MouseEvent, modes: Modes) -> Option<Vec<u8>> {
    let any = reporting(modes);
    let drag = modes.0 & (Modes::MOUSE_DRAG | Modes::MOUSE_MOTION) != 0;
    let motion = modes.has(Modes::MOUSE_MOTION);
    let allowed = match event.kind {
        MouseKind::Press(_) | MouseKind::Release(_) | MouseKind::WheelUp | MouseKind::WheelDown => {
            any
        }
        MouseKind::Drag(_) => drag,
        MouseKind::Move => motion,
    };
    if !allowed {
        return None;
    }

    let sgr = modes.has(Modes::SGR_MOUSE);
    let (base, released) = match event.kind {
        MouseKind::Press(b) => (b.code(), false),
        // X10 cannot say which button was released.
        MouseKind::Release(b) => (if sgr { b.code() } else { 3 }, true),
        MouseKind::Drag(b) => (b.code() + 32, false),
        MouseKind::Move => (3 + 32, false),
        MouseKind::WheelUp => (64, false),
        MouseKind::WheelDown => (65, false),
    };
    let code = base
        + if event.mods.shift { 4 } else { 0 }
        + if event.mods.alt { 8 } else { 0 }
        + if event.mods.ctrl { 16 } else { 0 };
    let col = event.col.checked_add(1)?;
    let row = event.row.checked_add(1)?;

    if sgr {
        let end = if released { 'm' } else { 'M' };
        return Some(format!("\x1b[<{code};{col};{row}{end}").into_bytes());
    }
    if col > X10_LIMIT || row > X10_LIMIT {
        return None;
    }
    // All three fit a byte: the code is below 128 and the coordinates are capped above.
    let byte = |v: u16| u8::try_from(v + 32).ok();
    Some(vec![0x1b, b'[', b'M', byte(code)?, byte(col)?, byte(row)?])
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLICK: Modes = Modes(Modes::MOUSE_CLICK);
    const SGR: Modes = Modes(Modes::MOUSE_CLICK | Modes::SGR_MOUSE);

    fn at(kind: MouseKind, col: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            col,
            row,
            mods: Modifiers::NONE,
        }
    }

    fn text(bytes: Option<Vec<u8>>) -> String {
        String::from_utf8(bytes.expect("bytes")).unwrap()
    }

    #[test]
    fn sgr_press_release_drag_and_wheel() {
        use MouseButton::{Left, Middle, Right};
        let all = Modes(Modes::MOUSE_MOTION | Modes::SGR_MOUSE);
        // The top-left inner cell is column 1, row 1; column 5, row 3 is (4, 2).
        assert_eq!(
            text(encode(&at(MouseKind::Press(Left), 4, 2), SGR)),
            "\x1b[<0;5;3M"
        );
        assert_eq!(
            text(encode(&at(MouseKind::Release(Left), 4, 2), SGR)),
            "\x1b[<0;5;3m"
        );
        assert_eq!(
            text(encode(&at(MouseKind::Press(Middle), 0, 0), SGR)),
            "\x1b[<1;1;1M"
        );
        assert_eq!(
            text(encode(&at(MouseKind::Release(Right), 9, 9), SGR)),
            "\x1b[<2;10;10m"
        );
        assert_eq!(
            text(encode(&at(MouseKind::Drag(Left), 1, 1), all)),
            "\x1b[<32;2;2M"
        );
        assert_eq!(
            text(encode(&at(MouseKind::Move, 1, 1), all)),
            "\x1b[<35;2;2M"
        );
        assert_eq!(
            text(encode(&at(MouseKind::WheelUp, 1, 1), SGR)),
            "\x1b[<64;2;2M"
        );
        assert_eq!(
            text(encode(&at(MouseKind::WheelDown, 1, 1), SGR)),
            "\x1b[<65;2;2M"
        );
        // Large coordinates are fine in SGR.
        assert_eq!(
            text(encode(&at(MouseKind::Press(Left), 299, 399), SGR)),
            "\x1b[<0;300;400M"
        );
    }

    #[test]
    fn x10_press_release_drag_and_wheel() {
        use MouseButton::{Left, Right};
        let drag = Modes(Modes::MOUSE_DRAG);
        assert_eq!(
            encode(&at(MouseKind::Press(Left), 4, 2), CLICK),
            Some(vec![0x1b, b'[', b'M', 32, 32 + 5, 32 + 3])
        );
        // The release does not name its button.
        assert_eq!(
            encode(&at(MouseKind::Release(Right), 4, 2), CLICK),
            Some(vec![0x1b, b'[', b'M', 32 + 3, 32 + 5, 32 + 3])
        );
        assert_eq!(
            encode(&at(MouseKind::Press(Right), 0, 0), CLICK),
            Some(vec![0x1b, b'[', b'M', 32 + 2, 33, 33])
        );
        assert_eq!(
            encode(&at(MouseKind::Drag(Left), 0, 0), drag),
            Some(vec![0x1b, b'[', b'M', 32 + 32, 33, 33])
        );
        assert_eq!(
            encode(&at(MouseKind::WheelUp, 0, 0), CLICK),
            Some(vec![0x1b, b'[', b'M', 32 + 64, 33, 33])
        );
        assert_eq!(
            encode(&at(MouseKind::WheelDown, 0, 0), CLICK),
            Some(vec![0x1b, b'[', b'M', 32 + 65, 33, 33])
        );
    }

    #[test]
    fn modifiers_add_to_the_button_code() {
        let ev = MouseEvent {
            kind: MouseKind::Press(MouseButton::Left),
            col: 0,
            row: 0,
            mods: Modifiers {
                ctrl: true,
                alt: true,
                shift: true,
            },
        };
        assert_eq!(text(encode(&ev, SGR)), "\x1b[<28;1;1M");
        assert_eq!(
            encode(&ev, CLICK),
            Some(vec![0x1b, b'[', b'M', 32 + 28, 33, 33])
        );
        let ctrl_wheel = MouseEvent {
            kind: MouseKind::WheelUp,
            mods: Modifiers::CTRL,
            ..ev
        };
        assert_eq!(text(encode(&ctrl_wheel, SGR)), "\x1b[<80;1;1M");
    }

    #[test]
    fn x10_drops_coordinates_above_223() {
        let press = |col, row| at(MouseKind::Press(MouseButton::Left), col, row);
        // 1-based 223 is the last cell X10 can name.
        assert_eq!(
            encode(&press(222, 0), CLICK),
            Some(vec![0x1b, b'[', b'M', 32, 255, 33])
        );
        assert_eq!(encode(&press(223, 0), CLICK), None);
        assert_eq!(encode(&press(0, 223), CLICK), None);
        assert_eq!(encode(&press(u16::MAX, 0), CLICK), None);
        assert!(encode(&press(223, 0), SGR).is_some());
    }

    #[test]
    fn the_modes_decide_what_is_reported() {
        let drag = at(MouseKind::Drag(MouseButton::Left), 0, 0);
        let moved = at(MouseKind::Move, 0, 0);
        let press = at(MouseKind::Press(MouseButton::Left), 0, 0);
        assert_eq!(encode(&press, Modes::default()), None);
        assert!(!reporting(Modes::default()));
        assert!(reporting(CLICK));
        assert_eq!(encode(&drag, CLICK), None);
        assert_eq!(encode(&moved, CLICK), None);
        assert!(encode(&drag, Modes(Modes::MOUSE_DRAG)).is_some());
        assert_eq!(encode(&moved, Modes(Modes::MOUSE_DRAG)), None);
        assert!(encode(&drag, Modes(Modes::MOUSE_MOTION)).is_some());
        assert!(encode(&moved, Modes(Modes::MOUSE_MOTION)).is_some());
        assert!(encode(&press, Modes(Modes::MOUSE_MOTION)).is_some());
    }
}
