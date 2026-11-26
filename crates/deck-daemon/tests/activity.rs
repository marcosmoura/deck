//! The activity line of an agent: what hook events set, keep and clear, its order, its view
//! throttle, and that it never reaches `state.json` or the log.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use std::fmt::Write as _;
use std::fs;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use deck_core::agents::{Activity, AgentKind, AgentStatus};
use deck_core::model::{PanelId, TerminalId};
use deck_core::protocol::{ClientMsg, Request, Response, ServerMsg, Target};
use deck_daemon::Options;
use support::{Attach, Client, TestDaemon};

/// A daemon that finds no agent by process, so only hook events set states.
async fn daemon() -> TestDaemon {
    TestDaemon::start_with(support::config(), Options::default()).await
}

async fn event(
    client: &mut Client,
    terminal: TerminalId,
    status: Option<AgentStatus>,
    activity: Activity,
    at_ms: u64,
) {
    let r = client
        .request(Request::AgentEvent {
            terminal,
            kind: AgentKind::Claude,
            status,
            session_id: None,
            prompt: None,
            raw: None,
            activity,
            at_ms,
        })
        .await;
    assert_eq!(r, Response::Ok);
}

fn line(client: &Client, terminal: TerminalId) -> Option<String> {
    client
        .view
        .as_ref()?
        .projects
        .iter()
        .flat_map(|p| &p.panels)
        .flat_map(|p| &p.terminals)
        .find(|t| t.id == terminal)?
        .agent
        .as_ref()?
        .activity
        .clone()
}

fn set(text: &str) -> Activity {
    Activity::Set(text.into())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hook_sequence_shows_the_line_of_each_step() {
    let daemon = daemon().await;
    let mut c = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let t = c.terminal();
    let working = Some(AgentStatus::Working);

    event(&mut c, t, working, set("thinking"), 10).await;
    c.wait(|c| line(c, t).as_deref() == Some("thinking")).await;
    event(&mut c, t, working, set("run the tests"), 11).await;
    c.wait(|c| line(c, t).as_deref() == Some("run the tests"))
        .await;
    // PostToolUse keeps it.
    event(&mut c, t, working, Activity::Keep, 12).await;
    event(&mut c, t, Some(AgentStatus::Idle), set("all done"), 13).await;
    c.wait(|c| line(c, t).as_deref() == Some("all done")).await;

    // A PreToolUse that began before the Stop and arrives after it: status only.
    event(&mut c, t, working, set("late tool"), 12).await;
    c.wait(|c| {
        c.view.as_ref().is_some_and(|v| {
            v.projects[0].panels[0].terminals[0]
                .agent
                .as_ref()
                .map(|a| a.status)
                == Some(AgentStatus::Working)
        })
    })
    .await;
    assert_eq!(line(&c, t).as_deref(), Some("all done"));

    // A raw request is cleaned and cut by the daemon.
    let raw = format!("\u{1b}  {}\nsecond", "y".repeat(300));
    event(&mut c, t, None, Activity::Set(raw), 0).await;
    c.wait(|c| line(c, t).is_some_and(|l| l.chars().count() == 80 && l.starts_with("yyy")))
        .await;

    // SessionEnd clears the state, and its line with it.
    event(&mut c, t, Some(AgentStatus::None), Activity::Clear, 0).await;
    c.wait(|c| line(c, t).is_none()).await;
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn typing_an_answer_clears_the_line_of_a_waiting_agent() {
    let daemon = daemon().await;
    let mut c = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let t = c.terminal();
    event(
        &mut c,
        t,
        Some(AgentStatus::Waiting),
        set("needs permission: running rm x"),
        1,
    )
    .await;
    c.wait(|c| line(c, t).is_some()).await;
    c.send(&ClientMsg::Input {
        terminal: t,
        bytes: b"y".to_vec(),
    })
    .await;
    c.wait(|c| line(c, t).is_none()).await;
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_burst_of_tool_events_sends_few_views() {
    let daemon = daemon().await;
    let mut c = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let t = c.terminal();
    event(&mut c, t, Some(AgentStatus::Working), set("thinking"), 1).await;
    c.wait(|c| line(c, t).as_deref() == Some("thinking")).await;
    c.recv(Duration::from_millis(300)).await;
    let before = c.views().count();

    let started = std::time::Instant::now();
    for n in 0..100 {
        event(
            &mut c,
            t,
            Some(AgentStatus::Working),
            set(&format!("reading file{n}.rs")),
            2 + n,
        )
        .await;
    }
    let elapsed = started.elapsed();
    c.wait(|c| line(c, t).as_deref() == Some("reading file99.rs"))
        .await;
    c.recv(Duration::from_millis(300)).await;
    let views = c.views().count() - before;
    // One at once and one per 100 ms while the burst lasts, then the last one.
    let allowed = 2 + usize::try_from(elapsed.as_millis() / 100).unwrap();
    assert!(views <= allowed, "{views} views in {elapsed:?}");
    assert!(
        views <= 11 || elapsed > Duration::from_secs(1),
        "{views} views"
    );
    daemon.stop().await.unwrap();
}

/// Everything the daemon logs while the tests of this binary run.
fn captured() -> &'static Arc<Mutex<String>> {
    static LOG: OnceLock<Arc<Mutex<String>>> = OnceLock::new();
    LOG.get_or_init(|| {
        let log = Arc::new(Mutex::new(String::new()));
        let _ = tracing::subscriber::set_global_default(Capture(Arc::clone(&log)));
        log
    })
}

struct Capture(Arc<Mutex<String>>);

struct Fields<'a>(&'a mut String);

impl tracing::field::Visit for Fields<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let _ = write!(self.0, "{}={value:?} ", field.name());
    }
}

