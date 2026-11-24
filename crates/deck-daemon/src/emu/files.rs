//! Kitty graphics commands that name a file (`t=f`, `t=t`), rewritten into direct
//! transmission before the emulator sees them.
//!
//! The emulator's own file mediums stay off: opening a path a program names (a FIFO, a
//! device) can block for good. The [`Scanner`] runs on the PTY reader thread, so file I/O
//! never runs on the emulator thread or under the state lock. It resolves the path, opens it
//! with `O_NONBLOCK` and `O_NOCTTY`, requires a regular file, and streams at most
//! [`MAX_FILE`] bytes as base64 chunks (`t=d`). The emulator gets pieces of at most
//! [`PIECE`] bytes. Reads share a budget of [`BUDGET`] bytes per second; past it, and for
//! anything that fails a check, the command passes on unchanged and the emulator answers it
//! with an error.

use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// An APC longer than this is passed on as it is, unexamined.
const MAX_APC: usize = 8192;
/// The most a file may give.
const MAX_FILE: u64 = 64 << 20;
/// The raw bytes of one chunk of a rewritten command: 4096 base64 bytes.
const RAW_CHUNK: usize = 3072;
/// The most bytes handed to the emulator at once.
const PIECE: usize = 64 * 1024;
/// The file bytes a terminal may have read in one [`WINDOW`].
const BUDGET: u64 = 128 << 20;
const WINDOW: Duration = Duration::from_secs(1);
const ESC: u8 = 0x1b;
const PREFIX: [u8; 3] = [ESC, b'_', b'G'];

/// Collects what the scanner produces and hands it on in pieces of at most [`PIECE`] bytes.
struct Out<'a> {
    buf: Vec<u8>,
    sink: &'a mut dyn FnMut(&[u8]),
}

impl Out<'_> {
    fn put(&mut self, mut bytes: &[u8]) {
        while !bytes.is_empty() {
            let room = PIECE - self.buf.len();
            let (now, later) = bytes.split_at(room.min(bytes.len()));
            self.buf.extend_from_slice(now);
            bytes = later;
            if self.buf.len() >= PIECE {
                self.flush();
            }
        }
    }

    fn push(&mut self, b: u8) {
        self.put(&[b]);
    }

    fn flush(&mut self) {
        if !self.buf.is_empty() {
            (self.sink)(&self.buf);
            self.buf.clear();
        }
    }
}

/// Finds kitty graphics APCs (`ESC _ G ... ESC \`) in program output, across the chunks it
/// arrives in. Every other byte passes in order.
#[derive(Debug)]
pub(crate) struct Scanner {
    /// The start of the APC being collected, or of what may become one.
    buf: Vec<u8>,
    /// File bytes allowed per [`WINDOW`].
    budget: u64,
    window: Duration,
    window_start: Instant,
    spent: u64,
}

impl Default for Scanner {
    fn default() -> Self {
        Self {
            buf: Vec::new(),
            budget: BUDGET,
            window: WINDOW,
            window_start: Instant::now(),
            spent: 0,
        }
    }
}

