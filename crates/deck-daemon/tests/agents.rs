//! Agent status: detection by the foreground process, hook events, the answered-prompt
//! rule and the 3 s and 60 s rules (on an injected clock).

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use deck_core::agents::{AgentKind, AgentSource, AgentState, AgentStatus};
use deck_core::model::{LaunchSpec, TerminalId};
use deck_core::protocol::{ClientMsg, Request, Response};
use deck_daemon::{ForegroundQuery, ManualClock, Options};
use support::{Attach, Client, TestDaemon};

const POLL: Duration = Duration::from_millis(20);

/// What the injected query answers: the argv of the foreground process.
type Foreground = Arc<Mutex<Option<Vec<String>>>>;

fn argv(items: &[&str]) -> Option<Vec<String>> {
    Some(items.iter().map(ToString::to_string).collect())
}

/// A daemon on a manual clock whose foreground process the test chooses.
async fn manual(initial: Option<Vec<String>>) -> (TestDaemon, Arc<ManualClock>, Foreground) {
    let clock = Arc::new(ManualClock::new());
    let foreground: Foreground = Arc::new(Mutex::new(initial));
    let seen = Arc::clone(&foreground);
    let options = Options {
        clock: clock.clone(),
        agent_poll: POLL,
        foreground_query: ForegroundQuery::new(move |_| seen.lock().unwrap().clone()),
        ..Options::default()
    };
    (
        TestDaemon::start_with(support::config(), options).await,
        clock,
        foreground,
    )
}

fn agent(client: &Client, terminal: TerminalId) -> Option<AgentState> {
    client
        .view
        .as_ref()?
        .projects
        .iter()
        .flat_map(|p| &p.panels)
        .flat_map(|p| &p.terminals)
        .find(|t| t.id == terminal)?
        .agent
        .clone()
}

fn is(client: &Client, terminal: TerminalId, status: AgentStatus, source: AgentSource) -> bool {
    agent(client, terminal).is_some_and(|a| a.status == status && a.source == source)
}

async fn event(
    client: &mut Client,
    terminal: TerminalId,
    status: Option<AgentStatus>,
    session_id: Option<&str>,
    prompt: Option<&str>,
) -> Response {
    client
        .request(Request::AgentEvent {
            terminal,
            kind: AgentKind::Claude,
            status,
            session_id: session_id.map(Into::into),
            prompt: prompt.map(Into::into),
            raw: None,
        })
        .await
}

async fn pause() {
    tokio::time::sleep(Duration::from_millis(250)).await;
}