impl tracing::Subscriber for Capture {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        let mut line = String::new();
        event.record(&mut Fields(&mut line));
        if let Ok(mut log) = self.0.lock() {
            log.push_str(&line);
            log.push('\n');
        }
    }

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}
}

#[tokio::test(flavor = "multi_thread")]
async fn the_line_is_never_saved_or_logged() {
    let log = captured();
    let daemon = daemon().await;
    let mut c = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let t = c.terminal();
    event(
        &mut c,
        t,
        Some(AgentStatus::Working),
        set("running marker-q7731-secret"),
        1,
    )
    .await;
    c.wait(|c| line(c, t).is_some()).await;
    let home = daemon.stop_keep().await;
    let state = fs::read_to_string(home.path().join("state/state.json")).unwrap();
    assert!(state.contains("terminals"), "the state was saved: {state}");
    assert!(!state.contains("q7731"), "no activity in state.json");
    let text = log.lock().unwrap().clone();
    assert!(text.contains("listening"), "the log is captured: {text}");
    assert!(!text.contains("q7731"), "no activity in the log:\n{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_alert_names_the_panel_it_is_about() {
    let daemon = daemon().await;
    let mut c = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let view = c.view.clone().unwrap();
    let r = c
        .request(Request::NewPanel {
            session: view.id,
            project: Target::Id(view.projects[0].id.0),
            name: Some("other".into()),
            launch: None,
            focus: false,
        })
        .await;
    let Response::Created { id } = r else {
        panic!("a panel: {r:?}");
    };
    let panel = PanelId(id);
    c.wait(|c| c.view.as_ref().unwrap().projects[0].panels.len() == 2)
        .await;
    let terminal = c.view.as_ref().unwrap().projects[0]
        .panels
        .iter()
        .find(|p| p.id == panel)
        .unwrap()
        .terminals[0]
        .id;
    event(
        &mut c,
        terminal,
        Some(AgentStatus::Working),
        Activity::Keep,
        0,
    )
    .await;
    event(&mut c, terminal, Some(AgentStatus::Idle), set("done"), 0).await;
    c.wait(|c| {
        c.log
            .iter()
            .any(|m| matches!(m, ServerMsg::AgentAlert { panel_id, .. } if *panel_id == panel))
    })
    .await;
    daemon.stop().await.unwrap();
}
