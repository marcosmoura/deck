//! Privacy review (DECK-26, sub-spec 03): what deck leaves on disk. A daemon is given random
//! values through every way user data comes in (the environment, the keyboard, a title, an
//! agent's prompt, a copy), and every file under its home is read after a live upgrade, a
//! failed one, a stop and a restart. No test here calls `Harness::capture`: it would write
//! the screen under `target/`.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use std::collections::hash_map::RandomState;
use std::fs;
use std::hash::BuildHasher;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use support::{Harness, Home, TIMEOUT, stderr, stdout, wait_until};

const SIZE: (u16, u16) = (160, 40);
const LIMIT: Duration = Duration::from_secs(20);
/// An agent session id deck accepts; it is saved by design and is no secret here.
const SESSION_ID: &str = "0f9e8d7c-6b5a-4f3e-8d2c-1b0a9f8e7d6c";

/// A value no other run or file has: `tag` and 16 random hex digits.
fn random(tag: &str) -> String {
    format!("{tag}{:016x}", RandomState::new().hash_one(tag))
}

/// How `value` reads inside base64 text wherever it starts: one form per alignment, without
/// the characters that also depend on its neighbours.
fn base64_forms(value: &str) -> Vec<String> {
    (0..3)
        .map(|pad| {
            let mut bytes = vec![0u8; pad];
            bytes.extend_from_slice(value.as_bytes());
            let text = deck_core::base64::encode(&bytes);
            // The first group mixes in what comes before the value, a last partial group
            // what comes after it.
            let from = if pad == 0 { 0 } else { 4 };
            let to = bytes.len() / 3 * 4;
            text[from..to].to_string()
        })
        .collect()
}

/// The random values of one test.
struct Secrets {
    /// `DECK_PRIVACY_SECRET` of the client that creates the session: `DECK_*` variables
    /// never reach a terminal.
    deck_var: String,
    /// `PRIVACY_TOKEN`, the path a real token takes: the session's environment and every
    /// terminal.
    token: String,
    typed: String,
    copied: String,
    /// The first words of the title and of the prompt: an auto panel name is their first
    /// 32 cells, the only place a file may hold them.
    title_head: String,
    title_tail: String,
    prompt_head: String,
    prompt_tail: String,
    prompt_second_line: String,
}

impl Secrets {
    fn new() -> Self {
        Self {
            deck_var: random("deckvar"),
            token: random("token"),
            typed: random("typed"),
            copied: random("copied"),
            title_head: random("title"),
            title_tail: random("titletail"),
            prompt_head: random("prompt"),
            prompt_tail: random("prompttail"),
            prompt_second_line: random("promptsecond"),
        }
    }

    /// Longer than 32 cells: the tail starts at cell 39.
    fn title(&self) -> String {
        format!("{} padding-padding {}", self.title_head, self.title_tail)
    }

    /// Longer than 32 cells, with a second line the daemon never keeps.
    fn prompt(&self) -> String {
        format!(
            "{} please look at this {}\nand then {}",
            self.prompt_head, self.prompt_tail, self.prompt_second_line
        )
    }