#[tokio::test]
async fn a_fake_agent_script_is_detected_while_it_runs() {
    let dir = tempfile::Builder::new()
        .prefix("ag")
        .tempdir_in("/tmp")
        .unwrap();
    let script = dir.path().join("claude");
    fs::write(&script, "#!/bin/sh\necho agent-ready\nread line\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let mut env = support::hello_env();
    let path = std::env::var("PATH").unwrap_or_default();
    env.retain(|(k, _)| k != "PATH");
    env.push((
        "PATH".into(),
        format!("{}:{path}", dir.path().display()).into(),
    ));

    let options = Options {
        agent_poll: Duration::from_millis(50),
        ..Options::default()
    };
    let daemon = TestDaemon::start_with(support::config(), options).await;
    let mut client = Client::attach(
        &daemon.paths.socket,
        Attach {
            env,
            ..Attach::default()
        },
    )
    .await;
    let terminal = client.terminal();
    assert_eq!(agent(&client, terminal), None);

    client.type_line("claude").await;
    client.wait_row("agent-ready").await;
    client
        .wait(|c| is(c, terminal, AgentStatus::Active, AgentSource::Process))
        .await;
    assert_eq!(agent(&client, terminal).unwrap().kind, AgentKind::Claude);
    // It prints nothing: quiet after two seconds.
    client
        .wait(|c| is(c, terminal, AgentStatus::Quiet, AgentSource::Process))
        .await;

    // It exits when its input ends a line.
    client
        .send(&ClientMsg::Input {
            terminal,
            bytes: b"\n".to_vec(),
        })
        .await;
    client.wait(|c| agent(c, terminal).is_none()).await;
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn hook_events_set_the_state_and_typing_answers_a_waiting_agent() {
    let (daemon, _clock, _fg) = manual(argv(&["claude"])).await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let terminal = client.terminal();
    // Without hooks the process is all there is.
    client
        .wait(|c| agent(c, terminal).is_some_and(|a| a.source == AgentSource::Process))
        .await;

    let r = event(
        &mut client,
        terminal,
        Some(AgentStatus::Working),
        None,
        None,
    )
    .await;
    assert_eq!(r, Response::Ok);
    client
        .wait(|c| is(c, terminal, AgentStatus::Working, AgentSource::Hooks))
        .await;
    event(
        &mut client,
        terminal,
        Some(AgentStatus::Waiting),
        None,
        None,
    )
    .await;
    client
        .wait(|c| is(c, terminal, AgentStatus::Waiting, AgentSource::Hooks))
        .await;

    // A focus report is not an answer.
    client
        .send(&ClientMsg::Input {
            terminal,
            bytes: b"\x1b[I".to_vec(),
        })
        .await;
    pause().await;
    client.recv(Duration::from_millis(50)).await;
    assert!(is(
        &client,
        terminal,
        AgentStatus::Waiting,
        AgentSource::Hooks
    ));

    client
        .send(&ClientMsg::Input {
            terminal,
            bytes: b"y".to_vec(),
        })
        .await;
    client
        .wait(|c| is(c, terminal, AgentStatus::Working, AgentSource::Hooks))
        .await;
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn a_new_session_id_replaces_the_old_one_and_resets_the_first_prompt() {
    let (daemon, _clock, _fg) = manual(argv(&["claude"])).await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let terminal = client.terminal();

    let long = format!("{}\nsecond line", "x".repeat(300));
    event(
        &mut client,
        terminal,
        Some(AgentStatus::Working),
        Some("one"),
        Some(&long),
    )
    .await;
    client
        .wait(|c| agent(c, terminal).is_some_and(|a| a.session_id.as_deref() == Some("one")))
        .await;
    let first = agent(&client, terminal).unwrap().first_prompt.unwrap();
    assert_eq!(first, "x".repeat(200));

    // The same session keeps its first prompt.
    event(
        &mut client,
        terminal,
        Some(AgentStatus::Idle),
        Some("one"),
        Some("later"),
    )
    .await;
    client
        .wait(|c| is(c, terminal, AgentStatus::Idle, AgentSource::Hooks))
        .await;
    assert_eq!(agent(&client, terminal).unwrap().first_prompt, Some(first));

    // Another session replaces both.
    event(
        &mut client,
        terminal,
        Some(AgentStatus::Working),
        Some("two"),
        None,
    )
    .await;
    client
        .wait(|c| agent(c, terminal).is_some_and(|a| a.session_id.as_deref() == Some("two")))
        .await;
    assert_eq!(agent(&client, terminal).unwrap().first_prompt, None);
    event(
        &mut client,
        terminal,
        None,
        Some("two"),
        Some("fresh prompt"),
    )
    .await;
    client
        .wait(|c| {
            agent(c, terminal).is_some_and(|a| a.first_prompt.as_deref() == Some("fresh prompt"))
        })
        .await;
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn a_working_agent_with_no_news_for_a_minute_falls_back_to_the_process_state() {
    let (daemon, clock, _fg) = manual(argv(&["claude"])).await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let terminal = client.terminal();
    // The shell's prompt, and the redraw a resize of its PTY can cause, are the last output
    // before the clock moves.
    client.wait_row("$").await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    event(
        &mut client,
        terminal,
        Some(AgentStatus::Working),
        None,
        None,
    )
    .await;
    client
        .wait(|c| is(c, terminal, AgentStatus::Working, AgentSource::Hooks))
        .await;

    clock.advance(Duration::from_secs(59));
    pause().await;
    client.recv(Duration::from_millis(50)).await;
    assert!(is(
        &client,
        terminal,
        AgentStatus::Working,
        AgentSource::Hooks
    ));

    clock.advance(Duration::from_secs(1));
    client
        .wait(|c| is(c, terminal, AgentStatus::Quiet, AgentSource::Process))
        .await;
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn a_hook_state_is_cleared_three_seconds_after_the_agent_left_the_foreground() {
    let (daemon, clock, foreground) = manual(argv(&["claude"])).await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let terminal = client.terminal();
    event(&mut client, terminal, Some(AgentStatus::Idle), None, None).await;
    client
        .wait(|c| is(c, terminal, AgentStatus::Idle, AgentSource::Hooks))
        .await;

    *foreground.lock().unwrap() = argv(&["-sh"]);
    pause().await;
    clock.advance(Duration::from_millis(2900));
    pause().await;
    client.recv(Duration::from_millis(50)).await;
    assert!(is(&client, terminal, AgentStatus::Idle, AgentSource::Hooks));

    clock.advance(Duration::from_millis(200));
    client.wait(|c| agent(c, terminal).is_none()).await;
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn an_event_for_a_terminal_that_does_not_exist_is_an_error() {
    let (daemon, _clock, _fg) = manual(None).await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let r = event(
        &mut client,
        TerminalId(999),
        Some(AgentStatus::Working),
        None,
        None,
    )
    .await;
    assert!(
        matches!(r, Response::Error(ref e) if e.contains("999")),
        "{r:?}"
    );
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn the_tree_carries_the_agent_state() {
    let (daemon, _clock, _fg) = manual(None).await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let terminal = client.terminal();
    event(
        &mut client,
        terminal,
        Some(AgentStatus::Waiting),
        Some("s9"),
        None,
    )
    .await;
    let Response::Tree(views) = client.request(Request::Tree { session: None }).await else {
        panic!("a tree");
    };
    let state = views[0].projects[0].panels[0].terminals[0]
        .agent
        .clone()
        .unwrap();
    assert_eq!(state.status, AgentStatus::Waiting);
    assert_eq!(state.session_id.as_deref(), Some("s9"));
    daemon.stop().await.unwrap();
}

/// A kept terminal running `script`, in the first panel.
async fn kept(client: &mut Client, script: &str) -> TerminalId {
    let panel = client.view.as_ref().unwrap().projects[0].panels[0].id;
    let response = client
        .request(Request::NewTerminal {
            panel,
            launch: Some(LaunchSpec {
                program: Some("/bin/sh".into()),
                args: vec!["-c".into(), script.into()],
                cwd: std::path::PathBuf::new(),
                env: Vec::new(),
            }),
            keep: true,
            focus: false,
        })
        .await;
    let Response::Created { id } = response else {
        panic!("a terminal: {response:?}");
    };
    TerminalId(id)
}

#[tokio::test]
async fn a_kept_terminal_that_exits_loses_its_agent_state_and_ignores_later_events() {
    let (daemon, _clock, _fg) = manual(argv(&["claude"])).await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    // One with a process-sourced state, one with a hook state.
    let by_process = kept(&mut client, "sleep 1").await;
    let by_hook = kept(&mut client, "sleep 1").await;
    client
        .wait(|c| agent(c, by_process).is_some_and(|a| a.source == AgentSource::Process))
        .await;
    event(&mut client, by_hook, Some(AgentStatus::Working), None, None).await;
    client
        .wait(|c| is(c, by_hook, AgentStatus::Working, AgentSource::Hooks))
        .await;

    client
        .wait(|c| agent(c, by_process).is_none() && agent(c, by_hook).is_none())
        .await;
    // A late hook event changes nothing.
    let r = event(&mut client, by_hook, Some(AgentStatus::Waiting), None, None).await;
    assert_eq!(r, Response::Ok);
    pause().await;
    client.recv(Duration::from_millis(50)).await;
    assert_eq!(agent(&client, by_hook), None);
    daemon.stop().await.unwrap();
}

async fn raw(client: &mut Client, terminal: TerminalId, json: &str) -> Response {
    client
        .request(Request::AgentEvent {
            terminal,
            kind: AgentKind::OpenCode,
            status: None,
            session_id: None,
            prompt: None,
            raw: Some(json.into()),
        })
        .await
}

#[tokio::test]
async fn raw_opencode_events_go_through_the_terminals_tracker() {
    let (daemon, _clock, _fg) = manual(argv(&["opencode"])).await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let terminal = client.terminal();
    let busy = |s: &str| {
        format!(
            r#"{{"type":"session.status","properties":{{"sessionID":"{s}","status":{{"type":"busy"}}}}}}"#
        )
    };
    let idle = |s: &str| {
        format!(
            r#"{{"type":"session.status","properties":{{"sessionID":"{s}","status":{{"type":"idle"}}}}}}"#
        )
    };

    raw(&mut client, terminal, &busy("root")).await;
    client
        .wait(|c| is(c, terminal, AgentStatus::Working, AgentSource::Hooks))
        .await;
    raw(
        &mut client,
        terminal,
        r#"{"type":"message.updated","properties":{"info":{"id":"m1","sessionID":"root","role":"user"}}}"#,
    )
    .await;
    raw(
        &mut client,
        terminal,
        r#"{"type":"message.part.updated","properties":{"part":{"id":"p1","sessionID":"root","messageID":"m1","type":"text","text":"add a test"}}}"#,
    )
    .await;
    client
        .wait(|c| {
            agent(c, terminal).is_some_and(|a| {
                a.kind == AgentKind::OpenCode
                    && a.session_id.as_deref() == Some("root")
                    && a.first_prompt.as_deref() == Some("add a test")
            })
        })
        .await;

    // A subagent finishing changes neither the status nor the session.
    raw(
        &mut client,
        terminal,
        r#"{"type":"session.created","properties":{"info":{"id":"sub","parentID":"root"}}}"#,
    )
    .await;
    raw(&mut client, terminal, &idle("sub")).await;
    pause().await;
    client.recv(Duration::from_millis(50)).await;
    let state = agent(&client, terminal).unwrap();
    assert_eq!(state.status, AgentStatus::Working);
    assert_eq!(state.session_id.as_deref(), Some("root"));

    raw(&mut client, terminal, &idle("root")).await;
    client
        .wait(|c| is(c, terminal, AgentStatus::Idle, AgentSource::Hooks))
        .await;
    daemon.stop().await.unwrap();
}
