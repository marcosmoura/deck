//! Asking the outer terminal for its foreground and background (OSC 10 and OSC 11), its 16
//! base colours (OSC 4), its name (XTVERSION), and whether it speaks the kitty graphics
//! protocol and the kitty keyboard protocol.

use std::time::{Duration, Instant};

use deck_core::protocol::OuterColors;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

/// How long the client waits for the answers.
pub const ANSWER_TIMEOUT: Duration = Duration::from_millis(200);
/// After a timeout, how long colour answers that come late are still taken out of the input.
pub const LATE_WINDOW: Duration = Duration::from_secs(3);

/// The colour queries (foreground, background, then palette entries 0 to 15), BEL-terminated,
/// XTVERSION, the kitty graphics query, the kitty keyboard flags query (`CSI ? u`), then a
/// device-attributes request (DA1). Terminals answer in order, so the DA1 reply tells that no
/// other answer is still on its way (a terminal without the keyboard protocol sends none).
pub const QUERY: &[u8] = b"\x1b]10;?\x07\x1b]11;?\x07\
\x1b]4;0;?\x07\x1b]4;1;?\x07\x1b]4;2;?\x07\x1b]4;3;?\x07\
\x1b]4;4;?\x07\x1b]4;5;?\x07\x1b]4;6;?\x07\x1b]4;7;?\x07\
\x1b]4;8;?\x07\x1b]4;9;?\x07\x1b]4;10;?\x07\x1b]4;11;?\x07\
\x1b]4;12;?\x07\x1b]4;13;?\x07\x1b]4;14;?\x07\x1b]4;15;?\x07\
\x1b[>0q\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\
\x1b[?u\x1b[c";

/// An RGB colour as the terminal reported it.
pub type Rgb = (u8, u8, u8);

/// The outer terminal's own 16 base colours; `None` for an entry it did not answer.
pub type Palette = [Option<Rgb>; 16];

/// What came back from the outer terminal.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Probe {
    /// Both colours, `None` unless the terminal gave both.
    pub colors: Option<OuterColors>,
    /// The foreground alone, for the logos' colour.
    pub fg: Option<Rgb>,
    /// The base colours it answered.
    pub palette: Palette,
    /// The XTVERSION reply: the terminal's name and version, as it wrote them.
    pub xtversion: Option<String>,
    /// The kitty graphics reply: `Some(true)` for `OK`, `Some(false)` for an error text.
    pub graphics: Option<bool>,
    /// The terminal answered the kitty keyboard query: it speaks the protocol.
    pub kitty_keyboard: bool,
    /// Bytes that were not part of an answer, to pass on as input.
    pub leftover: Vec<u8>,
    /// Answers that may still arrive: 0 once the DA1 reply was seen, else the number of
    /// colours, palette entries and other replies missing.
    pub late: u8,
}

/// Reads input for at most the given time: `Some(bytes)` (empty after a timeout), `None`
/// when the input is closed.
pub type ReadInput<'a> = &'a mut dyn FnMut(Duration) -> Option<Vec<u8>>;

