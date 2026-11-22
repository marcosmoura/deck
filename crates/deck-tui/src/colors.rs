//! Asking the outer terminal for its foreground and background (OSC 10 and OSC 11).

use std::time::{Duration, Instant};

use deck_core::protocol::OuterColors;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

/// How long the client waits for the answers.
pub const ANSWER_TIMEOUT: Duration = Duration::from_millis(200);
/// After a timeout, how long colour answers that come late are still taken out of the input.
pub const LATE_WINDOW: Duration = Duration::from_secs(3);

/// Both colour queries, BEL-terminated, then a device-attributes request (DA1). Terminals
/// answer in order, so the DA1 reply tells that no colour answer is still on its way.
pub const QUERY: &[u8] = b"\x1b]10;?\x07\x1b]11;?\x07\x1b[c";

/// What came back from the outer terminal.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Probe {
    /// Both colours, `None` unless the terminal gave both.
    pub colors: Option<OuterColors>,
    /// Bytes that were not part of an answer, to pass on as input.
    pub leftover: Vec<u8>,
    /// Colour answers that may still arrive: 0 once the DA1 reply was seen, else the
    /// number of colours missing.
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
        u8::from(parsed.fg.is_none()) + u8::from(parsed.bg.is_none())
    };
    Probe {
        late,
        colors: parsed
            .fg
            .zip(parsed.bg)
            .map(|(fg, bg)| OuterColors { fg, bg }),
        leftover: parsed.rest,
    }
}

/// Takes colour answers that arrive after [`probe`] gave up out of the input events.
///
/// Once the event reader owns the input, crossterm reads `ESC ] 11;rgb:…` `BEL` as keys:
/// `Alt+]`, then the characters, then `Ctrl+G` (or `Alt+\` for the `ESC \` ending).
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
        let step = answer_step(self.held.len(), key);
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

/// Whether `key`, after `held` keys of an answer, continues it, ends it or breaks it.
fn answer_step(held: usize, key: &KeyEvent) -> Step {
    let char_with = |mods: KeyModifiers| match key.code {
        KeyCode::Char(c) if key.modifiers == mods => Some(c),
        _ => None,
    };
    // Upper-case hex digits come with `Shift`.
    let plain = char_with(KeyModifiers::NONE).or_else(|| char_with(KeyModifiers::SHIFT));
    let ok = match held {
        0 => char_with(KeyModifiers::ALT) == Some(']'),
        1 => plain == Some('1'),
        2 => matches!(plain, Some('0' | '1')),
        3 => plain == Some(';'),
        _ => {
            if char_with(KeyModifiers::CONTROL) == Some('g')
                || char_with(KeyModifiers::ALT) == Some('\\')
            {
                return Step::Done;
            }
            plain.is_some_and(|c| c.is_ascii_hexdigit() || "rgb:/".contains(c))
        }
    };
    if ok { Step::Continue } else { Step::No }
}

type Rgb = (u8, u8, u8);

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Answers {
    pub fg: Option<Rgb>,
    pub bg: Option<Rgb>,
    /// The DA1 reply was seen.
    pub da1: bool,
    /// Everything else, without a trailing unfinished reply.
    pub rest: Vec<u8>,
}

/// Finds the OSC 10, OSC 11 and DA1 replies in `bytes`.
pub fn parse_answers(bytes: &[u8]) -> Answers {
    let mut out = Answers::default();
    let mut i = 0;
    while i < bytes.len() {
        match reply_at(&bytes[i..]) {
            Some((Reply::Color(10, c), len)) => {
                out.fg = out.fg.or(c);
                i += len;
            }
            Some((Reply::Color(_, c), len)) => {
                out.bg = out.bg.or(c);
                i += len;
            }
            Some((Reply::Da1, len)) => {
                out.da1 = true;
                i += len;
            }
            None => {
                out.rest.push(bytes[i]);
                i += 1;
            }
        }
    }
    // A reply cut short at the end (it will never finish) must not reach the shell.
    if let Some(esc) = out.rest.iter().rposition(|b| *b == 0x1b) {
        let tail = &out.rest[esc + 1..];
        let reply_like = tail.is_empty() || tail[0] == b']' || tail.starts_with(b"[?");
        if reply_like
            && tail
                .iter()
                .all(|b| b"[]?;:/rgb0123456789abcdefABCDEF".contains(b))
        {
            out.rest.truncate(esc);
        }
    }
    out
}

enum Reply {
    Color(u8, Option<Rgb>),
    Da1,
}

/// A reply at the start of `bytes` and its length.
fn reply_at(bytes: &[u8]) -> Option<(Reply, usize)> {
    if let Some(body) = bytes.strip_prefix(b"\x1b[?") {
        let end = body
            .iter()
            .position(|b| !(b.is_ascii_digit() || *b == b';'))?;
        return (body[end] == b'c').then_some((Reply::Da1, 3 + end + 1));
    }
    let body = bytes.strip_prefix(b"\x1b]")?;
    let slot = if body.starts_with(b"10;") {
        10
    } else if body.starts_with(b"11;") {
        11
    } else {
        return None;
    };
    let payload = &body[3..];
    let (end, term_len) = payload.iter().enumerate().find_map(|(i, &b)| match b {
        0x07 => Some((i, 1)),
        0x1b if payload.get(i + 1) == Some(&b'\\') => Some((i, 2)),
        _ => None,
    })?;
    let color = std::str::from_utf8(&payload[..end])
        .ok()
        .and_then(parse_color);
    Some((Reply::Color(slot, color), 2 + 3 + end + term_len))
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
        assert_eq!(p.late, 2);
    }

    #[test]
    fn nothing_is_late_once_the_da1_reply_came() {
        let mut chunks = vec![b"\x1b[?6c".to_vec()].into_iter();
        let mut read = |_: Duration| chunks.next();
        assert_eq!(probe(&mut read, Duration::from_secs(1)).late, 0);
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
                    key(KeyCode::Char(c), KeyModifiers::ALT)
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