impl Scanner {
    /// Hands `sink` the output of `input` as the emulator should see it, in pieces of at most
    /// 64 KiB; an APC split at the end of `input` waits for the next call.
    pub(crate) fn scan(&mut self, input: &[u8], mut sink: impl FnMut(&[u8])) {
        let mut out = Out {
            buf: Vec::new(),
            sink: &mut sink,
        };
        let mut rest = input;
        while !rest.is_empty() {
            // Runs without an ESC are copied whole: the bulk of the output.
            let run = rest.iter().position(|&b| b == ESC).unwrap_or(rest.len());
            let (head, tail) = rest.split_at(run);
            if self.buf.is_empty() {
                out.put(head);
            } else if self.buf.len() >= PREFIX.len() && self.buf.last() != Some(&ESC) {
                // Collecting: a byte that cannot be part of a command ends it.
                let stop = head.iter().position(|&b| !(0x20..=0x7e).contains(&b));
                let kept = head.get(..stop.unwrap_or(head.len())).unwrap_or_default();
                self.buf.extend_from_slice(kept);
                if stop.is_some() || self.buf.len() > MAX_APC {
                    self.give_up(&mut out);
                }
                if let Some(at) = stop {
                    // The offending byte goes through, and the scan goes on after it.
                    if let Some(&b) = head.get(at) {
                        out.push(b);
                    }
                    rest = rest.get(at + 1..).unwrap_or_default();
                    continue;
                }
            } else if let Some((&first, _)) = head.split_first() {
                // Inside the start of an APC or just after an ESC: byte by byte.
                self.step(first, &mut out);
                rest = rest.get(1..).unwrap_or_default();
                continue;
            }
            rest = tail;
            if let Some((&esc, after)) = rest.split_first() {
                self.step(esc, &mut out);
                rest = after;
            }
        }
        out.flush();
    }

    /// The bytes held back until more output shows what they are: a partial command.
    pub(crate) fn pending(&self) -> &[u8] {
        &self.buf
    }

    fn step(&mut self, b: u8, out: &mut Out) {
        let n = self.buf.len();
        if n == 0 {
            if b == ESC {
                self.buf.push(b);
            } else {
                out.push(b);
            }
        } else if n < PREFIX.len() {
            if PREFIX.get(n) == Some(&b) {
                self.buf.push(b);
            } else {
                self.give_up(out);
                self.step(b, out);
            }
        } else if self.buf.last() == Some(&ESC) && n > PREFIX.len() {
            if b == b'\\' {
                self.buf.push(b);
                let apc = std::mem::take(&mut self.buf);
                if self.direct(&apc, out).is_none() {
                    out.put(&apc);
                }
            } else {
                // An ESC that ends nothing: it starts something new.
                self.buf.pop();
                self.give_up(out);
                self.buf.push(ESC);
                self.step(b, out);
            }
        } else if b == ESC || (0x20..=0x7e).contains(&b) {
            self.buf.push(b);
            if self.buf.len() > MAX_APC {
                self.give_up(out);
            }
        } else {
            // CAN, SUB, C1 and the like end the command in the emulator too.
            self.give_up(out);
            out.push(b);
        }
    }

    /// Passes the collected bytes on as they are.
    fn give_up(&mut self, out: &mut Out) {
        out.put(&self.buf);
        self.buf.clear();
    }

