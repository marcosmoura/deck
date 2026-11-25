//! Agent status, end to end: the real `deck` in a PTY, with fake `#!/bin/sh` agents on `PATH`
//! (no real agent and no `node` is needed).

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;

use deck_core::agents::{AgentKind, AgentStatus};
use deck_core::model::TerminalId;
use deck_core::protocol::{
    ClientMsg, FrameReader, PROTOCOL_VERSION, Request, Response, Role, ServerMsg, encode,
};
use serde_json::Value;
use support::{Harness, TIMEOUT, wait_until};

const SIZE: (u16, u16) = (100, 24);
const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const ENTER: &str = "\r";
const PREFIX: &[u8] = b"\x1b ";

/// A directory with a fake agent named `name`: it prints, then waits for a line on stdin.
fn fake_agent(name: &str) -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .prefix("ag")
        .tempdir_in("/tmp")
        .unwrap();
    let script = dir.path().join(name);
    fs::write(&script, "#!/bin/sh\necho agent-ready\nread line\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    dir
}

/// `deck` with the sidebar on, a fast detection tick and `bin` first on the shell's `PATH`.
fn attached(bin: &tempfile::TempDir) -> Harness {
    let path = format!(
        "{}:{}",
        bin.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let h = Harness::builder(SIZE)
        .config_unset("sidebar")
        .env("PATH", &path)
        .env("DECK_TEST_AGENT_POLL_MS", "100")
        .deck();
    h.wait_for_text("│$", TIMEOUT);
    h
}

fn rows(h: &Harness) -> Vec<String> {
    h.screen_text().lines().map(str::to_string).collect()
}

/// The sidebar's status slot (column 25, before the margin) of the row of the first panel.
fn status_cell(h: &Harness) -> Option<char> {
    rows(h).get(3)?.chars().nth(25)
}

fn wait_status(h: &Harness, what: &str, accept: impl Fn(char) -> bool) {
    wait_until(what, TIMEOUT, || status_cell(h).is_some_and(&accept));
}

fn is_spinner(c: char) -> bool {
    SPINNER.contains(&c)
}

fn terminal_id(h: &Harness) -> TerminalId {
    let out = h.run_deck(&["ls", "--json"]);
    let doc: Value = serde_json::from_str(&support::stdout(&out)).unwrap();
    TerminalId(
        doc["sessions"][0]["spaces"][0]["panels"][0]["terminals"][0]["id"]
            .as_u64()
            .unwrap(),
    )
}

fn agent_json(h: &Harness) -> Value {
    let out = h.run_deck(&["ls", "--json"]);
    let doc: Value = serde_json::from_str(&support::stdout(&out)).unwrap();
    doc["sessions"][0]["spaces"][0]["panels"][0]["terminals"][0]["agent"].clone()
}

/// Sends an `AgentEvent` to the daemon of `h`, like the hooks of a later sub-spec will.
fn event(h: &Harness, terminal: TerminalId, status: AgentStatus, session_id: &str) {
    let mut stream = UnixStream::connect(h.home().socket()).unwrap();
    stream.set_read_timeout(Some(TIMEOUT)).unwrap();
    let hello = ClientMsg::Hello {
        version: PROTOCOL_VERSION,
        role: Role::Control,
        cwd: std::env::temp_dir(),
        env: Vec::new(),
        colors: None,
    };
    let request = ClientMsg::Request(Request::AgentEvent {
        terminal,
        kind: AgentKind::Claude,
        status: Some(status),
        session_id: Some(session_id.into()),
        prompt: Some("refactor the layout engine".into()),
        raw: None,
    });
    stream.write_all(&encode(&hello).unwrap()).unwrap();
    stream.write_all(&encode(&request).unwrap()).unwrap();
    let mut reader = FrameReader::new();
    let mut buf = [0u8; 16 * 1024];
    loop {
        while let Some(msg) = reader.next_message::<ServerMsg>().unwrap() {
            if let ServerMsg::Response(response) = msg {
                assert_eq!(response, Response::Ok);
                return;
            }
        }
        match stream.read(&mut buf).unwrap() {
            0 => panic!("the daemon closed the connection"),
            n => reader.push(&buf[..n]),
        }
    }
}

#[test]
fn a_fake_agent_is_detected_by_its_process() {
    let bin = fake_agent("claude");
    let mut h = attached(&bin);
    assert_eq!(status_cell(&h), Some(' '));

    h.send_keys("claude\r");
    h.wait_for_text("agent-ready", TIMEOUT);
    // It just printed: the spinner.
    wait_status(&h, "the spinner", is_spinner);
    h.capture("process-detect-active");
    // Then nothing for two seconds: the quiet circle.
    wait_status(&h, "the quiet circle", |c| c == '○');
    h.capture("process-detect");
    assert_eq!(agent_json(&h)["kind"], "claude");
    assert_eq!(agent_json(&h)["source"], "process");
    assert_eq!(agent_json(&h)["status"], "quiet");

    // Its line ends: the agent exits and the glyph goes.
    h.send_keys(ENTER);
    wait_status(&h, "no glyph", |c| c == ' ');
    assert_eq!(agent_json(&h), Value::Null);
}

#[test]
fn hook_events_show_working_and_waiting_and_typing_answers() {
    let bin = fake_agent("claude");
    let mut h = attached(&bin);
    h.send_keys("claude\r");
    h.wait_for_text("agent-ready", TIMEOUT);
    let terminal = terminal_id(&h);

    event(&h, terminal, AgentStatus::Working, "s1");
    wait_status(&h, "the working spinner", is_spinner);
    wait_until("the spinner slot and the agent's icon", TIMEOUT, || {
        rows(&h).get(3).is_some_and(|row| {
            let chars: Vec<char> = row.chars().collect();
            chars.get(25).is_some_and(|c| is_spinner(*c))
                && chars
                    .get(0..4)
                    .is_some_and(|s| s.iter().collect::<String>() == " ✻  ")
        })
    });
    event(&h, terminal, AgentStatus::Waiting, "s1");
    wait_status(&h, "the waiting mark", |c| c == '!');
    wait_until("the mark and the agent's icon", TIMEOUT, || {
        rows(&h)
            .get(3)
            .is_some_and(|row| row.starts_with(" ✻  ") && row.chars().nth(25) == Some('!'))
    });
    h.capture("waiting");
    let agent = agent_json(&h);
    assert_eq!(agent["status"], "waiting");
    assert_eq!(agent["source"], "hooks");
    assert_eq!(agent["session_id"], "s1");

    // Answering the prompt: the agent works again.
    h.send_keys("y");
    wait_status(&h, "the spinner again", is_spinner);
    assert_eq!(agent_json(&h)["status"], "working");

    event(&h, terminal, AgentStatus::Idle, "s1");
    wait_status(&h, "the idle dot", |c| c == '●');
    h.capture("idle");
}

#[test]
fn a_grid_title_shows_the_glyph_and_the_agent() {
    let bin = fake_agent("claude");
    let mut h = attached(&bin);
    h.send_keys("claude\r");
    h.wait_for_text("agent-ready", TIMEOUT);
    let terminal = terminal_id(&h);
    // A second terminal turns the panel into a grid with borders and titles.
    h.send_bytes(PREFIX);
    h.send_keys("t");
    wait_until("a grid", TIMEOUT, || {
        rows(&h).iter().any(|r| r.contains('╭'))
    });
    event(&h, terminal, AgentStatus::Waiting, "s1");
    wait_until("the title", TIMEOUT, || {
        rows(&h).iter().any(|r| r.contains("!  claude · "))
    });
    h.capture("grid-title");
    // The other terminal keeps its number.
    assert!(rows(&h).iter().any(|r| r.contains(" 2 · ")));
}

/// Opens a second panel (it takes focus) and returns the names of the project and of the
/// first panel, which is now inactive, and the first panel's terminal.
fn inactive_first_panel(h: &mut Harness) -> (String, String, TerminalId) {
    let terminal = terminal_id(h);
    h.send_bytes(PREFIX);
    h.send_keys("c");
    wait_until("a second panel", TIMEOUT, || h.is_active_row(4));
    let out = h.run_deck(&["ls", "--json"]);
    let doc: Value = serde_json::from_str(&support::stdout(&out)).unwrap();
    let project = &doc["sessions"][0]["spaces"][0];
    (
        project["name"].as_str().unwrap().to_string(),
        project["panels"][0]["name"].as_str().unwrap().to_string(),
        terminal,
    )
}

/// The name of the first panel, from `deck ls --json`.
fn panel_name(h: &Harness) -> String {
    let out = h.run_deck(&["ls", "--json"]);
    let doc: Value = serde_json::from_str(&support::stdout(&out)).unwrap();
    doc["sessions"][0]["spaces"][0]["panels"][0]["name"]
        .as_str()
        .unwrap()
        .to_string()
}

/// The bytes the program wrote after the first `from` ones.
fn written_since(h: &Harness, from: usize) -> Vec<u8> {
    h.raw_output().get(from..).unwrap_or_default().to_vec()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn an_agent_that_needs_you_in_another_panel_is_marked_counted_and_notified() {
    let bin = fake_agent("claude");
    let mut h = attached(&bin);
    h.send_keys("claude\r");
    h.wait_for_text("agent-ready", TIMEOUT);
    let (project, _, terminal) = inactive_first_panel(&mut h);

    let before = h.raw_output().len();
    event(&h, terminal, AgentStatus::Working, "s1");
    event(&h, terminal, AgentStatus::Waiting, "s1");
    wait_until("the unseen mark", TIMEOUT, || {
        rows(&h).get(3).is_some_and(|row| row.contains(" •"))
    });
    h.capture("attention");
    // The panel was named after the agent's first prompt by the events.
    let panel = panel_name(&h);
    assert_eq!(panel, "refactor the layout engine");
    let row = rows(&h)[3].clone();
    // The sidebar cuts the long name to its width.
    assert!(row.starts_with(" ✻  refactor the lay"), "{row}");
    assert!(h.cell(3, 4).bold(), "the name of an unseen panel is bold");

    let body = format!("claude needs you · {project} › {panel}");
    let expected = format!("\x1b]9;{body}\x07");
    let written = written_since(&h, before);
    assert!(contains(&written, expected.as_bytes()), "{written:?}");
    // Once only.
    let text = String::from_utf8_lossy(&written).into_owned();
    assert_eq!(text.matches("\x1b]9;").count(), 1, "{text:?}");

    // Focusing the panel clears the mark; the agent still waits.
    h.send_bytes(PREFIX);
    h.send_keys("1");
    wait_until("the mark gone", TIMEOUT, || {
        let r = rows(&h);
        r.get(3).is_some_and(|row| !row.contains('•')) && h.is_active_row(3)
    });
    assert_eq!(status_cell(&h), Some('!'));
    h.capture("attention-seen");
}

#[test]
fn notify_bell_rings_and_none_stays_silent() {
    for (mode, rings) in [("bell", true), ("none", false)] {
        let bin = fake_agent("claude");
        let path = format!(
            "{}:{}",
            bin.path().display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut h = Harness::builder(SIZE)
            .config_unset("sidebar")
            .config("notify", &format!("\"{mode}\""))
            .env("PATH", &path)
            .env("DECK_TEST_AGENT_POLL_MS", "100")
            .deck();
        h.wait_for_text("│$", TIMEOUT);
        h.send_keys("claude\r");
        h.wait_for_text("agent-ready", TIMEOUT);
        let (_, _, terminal) = inactive_first_panel(&mut h);

        let before = h.raw_output().len();
        event(&h, terminal, AgentStatus::Working, "s1");
        event(&h, terminal, AgentStatus::Idle, "s1");
        wait_until("the unseen mark", TIMEOUT, || {
            rows(&h).get(3).is_some_and(|row| row.contains(" •"))
        });
        let written = written_since(&h, before);
        assert_eq!(contains(&written, b"\x07"), rings, "{mode}: {written:?}");
        assert!(!contains(&written, b"\x1b]9;"), "{mode}: {written:?}");
    }
}

#[test]
fn the_active_panel_alerts_only_after_the_outer_terminal_lost_focus() {
    let bin = fake_agent("claude");
    let mut h = attached(&bin);
    h.send_keys("claude\r");
    h.wait_for_text("agent-ready", TIMEOUT);
    let terminal = terminal_id(&h);

    let before = h.raw_output().len();
    event(&h, terminal, AgentStatus::Working, "s1");
    event(&h, terminal, AgentStatus::Waiting, "s1");
    wait_status(&h, "the waiting mark", |c| c == '!');
    // Looking at it: no mark, no notification.
    assert!(!rows(&h)[3].contains('•'));
    assert!(!contains(&written_since(&h, before), b"\x1b]9;"));

    // Focus out, then in: only the unfocused one notifies. A new terminal (a grid appears)
    // is the sync point: the daemon reads one client's messages in order, so the focus
    // report was handled before the events below.
    h.send_bytes(b"\x1b[O");
    h.send_bytes(PREFIX);
    h.send_keys("t");
    wait_until("a grid", TIMEOUT, || {
        rows(&h).iter().any(|r| r.contains('╭'))
    });
    let before = h.raw_output().len();
    event(&h, terminal, AgentStatus::Working, "s1");
    event(&h, terminal, AgentStatus::Idle, "s1");
    wait_until("the unseen mark", TIMEOUT, || rows(&h)[3].contains(" •"));
    assert!(contains(
        &written_since(&h, before),
        b"\x1b]9;claude finished"
    ));
    h.send_bytes(b"\x1b[I");
    wait_until("the mark gone", TIMEOUT, || !rows(&h)[3].contains('•'));
}

#[test]
fn the_jump_cycles_waiting_agents_then_unseen_idle_ones_then_flashes() {
    let bin = fake_agent("claude");
    let mut h = attached(&bin);
    h.send_keys("claude\r");
    h.wait_for_text("agent-ready", TIMEOUT);
    let first = terminal_id(&h);
    // Two more panels, each with a terminal that is told to be an agent.
    let mut ids = vec![first];
    for _ in 0..2 {
        h.send_bytes(PREFIX);
        h.send_keys("c");
        let count = ids.len() + 1;
        wait_until("a new panel", TIMEOUT, || h.is_active_row(2 + count));
        let out = h.run_deck(&["ls", "--json"]);
        let doc: Value = serde_json::from_str(&support::stdout(&out)).unwrap();
        let panels = doc["sessions"][0]["spaces"][0]["panels"]
            .as_array()
            .unwrap();
        let last = panels.last().unwrap();
        ids.push(TerminalId(last["terminals"][0]["id"].as_u64().unwrap()));
    }
    // The first and the third wait; the second is idle and unseen.
    for t in &ids {
        event(&h, *t, AgentStatus::Working, "s");
    }
    event(&h, ids[0], AgentStatus::Waiting, "s");
    event(&h, ids[1], AgentStatus::Idle, "s");
    event(&h, ids[2], AgentStatus::Waiting, "s");
    let waiting = |h: &Harness, row: usize| {
        rows(h)
            .get(row)
            .is_some_and(|r| r.chars().nth(25) == Some('!'))
    };
    wait_until("two waiting", TIMEOUT, || waiting(&h, 3) && waiting(&h, 5));

    // From the third panel (active): wraps to the first, then back to the third.
    h.send_bytes(PREFIX);
    h.send_keys("a");
    wait_until("the first panel", TIMEOUT, || h.is_active_row(3));
    h.capture("jump");
    h.send_bytes(PREFIX);
    h.send_keys("a");
    wait_until("the third panel", TIMEOUT, || h.is_active_row(5));

    // Nobody waits any more: the unseen idle panel, then the flash.
    event(&h, ids[0], AgentStatus::Working, "s");
    event(&h, ids[2], AgentStatus::Working, "s");
    wait_until("no one waiting", TIMEOUT, || {
        !waiting(&h, 3) && !waiting(&h, 5)
    });
    h.send_bytes(PREFIX);
    h.send_keys("a");
    wait_until("the idle panel", TIMEOUT, || h.is_active_row(4));
    h.send_bytes(PREFIX);
    h.send_keys("a");
    wait_until("the flash", TIMEOUT, || {
        rows(&h)
            .last()
            .is_some_and(|b| b.contains("no agent needs you"))
    });
    h.capture("jump-none");
}

#[test]
fn a_collapsed_project_shows_how_many_of_its_panels_wait() {
    let bin = fake_agent("claude");
    let mut h = attached(&bin);
    h.send_keys("claude\r");
    h.wait_for_text("agent-ready", TIMEOUT);
    let terminal = terminal_id(&h);
    // A second panel to look at, so the first project can fold away from under the cursor.
    h.send_bytes(PREFIX);
    h.send_keys("c");
    event(&h, terminal, AgentStatus::Working, "s1");
    event(&h, terminal, AgentStatus::Waiting, "s1");
    wait_status(&h, "the waiting mark", |c| c == '!');
    // Fold the project: the sidebar's Left key.
    h.send_bytes(PREFIX);
    h.send_bytes(b"\t");
    h.send_bytes(b"\x1b[D");
    // The folded project row shows the badge, selected or not.
    wait_until("the badge", TIMEOUT, || {
        rows(&h).get(2).is_some_and(|r| r.contains(" !1"))
    });
    h.send_keys("\x1b");
    wait_until("the badge", TIMEOUT, || {
        rows(&h).get(2).is_some_and(|r| r.contains(" !1"))
    });
    h.capture("collapsed-badge");
}

#[test]
fn a_running_command_spins_and_stops_when_it_ends() {
    let bin = fake_agent("claude");
    let mut h = attached(&bin);
    assert_eq!(status_cell(&h), Some(' '));
    // A command of the shell: a spinner once it has run for a second.
    h.send_keys("sleep 30\r");
    wait_status(&h, "the spinner of the command", is_spinner);
    h.capture("command-busy");
    // Interrupted: the shell is in the foreground again and the slot goes.
    h.send_keys("\x03");
    wait_status(&h, "no status", |c| c == ' ');
}
