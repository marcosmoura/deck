//! The control requests that type into a terminal and read its screen.

use std::sync::Arc;

use deck_core::keys;
use deck_core::model::TerminalId;
use deck_core::protocol::{Modes, Request, Response};

use super::Shared;
use crate::terminal::TermHandle;

const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";

/// `bytes` between the bracketed-paste markers. Every ESC byte is left out, so pasted text
/// can never end the paste early, however it is built.
fn bracketed(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 12);
    out.extend_from_slice(PASTE_START);
    out.extend(bytes.iter().copied().filter(|b| *b != 0x1b));
    out.extend_from_slice(PASTE_END);
    out
}

impl Shared {
    /// The terminal `id` of any session; a kept one that exited is still there.
    pub(super) fn terminal_handle(&self, id: TerminalId) -> Result<Arc<TermHandle>, String> {
        self.st()
            .sessions
            .iter()
            .find_map(|s| s.terminals.get(&id).cloned())
            .ok_or_else(|| format!("no terminal {id}"))
    }

    /// The terminal `id`, when its program still runs.
    fn live_terminal(&self, id: TerminalId) -> Result<Arc<TermHandle>, String> {
        let handle = self.terminal_handle(id)?;
        if handle.has_exited() {
            return Err(format!("terminal {id} has exited"));
        }
        Ok(handle)
    }

    /// Answers `Send`, `SendKeys` and `Capture`.
    pub fn terminal_io(&self, request: Request) -> Response {
        let result = match request {
            Request::Send {
                terminal,
                bytes,
                paste,
            } => self.send(terminal, bytes, paste).map(|()| Response::Ok),
            Request::SendKeys { terminal, keys } => {
                self.send_keys(terminal, &keys).map(|()| Response::Ok)
            }
            Request::Capture {
                terminal,
                lines,
                history,
                ansi,
            } => self
                .terminal_handle(terminal)
                .and_then(|h| {
                    h.with_emu(move |e| e.capture(terminal, lines, history, ansi))
                        .ok_or_else(|| format!("no terminal {terminal}"))
                })
                .map(Response::Captured),
            _ => Err("not a terminal request".to_string()),
        };
        result.unwrap_or_else(Response::Error)
    }

    fn send(&self, terminal: TerminalId, bytes: Vec<u8>, paste: bool) -> Result<(), String> {
        let handle = self.live_terminal(terminal)?;
        let wrap = paste
            && handle
                .with_emu(|e| e.modes().has(Modes::BRACKETED_PASTE))
                .unwrap_or(false);
        handle.write(if wrap { bracketed(&bytes) } else { bytes });
        Ok(())
    }

    fn send_keys(&self, terminal: TerminalId, keys: &[keys::KeySpec]) -> Result<(), String> {
        let handle = self.live_terminal(terminal)?;
        let modes = handle
            .with_emu(|e| e.modes())
            .ok_or_else(|| format!("no terminal {terminal}"))?;
        let bytes: Vec<u8> = keys
            .iter()
            .flat_map(|k| keys::encode(&k.to_event(), modes))
            .collect();
        handle.write(bytes);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_paste_is_wrapped_and_cannot_end_itself() {
        assert_eq!(bracketed(b"a\nb"), b"\x1b[200~a\nb\x1b[201~");
        assert_eq!(bracketed(b""), b"\x1b[200~\x1b[201~");
        assert_eq!(
            bracketed(b"x\x1b[201~rm -rf\x1b[201~y"),
            b"\x1b[200~x[201~rm -rf[201~y\x1b[201~"
        );
        // Stripping must not rebuild the marker out of its halves.
        assert_eq!(
            bracketed(b"\x1b[20\x1b[201~1~"),
            b"\x1b[200~[20[201~1~\x1b[201~"
        );
    }
}
