//! `deck agent-event`: what an agent's hook runs to tell deck what the agent is doing.
//!
//! It never prints, always exits 0 and gives up after [`BUDGET`] in total (reading stdin
//! included), so a hook can never slow an agent down or break it.

use std::io::Read;
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clap::Parser;
use deck_core::agents::{
    Activity, AgentKind, clip_prompt, describe_hook, map_hook_event, reduce_opencode_event,
};
use deck_core::model::TerminalId;
use deck_core::protocol::Request;
use serde_json::Value;

use crate::control::{Conn, checked_paths, control_socket};

/// The whole run, stdin and the daemon's answer included.
pub const BUDGET: Duration = Duration::from_millis(500);
/// The most of the payload that is read.
const MAX_PAYLOAD: u64 = 1024 * 1024;
/// How often the wait for stdin looks at the clock.
const TICK: Duration = Duration::from_millis(10);

#[derive(Debug, Clone, Parser)]
#[command(name = "agent-event")]
pub struct AgentEventArgs {
    /// The agent that sends the event.
    #[arg(long, value_parser = ["claude", "codex", "opencode"])]
    pub agent: String,
    /// The event's name (default: `hook_event_name` of the payload).
    #[arg(long)]
    pub event: Option<String>,
}

/// Reads stdin to its end from `reader`, keeping at most [`MAX_PAYLOAD`] bytes and
/// discarding the rest, without waiting past [`BUDGET`] on `elapsed`'s clock; `None` when the
/// budget ran out first. Reading it all means the agent never gets a broken pipe.
fn read_payload<R: Read + Send + 'static>(
    mut reader: R,
    elapsed: &dyn Fn() -> Duration,
) -> Option<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        let mut sent = 0u64;
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let Some(chunk) = buf.get(..n) else { break };
                    let room = usize::try_from(MAX_PAYLOAD.saturating_sub(sent)).unwrap_or(0);
                    let keep = chunk.get(..room.min(n)).unwrap_or_default();
                    sent += keep.len() as u64;
                    if !keep.is_empty() && tx.send(keep.to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let mut payload = Vec::new();
    loop {
        let remaining = BUDGET.checked_sub(elapsed()).filter(|r| !r.is_zero())?;
        match rx.recv_timeout(remaining.min(TICK)) {
            Ok(chunk) => payload.extend_from_slice(&chunk),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return Some(payload),
        }
    }
}

/// The request for an event, or `None` when deck ignores it. `at_ms` is when the command began.
fn request(
    terminal: TerminalId,
    kind: AgentKind,
    event: Option<&str>,
    payload: &[u8],
    at_ms: u64,
) -> Option<Request> {
    let text = String::from_utf8_lossy(payload);
    let doc: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let field = |name: &str| doc.get(name).and_then(Value::as_str);
    if kind == AgentKind::OpenCode {
        // The plugin forwards raw events and the daemon interprets them, but only after the
        // tool and text ones are cut down to a line's worth of fields.
        if !doc.is_object() {
            return None;
        }
        return Some(Request::AgentEvent {
            terminal,
            kind,
            status: None,
            session_id: None,
            prompt: None,
            raw: Some(reduce_opencode_event(&doc).to_string()),
            activity: Activity::Keep,
            at_ms,
        });
    }
    let event = event.or_else(|| field("hook_event_name"))?;
    let update = map_hook_event(
        kind,
        event,
        field("notification_type"),
        field("session_id"),
        field("prompt"),
        // Only this line of the payload is sent, never the tool's input.
        describe_hook(kind, event, &doc),
    )?;
    Some(Request::AgentEvent {
        terminal,
        kind,
        status: Some(update.status),
        session_id: update.session_id,
        // The daemon keeps the first line, clipped: nothing more of the prompt is sent.
        prompt: update.prompt.as_deref().and_then(clip_prompt),
        raw: None,
        activity: update.activity,
        at_ms,
    })
}

/// Unix milliseconds now; 0 when the clock is before 1970.
fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

fn send(args: &AgentEventArgs) -> Option<()> {
    let start = Instant::now();
    let at_ms = unix_ms();
    // Stdin is read to its end first, in deck or not, so the agent never gets EPIPE.
    let payload = read_payload(std::io::stdin(), &|| start.elapsed())?;
    let terminal = std::env::var("DECK_TERMINAL_ID")
        .ok()
        .and_then(|v| v.parse().ok())
        .map(TerminalId)?;
    let kind = AgentKind::parse(&args.agent)?;
    let request = request(terminal, kind, args.event.as_deref(), &payload, at_ms)?;
    let remaining = BUDGET.checked_sub(start.elapsed())?;
    let paths = checked_paths().ok()?;
    let mut conn = Conn::open(&control_socket(&paths), remaining.max(TICK)).ok()?;
    let _ = conn.control(request);
    Some(())
}