    /// What no file may hold.
    fn never(&self) -> [(&'static str, &str); 7] {
        [
            ("DECK_PRIVACY_SECRET", &self.deck_var),
            ("PRIVACY_TOKEN", &self.token),
            ("the typed word", &self.typed),
            ("the copied text", &self.copied),
            ("the title beyond cell 32", &self.title_tail),
            ("the prompt beyond cell 32", &self.prompt_tail),
            ("the prompt's second line", &self.prompt_second_line),
        ]
    }

    /// What a file may hold only as a panel name in `state.json`.
    fn names_only(&self) -> [(&'static str, &str); 2] {
        [
            ("the title's first words", &self.title_head),
            ("the prompt's first words", &self.prompt_head),
        ]
    }
}

fn ok(home: &Home, args: &[&str]) -> String {
    let out = home.run_deck(args);
    assert!(
        out.status.success(),
        "deck {args:?} failed: {}{}",
        stderr(&out),
        stdout(&out)
    );
    stdout(&out).trim_end().to_string()
}

fn state_file(home: &Home) -> PathBuf {
    home.path().join("state").join("state.json")
}

fn log(home: &Home) -> String {
    fs::read_to_string(home.log_file()).unwrap_or_default()
}

fn first_panel(home: &Home) -> Value {
    let doc: Value = serde_json::from_str(&ok(home, &["ls", "--json"])).unwrap();
    doc["sessions"][0]["spaces"][0]["panels"][0].clone()
}

fn stop(home: &Home) {
    ok(home, &["kill-server"]);
    wait_until("the daemon to stop", LIMIT, || !home.daemon_running());
}

/// Every regular file under `dir`.
fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        let Ok(meta) = path.symlink_metadata() else {
            continue;
        };
        if meta.is_dir() {
            files(&path, out);
        } else if meta.is_file() {
            out.push(path);
        }
    }
}

fn holds(bytes: &[u8], needle: &str) -> bool {
    bytes.windows(needle.len()).any(|w| w == needle.as_bytes())
}

/// A saved state with every panel name emptied; text that is not a state as it is.
fn without_panel_names(bytes: &[u8]) -> Vec<u8> {
    let Ok(mut doc) = serde_json::from_slice::<Value>(bytes) else {
        return bytes.to_vec();
    };
    let list = |v: &mut Value, key: &str| -> Vec<Value> {
        v.get_mut(key)
            .and_then(Value::as_array_mut)
            .map(std::mem::take)
            .unwrap_or_default()
    };
    let mut sessions = list(&mut doc, "sessions");
    for session in &mut sessions {
        let mut projects = list(session, "projects");
        for project in &mut projects {
            let mut panels = list(project, "panels");
            for panel in &mut panels {
                panel["name"] = Value::String(String::new());
            }
            project["panels"] = Value::Array(panels);
        }
        session["projects"] = Value::Array(projects);
    }
    doc["sessions"] = Value::Array(sessions);
    serde_json::to_vec(&doc).unwrap()
}

/// The files under the home that hold one of the values, as it is or in base64. The one
/// exception (decided on 24 Nov 2026): the first 32 cells of a title or prompt as a panel
/// name in `state.json`.
fn leaks(home: &Home, secrets: &Secrets, all: &[PathBuf]) -> Vec<String> {
    let state = state_file(home);
    // A save on its way to `state.json`: the same content, with the same exception.
    let saving = state.with_extension("json.tmp");
    let mut found = Vec::new();
    for path in all {
        // A file that went away meanwhile was a temp file of a save.
        let Ok(bytes) = fs::read(path) else { continue };
        let mut check = |text: &[u8], what: &str, value: &str| {
            if holds(text, value) {
                found.push(format!("{what} in {}", path.display()));
            }
            for form in base64_forms(value) {
                if holds(text, &form) {
                    found.push(format!("{what}, in base64, in {}", path.display()));
                }
            }
        };
        for (what, value) in secrets.never() {
            check(&bytes, what, value);
        }
        let unnamed = if *path == state || *path == saving {
            without_panel_names(&bytes)
        } else {
            bytes
        };
        for (what, value) in secrets.names_only() {
            check(&unnamed, what, value);
        }
    }
    found
}