    /// Writes the command as direct transmission to `out` when it names a file that can be
    /// read; `None` (with nothing written) when it should pass as it is.
    fn direct(&mut self, apc: &[u8], out: &mut Out) -> Option<()> {
        let inner = apc.get(PREFIX.len()..apc.len().checked_sub(2)?)?;
        let (control, payload) = match inner.iter().position(|&c| c == b';') {
            Some(i) => (inner.get(..i)?, inner.get(i + 1..)?),
            None => (inner, &[][..]),
        };
        let control = std::str::from_utf8(control).ok()?;
        let (mut medium, mut size, mut offset, mut quiet) = ("", 0u64, 0u64, None);
        let mut kept: Vec<String> = Vec::new();
        for pair in control.split(',') {
            let (key, value) = pair.split_once('=')?;
            match key {
                "t" => medium = value,
                "S" => size = value.parse().ok()?,
                "O" => offset = value.parse().ok()?,
                // A command that continues another one is not a path.
                "m" if value != "0" => return None,
                "m" => {}
                "q" => {
                    quiet = Some(value);
                    kept.push(pair.to_owned());
                }
                _ => kept.push(pair.to_owned()),
            }
        }
        if medium != "f" && medium != "t" {
            return None;
        }
        let path = decode(payload)?;
        let (file, real) = open(Path::new(OsStr::from_bytes(&path)))?;
        // Kitty deletes what it was told to take, whether or not it could use it.
        if medium == "t" && is_kitty_temp(&real) {
            let _ = std::fs::remove_file(&real);
        }
        let len = file.metadata().ok()?.len();
        let available = len.checked_sub(offset)?;
        let want = if size > 0 {
            size.min(available)
        } else {
            available
        };
        if want > MAX_FILE || !self.allow(want) {
            return None;
        }
        // The first piece is read before anything is written, so a failure passes unchanged.
        let mut piece = vec![0; usize::try_from(want).ok()?.min(RAW_CHUNK)];
        file.read_exact_at(&mut piece, offset).ok()?;

        let mut done = 0u64;
        let mut first = true;
        loop {
            done += piece.len() as u64;
            let last = done >= want;
            let mut keys = if first {
                let mut keys = kept.clone();
                keys.push("t=d".into());
                keys
            } else {
                quiet.map(|q| format!("q={q}")).into_iter().collect()
            };
            if !first || !last {
                keys.push(format!("m={}", u8::from(!last)));
            }
            out.put(b"\x1b_G");
            out.put(keys.join(",").as_bytes());
            out.push(b';');
            out.put(encode(&piece).as_bytes());
            out.put(b"\x1b\\");
            first = false;
            if last {
                return Some(());
            }
            let next = usize::try_from(want - done).ok()?.min(RAW_CHUNK);
            piece.resize(next, 0);
            // The file shrank under us: end the command with what it gave.
            if file.read_exact_at(&mut piece, offset + done).is_err() {
                out.put(b"\x1b_Gm=0;\x1b\\");
                return Some(());
            }
        }
    }

    /// Takes `bytes` from the budget of the current window, if it has them.
    fn allow(&mut self, bytes: u64) -> bool {
        let now = Instant::now();
        if now.duration_since(self.window_start) >= self.window {
            self.window_start = now;
            self.spent = 0;
        }
        match self.spent.checked_add(bytes) {
            Some(total) if total <= self.budget => {
                self.spent = total;
                true
            }
            _ => false,
        }
    }
}

/// The regular file at `path` once resolved, opened for reading, and where it really is.
/// Never a FIFO, a device or anything under `/proc`, `/sys` and `/dev` (but `/dev/shm`).
fn open(path: &Path) -> Option<(File, PathBuf)> {
    if !path.is_absolute() {
        return None;
    }
    // Resolve first: `..` and symlinks must not get a device past the check.
    let real = path.canonicalize().ok()?;
    let shm = real.starts_with("/dev/shm");
    if real.starts_with("/proc") || real.starts_with("/sys") || (real.starts_with("/dev") && !shm) {
        return None;
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY | libc::O_CLOEXEC)
        .open(&real)
        .ok()?;
    file.metadata().ok()?.is_file().then_some((file, real))
}

/// Whether `t=t` should delete the file, as kitty does: it is in a temporary directory and
/// its name says it is meant for the terminal. `real` is the resolved path.
fn is_kitty_temp(real: &Path) -> bool {
    let named = real
        .file_name()
        .is_some_and(|n| n.to_string_lossy().contains("tty-graphics-protocol"));
    named
        && [std::env::temp_dir(), "/tmp".into(), "/dev/shm".into()]
            .iter()
            .filter_map(|dir| dir.canonicalize().ok())
            .any(|dir| real.starts_with(dir))
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                let index = usize::try_from((n >> (18 - 6 * i)) & 63).unwrap_or(0);
                out.push(char::from(ALPHABET.get(index).copied().unwrap_or(b'A')));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Standard base64, with or without padding; `None` for anything else.
fn decode(text: &[u8]) -> Option<Vec<u8>> {
    let text = text
        .strip_suffix(b"==")
        .or(text.strip_suffix(b"="))
        .unwrap_or(text);
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &c in text {
        let value = u32::try_from(ALPHABET.iter().position(|&a| a == c)?).ok()?;
        acc = (acc << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((acc >> bits) & 0xff).ok()?);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests;