/// Collects the answers to [`QUERY`], stopping at the DA1 reply and in any case after
/// `timeout`.
pub fn probe(read: ReadInput<'_>, timeout: Duration) -> Probe {
    let deadline = Instant::now() + timeout;
    let mut buf = Vec::new();
    loop {
        if parse_answers(&buf).da1 {
            break;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        match read(left) {
            Some(bytes) => buf.extend_from_slice(&bytes),
            None => break,
        }
    }
    let parsed = parse_answers(&buf);
    let late = if parsed.da1 {
        0
    } else {
        let missing = parsed.palette.iter().filter(|c| c.is_none()).count();
        u8::from(parsed.fg.is_none())
            + u8::from(parsed.bg.is_none())
            + u8::from(parsed.xtversion.is_none())
            + u8::from(parsed.graphics.is_none())
            + u8::try_from(missing).unwrap_or(16)
    };
    Probe {
        late,
        fg: parsed.fg,
        palette: parsed.palette,
        xtversion: parsed.xtversion,
        graphics: parsed.graphics,
        kitty_keyboard: parsed.kitty_keyboard,
        colors: parsed
            .fg
            .zip(parsed.bg)
            .map(|(fg, bg)| OuterColors { fg, bg }),
        leftover: parsed.rest,
    }
}

/// Takes answers that arrive after [`probe`] gave up out of the input events.
///
/// Once the event reader owns the input, crossterm reads `ESC ] 11;rgb:…` `BEL` as keys:
/// `Alt+]`, then the characters, then `Ctrl+G` (or `Alt+\` for the `ESC \` ending). The
/// XTVERSION reply (`Alt+P`, `>`, `|`, text, `Alt+\`) and the graphics reply (`Alt+_`, `G`,
/// text, `Alt+\`) come the same way.
/// Keys that start like an answer are held; they are dropped when the answer completes, and
/// passed on as soon as they stop matching or the window ends.
#[derive(Debug)]
pub struct LateAnswers {
    /// Answers still expected.
    left: u8,
    until: Instant,
    held: Vec<Event>,
}

impl LateAnswers {
    pub fn new(late: u8, now: Instant) -> Self {
        Self {
            left: late,
            until: now + LATE_WINDOW,
            held: Vec::new(),
        }
    }

    fn active(&self, now: Instant) -> bool {
        self.left > 0 && now < self.until
    }

    /// The events to pass on after `event`.
    pub fn feed(&mut self, event: Event, now: Instant) -> Vec<Event> {
        if !self.active(now) {
            let mut out = std::mem::take(&mut self.held);
            out.push(event);
            return out;
        }
        let Event::Key(key) = &event else {
            // Nothing else can be part of an answer: it is not one.
            let mut out = std::mem::take(&mut self.held);
            out.push(event);
            return out;
        };
        let step = answer_step(&self.held, key);
        match step {
            Step::Continue => {
                self.held.push(event);
                Vec::new()
            }
            Step::Done => {
                self.held.clear();
                self.left -= 1;
                Vec::new()
            }
            Step::No if self.held.is_empty() => vec![event],
            Step::No => {
                // The held keys were the user's; this key may start an answer itself.
                let mut out = std::mem::take(&mut self.held);
                out.extend(self.feed(event, now));
                out
            }
        }
    }

    /// How long until `flush` has something to do: the rest of the window while answers are
    /// expected, now when events are held past it, `None` when there is nothing to wait for.
    pub fn next_wake(&self, now: Instant) -> Option<Duration> {
        if self.active(now) {
            Some(self.until.saturating_duration_since(now))
        } else if self.held.is_empty() {
            None
        } else {
            Some(Duration::ZERO)
        }
    }

    /// The held events, once the window has ended; call it on idle wake-ups.
    pub fn flush(&mut self, now: Instant) -> Vec<Event> {
        if self.active(now) {
            Vec::new()
        } else {
            std::mem::take(&mut self.held)
        }
    }
}

enum Step {
    Continue,
    Done,
    No,
}

/// The most characters in the text of an XTVERSION or graphics reply that is still taken.
const REPLY_TEXT_MAX: usize = 64;

/// Whether `key`, after the `held` keys of an answer, continues it, ends it or breaks it.
fn answer_step(held: &[Event], key: &KeyEvent) -> Step {
    let char_with = |key: &KeyEvent, mods: KeyModifiers| match key.code {
        KeyCode::Char(c) if key.modifiers == mods => Some(c),
        _ => None,
    };
    // Upper-case letters come with `Shift`.
    let plain = |key: &KeyEvent| {
        char_with(key, KeyModifiers::NONE).or_else(|| char_with(key, KeyModifiers::SHIFT))
    };
    // `ESC P` is `Alt+Shift+P` to crossterm.
    let alt = |key: &KeyEvent| {
        char_with(key, KeyModifiers::ALT)
            .or_else(|| char_with(key, KeyModifiers::ALT | KeyModifiers::SHIFT))
    };
    let held_key = |i: usize| match held.get(i) {
        Some(Event::Key(k)) => Some(*k),
        _ => None,
    };
    let Some(lead) = held_key(0) else {
        return if matches!(alt(key), Some(']' | 'P' | '_')) {
            Step::Continue
        } else {
            Step::No
        };
    };
    let n = held.len();
    let ends = char_with(key, KeyModifiers::ALT) == Some('\\');
    let ok = match alt(&lead) {
        // XTVERSION: `P>|`, then printable ASCII.
        Some('P') => match n {
            1 => plain(key) == Some('>'),
            2 => plain(key) == Some('|'),
            _ => {
                if ends {
                    return Step::Done;
                }
                n - 3 < REPLY_TEXT_MAX && plain(key).is_some_and(|c| (' '..='~').contains(&c))
            }
        },
        // Graphics: `_G`, then `i=31;OK`-like text.
        Some('_') => match n {
            1 => plain(key) == Some('G'),
            _ => {
                if ends {
                    return Step::Done;
                }
                n - 2 < REPLY_TEXT_MAX
                    && plain(key)
                        .is_some_and(|c| c.is_ascii_alphanumeric() || "=,;:._ -".contains(c))
            }
        },
        // `]10;`, `]11;` or `]4;n;`.
        _ => match n {
            1 => matches!(plain(key), Some('1' | '4')),
            2 => matches!(plain(key), Some('0' | '1' | ';')),
            3 => matches!(plain(key), Some(';' | '0'..='9')),
            _ => {
                if char_with(key, KeyModifiers::CONTROL) == Some('g') || ends {
                    return Step::Done;
                }
                plain(key).is_some_and(|c| c.is_ascii_hexdigit() || "rgb:/;".contains(c))
            }
        },
    };
    if ok { Step::Continue } else { Step::No }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Answers {
    pub fg: Option<Rgb>,
    pub bg: Option<Rgb>,
    /// The OSC 4 replies for the base colours.
    pub palette: Palette,
    /// The XTVERSION reply's text.
    pub xtversion: Option<String>,
    /// The graphics reply: `OK` or not.
    pub graphics: Option<bool>,
    /// The kitty keyboard flags reply (`CSI ? flags u`) was seen.
    pub kitty_keyboard: bool,
    /// The DA1 reply was seen.
    pub da1: bool,
    /// Everything else, without a trailing unfinished reply.
    pub rest: Vec<u8>,
}

/// Finds the OSC 10, OSC 11, OSC 4, XTVERSION, graphics and DA1 replies in `bytes`.
pub fn parse_answers(bytes: &[u8]) -> Answers {
    let mut out = Answers::default();
    let mut i = 0;
    while let Some(remaining @ [byte, ..]) = bytes.get(i..) {
        match reply_at(remaining) {
            Some((Reply::Color(10, c), len)) => {
                out.fg = out.fg.or(c);
                i += len;
            }
            Some((Reply::Color(_, c), len)) => {
                out.bg = out.bg.or(c);
                i += len;
            }
            Some((Reply::Palette(n, c), len)) => {
                if let Some(slot) = out.palette.get_mut(n) {
                    *slot = slot.or(c);
                }
                i += len;
            }
            Some((Reply::Da1, len)) => {
                out.da1 = true;
                i += len;
            }
            Some((Reply::KittyKeyboard, len)) => {
                out.kitty_keyboard = true;
                i += len;
            }
            Some((Reply::Version(text), len)) => {
                out.xtversion = out.xtversion.take().or(Some(text));
                i += len;
            }
            Some((Reply::Graphics(ok), len)) => {
                out.graphics = out.graphics.or(Some(ok));
                i += len;
            }
            None => {
                out.rest.push(*byte);
                i += 1;
            }
        }
    }
    // A reply cut short at the end (it will never finish) must not reach the shell.
    if let Some(esc) = out.rest.iter().rposition(|b| *b == 0x1b)
        && let Some(tail) = out.rest.get(esc + 1..)
        && cut_reply(tail)
    {
        out.rest.truncate(esc);
    }
    out
}

/// Whether `tail`, what follows the last `ESC`, is the start of a reply that never finished.
fn cut_reply(tail: &[u8]) -> bool {
    match tail.first() {
        None => true,
        // A DCS reply (XTVERSION): printable text.
        Some(b'P') => tail.iter().skip(1).all(|b| (0x20..=0x7e).contains(b)),
        // An APC reply (graphics).
        Some(b'_') => tail
            .iter()
            .skip(1)
            .all(|b| b.is_ascii_alphanumeric() || b"=,;:._ -".contains(b)),
        Some(b']') => tail
            .iter()
            .all(|b| b"[]?;:/rgb0123456789abcdefABCDEF".contains(b)),
        Some(b'[') => {
            tail.starts_with(b"[?")
                && tail
                    .iter()
                    .all(|b| b"[]?;:/rgb0123456789abcdefABCDEF".contains(b))
        }
        Some(_) => false,
    }
}

enum Reply {
    Color(u8, Option<Rgb>),
    /// The XTVERSION text.
    Version(String),
    /// A graphics reply; `true` for `OK`.
    Graphics(bool),
    /// A base colour: its index and colour.
    Palette(usize, Option<Rgb>),
    /// The kitty keyboard flags of the terminal (`CSI ? flags u`).
    KittyKeyboard,
    Da1,
}

/// A reply at the start of `bytes` and its length.
fn reply_at(bytes: &[u8]) -> Option<(Reply, usize)> {
    if let Some(body) = bytes.strip_prefix(b"\x1bP>|") {
        let (text, len) = string_reply(body)?;
        return Some((Reply::Version(text.to_string()), 4 + len));
    }
    if let Some(body) = bytes.strip_prefix(b"\x1b_G") {
        let (text, len) = string_reply(body)?;
        let ok = text.rsplit(';').next() == Some("OK") && text.contains(';');
        return Some((Reply::Graphics(ok), 3 + len));
    }
    if let Some(body) = bytes.strip_prefix(b"\x1b[?") {
        let (end, byte) = body
            .iter()
            .enumerate()
            .find(|&(_, &b)| !(b.is_ascii_digit() || b == b';'))?;
        return match byte {
            b'c' => Some((Reply::Da1, 3 + end + 1)),
            // Only digits: the flags.
            b'u' if body.get(..end)?.iter().all(u8::is_ascii_digit) => {
                Some((Reply::KittyKeyboard, 3 + end + 1))
            }
            _ => None,
        };
    }
    let body = bytes.strip_prefix(b"\x1b]")?;
    // `4;n;` for a palette entry, `10;` or `11;` for the foreground and background.
    let (slot, index, payload, prefix_len) = if let Some(rest) = body.strip_prefix(b"4;") {
        let digits = rest.iter().take_while(|b| b.is_ascii_digit()).count();
        let n: usize = std::str::from_utf8(rest.get(..digits)?)
            .ok()?
            .parse()
            .ok()?;
        (
            4,
            Some(n),
            rest.get(digits..)?.strip_prefix(b";")?,
            2 + digits + 1,
        )
    } else if let Some(payload) = body.strip_prefix(b"10;") {
        (10, None, payload, 3)
    } else {
        (11, None, body.strip_prefix(b"11;")?, 3)
    };
    let (end, term_len) = payload.iter().enumerate().find_map(|(i, &b)| match b {
        0x07 => Some((i, 1)),
        0x1b if payload.get(i + 1) == Some(&b'\\') => Some((i, 2)),
        _ => None,
    })?;
    let color = std::str::from_utf8(payload.get(..end)?)
        .ok()
        .and_then(parse_color);
    let len = 2 + prefix_len + end + term_len;
    match index {
        Some(n) => Some((Reply::Palette(n, color), len)),
        None => Some((Reply::Color(slot, color), len)),
    }
}

/// The text of a string reply up to its `ST` or `BEL`, and the length with the terminator.
fn string_reply(body: &[u8]) -> Option<(&str, usize)> {
    let (end, term_len) = body.iter().enumerate().find_map(|(i, &b)| match b {
        0x07 => Some((i, 1)),
        0x1b if body.get(i + 1) == Some(&b'\\') => Some((i, 2)),
        _ => None,
    })?;
    let text = std::str::from_utf8(body.get(..end)?).ok()?;
    Some((text, end + term_len))
}

/// `rgb:R/G/B` with one to four hex digits per channel, scaled to 8 bits.
fn parse_color(spec: &str) -> Option<Rgb> {
    let channels = spec.strip_prefix("rgb:")?;
    let mut out = [0u8; 3];
    let mut parts = channels.split('/');
    for slot in &mut out {
        let part = parts.next()?;
        if part.is_empty() || part.len() > 4 || !part.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let value = u32::from_str_radix(part, 16).ok()?;
        let max = (1u32 << (4 * part.len())) - 1;
        *slot = u8::try_from((value * 255 + max / 2) / max).ok()?;
    }
    parts.next().is_none().then_some((out[0], out[1], out[2]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reader_has_nothing_to_wake_for_without_late_answers() {
        let now = Instant::now();
        assert_eq!(LateAnswers::new(0, now).next_wake(now), None);
        assert_eq!(LateAnswers::new(1, now).next_wake(now), Some(LATE_WINDOW));
        assert_eq!(LateAnswers::new(1, now).next_wake(now + LATE_WINDOW), None);
    }

    #[test]
    fn parses_both_terminators_and_every_digit_count() {
        let a = parse_answers(b"\x1b]10;rgb:cccc/cccc/cccc\x07\x1b]11;rgb:0/80/ff\x1b\\");
        assert_eq!(a.fg, Some((0xcc, 0xcc, 0xcc)));
        assert_eq!(a.bg, Some((0, 0x80, 0xff)));
        assert!(a.rest.is_empty() && !a.da1);
        assert_eq!(parse_color("rgb:ffff/ffff/ffff"), Some((255, 255, 255)));
        assert_eq!(parse_color("rgb:12/34/56"), Some((0x12, 0x34, 0x56)));
    }

    #[test]
    fn rejects_bad_colours() {
        for bad in [
            "",
            "rgb:1/2",
            "rgb:1/2/3/4",
            "rgb:zz/0/0",
            "rgb:12345/0/0",
            "#fff",
        ] {
            assert_eq!(parse_color(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_da1_reply_is_recognised_and_removed() {
        let a = parse_answers(b"\x1b]11;rgb:0/0/0\x07\x1b[?6c");
        assert!(a.da1);
        assert_eq!(a.bg, Some((0, 0, 0)));
        assert!(a.rest.is_empty());
        let a = parse_answers(b"\x1b[?62;4;22c");
        assert!(a.da1 && a.rest.is_empty());
    }

    #[test]
    fn other_bytes_are_left_over_in_order() {
        let a = parse_answers(b"ab\x1b]11;rgb:0/0/0\x07cd\x1b[A\x1b]10;rgb:f/f/f\x07");
        assert_eq!(a.fg, Some((255, 255, 255)));
        assert_eq!(a.bg, Some((0, 0, 0)));
        assert_eq!(a.rest, b"abcd\x1b[A");
    }

    #[test]
    fn an_unfinished_reply_at_the_end_is_dropped() {
        for tail in [
            &b"\x1b]10;rgb:0/0/0"[..],
            b"\x1b]11;rgb:ff",
            b"\x1b]1",
            b"\x1b]",
            b"\x1b[?6",
            b"\x1b",
        ] {
            let mut input = b"hi".to_vec();
            input.extend_from_slice(tail);
            let a = parse_answers(&input);
            assert_eq!((a.fg, a.bg, a.da1), (None, None, false));
            assert_eq!(a.rest, b"hi", "{tail:?}");
        }
        // A finished key sequence is the user's and stays.
        assert_eq!(parse_answers(b"\x1b[A").rest, b"\x1b[A");
    }

    #[test]
    fn probe_stops_at_the_da1_reply_even_with_both_colours_in() {
        let mut chunks = vec![
            b"\x1b]10;rgb:1111/".to_vec(),
            b"2222/3333\x07x".to_vec(),
            b"\x1b]11;rgb:0/0/0\x07".to_vec(),
            b"\x1b[?6c".to_vec(),
            b"never read".to_vec(),
        ]
        .into_iter();
        let mut reads = 0;
        let mut read = |_: Duration| {
            reads += 1;
            chunks.next()
        };
        let p = probe(&mut read, Duration::from_secs(1));
        assert_eq!(reads, 4);
        assert_eq!(
            p.colors,
            Some(OuterColors {
                fg: (0x11, 0x22, 0x33),
                bg: (0, 0, 0)
            })
        );
        assert_eq!(p.leftover, b"x");
    }

    #[test]
    fn probe_gives_none_on_timeout_and_keeps_the_typeahead() {
        let mut calls = 0;
        let mut read = |_: Duration| {
            calls += 1;
            if calls == 1 {
                Some(b"hi\x1b]11;rgb:0".to_vec())
            } else {
                Some(Vec::new())
            }
        };
        let p = probe(&mut read, Duration::from_millis(30));
        assert_eq!(p.colors, None);
        assert_eq!(p.leftover, b"hi");
        // Both colours, the 16 base colours, XTVERSION and the graphics reply are missing.
        assert_eq!(p.late, 20);
    }

    #[test]
    fn osc_4_replies_fill_the_palette_in_both_forms_and_stay_out_of_the_input() {
        let a = parse_answers(
            b"\x1b]10;rgb:ff/ff/ff\x07\x1b]4;0;rgb:00/00/00\x07\x1b]4;4;rgb:1111/2222/3333\x1b\\\
              \x1b]4;15;rgb:f/f/f\x07\x1b]4;16;rgb:1/1/1\x07x\x1b[?6c",
        );
        assert_eq!(a.palette[0], Some((0, 0, 0)));
        assert_eq!(a.palette[4], Some((0x11, 0x22, 0x33)));
        assert_eq!(a.palette[15], Some((255, 255, 255)));
        assert_eq!(a.palette[1], None);
        assert_eq!(a.fg, Some((255, 255, 255)));
        // An index out of the base 16 is dropped, not passed on.
        assert_eq!(a.rest, b"x");
        assert!(a.da1);
    }

    #[test]
    fn an_unfinished_osc_4_reply_is_dropped_and_the_query_ends_on_da1() {
        let a = parse_answers(b"hi\x1b]4;12;rgb:ff");
        assert_eq!(a.rest, b"hi");
        assert!(QUERY.ends_with(b"\x1b[c"));
        assert!(QUERY.starts_with(b"\x1b]10;?\x07\x1b]11;?\x07\x1b]4;0;?\x07"));
        assert_eq!(QUERY.windows(3).filter(|w| w == b"]4;").count(), 16);
    }

    #[test]
    fn late_osc_4_answers_are_dropped_too() {
        let now = Instant::now();
        let mut late = LateAnswers::new(2, now);
        let mut input = b"a".to_vec();
        input.extend_from_slice(b"\x1b]4;12;rgb:1111/2222/3333\x07b");
        input.extend_from_slice(b"\x1b]4;3;rgb:0/0/0\x1b\\c");
        assert_eq!(feed_all(&mut late, keys(&input), now), keys(b"abc"));
    }

    #[test]
    fn nothing_is_late_once_the_da1_reply_came() {
        let mut chunks = vec![b"\x1b[?6c".to_vec()].into_iter();
        let mut read = |_: Duration| chunks.next();
        assert_eq!(probe(&mut read, Duration::from_secs(1)).late, 0);
    }

    const VERSIONS: [&[u8]; 2] = [
        b"\x1bP>|kitty(0.36.4)\x1b\\",
        b"\x1bP>|WezTerm 20240203-110809-5046fc22\x07",
    ];

    #[test]
    fn the_query_asks_xtversion_and_graphics_before_da1() {
        let tail = b"\x1b[>0q\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b[?u\x1b[c";
        assert!(QUERY.ends_with(tail));
    }

    #[test]
    fn the_kitty_keyboard_reply_is_read_with_the_others_and_stays_out_of_the_input() {
        for flags in ["0", "1", "31", ""] {
            let mut input = b"a\x1b[?".to_vec();
            input.extend_from_slice(flags.as_bytes());
            input.extend_from_slice(b"ub\x1b[?62;4c");
            let a = parse_answers(&input);
            assert!(a.kitty_keyboard && a.da1, "{flags}");
            assert_eq!(a.rest, b"ab");
        }
        // No answer: DA1 alone.
        let a = parse_answers(b"\x1b[?6c");
        assert!(!a.kitty_keyboard && a.da1);
        // The probe reports it, in one round trip with the rest.
        let mut chunks = vec![b"\x1b]11;rgb:0/0/0\x07\x1b[?0u\x1b[?6c".to_vec()].into_iter();
        let mut read = |_: Duration| chunks.next();
        let p = probe(&mut read, Duration::from_secs(1));
        assert!(p.kitty_keyboard);
        assert_eq!(p.late, 0);
        assert!(p.leftover.is_empty());
        let mut chunks = vec![b"\x1b[?6c".to_vec()].into_iter();
        let mut read = |_: Duration| chunks.next();
        assert!(!probe(&mut read, Duration::from_secs(1)).kitty_keyboard);
    }

    #[test]
    fn xtversion_and_graphics_replies_parse_in_both_forms_and_stay_out_of_the_input() {
        for version in VERSIONS {
            let mut input = b"a".to_vec();
            input.extend_from_slice(version);
            input.extend_from_slice(b"b\x1b_Gi=31;OK\x1b\\\x1b[?6c");
            let a = parse_answers(&input);
            assert!(
                a.xtversion
                    .as_deref()
                    .is_some_and(|v| v.contains("(0.36.4)") || v.starts_with("WezTerm"))
            );
            assert_eq!(a.graphics, Some(true));
            assert!(a.da1);
            assert_eq!(a.rest, b"ab");
        }
        let a = parse_answers(b"\x1b_Gi=31;ENOTSUPPORTED:no\x07\x1b_Gi=31;OK\x1b\\");
        assert_eq!(a.graphics, Some(false));
        assert!(a.rest.is_empty());
        assert_eq!(parse_answers(b"\x1b[?6c").xtversion, None);
    }

    #[test]
    fn unfinished_dcs_and_apc_replies_at_the_end_are_dropped() {
        for tail in [
            &b"\x1bP"[..],
            b"\x1bP>|kitty(0.36",
            b"\x1b_",
            b"\x1b_Gi=31;O",
        ] {
            let mut input = b"hi".to_vec();
            input.extend_from_slice(tail);
            let a = parse_answers(&input);
            assert_eq!(a.rest, b"hi", "{tail:?}");
        }
    }

    #[test]
    fn probe_reads_both_new_answers_and_stops_at_da1() {
        let mut chunks = vec![
            b"\x1bP>|ghostty 1.2".to_vec(),
            b".0\x1b\\\x1b_Gi=31;OK\x1b".to_vec(),
            b"\\\x1b[?62c".to_vec(),
            b"never read".to_vec(),
        ]
        .into_iter();
        let mut read = |_: Duration| chunks.next();
        let p = probe(&mut read, Duration::from_secs(1));
        assert_eq!(p.xtversion.as_deref(), Some("ghostty 1.2.0"));
        assert_eq!(p.graphics, Some(true));
        assert_eq!((p.late, p.leftover), (0, Vec::new()));
    }

    #[test]
    fn late_xtversion_and_graphics_replies_are_swallowed() {
        let now = Instant::now();
        let mut late = LateAnswers::new(2, now);
        let mut input = b"a".to_vec();
        input.extend_from_slice(b"\x1bP>|kitty(0.36.4)\x1b\\b");
        input.extend_from_slice(b"\x1b_Gi=31;OK\x1b\\c");
        assert_eq!(feed_all(&mut late, keys(&input), now), keys(b"abc"));
    }

    #[test]
    fn a_late_alt_p_that_breaks_the_shape_is_delivered_as_typed() {
        let now = Instant::now();
        for typed in [&b"\x1bPx"[..], b"\x1bP>x", b"\x1b_x", b"\x1b_Gi=31;\x01"] {
            let mut late = LateAnswers::new(1, now);
            let mut events = feed_all(&mut late, keys(typed), now);
            events.extend(late.flush(now + LATE_WINDOW));
            assert_eq!(events, keys(typed), "{typed:?}");
        }
        // Too long a text is not a reply either.
        let mut late = LateAnswers::new(1, now);
        let mut long = b"\x1bP>|".to_vec();
        long.extend(std::iter::repeat_n(b'x', 70));
        let mut events = feed_all(&mut late, keys(&long), now);
        events.extend(late.flush(now + LATE_WINDOW));
        assert_eq!(events, keys(&long));
    }

    fn key(code: KeyCode, mods: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(code, mods))
    }

    /// The keys crossterm makes of `bytes` (the subset an answer uses).
    fn keys(bytes: &[u8]) -> Vec<Event> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            i += 1;
            out.push(match b {
                0x1b => {
                    let c = char::from(bytes[i]);
                    i += 1;
                    let shift = if c.is_ascii_uppercase() {
                        KeyModifiers::SHIFT
                    } else {
                        KeyModifiers::NONE
                    };
                    key(KeyCode::Char(c), KeyModifiers::ALT | shift)
                }
                0x07 => key(KeyCode::Char('g'), KeyModifiers::CONTROL),
                b if b.is_ascii_uppercase() => {
                    key(KeyCode::Char(char::from(b)), KeyModifiers::SHIFT)
                }
                b => key(KeyCode::Char(char::from(b)), KeyModifiers::NONE),
            });
        }
        out
    }

    fn feed_all(late: &mut LateAnswers, events: Vec<Event>, now: Instant) -> Vec<Event> {
        events.into_iter().flat_map(|e| late.feed(e, now)).collect()
    }

    #[test]
    fn late_answers_with_either_ending_are_dropped_and_typing_passes() {
        let now = Instant::now();
        let mut late = LateAnswers::new(2, now);
        let mut input = b"ab".to_vec();
        input.extend_from_slice(b"\x1b]10;rgb:FFFF/0000/0000\x07");
        input.extend_from_slice(b"c");
        input.extend_from_slice(b"\x1b]11;rgb:0/0/0\x1b\\");
        input.extend_from_slice(b"d");
        assert_eq!(feed_all(&mut late, keys(&input), now), keys(b"abcd"));
        // Both came: an `Alt+]` the user types now passes at once.
        assert_eq!(late.feed(keys(b"\x1b]")[0].clone(), now), keys(b"\x1b]"));
    }

    #[test]
    fn keys_that_only_start_like_an_answer_are_passed_on_in_order() {
        let now = Instant::now();
        let mut late = LateAnswers::new(1, now);
        assert_eq!(feed_all(&mut late, keys(b"\x1b]1x"), now), keys(b"\x1b]1x"));
        // A held `Alt+]` followed by another one: the first passes, the second is held.
        assert_eq!(
            feed_all(&mut late, keys(b"\x1b]\x1b]"), now),
            keys(b"\x1b]")
        );
        assert_eq!(
            late.feed(Event::FocusGained, now),
            [keys(b"\x1b]"), vec![Event::FocusGained]].concat()
        );
    }

    #[test]
    fn held_keys_are_released_when_the_window_ends() {
        let now = Instant::now();
        let mut late = LateAnswers::new(1, now);
        assert!(feed_all(&mut late, keys(b"\x1b]11"), now).is_empty());
        assert!(late.flush(now).is_empty());
        assert_eq!(late.flush(now + LATE_WINDOW), keys(b"\x1b]11"));
        // After the window nothing is held.
        assert_eq!(
            feed_all(&mut late, keys(b"\x1b]"), now + LATE_WINDOW),
            keys(b"\x1b]")
        );
    }

    #[test]
    fn nothing_is_held_when_no_answer_is_late() {
        let now = Instant::now();
        let mut late = LateAnswers::new(0, now);
        assert_eq!(
            feed_all(&mut late, keys(b"\x1b]11;"), now),
            keys(b"\x1b]11;")
        );
    }
}
