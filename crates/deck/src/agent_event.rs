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
use deck_core::model::{SessionView, TerminalId};
use deck_core::protocol::{Request, Response};
use serde_json::Value;

use crate::control::{Conn, Outcome, checked_paths, control_socket};

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

/// The terminal a Codex event is about. Codex runs its hooks in one background server shared
/// by every Codex on the machine, started with the environment of the terminal that started
/// it first, so `DECK_TERMINAL_ID` names that terminal, which may be another one or gone. The
/// event goes instead to the Codex terminal that already has its session; else to a Codex
/// terminal in the event's directory, else in any: one without a session first, then the
/// focused terminal of an attached session (where the user just started Codex or typed a
/// prompt), then the one the environment names. With no Codex terminal, the environment's.
fn route(
    tree: &[SessionView],
    kind: AgentKind,
    named: TerminalId,
    session_id: Option<&str>,
    cwd: Option<&std::path::Path>,
) -> TerminalId {
    struct Seen<'a> {
        id: TerminalId,
        session: Option<&'a str>,
        cwd: Option<&'a std::path::Path>,
        focused: bool,
    }
    let agents: Vec<Seen> = tree
        .iter()
        .flat_map(|s| {
            s.projects.iter().flat_map(move |p| {
                p.panels.iter().flat_map(move |panel| {
                    let shown = s.attached_clients > 0 && s.active_panel == Some(panel.id);
                    panel.terminals.iter().filter_map(move |t| {
                        let agent = t.agent.as_ref().filter(|a| a.kind == kind)?;
                        Some(Seen {
                            id: t.id,
                            session: agent.session_id.as_deref(),
                            cwd: t.cwd.as_deref(),
                            focused: shown && panel.focused_terminal == Some(t.id),
                        })
                    })
                })
            })
        })
        .collect();
    if let Some(sid) = session_id
        && let Some(a) = agents.iter().find(|a| a.session == Some(sid))
    {
        return a.id;
    }
    let best = |pool: Vec<&Seen>| {
        pool.into_iter()
            .max_by_key(|a| (a.session.is_none(), a.focused, a.id == named))
            .map(|a| a.id)
    };
    let here = cwd.map_or_else(Vec::new, |cwd| {
        agents.iter().filter(|a| a.cwd == Some(cwd)).collect()
    });
    best(here)
        .or_else(|| best(agents.iter().collect()))
        .unwrap_or(named)
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
    // Built first: an event deck ignores costs no connection.
    request(terminal, kind, args.event.as_deref(), &payload, at_ms)?;
    let remaining = BUDGET.checked_sub(start.elapsed())?;
    let paths = checked_paths().ok()?;
    let mut conn = Conn::open(&control_socket(&paths), remaining.max(TICK)).ok()?;
    let terminal = if kind == AgentKind::Codex {
        let doc: Value = serde_json::from_slice(&payload).unwrap_or(Value::Null);
        let field = |name: &str| doc.get(name).and_then(Value::as_str);
        match conn.control(Request::Tree { session: None }) {
            Outcome::Response(Response::Tree(tree)) => route(
                &tree,
                kind,
                terminal,
                field("session_id"),
                field("cwd").map(std::path::Path::new),
            ),
            _ => terminal,
        }
    } else {
        terminal
    };
    let request = request(terminal, kind, args.event.as_deref(), &payload, at_ms)?;
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
            "last_assistant_message": format!("{}\nmore", "z".repeat(600)),
        })
        .to_string();
        let Some(Request::AgentEvent {
            activity: Activity::Set(line),
            ..
        }) = request(TerminalId(2), AgentKind::Codex, None, long.as_bytes(), 1)
        else {
            panic!("no line");
        };
        assert_eq!(line, "z".repeat(deck_core::agents::ACTIVITY_MAX));
    }

    /// A session (attached when `attached`) whose panel `i` holds terminal `t` with an agent
    /// of `kind` in `cwd`, with `session`; panel 1 is active and focused on its terminal.
    fn tree(
        attached: bool,
        terminals: &[(u64, AgentKind, Option<&str>, &str)],
    ) -> Vec<SessionView> {
        use deck_core::agents::{AgentSource, AgentState};
        use deck_core::layout::Ratios;
        use deck_core::model::{
            NameSource, PanelId, PanelView, ProjectId, ProjectView, SessionId, TerminalView,
        };
        let panels = terminals
            .iter()
            .enumerate()
            .map(|(i, (t, kind, session, cwd))| PanelView {
                id: PanelId(100 + i as u64),
                name: format!("p{i}"),
                name_source: NameSource::Auto,
                terminals: vec![TerminalView {
                    id: TerminalId(*t),
                    program: kind.name().into(),
                    title: None,
                    cwd: Some(std::path::PathBuf::from(cwd)),
                    exit_status: None,
                    pid: None,
                    keep: false,
                    agent: Some(AgentState {
                        kind: *kind,
                        status: AgentStatus::Quiet,
                        source: AgentSource::Process,
                        session_id: session.map(str::to_string),
                        first_prompt: None,
                        since: Duration::ZERO,
                        activity: None,
                    }),
                    busy: false,
                }],
                focused_terminal: Some(TerminalId(*t)),
                ratios: Ratios::default_for(1),
                unseen: false,
            })
            .collect();
        vec![SessionView {
            id: SessionId(1),
            name: "main".into(),
            projects: vec![ProjectView {
                id: ProjectId(2),
                name: "deck".into(),
                root: "/p".into(),
                collapsed: false,
                panels,
                active_panel: Some(PanelId(101)),
            }],
            active_project: Some(ProjectId(2)),
            active_panel: Some(PanelId(101)),
            layout_viewport: None,
            attached_clients: u32::from(attached),
        }]
    }

    const CX: AgentKind = AgentKind::Codex;
    const GONE: TerminalId = TerminalId(99);

    fn at(cwd: &str) -> Option<&std::path::Path> {
        Some(std::path::Path::new(cwd))
    }

    #[test]
    fn a_codex_event_goes_to_the_terminal_with_its_session() {
        let t = tree(
            true,
            &[(10, CX, Some("a"), "/p"), (11, CX, Some("b"), "/p")],
        );
        assert_eq!(route(&t, CX, GONE, Some("a"), at("/p")), TerminalId(10));
        assert_eq!(
            route(&t, CX, GONE, Some("b"), at("/elsewhere")),
            TerminalId(11)
        );
    }

    #[test]
    fn a_new_codex_session_goes_to_a_terminal_in_its_directory_without_one() {
        // The terminal in another directory is not a candidate while one in this one is.
        let t = tree(
            true,
            &[
                (10, CX, None, "/other"),
                (11, CX, Some("old"), "/p"),
                (12, CX, None, "/p"),
            ],
        );
        assert_eq!(route(&t, CX, GONE, Some("new"), at("/p")), TerminalId(12));
        // In this directory only one with a session: it is the one (a `/new` in it).
        let t = tree(
            true,
            &[(10, CX, None, "/other"), (11, CX, Some("old"), "/p")],
        );
        assert_eq!(route(&t, CX, GONE, Some("new"), at("/p")), TerminalId(11));
    }

    #[test]
    fn between_equal_codex_terminals_the_focused_one_wins() {
        // Panel 1 (terminal 11) is the active panel of an attached session.
        let t = tree(true, &[(10, CX, None, "/p"), (11, CX, None, "/p")]);
        assert_eq!(route(&t, CX, GONE, Some("new"), at("/p")), TerminalId(11));
        // Detached: nobody focuses anything, the environment's terminal breaks the tie.
        let t = tree(false, &[(10, CX, None, "/p"), (11, CX, None, "/p")]);
        assert_eq!(
            route(&t, CX, TerminalId(10), Some("new"), at("/p")),
            TerminalId(10)
        );
    }

    #[test]
    fn with_no_codex_terminal_the_environments_terminal_is_kept() {
        let t = tree(true, &[(10, AgentKind::Claude, None, "/p")]);
        assert_eq!(route(&t, CX, GONE, Some("new"), at("/p")), GONE);
        // A Codex terminal in another directory is still better than a dead one.
        let t = tree(true, &[(10, CX, None, "/other")]);
        assert_eq!(route(&t, CX, GONE, Some("new"), at("/p")), TerminalId(10));
    }
}
