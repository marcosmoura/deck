//! A streaming scanner for OSC 7 ("the shell is in this directory"):
//! `ESC ] 7 ; file://<host>/<path> (BEL | ESC \)`.
//!
//! It is fed the chunks of a PTY's output as they come (a sequence may be split anywhere)
//! and yields the paths. The bytes still go to the emulator unchanged.

use std::path::PathBuf;

/// One sequence is buffered up to this many bytes; a longer one is dropped.
pub const MAX_SEQUENCE: usize = 4096;

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Outside any sequence.
    Ground,
    /// After `ESC`.
    Esc,
    /// After `ESC ]`: the `7` comes next.
    Number,
    /// After `ESC ] 7`: the `;` comes next.
    Semicolon,
    /// Collecting the URI.
    Body,
    /// In the URI, after `ESC`.
    BodyEsc,
}

#[derive(Debug)]
pub struct Osc7Scanner {
    hostname: String,
    state: State,
    body: Vec<u8>,
}

impl Osc7Scanner {
    /// A scanner that accepts the host names empty, `localhost` and `hostname` (in full or
    /// as its short name, before the first `.`), in any case.
    pub fn new(hostname: &str) -> Self {
        Self {
            hostname: hostname.to_ascii_lowercase(),
            state: State::Ground,
            body: Vec::new(),
        }
    }

    /// Scans a chunk and returns the paths reported in it, in order.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<PathBuf> {
        let mut found = Vec::new();
        for &byte in chunk {
            self.step(byte, &mut found);
        }
        found
    }

    fn step(&mut self, byte: u8, found: &mut Vec<PathBuf>) {
        self.state = match (self.state, byte) {
            (State::Body, BEL) | (State::BodyEsc, b'\\') => {
                if let Some(path) = self.parse() {
                    found.push(path);
                }
                self.body.clear();
                State::Ground
            }
            (State::Body, ESC) => State::BodyEsc,
            (State::Body, b) => {
                if self.body.len() >= MAX_SEQUENCE {
                    self.body.clear();
                    State::Ground
                } else {
                    self.body.push(b);
                    State::Body
                }
            }
            // An escape inside the URI that is not the terminator abandons it and starts
            // over with this byte.
            (State::BodyEsc, b) => {
                self.body.clear();
                if b == b']' {
                    State::Number
                } else {
                    Self::from_ground(b)
                }
            }
            (State::Number, b'7') => State::Semicolon,
            (State::Semicolon, b';') => State::Body,
            (State::Esc, b']') => State::Number,
            (_, b) => Self::from_ground(b),
        };
    }

    fn from_ground(byte: u8) -> State {
        if byte == ESC {
            State::Esc
        } else {
            State::Ground
        }
    }

    fn parse(&self) -> Option<PathBuf> {
        let text = self.body.strip_prefix(b"file://")?;
        let slash = text.iter().position(|&b| b == b'/')?;
        let (host, path) = text.split_at(slash);
        if !self.accepts(host) {
            return None;
        }
        let path = String::from_utf8(percent_decode(path)).ok()?;
        // Output chooses this text, and it is printed later (`deck ls`): a path that hides
        // an escape sequence behind percent-encoding is not a directory to report.
        if path.chars().any(char::is_control) {
            return None;
        }
        Some(PathBuf::from(path))
    }

    fn accepts(&self, host: &[u8]) -> bool {
        let Ok(host) = std::str::from_utf8(host) else {
            return false;
        };
        let host = host.to_ascii_lowercase();
        let short = self.hostname.split('.').next().unwrap_or_default();
        host.is_empty()
            || host == "localhost"
            || (!self.hostname.is_empty() && (host == self.hostname || host == short))
    }
}

