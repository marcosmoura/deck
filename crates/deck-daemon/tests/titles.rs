//! Panel names that follow what runs in the panel: titles, programs, agent prompts, the
//! manual lock and `AutoNamePanel`.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use deck_core::action::Action;
use deck_core::agents::{AgentKind, AgentStatus};
use deck_core::model::NameSource;
use deck_core::protocol::{Request, Response};
use deck_daemon::{ForegroundQuery, ManualClock, Options};
use support::{Attach, Client, TestDaemon};

/// What the injected query answers: the argv of the foreground process, `None` when it
/// cannot be read.
type Foreground = Arc<Mutex<Option<Vec<String>>>>;

fn argv(items: &[&str]) -> Option<Vec<String>> {
    Some(items.iter().map(ToString::to_string).collect())
}

/// A daemon on a manual clock whose foreground process the test chooses.
async fn manual(initial: Option<Vec<String>>) -> (TestDaemon, Foreground) {
    daemon(initial, Some(Arc::new(ManualClock::new()))).await
}

/// The same on the real clock, for the title throttle, which a manual clock never lets pass.
async fn live(initial: Option<Vec<String>>) -> (TestDaemon, Foreground) {
    daemon(initial, None).await
}

async fn daemon(
    initial: Option<Vec<String>>,
    clock: Option<Arc<ManualClock>>,
) -> (TestDaemon, Foreground) {
    let foreground: Foreground = Arc::new(Mutex::new(initial));
    let seen = Arc::clone(&foreground);
    let mut options = Options {
        agent_poll: Duration::from_millis(20),
        foreground_query: ForegroundQuery::new(move |_| seen.lock().unwrap().clone()),
        ..Options::default()
    };
    if let Some(clock) = clock {
        options.clock = clock;
    }
    (
        TestDaemon::start_with(support::config(), options).await,
        foreground,
    )
}

fn panel_name(c: &Client) -> String {
    c.view.as_ref().unwrap().projects[0].panels[0].name.clone()
}

fn name_source(c: &Client) -> NameSource {
    c.view.as_ref().unwrap().projects[0].panels[0].name_source
}

fn program(c: &Client) -> String {
    c.view.as_ref().unwrap().projects[0].panels[0].terminals[0]
        .program
        .clone()
}

// Spinner glyphs as octal escapes of their UTF-8 bytes, so the line editor of the test
// shell never sees them.
const STAR: &str = "\\342\\234\\263";
const SPIN1: &str = "\\342\\240\\271";
const SPIN2: &str = "\\342\\240\\270";
const SPIN3: &str = "\\342\\240\\274";

/// Has the shell set a title (OSC 2) and print a marker once the daemon has read it.
async fn set_title(c: &mut Client, title: &str, marker: &str) {
    c.type_line(&format!("printf '\\033]2;{title}\\007'; echo {marker}"))
        .await;
    c.wait_row(marker).await;
}

