//! `deck agent-event` and `deck hooks`, end to end. Every test points `HOME`,
//! `CLAUDE_CONFIG_DIR`, `CODEX_HOME` and `XDG_STATE_HOME` at temp directories: the real
//! agent config is never read or written.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use std::fs;
use std::io::Write;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;
use support::{Harness, TIMEOUT, stderr, stdout, wait_until};

/// A temp `HOME` with the variables the hooks code reads.
struct Sandbox {
    dir: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn path(&self, rest: &str) -> PathBuf {
        self.dir.path().join(rest)
    }

    fn deck_with(&self, bin: &Path, args: &[&str]) -> Command {
        let mut cmd = Command::new(bin);
        cmd.args(args)
            .env_clear()
            .env("HOME", self.path("home"))
            .env("XDG_STATE_HOME", self.path("state"))
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::null());
        cmd
    }

    fn deck(&self, args: &[&str]) -> Command {
        self.deck_with(Path::new(env!("CARGO_BIN_EXE_deck")), args)
    }

    fn run(&self, args: &[&str]) -> Output {
        self.deck(args).output().unwrap()
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(out.status.success(), "{args:?}: {}", stderr(&out));
        stdout(&out)
    }

    fn claude_file(&self) -> PathBuf {
        self.path("home/.claude/settings.json")
    }

    fn backups(&self) -> Vec<PathBuf> {
        let dir = self.path("state/deck/backups");
        fs::read_dir(dir)
            .map(|d| d.map(|e| e.unwrap().path()).collect())
            .unwrap_or_default()
    }
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

const EXISTING: &str = r#"{
  "theme": "dark",
  "hooks": {
    "Stop": [
      { "hooks": [{ "type": "command", "command": "afplay done.wav" }] }
    ],
    "PreToolUse": [
      { "matcher": "Bash", "hooks": [{ "type": "command", "command": "audit.sh" }] }
    ]
  },
  "permissions": { "allow": ["Bash(ls)"] }
}
"#;

#[test]
fn claude_install_keeps_hooks_backs_up_and_is_idempotent() {
    let sb = Sandbox::new();
    write(&sb.claude_file(), EXISTING);

    sb.ok(&["hooks", "install", "claude"]);
    let first = fs::read_to_string(sb.claude_file()).unwrap();
    let doc: Value = serde_json::from_str(&first).unwrap();
    let keys: Vec<&String> = doc.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["theme", "hooks", "permissions"]);
    let hooks = doc["hooks"].as_object().unwrap();
    let events: Vec<&String> = hooks.keys().collect();
    assert_eq!(events[..2], ["Stop", "PreToolUse"]);
    // The user's hooks are still there, first in their lists.
    assert_eq!(hooks["Stop"][0]["hooks"][0]["command"], "afplay done.wav");
    assert_eq!(hooks["PreToolUse"][0]["hooks"][0]["command"], "audit.sh");
    // Deck's entries: one per event of the table, async, no matcher.
    for event in [
        "SessionStart",
        "UserPromptSubmit",
        "PostToolUse",
        "PermissionRequest",
        "Notification",
        "Stop",
        "SessionEnd",
    ] {
        let list = hooks[event].as_array().unwrap();
        let deck: Vec<&Value> = list
            .iter()
            .filter(|g| {
                g["hooks"][0]["command"]
                    .as_str()
                    .is_some_and(|c| c.ends_with("agent-event --agent claude"))
            })
            .collect();
        assert_eq!(deck.len(), 1, "{event}");
        assert_eq!(deck[0]["hooks"][0]["async"], true, "{event}");
        assert!(deck[0].get("matcher").is_none(), "{event}");
    }
    // The backup is the file as it was.
    let backups = sb.backups();
    assert_eq!(backups.len(), 1, "{backups:?}");
    let name = backups[0]
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(name.starts_with("claude-settings.json-"), "{name}");
    assert_eq!(fs::read_to_string(&backups[0]).unwrap(), EXISTING);

    // A second run changes nothing, and writes no second backup.
    let again = sb.ok(&["hooks", "install", "claude"]);
    assert!(again.contains("already installed"), "{again}");
    assert_eq!(fs::read_to_string(sb.claude_file()).unwrap(), first);
    assert_eq!(sb.backups().len(), 1);
}