/// Fails when [`leaks`] finds anything, or when an upgrade left its handoff file.
fn assert_nothing_private_on_disk(home: &Home, secrets: &Secrets, when: &str) {
    let mut all = Vec::new();
    files(home.path(), &mut all);
    let found = leaks(home, secrets, &all);
    assert!(found.is_empty(), "{when}: {found:#?}");
    assert!(
        all.iter().any(|p| *p == home.log_file()) && all.iter().any(|p| *p == state_file(home)),
        "{when}: the log and the state were read: {all:?}"
    );
    // The handoff file is unlinked before its first byte: no upgrade leaves one behind.
    let left: Vec<&PathBuf> = all
        .iter()
        .filter(|p| {
            let name = p.file_name().unwrap().to_string_lossy();
            name.starts_with("deck.handoff.") && name.ends_with(".tmp")
        })
        .collect();
    assert!(left.is_empty(), "{when}: a handoff file was left: {left:?}");
}

/// The name of the first panel in the saved state.
fn saved_panel_name(home: &Home) -> String {
    let doc: Value = serde_json::from_slice(&fs::read(state_file(home)).unwrap()).unwrap();
    doc["sessions"][0]["projects"][0]["panels"][0]["name"]
        .as_str()
        .unwrap()
        .to_string()
}

