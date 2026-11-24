//! Where a chunk of PTY output may be cut without splitting a sequence: the reader holds
//! back an incomplete escape sequence or UTF-8 character at the end of a chunk, so that a
//! pause leaves the emulator's parser in its ground state.

/// A tail longer than this is not held back: a long string sequence is fed as it comes.
const MAX_HELD: usize = 8192;

/// The index at which the incomplete sequence or character at the end of `data` starts,
/// `data.len()` when it ends cleanly.
pub fn incomplete_tail(data: &[u8]) -> usize {
    let len = data.len();
    let window = len.saturating_sub(MAX_HELD);
    if let Some(at) = data
        .get(window..)
        .and_then(|w| w.iter().rposition(|&b| b == 0x1b))
        .map(|i| i + window)
        && escape_is_open(data.get(at + 1..).unwrap_or_default())
    {
        return at;
    }
    // A character cut short: a lead byte with fewer continuation bytes than it needs.
    for back in 1..=3usize.min(len) {
        let Some(&b) = data.get(len - back) else {
            break;
        };
        if b & 0xc0 == 0x80 {
            continue;
        }
        let need = match b {
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => 1,
        };
        return if need > back { len - back } else { len };
    }
    len
}

/// Whether the bytes after the last `ESC` of a chunk leave its sequence unfinished.
fn escape_is_open(after: &[u8]) -> bool {
    let Some((&first, rest)) = after.split_first() else {
        return true;
    };
    match first {
        // A string terminator, which ends what an earlier `ESC` began.
        b'\\' => false,
        b'[' => rest.iter().all(|b| (0x20..=0x3f).contains(b)),
        b']' => !rest.contains(&0x07),
        b'P' | b'_' | b'^' | b'X' => true,
        // Intermediates (`ESC ( B`) wait for their final byte.
        0x20..=0x2f => rest.iter().all(|b| (0x20..=0x2f).contains(b)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cut(data: &[u8]) -> &[u8] {
        &data[incomplete_tail(data)..]
    }

    #[test]
    fn a_clean_end_holds_nothing() {
        for data in [
            &b""[..],
            b"plain",
            b"a\x1b[31mb",
            b"\x1b]2;t\x07",
            b"\x1b]2;t\x1b\\",
            "é".as_bytes(),
            b"\x1bc",
            b"\x1b(B",
        ] {
            assert_eq!(cut(data), b"", "{data:?}");
        }
    }

    #[test]
    fn an_unfinished_sequence_or_character_is_held() {
        assert_eq!(cut(b"ab\x1b"), b"\x1b");
        assert_eq!(cut(b"ab\x1b[31"), b"\x1b[31");
        assert_eq!(cut(b"ab\x1b[?25"), b"\x1b[?25");
        assert_eq!(cut(b"a\x1b]2;tit"), b"\x1b]2;tit");
        assert_eq!(cut(b"a\x1bP1$r"), b"\x1bP1$r");
        assert_eq!(cut(b"a\x1b_Gi=1"), b"\x1b_Gi=1");
        assert_eq!(cut(b"a\x1b("), b"\x1b(");
        assert_eq!(cut(b"caf\xc3"), b"\xc3");
        assert_eq!(cut(b"x\xe6\x97"), b"\xe6\x97");
        assert_eq!(cut(b"x\xf0\x9f\x99"), b"\xf0\x9f\x99");
        assert_eq!(cut(b"x\xe6\x97\xa5"), b"");
    }

    #[test]
    fn a_long_string_is_not_held() {
        let mut data = b"\x1b]0;".to_vec();
        data.extend(std::iter::repeat_n(b'x', MAX_HELD + 10));
        assert_eq!(incomplete_tail(&data), data.len());
    }
}