#[test]
fn a_symlinked_settings_file_is_written_in_place() {
    let sb = Sandbox::new();
    let real = sb.path("dotfiles/claude-settings.json");
    write(&real, EXISTING);
    fs::create_dir_all(sb.path("home/.claude")).unwrap();
    symlink(&real, sb.claude_file()).unwrap();

    sb.ok(&["hooks", "install", "claude"]);
    assert!(fs::symlink_metadata(sb.claude_file()).unwrap().is_symlink());
    assert_eq!(fs::read_link(sb.claude_file()).unwrap(), real);
    let doc = read_json(&real);
    assert!(doc["hooks"]["SessionEnd"].is_array());
    assert_eq!(doc["theme"], "dark");
    // The write kept the file's permissions.
    fs::set_permissions(&real, fs::Permissions::from_mode(0o600)).unwrap();
    sb.ok(&["hooks", "uninstall", "claude"]);
    assert_eq!(
        fs::metadata(&real).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn the_config_directories_of_the_agents_are_honoured() {
    let sb = Sandbox::new();
    let claude = sb.path("elsewhere/claude");
    let codex = sb.path("elsewhere/codex");
    let out = sb
        .deck(&["hooks", "install", "claude", "codex"])
        .env("CLAUDE_CONFIG_DIR", &claude)
        .env("CODEX_HOME", &codex)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(claude.join("settings.json").is_file());
    assert!(codex.join("hooks.json").is_file());
    assert!(!sb.path("home/.claude").exists());
    assert!(!sb.path("home/.codex").exists());
    // Without the variables the defaults under HOME are used.
    sb.ok(&["hooks", "install", "codex"]);
    assert!(sb.path("home/.codex/hooks.json").is_file());
}

#[test]
fn claude_uninstall_restores_the_hooks() {
    let sb = Sandbox::new();
    write(&sb.claude_file(), EXISTING);
    sb.ok(&["hooks", "install", "claude"]);
    sb.ok(&["hooks", "uninstall", "claude"]);
    let restored = read_json(&sb.claude_file());
    assert_eq!(restored, serde_json::from_str::<Value>(EXISTING).unwrap());
    assert!(
        sb.ok(&["hooks", "uninstall", "claude"])
            .contains("not installed")
    );
}

/// Codex rejects unknown fields: this checks a file against its schema by hand.
fn assert_codex_schema(doc: &Value) {
    let top = doc.as_object().unwrap();
    for key in top.keys() {
        assert!(["description", "hooks"].contains(&key.as_str()), "{key}");
    }
    let events = [
        "PreModelRequest",
        "PostModelRequest",
        "PreToolUse",
        "PermissionRequest",
        "PostToolUse",
        "PreCompact",
        "PostCompact",
        "SessionStart",
        "SessionEnd",
        "UserPromptSubmit",
        "SubagentStart",
        "SubagentStop",
        "Stop",
        "Interrupt",
    ];
    for (event, groups) in top["hooks"].as_object().unwrap() {
        assert!(events.contains(&event.as_str()), "{event}");
        for group in groups.as_array().unwrap() {
            for key in group.as_object().unwrap().keys() {
                assert!(["matcher", "hooks"].contains(&key.as_str()), "{key}");
            }
            for handler in group["hooks"].as_array().unwrap() {
                for key in handler.as_object().unwrap().keys() {
                    assert!(
                        ["type", "command", "timeout", "async"].contains(&key.as_str()),
                        "{key}"
                    );
                }
                assert_eq!(handler["type"], "command");
                assert!(handler["command"].is_string());
                assert!(handler["timeout"].is_u64());
            }
        }
    }
}

#[test]
fn codex_install_writes_the_hooks_json_wrapper_and_status_sees_config_toml() {
    let sb = Sandbox::new();
    assert!(sb.ok(&["hooks", "status"]).contains("codex: not installed"));
    sb.ok(&["hooks", "install", "codex"]);
    let path = sb.path("home/.codex/hooks.json");
    let doc = read_json(&path);
    assert_codex_schema(&doc);
    for event in [
        "SessionStart",
        "UserPromptSubmit",
        "PostToolUse",
        "PermissionRequest",
        "Stop",
        "Interrupt",
        "SessionEnd",
    ] {
        let group = &doc["hooks"][event][0];
        assert_eq!(group["matcher"], "", "{event}");
        assert_eq!(group["hooks"][0]["timeout"], 2, "{event}");
        assert!(
            group["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .ends_with("agent-event --agent codex")
        );
    }
    let status = sb.ok(&["hooks", "status"]);
    assert!(
        status.contains("codex: installed (trust them in /hooks"),
        "{status}"
    );

    // Trusted hooks and a duplicate in config.toml.
    write(
        &sb.path("home/.codex/config.toml"),
        "[hooks.state.\"x\"]\ntrusted_hash = \"abc\"\n\n[[hooks.Stop]]\ncommand = \"deck agent-event --agent codex\"\n",
    );
    let status = sb.ok(&["hooks", "status"]);
    assert!(status.contains("codex: installed"), "{status}");
    assert!(status.contains("also found in config.toml"), "{status}");
    assert!(status.contains("claude: not installed"), "{status}");
}

#[test]
fn print_writes_nothing_and_status_notices_a_removed_binary() {
    let sb = Sandbox::new();
    let printed = sb.ok(&["hooks", "install", "claude", "codex", "--print"]);
    assert!(printed.contains("\"SessionEnd\""), "{printed}");
    assert!(!sb.path("home").exists());
    assert!(!sb.path("state").exists());

    // Install with a copy of deck that is then deleted.
    let bin = sb.path("bin/deck");
    write(&bin, "");
    fs::copy(env!("CARGO_BIN_EXE_deck"), &bin).unwrap();
    let out = sb
        .deck_with(&bin, &["hooks", "install", "claude"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("stale"), "{}", stdout(&out));
    assert!(sb.ok(&["hooks", "status"]).contains("claude: installed"));
    fs::remove_file(&bin).unwrap();
    let status = sb.ok(&["hooks", "status"]);
    assert!(status.contains("claude: stale"), "{status}");
}

#[test]
fn backups_never_overwrite_each_other() {
    let sb = Sandbox::new();
    write(&sb.claude_file(), EXISTING);
    for _ in 0..2 {
        sb.ok(&["hooks", "install", "claude"]);
        sb.ok(&["hooks", "uninstall", "claude"]);
    }
    // Four writes of an existing file, all within about a second: four distinct backups.
    assert_eq!(sb.backups().len(), 4, "{:?}", sb.backups());
}

#[test]
fn a_dangling_symlink_is_refused_and_kept() {
    let sb = Sandbox::new();
    fs::create_dir_all(sb.path("home/.claude")).unwrap();
    let missing = sb.path("dotfiles/missing.json");
    symlink(&missing, sb.claude_file()).unwrap();
    let out = sb.run(&["hooks", "install", "claude"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("symlink"), "{}", stderr(&out));
    assert_eq!(fs::read_link(sb.claude_file()).unwrap(), missing);
}

#[test]
fn a_private_file_stays_private_and_agent_event_ignores_bad_arguments() {
    let sb = Sandbox::new();
    write(&sb.claude_file(), EXISTING);
    fs::set_permissions(sb.claude_file(), fs::Permissions::from_mode(0o600)).unwrap();
    sb.ok(&["hooks", "install", "claude"]);
    let mode = fs::metadata(sb.claude_file()).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    let out = sb.run(&["agent-event", "--agent", "nobody"]);
    assert!(out.status.success() && out.stdout.is_empty() && out.stderr.is_empty());
    let out = sb.run(&["agent-event"]);
    assert!(out.status.success() && out.stdout.is_empty() && out.stderr.is_empty());
}

#[test]
fn an_invalid_file_is_left_alone() {
    let sb = Sandbox::new();
    write(&sb.claude_file(), "{ not json");
    let out = sb.run(&["hooks", "install", "claude"]);
    assert!(!out.status.success());
    assert_eq!(fs::read_to_string(sb.claude_file()).unwrap(), "{ not json");
    assert!(sb.backups().is_empty());
}

#[test]
fn the_hooks_are_not_part_of_the_main_help() {
    let sb = Sandbox::new();
    let help = sb.ok(&["--help"]);
    assert!(!help.contains("agent-event"), "{help}");
    assert!(help.contains("hooks"), "{help}");
}

/// A fake agent on `PATH` so deck has a terminal with an agent to report on.
fn fake_claude() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .prefix("ag")
        .tempdir_in("/tmp")
        .unwrap();
    let script = dir.path().join("claude");
    fs::write(&script, "#!/bin/sh\necho agent-ready\nread line\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    dir
}

fn first_terminal(h: &Harness) -> Value {
    let doc: Value = serde_json::from_str(&stdout(&h.run_deck(&["ls", "--json"]))).unwrap();
    doc["sessions"][0]["spaces"][0]["panels"][0]["terminals"][0].clone()
}

fn agent_event(h: &Harness, terminal: Option<u64>, payload: &str) -> (Output, Duration) {
    let mut cmd = h.home().deck_command(&["agent-event", "--agent", "claude"]);
    if let Some(id) = terminal {
        cmd.env("DECK_TERMINAL_ID", id.to_string());
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let start = Instant::now();
    let mut child = cmd.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (out, start.elapsed())
}

#[test]
fn agent_event_sets_the_state_inside_deck_and_is_silent_outside() {
    let bin = fake_claude();
    let path = format!(
        "{}:{}",
        bin.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    // A fast detection tick, as in `agents.rs`: the production one (1 s) leaves a slow
    // machine little of the wait below.
    let mut h = Harness::builder((100, 24))
        .env("PATH", &path)
        .env("DECK_TEST_AGENT_POLL_MS", "100")
        .deck();
    h.wait_for_text("$", TIMEOUT);
    h.send_keys("claude\r");
    h.wait_for_text("agent-ready", TIMEOUT);
    wait_until("the agent is detected", TIMEOUT, || {
        first_terminal(&h)["agent"].is_object()
    });
    let id = first_terminal(&h)["id"].as_u64().unwrap();

    let payload = r#"{"hook_event_name":"Stop","session_id":"abc"}"#;
    let (out, _) = agent_event(&h, Some(id), payload);
    assert!(out.status.success());
    assert!(out.stdout.is_empty() && out.stderr.is_empty());
    wait_until("idle with the session", TIMEOUT, || {
        let agent = &first_terminal(&h)["agent"];
        agent["status"] == "idle" && agent["session_id"] == "abc" && agent["source"] == "hooks"
    });

    // Outside deck (no terminal id): nothing happens, quickly.
    let (out, took) = agent_event(
        &h,
        None,
        r#"{"hook_event_name":"UserPromptSubmit","session_id":"zzz"}"#,
    );
    assert!(out.status.success());
    assert!(out.stdout.is_empty() && out.stderr.is_empty());
    assert!(took < Duration::from_secs(2), "{took:?}");
    assert_eq!(first_terminal(&h)["agent"]["status"], "idle");
}

#[test]
fn agent_event_with_the_daemon_stopped_exits_quietly_and_quickly() {
    let home = support::Home::new();
    let mut cmd = home.deck_command(&["agent-event", "--agent", "claude"]);
    cmd.env("DECK_TERMINAL_ID", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let start = Instant::now();
    let mut child = cmd.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(br#"{"hook_event_name":"Stop"}"#)
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    assert!(out.stdout.is_empty() && out.stderr.is_empty());
    assert!(start.elapsed() < Duration::from_secs(2));
    assert!(!home.daemon_running());
}

#[test]
fn agent_event_gives_up_on_a_stdin_that_never_closes() {
    let home = support::Home::new();
    let mut cmd = home.deck_command(&["agent-event", "--agent", "claude"]);
    cmd.env("DECK_TERMINAL_ID", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let start = Instant::now();
    let mut child = cmd.spawn().unwrap();
    let stdin = child.stdin.take().unwrap();
    let out = child.wait_with_output().unwrap();
    drop(stdin);
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
    let took = start.elapsed();
    assert!(took >= Duration::from_millis(400), "{took:?}");
    assert!(took < Duration::from_secs(2), "{took:?}");
}

fn opencode_plugin(sb: &Sandbox) -> PathBuf {
    sb.path("home/.config/opencode/plugins/deck/tui.js")
}

#[test]
fn opencode_install_writes_the_plugin_once_and_uninstall_removes_it() {
    let sb = Sandbox::new();
    assert!(
        sb.ok(&["hooks", "status"])
            .contains("opencode: not installed")
    );
    let printed = sb.ok(&["hooks", "install", "opencode", "--print"]);
    assert!(printed.contains("session.status"), "{printed}");
    assert!(!sb.path("home").exists());

    let out = sb.ok(&["hooks", "install", "opencode"]);
    assert!(out.contains("installed in"), "{out}");
    let text = fs::read_to_string(opencode_plugin(&sb)).unwrap();
    for event in [
        "session.created",
        "session.status",
        "session.idle",
        "permission.asked",
        "permission.replied",
        "session.inbox.enqueued",
    ] {
        assert!(text.contains(&format!("\"{event}\"")), "{event}");
    }
    assert!(text.contains("export default {"), "{text}");
    assert!(text.contains("\"agent-event\", \"--agent\", \"opencode\", \"--event\""));
    // It forwards events as they are: no interpreting of what they say.
    for logic in ["properties", "role", "\"busy\"", "\"idle\""] {
        assert!(!text.contains(logic), "{logic}");
    }
    assert!(sb.ok(&["hooks", "status"]).contains("opencode: installed"));

    let again = sb.ok(&["hooks", "install", "opencode"]);
    assert!(again.contains("already installed"), "{again}");
    assert_eq!(fs::read_to_string(opencode_plugin(&sb)).unwrap(), text);
    assert!(sb.backups().is_empty());

    let removed = sb.ok(&["hooks", "uninstall", "opencode"]);
    assert!(removed.contains("removed"), "{removed}");
    assert!(!opencode_plugin(&sb).exists());
    assert!(
        sb.ok(&["hooks", "status"])
            .contains("opencode: not installed")
    );
    assert!(
        sb.ok(&["hooks", "uninstall", "opencode"])
            .contains("not installed")
    );
}

#[test]
fn opencode_plugin_follows_its_config_dir_and_spares_files_that_are_not_deck_s() {
    let sb = Sandbox::new();
    let dir = sb.path("oc-config");
    let run = |args: &[&str]| {
        sb.deck(args)
            .env("OPENCODE_CONFIG_DIR", &dir)
            .output()
            .unwrap()
    };
    let out = run(&["hooks", "install", "opencode"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(dir.join("plugins/deck/tui.js").is_file());
    assert!(!opencode_plugin(&sb).exists());

    // XDG_CONFIG_HOME moves the default directory.
    let out = sb
        .deck(&["hooks", "install", "opencode"])
        .env("XDG_CONFIG_HOME", sb.path("xdg"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(sb.path("xdg/opencode/plugins/deck/tui.js").is_file());

    // A tui.js of the user's is never overwritten or removed.
    write(
        &opencode_plugin(&sb),
        "export const Mine = async () => ({})\n",
    );
    for args in [
        &["hooks", "install", "opencode"],
        &["hooks", "uninstall", "opencode"],
    ] {
        let out = sb.run(args);
        assert!(!out.status.success(), "{args:?}");
    }
    assert_eq!(
        fs::read_to_string(opencode_plugin(&sb)).unwrap(),
        "export const Mine = async () => ({})\n"
    );
    assert!(
        sb.ok(&["hooks", "status"])
            .contains("opencode: not installed")
    );
}

#[test]
fn an_old_v1_deck_js_is_stale_and_replaced_by_an_install_and_removed_by_an_uninstall() {
    let sb = Sandbox::new();
    let old = sb.path("home/.config/opencode/plugins/deck.js");
    let v1 = "// Written by \"deck hooks install opencode\"; do not edit.\nconst DECK = \"deck\";\nexport const DeckPlugin = async ({ $ }) => ({});\n";
    write(&old, v1);
    assert!(sb.ok(&["hooks", "status"]).contains("opencode: stale"));
    let out = sb.ok(&["hooks", "install", "opencode"]);
    assert!(out.contains("removed the old plugin"), "{out}");
    assert!(!old.exists());
    assert!(opencode_plugin(&sb).is_file());
    assert!(sb.ok(&["hooks", "status"]).contains("opencode: installed"));

    write(&old, v1);
    let out = sb.ok(&["hooks", "uninstall", "opencode"]);
    assert!(out.contains("removed"), "{out}");
    assert!(!old.exists() && !opencode_plugin(&sb).exists());

    // A deck.js of the user's is left alone.
    write(&old, "export const Mine = async () => ({})\n");
    sb.ok(&["hooks", "install", "opencode"]);
    assert!(old.exists());
}

#[test]
fn opencode_status_notices_a_removed_binary_and_a_changed_plugin_is_backed_up() {
    let sb = Sandbox::new();
    let bin = sb.path("bin/deck");
    write(&bin, "");
    fs::copy(env!("CARGO_BIN_EXE_deck"), &bin).unwrap();
    let out = sb
        .deck_with(&bin, &["hooks", "install", "opencode"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("stale"));
    assert!(sb.ok(&["hooks", "status"]).contains("opencode: installed"));
    fs::remove_file(&bin).unwrap();
    assert!(sb.ok(&["hooks", "status"]).contains("opencode: stale"));

    // Installing from the real binary replaces the plugin and keeps the old one.
    sb.ok(&["hooks", "install", "opencode"]);
    assert_eq!(sb.backups().len(), 1, "{:?}", sb.backups());
    assert!(sb.ok(&["hooks", "status"]).contains("opencode: installed"));

    // A plugin that lost an event is stale.
    let text = fs::read_to_string(opencode_plugin(&sb)).unwrap();
    fs::write(
        opencode_plugin(&sb),
        text.replace("\"permission.asked\"", "\"x\""),
    )
    .unwrap();
    assert!(sb.ok(&["hooks", "status"]).contains("opencode: stale"));
}

#[test]
fn an_opencode_event_piped_into_agent_event_sets_the_state() {
    let dir = tempfile::Builder::new()
        .prefix("ag")
        .tempdir_in("/tmp")
        .unwrap();
    let script = dir.path().join("opencode");
    fs::write(&script, "#!/bin/sh\necho agent-ready\nread line\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!(
        "{}:{}",
        dir.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    // A fast detection tick, as in `agents.rs`: the production one (1 s) leaves a slow
    // machine little of the wait below.
    let mut h = Harness::builder((100, 24))
        .env("PATH", &path)
        .env("DECK_TEST_AGENT_POLL_MS", "100")
        .deck();
    h.wait_for_text("$", TIMEOUT);
    h.send_keys("opencode\r");
    h.wait_for_text("agent-ready", TIMEOUT);
    wait_until("the agent is detected", TIMEOUT, || {
        first_terminal(&h)["agent"].is_object()
    });
    let id = first_terminal(&h)["id"].as_u64().unwrap();

    let send = |event: &str, payload: &str| {
        let mut cmd =
            h.home()
                .deck_command(&["agent-event", "--agent", "opencode", "--event", event]);
        cmd.env("DECK_TERMINAL_ID", id.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success() && out.stdout.is_empty() && out.stderr.is_empty());
    };
    send(
        "session.status",
        r#"{"type":"session.status","properties":{"sessionID":"root","status":{"type":"busy"}}}"#,
    );
    wait_until("working", TIMEOUT, || {
        let agent = &first_terminal(&h)["agent"];
        agent["status"] == "working" && agent["session_id"] == "root"
    });
    // A subagent finishing changes nothing; the root finishing does.
    send(
        "session.created",
        r#"{"type":"session.created","properties":{"info":{"id":"sub","parentID":"root"}}}"#,
    );
    send(
        "session.status",
        r#"{"type":"session.status","properties":{"sessionID":"sub","status":{"type":"idle"}}}"#,
    );
    send(
        "permission.asked",
        r#"{"type":"permission.asked","properties":{"sessionID":"root"}}"#,
    );
    wait_until("waiting", TIMEOUT, || {
        first_terminal(&h)["agent"]["status"] == "waiting"
    });
    send(
        "session.idle",
        r#"{"type":"session.idle","properties":{"sessionID":"root"}}"#,
    );
    wait_until("idle", TIMEOUT, || {
        first_terminal(&h)["agent"]["status"] == "idle"
    });
}

#[test]
fn opencode_uninstall_removes_a_symlink_but_not_its_target_and_a_dangling_one_is_not_installed() {
    let sb = Sandbox::new();
    let target = sb.path("dotfiles/deck.js");
    // Install, then move the file away and link to it.
    sb.ok(&["hooks", "install", "opencode"]);
    write(&target, "");
    fs::rename(opencode_plugin(&sb), &target).unwrap();
    symlink(&target, opencode_plugin(&sb)).unwrap();
    assert!(sb.ok(&["hooks", "status"]).contains("opencode: installed"));

    let out = sb.ok(&["hooks", "uninstall", "opencode"]);
    assert!(out.contains("removed"), "{out}");
    assert!(target.is_file(), "the target stays");
    assert!(
        fs::symlink_metadata(opencode_plugin(&sb)).is_err(),
        "the link goes"
    );

    // A dangling link: status does not fail and says not installed; uninstall leaves it.
    fs::remove_file(&target).unwrap();
    // Uninstall removed the empty `plugins/deck` directory too.
    fs::create_dir_all(opencode_plugin(&sb).parent().unwrap()).unwrap();
    symlink(&target, opencode_plugin(&sb)).unwrap();
    let status = sb.ok(&["hooks", "status"]);
    assert!(status.contains("opencode: not installed"), "{status}");
    let out = sb.ok(&["hooks", "uninstall", "opencode"]);
    assert!(out.contains("not installed"), "{out}");
    assert!(fs::symlink_metadata(opencode_plugin(&sb)).is_ok());
}