/// A fake agent in the home: deck sees a `claude` in the foreground, so the state its hook
/// reports stays. Each line it reads makes it clear its title.
fn fake_claude(home: &Home) {
    let script = home.path().join("claude");
    fs::write(
        &script,
        "#!/bin/sh\necho agent-ready\nwhile read line; do printf '\\033]2;\\007'; done\n",
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
}

fn prompt_event(home: &Home, terminal: u64, prompt: &str) {
    let payload = serde_json::json!({
        "hook_event_name": "UserPromptSubmit",
        "session_id": SESSION_ID,
        "prompt": prompt,
    })
    .to_string();
    let mut cmd = home.deck_command(&["agent-event", "--agent", "claude"]);
    cmd.env("DECK_TERMINAL_ID", terminal.to_string())
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
    assert!(child.wait_with_output().unwrap().status.success());
}

/// A deck whose creating client has the two variables, with every value given to it: typed,
/// shown, copied, set as a title and reported as an agent's prompt.
fn deck_with_secrets(secrets: &Secrets, vars: &[(&str, &str)]) -> Harness {
    let mut builder = Harness::builder(SIZE)
        // A restart must not start a real `claude` from the developer's PATH.
        .config("resume_agents", "false")
        .env("DECK_PRIVACY_SECRET", &secrets.deck_var)
        .env("PRIVACY_TOKEN", &secrets.token);
    for (key, value) in vars {
        builder = builder.env(key, value);
    }
    let mut h = builder.deck();
    h.wait_for_text("$", TIMEOUT);

    // Typed, and on the screen.
    h.send_keys(&format!("echo {}\r", secrets.typed));
    h.wait_for_row_start(&secrets.typed, TIMEOUT);
    // The token reaches the terminal with the session's environment.
    h.send_keys("echo $PRIVACY_TOKEN\r");
    h.wait_for_row_start(&secrets.token, TIMEOUT);
    // A `DECK_*` variable does not.
    h.send_keys("echo deck-var=${DECK_PRIVACY_SECRET:-unset}\r");
    h.wait_for_row_start("deck-var=unset", TIMEOUT);

    // A copy: the client passes it to the outer terminal.
    let copied = deck_core::base64::encode(secrets.copied.as_bytes());
    h.send_keys(&format!("printf '\\033]52;c;{copied}\\007'\r"));
    // The whole sequence, as the client writes it: the shell's echo of the command line has
    // the payload too, but no escape before it.
    let osc52 = format!("\x1b]52;c;{copied}\x07");
    wait_until("the copy to reach the outer terminal", LIMIT, || {
        holds(&h.raw_output(), &osc52)
    });

    // A title: the panel is named after its first 32 cells.
    let title = secrets.title();
    h.send_keys(&format!("printf '\\033]2;{title}\\007'\r"));
    wait_until("the title", LIMIT, || {
        first_panel(h.home())["terminals"][0]["title"] == title.as_str()
    });

    // An agent and its first prompt.
    fake_claude(h.home());
    h.send_keys("./claude\r");
    h.wait_for_text("agent-ready", TIMEOUT);
    wait_until("the agent to be seen", LIMIT, || {
        first_panel(h.home())["terminals"][0]["agent"].is_object()
    });
    let terminal = first_panel(h.home())["terminals"][0]["id"]
        .as_u64()
        .unwrap();
    prompt_event(h.home(), terminal, &secrets.prompt());
    wait_until("the agent's session", LIMIT, || {
        first_panel(h.home())["terminals"][0]["agent"]["session_id"] == SESSION_ID
    });

    // The title names the panel, cut at 32 cells.
    wait_until("the panel to be named after the title", LIMIT, || {
        let name = first_panel(h.home())["name"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        name.starts_with(&secrets.title_head) && name.chars().count() == 32
    });
    h
}

/// Waits until the saved state names the first panel after `head`, cut at 32 cells.
fn wait_saved_name(home: &Home, head: &str) {
    wait_until("the panel name to be saved", LIMIT, || {
        fs::read(state_file(home))
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .is_some()
            && saved_panel_name(home).starts_with(head)
    });
    assert_eq!(saved_panel_name(home).chars().count(), 32);
}

/// The check itself: it finds a value as it is and inside base64 at any alignment, and a
/// title's first words anywhere but in a panel name of `state.json`.
#[test]
fn the_check_finds_what_it_looks_for() {
    let home = Home::new();
    let secrets = Secrets::new();
    let (state, run) = (home.path().join("state"), home.path().join("run"));
    fs::create_dir_all(&state).unwrap();
    fs::create_dir_all(&run).unwrap();
    let saved = |title: &str| {
        serde_json::json!({ "sessions": [{ "projects": [{ "name": "p", "panels": [
            { "name": secrets.title_head, "terminals": [{ "title": title }] }
        ]}]}]})
        .to_string()
    };
    let found = |home: &Home| {
        let mut all = Vec::new();
        files(home.path(), &mut all);
        leaks(home, &secrets, &all)
    };

    // A panel name in the saved state is the one exception.
    fs::write(state_file(&home), saved("")).unwrap();
    assert_eq!(found(&home), Vec::<String>::new());
    // The same words in another field of the state, or in another file, are not.
    fs::write(state_file(&home), saved(&secrets.title_head)).unwrap();
    assert_eq!(found(&home).len(), 1, "{:?}", found(&home));
    fs::write(state_file(&home), saved("")).unwrap();
    fs::write(state.join("state.json.bad-1"), saved("")).unwrap();
    assert_eq!(found(&home).len(), 1, "{:?}", found(&home));
    fs::remove_file(state.join("state.json.bad-1")).unwrap();
    // The temp file of a save is the state on its way: the same exception, and no other.
    fs::write(state.join("state.json.tmp"), saved("")).unwrap();
    assert_eq!(found(&home), Vec::<String>::new());
    fs::write(state.join("state.json.tmp"), saved(&secrets.title_head)).unwrap();
    assert_eq!(found(&home).len(), 1, "{:?}", found(&home));
    fs::remove_file(state.join("state.json.tmp")).unwrap();

    fs::write(home.log_file(), format!("x {} y", secrets.token)).unwrap();
    assert_eq!(found(&home).len(), 1, "{:?}", found(&home));
    fs::remove_file(home.log_file()).unwrap();
    for before in ["", "a", "ab", "abc"] {
        for after in ["", "z", "zy"] {
            let text = format!("{before}{}{after}", secrets.typed);
            let encoded = deck_core::base64::encode(text.as_bytes());
            fs::write(run.join("blob"), format!("{{\"snapshot\":\"{encoded}\"}}")).unwrap();
            assert_eq!(found(&home).len(), 1, "{before:?} {after:?}");
        }
    }
}

#[test]
fn nothing_private_is_on_disk_after_an_upgrade_a_stop_and_a_restart() {
    let secrets = Secrets::new();
    let h = deck_with_secrets(&secrets, &[]);
    wait_saved_name(h.home(), &secrets.title_head);
    assert_nothing_private_on_disk(h.home(), &secrets, "running");

    // A live upgrade to the same binary: the handoff carries the screens, the session's
    // environment and the prompt in a file that has no name.
    let pid = h.home().daemon_pid();
    let out = h.run_deck(&["upgrade"]);
    assert!(out.status.success(), "{}{}", stderr(&out), stdout(&out));
    assert!(
        stdout(&out).contains("restarted in place"),
        "{}",
        stdout(&out)
    );
    assert_eq!(h.home().daemon_pid(), pid);
    wait_until("the client to reattach", LIMIT, || {
        stdout(&h.run_deck(&["ls"])).contains("(attached)")
    });
    assert_nothing_private_on_disk(h.home(), &secrets, "after a live upgrade");

    // Without a title the panel is named after the prompt, which came through the handoff.
    let terminal = first_panel(h.home())["terminals"][0]["id"].to_string();
    ok(h.home(), &["send-keys", "--terminal", &terminal, "Enter"]);
    wait_until("the panel to be named after the prompt", LIMIT, || {
        first_panel(h.home())["name"]
            .as_str()
            .is_some_and(|n| n.starts_with(&secrets.prompt_head))
    });

    stop(h.home());
    assert!(saved_panel_name(h.home()).starts_with(&secrets.prompt_head));
    assert_eq!(saved_panel_name(h.home()).chars().count(), 32);
    assert_nothing_private_on_disk(h.home(), &secrets, "after kill-server");

    // A restart: the daemon restores the session from the state file.
    let probe = h.run_deck(&["_probe"]);
    assert!(probe.status.success(), "{}", stderr(&probe));
    wait_until("the restored terminal", LIMIT, || {
        first_panel(h.home())["terminals"][0]["pid"].is_u64()
    });
    assert_nothing_private_on_disk(h.home(), &secrets, "after a restart");
    stop(h.home());
    assert_nothing_private_on_disk(h.home(), &secrets, "after the second kill-server");
}

#[test]
fn nothing_private_is_on_disk_after_a_failed_upgrade() {
    let secrets = Secrets::new();
    let h = deck_with_secrets(&secrets, &[("DECK_TEST_EXEC_FAIL", "1")]);
    wait_saved_name(h.home(), &secrets.title_head);

    // The exec fails after the handoff file was written: the old daemon rolls back.
    let pid = h.home().daemon_pid();
    let out = h.run_deck(&["upgrade"]);
    assert!(!out.status.success(), "{}", stdout(&out));
    let result = fs::read_to_string(h.home().path().join("run").join("deck.upgrade.result"))
        .unwrap_or_default();
    assert!(result.starts_with("failed: "), "{result}");
    assert!(log(h.home()).contains("upgrade failed: "));
    assert_eq!(h.home().daemon_pid(), pid);
    assert!(
        first_panel(h.home())["name"]
            .as_str()
            .is_some_and(|n| n.starts_with(&secrets.title_head)),
        "the session goes on"
    );
    assert_nothing_private_on_disk(h.home(), &secrets, "after a failed upgrade");

    stop(h.home());
    assert_nothing_private_on_disk(h.home(), &secrets, "after kill-server");
}

/// P-2: a panel name can be the first words of a prompt, so the log names a terminal that
/// could not be restored by its ids; the notice the user sees keeps the names.
#[test]
fn a_failed_restore_logs_ids_not_names() {
    let home = Home::new();
    let (space, panel) = (random("space"), random("panel"));
    ok(&home, &["new", "main", "-d"]);
    let program = home.path().join("gone-prog.sh");
    fs::write(&program, "#!/bin/sh\nsleep 600\n").unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    let dir = home.path().to_string_lossy().into_owned();
    ok(&home, &["new-space", &space, "--dir", &dir]);
    let panel_id = ok(
        &home,
        &[
            "new-panel",
            "--space",
            &space,
            "--name",
            &panel,
            "--",
            &program.to_string_lossy(),
        ],
    );
    stop(&home);
    let saved = fs::read_to_string(state_file(&home)).unwrap();
    assert!(saved.contains(&space) && saved.contains(&panel), "{saved}");

    // The program is gone when the daemon comes back; the client that starts it is told,
    // with the names.
    fs::remove_file(&program).unwrap();
    let h = Harness::builder(SIZE)
        .env("DECK_HOME", &dir)
        .spawn(&[env!("CARGO_BIN_EXE_deck")]);
    h.wait_for_text(&format!("{space}/{panel}"), TIMEOUT);

    wait_until("the failed restore in the log", LIMIT, || {
        log(&home).contains("could not restore terminal")
    });
    let log = log(&home);
    let line = log
        .lines()
        .find(|l| l.contains("could not restore terminal"))
        .unwrap();
    assert!(line.contains(&format!("panel {panel_id})")), "{line}");
    assert!(line.contains("gone-prog.sh"), "{line}");
    assert!(!log.contains(&space) && !log.contains(&panel), "{log}");
    drop(h);
}

/// P-4, accepted and documented: `--env` values and a program's arguments are in the state
/// file while their terminal lives (a restart needs them), and gone once it is closed.
#[test]
fn env_values_and_arguments_are_saved_only_while_their_terminal_lives() {
    let home = Home::new();
    ok(&home, &["new", "main", "-d"]);
    let (value, argument) = (random("envvalue"), random("argument"));
    let id = ok(
        &home,
        &[
            "new-terminal",
            "--env",
            &format!("PRIVACY_PIN={value}"),
            "--",
            "sh",
            "-c",
            &format!("sleep 600 # {argument}"),
        ],
    );
    let saved = || fs::read_to_string(state_file(&home)).unwrap_or_default();
    wait_until("the terminal to be saved", LIMIT, || {
        saved().contains(&value) && saved().contains(&argument)
    });
    assert!(!log(&home).contains(&value) && !log(&home).contains(&argument));

    ok(&home, &["close", &id]);
    wait_until("the closed terminal to leave the state", LIMIT, || {
        let saved = saved();
        !saved.is_empty() && !saved.contains(&value) && !saved.contains(&argument)
    });
    stop(&home);
    let mut all = Vec::new();
    files(home.path(), &mut all);
    for path in all {
        let bytes = fs::read(&path).unwrap();
        assert!(
            !holds(&bytes, &value) && !holds(&bytes, &argument),
            "{}",
            path.display()
        );
    }
}

/// P-5 (a): a state file moved aside holds what the state held.
#[test]
fn forget_removes_the_files_moved_aside() {
    let home = Home::new();
    let state = home.path().join("state");
    let aside = [
        state.join("state.json.bad-1764000000"),
        state.join("state.json.bad-1764000001"),
    ];
    let seed = || {
        fs::create_dir_all(&state).unwrap();
        for file in &aside {
            fs::write(file, b"{ \"old\": \"state\"").unwrap();
        }
    };

    // Without a daemon.
    seed();
    fs::write(state.join("notes.txt"), b"not deck's").unwrap();
    let out = home.run_deck(&["kill-server", "--forget"]);
    assert_eq!(stdout(&out), "state cleared\n");
    assert!(aside.iter().all(|f| !f.exists()));
    assert!(state.join("notes.txt").exists(), "only deck's state goes");

    // With one: it stops, and the files go with the state.
    ok(&home, &["new", "main", "-d"]);
    seed();
    let out = home.run_deck(&["kill-server", "--forget"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(!home.daemon_running());
    assert!(!state_file(&home).exists());
    assert!(aside.iter().all(|f| !f.exists()));
    assert!(home.log_file().exists(), "the log stays");
}