fn percent_decode(bytes: &[u8]) -> Vec<u8> {
    let hex = |b: u8| {
        char::from(b)
            .to_digit(16)
            .and_then(|d| u8::try_from(d).ok())
    };
    let mut out = Vec::with_capacity(bytes.len());
    let mut rest = bytes;
    while let Some((&b, tail)) = rest.split_first() {
        rest = tail;
        if b == b'%'
            && let [hi, lo, after @ ..] = tail
            && let (Some(hi), Some(lo)) = (hex(*hi), hex(*lo))
        {
            out.push(hi * 16 + lo);
            rest = after;
        } else {
            out.push(b);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(host: &str, input: &[u8]) -> Vec<PathBuf> {
        Osc7Scanner::new(host).feed(input)
    }

    fn paths(list: &[&str]) -> Vec<PathBuf> {
        list.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn yields_the_decoded_path_for_both_terminators() {
        let want = paths(&["/tmp/a b"]);
        assert_eq!(scan("mac", b"\x1b]7;file://localhost/tmp/a%20b\x07"), want);
        assert_eq!(
            scan("mac", b"\x1b]7;file://localhost/tmp/a%20b\x1b\\"),
            want
        );
        assert_eq!(scan("mac", b"\x1b]7;file:///tmp/a%20b\x07"), want);
    }

    #[test]
    fn works_split_at_every_byte_boundary() {
        let input = b"junk\x1b]7;file://localhost/tmp/a%20b\x07more\x1b]7;file:///x\x1b\\";
        for cut in 0..=input.len() {
            let mut scanner = Osc7Scanner::new("mac");
            let mut got = scanner.feed(&input[..cut]);
            got.extend(scanner.feed(&input[cut..]));
            assert_eq!(got, paths(&["/tmp/a b", "/x"]), "cut at {cut}");
        }
        let mut scanner = Osc7Scanner::new("mac");
        let got: Vec<PathBuf> = input.iter().flat_map(|b| scanner.feed(&[*b])).collect();
        assert_eq!(got, paths(&["/tmp/a b", "/x"]));
    }

    #[test]
    fn accepts_the_hostname_in_any_case_and_as_a_short_name() {
        for host in ["Mac.local", "mac.LOCAL", "MAC", "mac", "LocalHost", ""] {
            let seq = format!("\x1b]7;file://{host}/p\x07");
            assert_eq!(scan("Mac.local", seq.as_bytes()), paths(&["/p"]), "{host}");
        }
        assert_eq!(
            scan("mac", b"\x1b]7;file://MAC/p\x07"),
            paths(&["/p"]),
            "hostname without a domain"
        );
    }

    #[test]
    fn ignores_what_it_must_not_report() {
        let cases: [&[u8]; 8] = [
            b"\x1b]7;file://other.host/tmp\x07",
            b"\x1b]7;file://mac.example/tmp\x07",
            b"\x1b]7;file://localhost\x07",
            b"\x1b]7;file://localhost/\xff\x07",
            b"\x1b]7;file://localhost/a%FFb\x07",
            b"\x1b]7;file://localhost/a%00b\x07",
            b"\x1b]7;http://localhost/tmp\x07",
            b"\x1b]7;tmp/relative\x07",
        ];
        for case in cases {
            assert_eq!(scan("mac", case), Vec::<PathBuf>::new(), "{case:?}");
        }
        assert_eq!(
            scan("", b"\x1b]7;file://mac/tmp\x07"),
            Vec::<PathBuf>::new(),
            "no hostname known"
        );
    }

    #[test]
    fn a_path_with_control_characters_is_dropped() {
        let cases: [&[u8]; 6] = [
            // An OSC 52 behind percent-encoding, as the audit's attack.
            b"\x1b]7;file:///tmp/%1b]52;c;ZXZpbA==%07\x07",
            b"\x1b]7;file:///tmp/a%0Ab\x07",
            b"\x1b]7;file:///tmp/a%09b\x07",
            b"\x1b]7;file:///tmp/a%7Fb\x07",
            // C1 CSI (U+009B), percent-encoded and raw.
            b"\x1b]7;file:///tmp/a%C2%9B2Jb\x07",
            b"\x1b]7;file:///tmp/a\xc2\x9b2Jb\x07",
        ];
        for case in cases {
            assert_eq!(scan("mac", case), Vec::<PathBuf>::new(), "{case:?}");
        }
        assert_eq!(scan("mac", b"\x1b]7;file:///tmp/a%20b\x07").len(), 1);
    }

    #[test]
    fn other_sequences_are_not_reported() {
        assert!(scan("mac", b"\x1b]0;file://localhost/tmp\x07").is_empty());
        assert!(scan("mac", b"\x1b]77;file://localhost/tmp\x07").is_empty());
        assert!(scan("mac", b"\x1b[7;file://localhost/tmp\x07").is_empty());
    }

    #[test]
    fn a_literal_percent_stays() {
        assert_eq!(
            scan("mac", b"\x1b]7;file:///a%zz%4\x07"),
            paths(&["/a%zz%4"])
        );
        assert_eq!(
            scan("mac", b"\x1b]7;file:///caf%C3%A9\x07"),
            paths(&["/caf\u{e9}"])
        );
    }

    #[test]
    fn an_overlong_or_unterminated_sequence_is_dropped_and_the_next_one_found() {
        let mut scanner = Osc7Scanner::new("mac");
        let mut input = b"\x1b]7;file://localhost/".to_vec();
        input.extend(std::iter::repeat_n(b'a', 5 * 1024));
        assert!(scanner.feed(&input).is_empty());
        assert!(scanner.feed(b"\x07").is_empty());
        assert_eq!(scanner.feed(b"\x1b]7;file:///next\x07"), paths(&["/next"]));

        // Unterminated and short: the next sequence restarts it.
        let mut scanner = Osc7Scanner::new("mac");
        assert_eq!(
            scanner.feed(b"\x1b]7;file:///lost\x1b]7;file:///found\x07"),
            paths(&["/found"])
        );
    }

    #[test]
    fn the_longest_allowed_sequence_still_parses() {
        let mut input = b"\x1b]7;file:///".to_vec();
        let used = b"file:///".len();
        input.extend(std::iter::repeat_n(b'a', MAX_SEQUENCE - used));
        input.push(BEL);
        assert_eq!(Osc7Scanner::new("mac").feed(&input).len(), 1);
    }
}