async fn prompt_event(c: &mut Client, prompt: &str) {
    let terminal = c.terminal();
    let r = c
        .request(Request::AgentEvent {
            terminal,
            kind: AgentKind::Claude,
            status: None,
            session_id: Some("s1".into()),
            prompt: Some(prompt.into()),
            raw: None,
        })
        .await;
    assert_eq!(r, Response::Ok);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shells_title_names_its_panel() {
    let daemon = TestDaemon::start().await;
    let mut c = Client::attach(&daemon.paths.socket, Attach::default()).await;
    assert_eq!(name_source(&c), NameSource::Auto);
    set_title(&mut c, "~/src", "T1").await;
    c.wait(|c| panel_name(c) == "~/src").await;
    assert_eq!(name_source(&c), NameSource::Auto);
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_title_names_the_panel_without_its_spinner() {
    let (daemon, _fg) = manual(argv(&["claude"])).await;
    let mut c = Client::attach(&daemon.paths.socket, Attach::default()).await;
    c.wait(|c| program(c) == "claude").await;
    // The agent has no title and no prompt yet: the name stays.
    assert_eq!(name_source(&c), NameSource::Auto);
    set_title(&mut c, &format!("{STAR} Fix login bug"), "T1").await;
    c.wait(|c| panel_name(c) == "Fix login bug").await;
    assert_eq!(name_source(&c), NameSource::Auto);
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_with_only_a_prompt_gets_its_first_line() {
    let (daemon, _fg) = manual(argv(&["claude"])).await;
    let mut c = Client::attach(&daemon.paths.socket, Attach::default()).await;
    c.wait(|c| program(c) == "claude").await;
    prompt_event(&mut c, "Review the API changes\nand the docs").await;
    c.wait(|c| panel_name(c) == "Review the API changes").await;
    // A title that is only the product name does not replace it.
    set_title(&mut c, "claude", "T1").await;
    assert_eq!(panel_name(&c), "Review the API changes");
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_program_follows_the_foreground_process_and_a_failed_read_keeps_it() {
    let (daemon, fg) = manual(argv(&["/usr/bin/nvim", "a.txt"])).await;
    let mut c = Client::attach(&daemon.paths.socket, Attach::default()).await;
    c.wait(|c| program(c) == "nvim").await;
    assert_eq!(
        panel_name(&c),
        "nvim",
        "no title: the program names the panel"
    );

    // The process cannot be read (it belongs to another user, or exited meanwhile).
    *fg.lock().unwrap() = None;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut control = Client::control(&daemon.paths.socket).await;
    let Response::Tree(tree) = control.request(Request::Tree { session: None }).await else {
        panic!("a tree")
    };
    let terminal = &tree[0].projects[0].panels[0].terminals[0];
    assert_eq!(terminal.program, "nvim", "the last program is kept");
    assert_eq!(tree[0].projects[0].panels[0].name, "nvim");

    // It reads again: the shell is back, and the panel with it.
    *fg.lock().unwrap() = argv(&["-sh"]);
    c.wait(|c| program(c) == "sh" && panel_name(c) == "sh")
        .await;
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_manual_name_is_locked_until_auto_name_panel() {
    let daemon = TestDaemon::start().await;
    let mut c = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let panel = c.view.as_ref().unwrap().projects[0].panels[0].id;
    set_title(&mut c, "first", "T1").await;
    c.wait(|c| panel_name(c) == "first").await;

    c.action(Action::RenamePanel {
        panel,
        name: "api".into(),
    })
    .await
    .unwrap();
    c.wait(|c| panel_name(c) == "api").await;
    assert_eq!(name_source(&c), NameSource::Manual);
    set_title(&mut c, "second", "T2").await;
    assert_eq!(panel_name(&c), "api", "a new title does not change it");

    c.action(Action::AutoNamePanel { panel }).await.unwrap();
    c.wait(|c| panel_name(c) == "second").await;
    assert_eq!(name_source(&c), NameSource::Auto);

    // The control request does the same.
    c.action(Action::RenamePanel {
        panel,
        name: "api".into(),
    })
    .await
    .unwrap();
    let mut control = Client::control(&daemon.paths.socket).await;
    assert_eq!(
        control.request(Request::AutoNamePanel { panel }).await,
        Response::Ok
    );
    c.wait(|c| panel_name(c) == "second").await;
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_title_that_changes_only_its_spinner_sends_no_view() {
    let (daemon, _fg) = live(argv(&["claude"])).await;
    let mut c = Client::attach(&daemon.paths.socket, Attach::default()).await;
    c.wait(|c| program(c) == "claude").await;
    // Hooks fix the state (idle), so no tick of the detection moves it while we count.
    let terminal = c.terminal();
    c.request(Request::AgentEvent {
        terminal,
        kind: AgentKind::Claude,
        status: Some(AgentStatus::Idle),
        session_id: Some("s1".into()),
        prompt: None,
        raw: None,
    })
    .await;
    set_title(&mut c, &format!("{SPIN1} Fix login bug"), "T1").await;
    c.wait(|c| panel_name(c) == "Fix login bug").await;

    let before = c.views().count();
    set_title(&mut c, &format!("{SPIN2} Fix login bug"), "T2").await;
    set_title(&mut c, &format!("{SPIN3} Fix login bug"), "T3").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    c.recv(Duration::from_millis(50)).await;
    assert_eq!(c.views().count(), before, "no view for a spinner");
    assert_eq!(panel_name(&c), "Fix login bug");

    // A real change still arrives.
    set_title(&mut c, &format!("{SPIN3} Fix the build"), "T4").await;
    c.wait(|c| panel_name(c) == "Fix the build").await;
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_read_keeps_the_agent_and_a_later_good_read_clears_it() {
    let (daemon, fg) = manual(argv(&["claude"])).await;
    let mut c = Client::attach(&daemon.paths.socket, Attach::default()).await;
    c.wait(|c| program(c) == "claude").await;
    let terminal = c.terminal();
    c.wait(|c| {
        c.view.as_ref().unwrap().projects[0].panels[0].terminals[0]
            .agent
            .is_some()
    })
    .await;

    *fg.lock().unwrap() = None;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut control = Client::control(&daemon.paths.socket).await;
    let Response::Tree(tree) = control.request(Request::Tree { session: None }).await else {
        panic!("a tree")
    };
    let t = &tree[0].projects[0].panels[0].terminals[0];
    assert_eq!(t.id, terminal);
    assert!(t.agent.is_some(), "the agent stays through a failed read");
    assert_eq!(t.program, "claude");

    // The agent really leaves: the shell is the foreground process again.
    *fg.lock().unwrap() = argv(&["-sh"]);
    c.wait(|c| {
        c.view.as_ref().unwrap().projects[0].panels[0].terminals[0]
            .agent
            .is_none()
    })
    .await;
    daemon.stop().await.unwrap();
}
