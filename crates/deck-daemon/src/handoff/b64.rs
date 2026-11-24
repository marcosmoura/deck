//! Base64 (standard alphabet, padded) for the byte fields of the handoff file.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = |i: usize| u32::from(chunk.get(i).copied().unwrap_or(0));
        let n = (b(0) << 16) | (b(1) << 8) | b(2);
        let sextet = |shift: u32| {
            ALPHABET
                .get(((n >> shift) & 63) as usize)
                .map_or('=', |&c| char::from(c))
        };
        out.push(sextet(18));
        out.push(sextet(12));
        out.push(if chunk.len() > 1 { sextet(6) } else { '=' });
        out.push(if chunk.len() > 2 { sextet(0) } else { '=' });
    }
    out
}

fn value(c: u8) -> Option<u32> {
    match c {
        b'A'..=b'Z' => Some(u32::from(c - b'A')),
        b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
        b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

pub fn decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    let last = bytes.len() / 4;
    for (i, quad) in bytes.chunks(4).enumerate() {
        let mut n = 0u32;
        let mut real = 0;
        for (j, &c) in quad.iter().enumerate() {
            n <<= 6;
            if c == b'=' {
                // Padding only ends the last quad.
                if i + 1 != last || j < 2 {
                    return None;
                }
            } else {
                if real != j {
                    return None;
                }
                n |= value(c)?;
                real += 1;
            }
        }
        out.push((n >> 16) as u8);
        if real > 2 {
            out.push((n >> 8) as u8);
        }
        if real > 3 {
            out.push(n as u8);
        }
    }
    Some(out)
}

/// `#[serde(with = "...")]` for a `Vec<u8>` held as base64 text.
pub mod bytes {
    use super::*;

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        encode(bytes).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        decode(&text).ok_or_else(|| serde::de::Error::custom("not base64"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_matches_the_known_vectors() {
        for (plain, coded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(encode(plain.as_bytes()), coded);
            assert_eq!(decode(coded).as_deref(), Some(plain.as_bytes()));
        }
    }

    #[test]
    fn every_byte_value_round_trips_and_junk_is_refused() {
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(decode(&encode(&all)), Some(all));
        for junk in ["Zg=", "Z", "Zg==Zm9v", "Z===", "Zm9*", "=m9v"] {
            assert_eq!(decode(junk), None, "{junk}");
        }
    }
}