/// Runs the command. A watchdog ends the process with status 0 once [`BUDGET`] is spent,
/// whatever blocks (a connect, a read, the sum of several timeouts).
pub fn run(args: &AgentEventArgs) -> ExitCode {
    std::thread::spawn(|| {
        std::thread::sleep(BUDGET);
        std::process::exit(0);
    });
    let _ = send(args);
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::io::Cursor;

    use deck_core::agents::AgentStatus;

    use super::*;

    /// A reader that blocks until its sender is dropped: a stdin that never closes.
    struct Blocked(mpsc::Receiver<()>);

    impl Read for Blocked {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            let _ = self.0.recv();
            Ok(0)
        }
    }

    #[test]
    fn a_stdin_that_never_closes_gives_up_at_the_budget() {
        let (keep_open, rx) = mpsc::channel();
        // An injected clock that moves 100 ms per look: the budget is gone after five.
        let looks = Cell::new(0u32);
        let clock = || {
            looks.set(looks.get() + 1);
            Duration::from_millis(100) * looks.get()
        };
        assert_eq!(read_payload(Blocked(rx), &clock), None);
        assert_eq!(looks.get(), 5);
        drop(keep_open);
    }

    #[test]
    fn a_payload_is_read_to_its_end() {
        let clock = || Duration::ZERO;
        let data = Cursor::new(b"{\"a\":1}".to_vec());
        assert_eq!(read_payload(data, &clock), Some(b"{\"a\":1}".to_vec()));
    }

    #[test]
    fn a_payload_is_cut_at_one_mebibyte() {
        let clock = || Duration::ZERO;
        let data = Cursor::new(vec![b'x'; 2 * 1024 * 1024]);
        let read = read_payload(data, &clock).unwrap_or_default();
        assert_eq!(read.len(), 1024 * 1024);
    }

    #[test]
    fn the_event_comes_from_the_flag_or_the_payload() {
        let payload = br#"{"hook_event_name":"Stop","session_id":"abc"}"#;
        let t = TerminalId(3);
        let from_payload = request(t, AgentKind::Claude, None, payload, 7);
        assert_eq!(
            from_payload,
            Some(Request::AgentEvent {
                terminal: t,
                kind: AgentKind::Claude,
                status: Some(AgentStatus::Idle),
                session_id: Some("abc".into()),
                prompt: None,
                raw: None,
                activity: Activity::Set("done".into()),
                at_ms: 7,
            })
        );
        let flag = request(t, AgentKind::Claude, Some("UserPromptSubmit"), payload, 0);
        assert!(matches!(
            flag,
            Some(Request::AgentEvent {
                status: Some(AgentStatus::Working),
                session_id: Some(ref s),
                ..
            }) if s == "abc"
        ));
        assert_eq!(request(t, AgentKind::Claude, None, b"not json", 0), None);
        assert_eq!(
            request(t, AgentKind::Claude, Some("Notification"), payload, 0),
            None
        );

        // Only what the daemon keeps of a prompt is sent: its first line, 200 characters.
        let long = serde_json::json!({
            "hook_event_name": "UserPromptSubmit",
            "session_id": "abc",
            "prompt": format!("\n  {}\nsecond line with a secret", "p".repeat(500)),
        })
        .to_string();
        let sent = request(t, AgentKind::Claude, None, long.as_bytes(), 0);
        assert!(matches!(
            sent,
            Some(Request::AgentEvent { prompt: Some(ref p), .. }) if *p == "p".repeat(200)
        ));
    }

    #[test]
    fn opencode_forwards_other_events_as_they_are() {
        let r = request(
            TerminalId(1),
            AgentKind::OpenCode,
            None,
            b"{\"type\":\"x\"}",
            0,
        );
        assert!(matches!(
            r,
            Some(Request::AgentEvent { status: None, raw: Some(ref raw), .. }) if raw == "{\"type\":\"x\"}"
        ));
        assert_eq!(
            request(TerminalId(1), AgentKind::OpenCode, None, b"nope", 0),
            None
        );
    }

    #[test]
    fn an_opencode_tool_or_text_event_sends_only_its_line() {
        let marker = "MARKER-SECRET";
        let tool = serde_json::json!({"type": "session.tool.called", "data": {
            "sessionID": "s1", "id": "c1", "name": "write",
            "input": {"filePath": "/x/a.rs", "content": marker, "command": format!("ls\n{marker}")},
        }});
        let text = serde_json::json!({"type": "deck.text", "data": {
            "sessionID": "s1", "text": format!("done\n{marker}"),
        }});
        for doc in [tool, text] {
            let sent = request(
                TerminalId(1),
                AgentKind::OpenCode,
                None,
                doc.to_string().as_bytes(),
                0,
            );
            assert!(sent.is_some());
            assert!(!format!("{sent:?}").contains(marker), "{sent:?}");
        }
    }

    #[test]
    fn a_tool_event_sends_its_line_and_nothing_of_the_input() {
        let payload = serde_json::json!({
            "hook_event_name": "PreToolUse",
            "session_id": "abc",
            "tool_name": "Bash",
            "tool_input": {"command": "curl -H 'Authorization: sk-secret' x", "description": "Fetch the page"},
            "agent_id": "a1",
            "agent_type": "worker",
        })
        .to_string();
        let sent = request(
            TerminalId(2),
            AgentKind::Claude,
            None,
            payload.as_bytes(),
            99,
        );
        assert_eq!(
            sent,
            Some(Request::AgentEvent {
                terminal: TerminalId(2),
                kind: AgentKind::Claude,
                status: Some(AgentStatus::Working),
                session_id: Some("abc".into()),
                prompt: None,
                raw: None,
                activity: Activity::Set("worker: fetch the page".into()),
                at_ms: 99,
            })
        );
        assert!(!format!("{sent:?}").contains("sk-secret"));
        // A very long line is cut by the sender already.
        let long = serde_json::json!({
            "hook_event_name": "Stop",
            "last_assistant_message": format!("{}\nmore", "z".repeat(400)),
        })
        .to_string();
        let Some(Request::AgentEvent {
            activity: Activity::Set(line),
            ..
        }) = request(TerminalId(2), AgentKind::Codex, None, long.as_bytes(), 1)
        else {
            panic!("no line");
        };
        assert_eq!(line, "z".repeat(80));
    }
}
