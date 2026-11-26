//! Agents reopening their conversations after `deck kill-server`, end to end: the real
//! `deck` in a PTY with fake `#!/bin/sh` agents on `PATH`. No real agent runs.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use deck_core::action::{Action, PanelLaunch};
use deck_core::agents::AgentKind;
use deck_core::protocol::{ClientMsg, FrameReader, PROTOCOL_VERSION, Role, ServerMsg, encode};
use serde_json::Value;
use support::{Harness, Home, TIMEOUT, stderr, wait_until};

const SIZE: (u16, u16) = (100, 24);

/// Fake agents and shells in one directory, which goes first on `PATH`.
struct Fakes {
    dir: tempfile::TempDir,
}

impl Fakes {
    fn new() -> Self {
        Self {
            dir: tempfile::Builder::new()
                .prefix("rs")
                .tempdir_in("/tmp")
                .unwrap(),
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn rec(&self, name: &str) -> PathBuf {
        self.path(&format!("{name}.rec"))
    }

    fn write_script(&self, name: &str, body: &str) {
        let path = self.path(name);
        fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A fake `claude`. It records `<directory>|<arguments>` for each start. Started with no
    /// arguments it reports session `sid` through its hook (none when `sid` is empty) and
    /// waits; with `trap_hup` it sends `SessionEnd` when hung up, like the real agent does
    /// at the shutdown. Started with arguments (resumed) it waits for a line on stdin.
    fn claude(&self, sid: &str, trap_hup: bool) {
        let rec = self.rec("claude").display().to_string();
        let deck = env!("CARGO_BIN_EXE_deck");
        let hook = |event: &str, payload: &str| {
            format!(
                "printf '%s' '{payload}' | '{deck}' agent-event --agent claude --event {event}\n"
            )
        };
        let start = if sid.is_empty() {
            String::new()
        } else {
            // The marker says the hook has returned: the daemon has answered the event.
            format!(
                "{}: > '{}'\n",
                hook(
                    "SessionStart",
                    &format!(r#"{{"session_id":"{sid}","hook_event_name":"SessionStart"}}"#),
                ),
                self.path("claude.hooked").display()
            )
        };
        let end = hook("SessionEnd", r#"{"hook_event_name":"SessionEnd"}"#);
        let trap = if trap_hup {
            format!("trap 'kill $! 2>/dev/null; {end} exit 0' HUP TERM\n")
        } else {
            "trap 'kill $! 2>/dev/null; exit 0' HUP TERM\n".to_string()
        };
        self.write_script(
            "claude",
            &format!(
                "printf '%s|%s\\n' \"$(pwd -P)\" \"$*\" >> '{rec}'\nif [ -z \"$1\" ]; then\n{start}{trap}sleep 300 &\nwait\nelse\nread line\nfi\n"
            ),
        );
    }

    /// A fake shell that records its arguments and becomes `/bin/sh`.
    fn shell(&self) {
        let rec = self.rec("shell").display().to_string();
        self.write_script(
            "shell",
            &format!("echo \"$*\" >> '{rec}'\nexec /bin/sh \"$@\"\n"),
        );
    }

    fn lines(&self, name: &str) -> Vec<String> {
        fs::read_to_string(self.rec(name))
            .map(|t| t.lines().map(str::to_string).collect())
            .unwrap_or_default()
    }

    fn wait_lines(&self, name: &str, count: usize) {
        wait_until(&format!("{count} start(s) of {name}"), TIMEOUT, || {
            self.lines(name).len() >= count
        });
    }

    fn path_var(&self) -> String {
        format!(
            "{}:{}",
            self.dir.path().display(),
            std::env::var("PATH").unwrap_or_default()
        )
    }
}

fn builder(fakes: &Fakes) -> support::Builder {
    Harness::builder(SIZE)
        .config_unset("sidebar")
        .env("PATH", &fakes.path_var())
        .env("DECK_TEST_AGENT_POLL_MS", "100")
}

/// The agent of the first terminal that has one, as the saved state has it.
fn saved_agent(home: &Home) -> Option<Value> {
    let text = fs::read_to_string(home.path().join("state").join("state.json")).ok()?;
    let doc: Value = serde_json::from_str(&text).ok()?;
    doc["sessions"]
        .as_array()?
        .iter()
        .flat_map(|s| s["projects"].as_array().into_iter().flatten())
        .flat_map(|p| p["panels"].as_array().into_iter().flatten())
        .flat_map(|p| p["terminals"].as_array().into_iter().flatten())
        .map(|t| t["agent"].clone())
        .find(|a| !a.is_null())
}

fn wait_saved(home: &Home, sid: &str) {
    wait_until("the agent to be saved", TIMEOUT, || {
        saved_agent(home).is_some_and(|a| {
            a["kind"] == "Claude"
                && if sid.is_empty() {
                    a["session_id"].is_null()
                } else {
                    a["session_id"] == sid
                }
        })
    });
}

/// `deck kill-server`, then `deck` again in a PTY on the same home.
/// Returns the new client and the first one, which owns the home and must outlive it.
fn restart(first: Harness, fakes: &Fakes) -> (Harness, Harness) {
    let path = first.home().path().to_string_lossy().into_owned();
    restart_on(first, &path, fakes)
}

/// Stops the daemon of the home `path` (`DECK_HOME`), then starts `deck` there again.
fn restart_on(mut first: Harness, path: &str, fakes: &Fakes) -> (Harness, Harness) {
    let stop = first.run_deck(&["kill-server"]);
    assert!(stop.status.success(), "{}", stderr(&stop));
    first.wait_for_exit(TIMEOUT);
    let again = Harness::builder(SIZE)
        .config_unset("sidebar")
        .env("DECK_HOME", path)
        .env("PATH", &fakes.path_var())
        .env("DECK_TEST_AGENT_POLL_MS", "100")
        .deck();
    (again, first)
}

/// Types `claude` in the first terminal and waits until the agent runs and is saved.
fn start_agent(h: &mut Harness, fakes: &Fakes, sid: &str) {
    h.wait_for_text("$", TIMEOUT);
    h.send_keys("claude\r");
    fakes.wait_lines("claude", 1);
    wait_saved(h.home(), sid);
}

fn here(home: &Home) -> String {
    fs::canonicalize(home.path()).unwrap().display().to_string()
}

/// The agent quits and the terminal shows a shell: it answers a command.
fn expect_shell(h: &mut Harness) {
    h.send_keys("echo back$((40+2))\r");
    h.wait_for_text("back42", TIMEOUT);
}

#[test]
fn an_agent_with_a_session_id_is_resumed_and_leaves_a_shell_behind() {
    let fakes = Fakes::new();
    fakes.claude("abc", false);
    let mut h = builder(&fakes).deck();
    start_agent(&mut h, &fakes, "abc");
    let home = here(h.home());

    let (mut again, _first) = restart(h, &fakes);
    fakes.wait_lines("claude", 2);
    assert_eq!(
        fakes.lines("claude"),
        [format!("{home}|"), format!("{home}|--resume abc")]
    );
    again.capture("resumed");
    // The agent quits: the shell is still there, in the same directory.
    again.send_keys("\r");
    expect_shell(&mut again);
    again.send_keys("pwd -P\r");
    again.wait_for_text(&home, TIMEOUT);
}

#[test]
fn a_session_end_hook_during_the_shutdown_does_not_lose_the_agent() {
    let fakes = Fakes::new();
    fakes.claude("abc", true);
    let mut h = builder(&fakes).deck();
    start_agent(&mut h, &fakes, "abc");
    let home = here(h.home());

    let (_again, _first) = restart(h, &fakes);
    fakes.wait_lines("claude", 2);
    assert_eq!(fakes.lines("claude")[1], format!("{home}|--resume abc"));
}

#[test]
fn shell_args_apply_to_the_shell_that_follows_the_agent() {
    let fakes = Fakes::new();
    fakes.claude("abc", false);
    fakes.shell();
    let shell = format!("\"{}\"", fakes.path("shell").display());
    let mut h = builder(&fakes)
        .config("shell", &shell)
        .config("shell_args", "[\"-i\"]")
        .deck();
    fakes.wait_lines("shell", 1);
    start_agent(&mut h, &fakes, "abc");

    let (mut again, _first) = restart(h, &fakes);
    fakes.wait_lines("claude", 2);
    assert_eq!(
        fakes.lines("shell"),
        ["-i"],
        "the shell waits for the agent"
    );
    again.send_keys("\r");
    fakes.wait_lines("shell", 2);
    assert_eq!(fakes.lines("shell"), ["-i", "-i"]);
    expect_shell(&mut again);
}

#[test]
fn without_a_session_id_the_latest_conversation_resumes() {
    let fakes = Fakes::new();
    fakes.claude("", false);
    let mut h = builder(&fakes).deck();
    start_agent(&mut h, &fakes, "");
    let home = here(h.home());

    let (_again, _first) = restart(h, &fakes);
    fakes.wait_lines("claude", 2);
    assert_eq!(fakes.lines("claude")[1], format!("{home}|--continue"));
}

fn bad_id_is_dropped(sid: &str) {
    let fakes = Fakes::new();
    fakes.claude(sid, false);
    let mut h = builder(&fakes).deck();
    // The daemon refuses the id when the hook reports it: the agent is saved without one.
    start_agent(&mut h, &fakes, "");
    // An agent with no id is also what is saved before the hook arrives: wait until the hook
    // has returned, so the daemon has had the id and refused it.
    wait_until("the hook to return", TIMEOUT, || {
        fakes.path("claude.hooked").exists()
    });
    let home = here(h.home());

    let (_again, first) = restart(h, &fakes);
    fakes.wait_lines("claude", 2);
    assert_eq!(
        fakes.lines("claude")[1],
        format!("{home}|--continue"),
        "{sid}"
    );
    // The id was not saved either, by the daemon that stopped or by the one that restored.
    let id = saved_agent(first.home()).map(|agent| agent["session_id"].clone());
    assert!(matches!(id, None | Some(serde_json::Value::Null)), "{id:?}");
    // Nothing of the id was run by a shell.
    assert!(!Path::new(&home).join("PWNED").exists());
}

#[test]
fn an_id_with_shell_syntax_is_dropped() {
    bad_id_is_dropped("abc; touch PWNED");
}

#[test]
fn an_id_that_looks_like_an_option_is_dropped() {
    bad_id_is_dropped("-p");
}

#[test]
fn resume_agents_off_restores_a_plain_shell() {
    let fakes = Fakes::new();
    fakes.claude("abc", false);
    let mut h = builder(&fakes).config("resume_agents", "false").deck();
    start_agent(&mut h, &fakes, "abc");

    let (mut again, _first) = restart(h, &fakes);
    expect_shell(&mut again);
    assert_eq!(
        fakes.lines("claude").len(),
        1,
        "the agent did not run again"
    );
}

#[test]
fn a_custom_resume_template_is_used() {
    let fakes = Fakes::new();
    fakes.claude("abc", false);
    let mut h = builder(&fakes)
        .config(
            "agents",
            r#"{ claude = { resume = "claude --open {session_id} --verbose" } }"#,
        )
        .deck();
    start_agent(&mut h, &fakes, "abc");
    let home = here(h.home());

    let (_again, _first) = restart(h, &fakes);
    fakes.wait_lines("claude", 2);
    assert_eq!(
        fakes.lines("claude")[1],
        format!("{home}|--open abc --verbose")
    );
}

#[test]
fn a_template_with_a_quote_or_a_dollar_is_rejected_naming_the_key() {
    for (template, key) in [
        (
            r#"{ claude = { resume = "claude 'x'" } }"#,
            "agents.claude.resume",
        ),
        (
            r#"{ codex = { resume_latest = "codex $HOME" } }"#,
            "agents.codex.resume_latest",
        ),
    ] {
        let home = Home::with(&[("agents", template)], &[]);
        let out = home.run_deck(&["config", "check"]);
        let text = format!("{}{}", support::stdout(&out), stderr(&out));
        assert!(!out.status.success(), "{template}: {text}");
        assert!(text.contains(key), "{template}: {text}");
    }
}

#[test]
fn an_agent_started_as_the_terminal_program_is_relaunched_directly() {
    let fakes = Fakes::new();
    fakes.claude("xyz", false);
    let h = builder(&fakes).deck();
    h.wait_for_text("$", TIMEOUT);
    // A new terminal starts in the project's directory, which is also where it must resume.
    let out = h.home().run_deck(&["new-terminal", "--", "claude"]);
    assert!(out.status.success(), "{}", stderr(&out));
    fakes.wait_lines("claude", 1);
    wait_saved(h.home(), "xyz");
    let dir = here(h.home());

    let (_again, _first) = restart(h, &fakes);
    fakes.wait_lines("claude", 2);
    assert_eq!(fakes.lines("claude")[0], format!("{dir}|"));
    assert_eq!(fakes.lines("claude")[1], format!("{dir}|--resume xyz"));
}

#[test]
fn a_resumed_agent_is_detected_again_and_resumes_a_second_time() {
    let fakes = Fakes::new();
    fakes.claude("abc", false);
    let mut h = builder(&fakes).deck();
    start_agent(&mut h, &fakes, "abc");
    let home = here(h.home());

    let path = h.home().path().to_string_lossy().into_owned();
    let (again, first) = restart(h, &fakes);
    fakes.wait_lines("claude", 2);
    // Past several detection ticks and a save: the wrapper's agent is still found.
    std::thread::sleep(std::time::Duration::from_millis(1500));
    wait_saved(first.home(), "abc");
    assert_eq!(fakes.lines("claude")[1], format!("{home}|--resume abc"));

    let (third, _second) = restart_on(again, &path, &fakes);
    fakes.wait_lines("claude", 3);
    assert_eq!(fakes.lines("claude")[2], format!("{home}|--resume abc"));
    drop(third);
    drop(first);
}

/// Asks the running daemon for a panel with `launch` in the first project, as an attached
/// client does (there is no command for it), and returns the action's answer.
fn new_panel_with(home: &Home, launch: PanelLaunch) -> Result<(), deck_core::action::ActionError> {
    use std::io::{Read, Write};
    let mut stream = std::os::unix::net::UnixStream::connect(home.socket()).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .unwrap();
    // No environment: the session's `PATH` is the one of the client that created it.
    let hello = ClientMsg::Hello {
        version: PROTOCOL_VERSION,
        role: Role::Attach {
            session: None,
            create: false,
            cols: SIZE.0,
            rows: SIZE.1,
            cell: None,
            images: false,
            outer_name: None,
        },
        cwd: std::env::temp_dir(),
        env: Vec::new(),
        colors: None,
    };
    stream.write_all(&encode(&hello).unwrap()).unwrap();
    let mut reader = FrameReader::new();
    let mut sent = false;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        while let Some(msg) = reader.next_message::<ServerMsg>().unwrap() {
            match msg {
                ServerMsg::SessionView(view) if !sent => {
                    sent = true;
                    let action = Action::NewPanelWith {
                        project: view.projects[0].id,
                        launch: launch.clone(),
                    };
                    stream
                        .write_all(&encode(&ClientMsg::Action(action)).unwrap())
                        .unwrap();
                }
                ServerMsg::ActionResult(result) => return result,
                _ => {}
            }
        }
        let n = stream.read(&mut buf).unwrap();
        assert!(n > 0, "the daemon closed the connection");
        reader.push(&buf[..n]);
    }
}

#[test]
fn an_agent_panel_is_resumed_after_a_restart() {
    let fakes = Fakes::new();
    fakes.claude("abc", false);
    let h = builder(&fakes).deck();
    h.wait_for_text("$", TIMEOUT);
    new_panel_with(h.home(), PanelLaunch::Agent(AgentKind::Claude)).unwrap();
    fakes.wait_lines("claude", 1);
    wait_saved(h.home(), "abc");
    let home = here(h.home());

    let (_again, _first) = restart(h, &fakes);
    fakes.wait_lines("claude", 2);
    assert_eq!(
        fakes.lines("claude"),
        [format!("{home}|"), format!("{home}|--resume abc")]
    );
}

#[test]
fn a_command_outside_the_process_list_is_relaunched_fresh() {
    let fakes = Fakes::new();
    let rec = fakes.rec("myclaude").display().to_string();
    fakes.write_script(
        "myclaude",
        &format!("printf '%s|%s\\n' \"$(pwd -P)\" \"$*\" >> '{rec}'\nread line\n"),
    );
    let command = fakes.path("myclaude").display().to_string();
    let h = builder(&fakes)
        .config(
            "agents",
            &format!(r#"{{ claude = {{ command = "{command}" }} }}"#),
        )
        .deck();
    h.wait_for_text("$", TIMEOUT);
    new_panel_with(h.home(), PanelLaunch::Agent(AgentKind::Claude)).unwrap();
    fakes.wait_lines("myclaude", 1);
    let home = here(h.home());

    let (_again, _first) = restart(h, &fakes);
    fakes.wait_lines("myclaude", 2);
    assert_eq!(
        fakes.lines("myclaude"),
        [format!("{home}|"), format!("{home}|")],
        "relaunched as the plain command, not resumed"
    );
}
