//! Panels named after what runs in them, end to end: the real `deck` in a PTY, with fake
//! `#!/bin/sh` programs named `claude` and `nvim` on `PATH`.

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

use deck_core::agents::AgentKind;
use deck_core::model::TerminalId;
use deck_core::protocol::{
    ClientMsg, FrameReader, PROTOCOL_VERSION, Request, Response, Role, ServerMsg, encode,
};
use serde_json::Value;
use support::{Harness, TIMEOUT, wait_until};

const SIZE: (u16, u16) = (100, 24);
const PREFIX: &[u8] = b"\x1b ";

/// A directory with a fake program named `name` that runs `body`.
fn fake(name: &str, body: &str) -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .prefix("ti")
        .tempdir_in("/tmp")
        .unwrap();
    let script = dir.path().join(name);
    fs::write(&script, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    dir
}

/// `deck` with the sidebar on, a fast detection tick and `bin` first on the shell's `PATH`.
fn attached(bin: &tempfile::TempDir) -> Harness {
    attached_in(bin, &[])
}

/// [`attached`] with more variables in the client's environment.
fn attached_in(bin: &tempfile::TempDir, env: &[(&str, &str)]) -> Harness {
    let path = format!(
        "{}:{}",
        bin.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut builder = Harness::builder(SIZE)
        .config_unset("sidebar")
        .env("PATH", &path)
        .env("DECK_TEST_AGENT_POLL_MS", "100");
    for (key, value) in env {
        builder = builder.env(key, value);
    }
    let h = builder.deck();
    h.wait_for_text("│$", TIMEOUT);
    h
}

fn rows(h: &Harness) -> Vec<String> {
    h.screen_text().lines().map(str::to_string).collect()
}

fn tree(h: &Harness) -> Value {
    let out = h.run_deck(&["ls", "--json"]);
    serde_json::from_str(&support::stdout(&out)).unwrap()
}

fn panel(h: &Harness) -> Value {
    tree(h)["sessions"][0]["spaces"][0]["panels"][0].clone()
}

fn panel_name(h: &Harness) -> String {
    panel(h)["name"].as_str().unwrap().to_string()
}

fn wait_name(h: &Harness, name: &str) {
    wait_until(&format!("the panel named {name}"), TIMEOUT, || {
        panel_name(h) == name
    });
}

fn terminal_id(h: &Harness) -> TerminalId {
    TerminalId(panel(h)["terminals"][0]["id"].as_u64().unwrap())
}

/// Sends an `AgentEvent` with a prompt to the daemon of `h`, as an agent's hook does.
fn prompt_event(h: &Harness, terminal: TerminalId, prompt: &str) {
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
        status: None,
        session_id: Some("s1".into()),
        prompt: Some(prompt.into()),
        raw: None,
        activity: deck_core::agents::Activity::Keep,
        at_ms: 0,
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
fn an_agents_title_names_its_panel_in_the_sidebar() {
    let bin = fake(
        "claude",
        "printf '\\033]2;\u{2733} Fix login bug\\007'\necho agent-ready\nread line",
    );
    let mut h = attached(&bin);
    h.send_keys("claude\r");
    h.wait_for_text("agent-ready", TIMEOUT);
    // Before the agent is detected its title is the name as it is; then the spinner goes.
    wait_name(&h, "Fix login bug");
    wait_until("the sidebar row without the glyph", TIMEOUT, || {
        let row = &rows(&h)[3];
        row.contains("Fix login bug") && !row.contains('\u{2733}')
    });
    let row = rows(&h)[3].clone();
    assert!(row.contains("Fix login bug"), "{row}");
    assert!(!row.contains('\u{2733}'), "{row}");
    assert_eq!(panel_name(&h), "Fix login bug");
    h.capture("agent-title");

    // `ls --json` shows the program and where the name came from.
    let p = panel(&h);
    assert_eq!(p["name_source"], "auto");
    assert_eq!(p["terminals"][0]["program"], "claude");
}

#[test]
fn an_agent_that_only_has_a_prompt_gets_its_first_line() {
    let bin = fake("claude", "echo agent-ready\nread line");
    let mut h = attached(&bin);
    h.send_keys("claude\r");
    h.wait_for_text("agent-ready", TIMEOUT);
    wait_until("the agent", TIMEOUT, || {
        panel(&h)["terminals"][0]["agent"].is_object()
    });
    let terminal = terminal_id(&h);
    prompt_event(&h, terminal, "Review the API changes\nthen the docs");
    wait_name(&h, "Review the API changes");
    h.wait_for_text("Review the API chan", TIMEOUT);
    h.capture("agent-prompt");
}

#[test]
fn a_program_without_a_title_names_the_panel_until_it_exits() {
    let bin = fake("nvim", "echo nvim-ready\nread line");
    let mut h = attached(&bin);
    let shell = panel(&h)["terminals"][0]["program"]
        .as_str()
        .unwrap()
        .to_string();

    h.send_keys("nvim\r");
    h.wait_for_text("nvim-ready", TIMEOUT);
    wait_name(&h, "nvim");
    assert_eq!(panel(&h)["terminals"][0]["program"], "nvim");
    h.wait_for_active_row(" \u{270e}  nvim", TIMEOUT);
    h.capture("program-name");

    // It exits: the panel is named after the shell again.
    h.send_keys("\r");
    wait_name(&h, &shell);
    assert_eq!(panel(&h)["name_source"], "auto");
    h.capture("program-exited");
}

#[test]
fn a_manual_name_is_locked_until_rename_panel_auto() {
    let bin = fake("nvim", "echo nvim-ready\nread line");
    let mut h = attached(&bin);
    h.send_bytes(PREFIX);
    h.send_keys(",");
    h.wait_for_text("Rename panel", TIMEOUT);
    h.send_keys("\x15api\r");
    wait_name(&h, "api");
    assert_eq!(panel(&h)["name_source"], "manual");

    // A new title does not change it.
    h.send_keys("printf '\\033]2;newtitle\\007'; echo T1\r");
    h.wait_for_text("│T1", TIMEOUT);
    h.send_keys("nvim\r");
    h.wait_for_text("nvim-ready", TIMEOUT);
    wait_until("the program", TIMEOUT, || {
        panel(&h)["terminals"][0]["program"] == "nvim"
    });
    assert_eq!(panel_name(&h), "api");
    assert_eq!(panel(&h)["name_source"], "manual");
    h.send_keys("\r");

    // `--auto` brings the live name back.
    let out = h.run_deck(&["rename-panel", "--auto"]);
    assert!(out.status.success(), "{}", support::stderr(&out));
    wait_name(&h, "newtitle");
    assert_eq!(panel(&h)["name_source"], "auto");
}

#[test]
fn a_name_and_auto_together_are_a_usage_error() {
    let bin = fake("nvim", "read line");
    let h = attached(&bin);
    let out = h.run_deck(&["rename-panel", "--auto", "build"]);
    assert_eq!(out.status.code(), Some(2), "{}", support::stderr(&out));
    assert!(support::stderr(&out).contains("--auto"));
    let none = h.run_deck(&["rename-panel"]);
    assert_eq!(none.status.code(), Some(2), "{}", support::stderr(&none));
    assert_eq!(panel(&h)["name_source"], "auto");
}

/// Waits until sidebar row `n` satisfies `ok`.
fn wait_row(h: &Harness, n: usize, what: &str, ok: impl Fn(&str) -> bool) {
    wait_until(what, TIMEOUT, || rows(h).get(n).is_some_and(|r| ok(r)));
}

#[test]
fn an_agent_has_its_own_icon_and_a_shell_the_terminal_icon() {
    let bin = fake(
        "claude",
        "printf '\\033]2;\u{2733} Fix login bug\\007'\necho agent-ready\nread line",
    );
    let mut h = attached(&bin);
    wait_row(&h, 3, "the shell icon", |r| r.starts_with(" \u{276f} "));
    h.send_keys("claude\r");
    h.wait_for_text("agent-ready", TIMEOUT);
    wait_row(&h, 3, "the agent icon", |r| {
        r.contains("\u{273b}  Fix login bug")
    });
    // A second panel with a shell: its row starts with the terminal icon.
    h.send_bytes(PREFIX);
    h.send_keys("c");
    // The agent's row has its activity line under it, so the second panel is on row 5.
    wait_row(&h, 5, "the second panel", |r| r.starts_with(" \u{276f} "));
    let first = rows(&h)[3].clone();
    // Names start at the same column whatever the icon.
    assert_eq!(first.chars().nth(4), Some('F'), "{first}");
    assert_eq!(rows(&h)[5].chars().nth(4), Some('s'), "{}", rows(&h)[5]);
    assert!(
        first.chars().take(4).collect::<String>() == " \u{273b}  ",
        "{first}"
    );
    h.capture("icons");
}

/// The harness's terminal answers neither XTVERSION nor the graphics query, so the
/// environment names it: WezTerm bundles the Nerd Font symbols, so the icons are Nerd glyphs.
#[test]
fn a_terminal_that_bundles_nerd_symbols_gets_nerd_glyphs() {
    let bin = fake("unused", "true");
    let h = attached_in(&bin, &[("TERM_PROGRAM", "WezTerm")]);
    wait_row(&h, 3, "the Nerd terminal icon", |r| {
        r.starts_with(" \u{f120} ") && !r.contains('\u{276f}')
    });
    h.capture("nerd-tier");
}

#[test]
fn a_program_switches_the_icon_and_the_shell_brings_it_back() {
    let bin = fake("nvim", "echo nvim-ready\nread line");
    let mut h = attached(&bin);
    wait_row(&h, 3, "the shell icon", |r| r.starts_with(" \u{276f} "));
    h.send_keys("nvim\r");
    h.wait_for_text("nvim-ready", TIMEOUT);
    wait_row(&h, 3, "the editor icon", |r| {
        r.starts_with(" \u{270e}  nvim")
    });
    h.send_keys("\r");
    wait_row(&h, 3, "the shell icon again", |r| {
        r.starts_with(" \u{276f} ")
    });
}

/// Runs the fake `claude` in an attached client with `env` and returns the harness once its
/// sidebar row has the agent's title.
fn claude_panel(env: &[(&str, &str)]) -> Harness {
    let bin = fake(
        "claude",
        "printf '\\033]2;\u{2733} Fix login bug\\007'\necho agent-ready\nread line",
    );
    let mut h = attached_in(&bin, env);
    h.send_keys("claude\r");
    h.wait_for_text("agent-ready", TIMEOUT);
    // The shell's icon gives way to the agent's.
    wait_row(&h, 3, "the agent's icon", |r| {
        r.contains("Fix login bug") && !r.contains(['\u{276f}', '\u{f120}', '\u{2733}'])
    });
    h
}

#[test]
fn ghostty_shows_every_icon_as_an_image_and_tmux_keeps_the_nerd_glyphs() {
    let image = '\u{10EEEE}';
    let h = claude_panel(&[("TERM_PROGRAM", "ghostty"), ("COLORTERM", "truecolor")]);
    let screen = rows(&h);
    assert!(h.is_active_row(3), "{}", screen[3]);
    // The program slot (columns 1 and 2) and the space row's caret slot hold placeholders.
    assert_eq!(h.cell(3, 1).ch, image, "{}", screen[3]);
    assert_eq!(h.cell(3, 2).ch, image, "{}", screen[3]);
    assert_eq!(screen[2].matches(image).count(), 2, "{}", screen[2]);
    assert!(!screen[3].contains('\u{273b}'), "{}", screen[3]);
    // The name starts at column 4, as it does after a text icon.
    assert_eq!(h.cell(3, 4).ch, 'F', "{}", screen[3]);
    h.capture("all-icon-images");

    // Inside tmux the same terminal gets the Nerd Font glyphs.
    let h = claude_panel(&[
        ("TERM_PROGRAM", "ghostty"),
        ("COLORTERM", "truecolor"),
        ("TMUX", "/tmp/x,1,0"),
    ]);
    let screen = rows(&h);
    assert!(
        screen[3].contains('\u{ec82}') && !screen[3].contains(image),
        "{}",
        screen[3]
    );
    assert!(
        screen[2].contains('\u{eab4}') && !screen[2].contains(image),
        "{}",
        screen[2]
    );
}

/// A working agent's status slot holds the loader's image in Ghostty. The harness cannot see
/// the rotation; the frame maths is covered by the unit tests of `deck-tui`.
#[test]
fn a_working_agent_shows_the_loader_image_in_ghostty() {
    let image = '\u{10EEEE}';
    // The agent keeps printing, so it stays active and the loader keeps turning.
    let bin = fake(
        "claude",
        "printf '\\033]2;\u{2733} Fix login bug\\007'\necho agent-ready\nwhile :; do echo tick; sleep 0.5; done",
    );
    let mut h = attached_in(
        &bin,
        &[("TERM_PROGRAM", "ghostty"), ("COLORTERM", "truecolor")],
    );
    h.send_keys("claude\r");
    h.wait_for_text("agent-ready", TIMEOUT);
    wait_row(&h, 3, "the loader's image", |r| {
        r.contains("Fix login bug") && r.chars().nth(25) == Some(image)
    });
    assert_eq!(h.cell(3, 25).ch, image, "{}", rows(&h)[3]);
    assert_eq!(h.cell(3, 26).ch, image, "{}", rows(&h)[3]);
    // Only the loader changes the image id (the cell's colour) from one sample to the next.
    let mut seen = Vec::new();
    for _ in 0..10 {
        let fg = format!("{:?}", h.cell(3, 25).fg);
        if !seen.contains(&fg) {
            seen.push(fg);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(seen.len() >= 3, "the status image never turned: {seen:?}");
    h.capture("rotating-loader");
}
