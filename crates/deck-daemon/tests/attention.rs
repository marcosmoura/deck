//! Attention: which agent transitions mark a panel unseen and alert a client, who is
//! alerted, and what clears the mark.

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
use deck_core::model::{PanelId, TerminalId};
use deck_core::protocol::{ClientMsg, Request, Response, ServerMsg, Target};
use deck_daemon::{ForegroundQuery, ManualClock, Options};
use support::{Attach, Client, TestDaemon};

async fn pause() {
    tokio::time::sleep(Duration::from_millis(250)).await;
}

/// A daemon that finds no agent by process, so only hook events set states.
async fn daemon() -> TestDaemon {
    TestDaemon::start_with(support::config(), Options::default()).await
}

async fn event(client: &mut Client, terminal: TerminalId, status: AgentStatus) {
    let r = client
        .request(Request::AgentEvent {
            terminal,
            kind: AgentKind::Claude,
            status: Some(status),
            session_id: None,
            prompt: None,
            raw: None,
        })
        .await;
    assert_eq!(r, Response::Ok);
}

/// The alerts received so far: status, project and panel names.
fn alerts(client: &Client) -> Vec<(AgentStatus, String, String)> {
    client
        .log
        .iter()
        .filter_map(|m| match m {
            ServerMsg::AgentAlert {
                status,
                project,
                panel,
                ..
            } => Some((*status, project.clone(), panel.clone())),
            _ => None,
        })
        .collect()
}

fn unseen(client: &Client, panel: PanelId) -> bool {
    client
        .view
        .as_ref()
        .unwrap()
        .projects
        .iter()
        .flat_map(|p| &p.panels)
        .find(|p| p.id == panel)
        .unwrap()
        .unseen
}

fn any_unseen(client: &Client) -> bool {
    client
        .view
        .as_ref()
        .unwrap()
        .projects
        .iter()
        .flat_map(|p| &p.panels)
        .any(|p| p.unseen)
}

/// Takes in every message the daemon has sent so far.
async fn drain(client: &mut Client) {
    while client.recv(Duration::from_millis(50)).await.is_some() {}
}

/// Adds a panel named `other` that stays inactive; returns it and its terminal.
async fn inactive_panel(client: &mut Client) -> (PanelId, TerminalId) {
    let view = client.view.clone().unwrap();
    let r = client
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
    client
        .wait(|c| c.view.as_ref().unwrap().projects[0].panels.len() == 2)
        .await;
    let terminal = client.view.as_ref().unwrap().projects[0]
        .panels
        .iter()
        .find(|p| p.id == panel)
        .unwrap()
        .terminals[0]
        .id;
    (panel, terminal)
}

#[tokio::test]
async fn a_transition_in_an_inactive_panel_marks_it_unseen_and_alerts_once() {
    let daemon = daemon().await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let (panel, terminal) = inactive_panel(&mut client).await;

    event(&mut client, terminal, AgentStatus::Working).await;
    pause().await;
    client.recv(Duration::from_millis(50)).await;
    assert!(alerts(&client).is_empty() && !unseen(&client, panel));

    event(&mut client, terminal, AgentStatus::Waiting).await;
    client.wait(|c| unseen(c, panel)).await;
    let project = client.view.as_ref().unwrap().projects[0].name.clone();
    assert_eq!(
        alerts(&client),
        [(AgentStatus::Waiting, project.clone(), "other".to_string())]
    );

    // Focusing the panel in a focused client clears the mark.
    client.action(Action::FocusPanel { panel }).await.unwrap();
    client.wait(|c| !unseen(c, panel)).await;

    // Finishing a turn alerts too, with the status that came.
    event(&mut client, terminal, AgentStatus::Working).await;
    let first = client.view.as_ref().unwrap().projects[0].panels[0].id;
    client
        .action(Action::FocusPanel { panel: first })
        .await
        .unwrap();
    event(&mut client, terminal, AgentStatus::Idle).await;
    client.wait(|c| unseen(c, panel)).await;
    assert_eq!(alerts(&client).len(), 2);
    assert_eq!(alerts(&client)[1].0, AgentStatus::Idle);

    // Only a `working` agent raises attention: idle to waiting does not.
    client.action(Action::FocusPanel { panel }).await.unwrap();
    client.wait(|c| !unseen(c, panel)).await;
    client
        .action(Action::FocusPanel { panel: first })
        .await
        .unwrap();
    event(&mut client, terminal, AgentStatus::Waiting).await;
    pause().await;
    client.recv(Duration::from_millis(50)).await;
    assert!(!unseen(&client, panel));
    assert_eq!(alerts(&client).len(), 2);
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn the_active_panel_of_a_focused_client_never_alerts_until_it_loses_focus() {
    let daemon = daemon().await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let terminal = client.terminal();
    let panel = client.view.as_ref().unwrap().projects[0].panels[0].id;

    // A client that never reported counts as focused.
    event(&mut client, terminal, AgentStatus::Working).await;
    event(&mut client, terminal, AgentStatus::Waiting).await;
    pause().await;
    client.recv(Duration::from_millis(50)).await;
    assert!(alerts(&client).is_empty() && !any_unseen(&client));

    client.send(&ClientMsg::Focus(false)).await;
    event(&mut client, terminal, AgentStatus::Working).await;
    event(&mut client, terminal, AgentStatus::Waiting).await;
    client.wait(|c| unseen(c, panel)).await;
    assert_eq!(alerts(&client).len(), 1);

    // Focus returns: the panel in view is seen.
    client.send(&ClientMsg::Focus(true)).await;
    client.wait(|c| !unseen(c, panel)).await;
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn one_alert_goes_to_the_client_that_was_active_last() {
    let daemon = daemon().await;
    let mut a = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let name = a.view.as_ref().unwrap().name.clone();
    let mut b = Client::attach(
        &daemon.paths.socket,
        Attach {
            session: Some(name),
            create: false,
            ..Attach::default()
        },
    )
    .await;
    let (panel, terminal) = inactive_panel(&mut a).await;
    b.wait(|c| c.view.as_ref().unwrap().projects[0].panels.len() == 2)
        .await;

    // `b` types last.
    let typed = b.terminal();
    b.send(&ClientMsg::Input {
        terminal: typed,
        bytes: b"x".to_vec(),
    })
    .await;
    pause().await;

    event(&mut a, terminal, AgentStatus::Working).await;
    event(&mut a, terminal, AgentStatus::Idle).await;
    b.wait(|c| unseen(c, panel)).await;
    a.wait(|c| unseen(c, panel)).await;
    pause().await;
    a.recv(Duration::from_millis(50)).await;
    b.recv(Duration::from_millis(50)).await;
    assert_eq!(alerts(&b).len(), 1);
    assert!(alerts(&a).is_empty());
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn process_only_changes_never_mark_or_alert() {
    let clock = Arc::new(ManualClock::new());
    let foreground = Arc::new(Mutex::new(Some(vec!["claude".to_string()])));
    let seen = Arc::clone(&foreground);
    let options = Options {
        clock: clock.clone(),
        agent_poll: Duration::from_millis(20),
        foreground_query: ForegroundQuery::new(move |_| seen.lock().unwrap().clone()),
        ..Options::default()
    };
    let daemon = TestDaemon::start_with(support::config(), options).await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    inactive_panel(&mut client).await;
    client
        .wait(|c| {
            c.view.as_ref().unwrap().projects[0].panels[1].terminals[0]
                .agent
                .as_ref()
                .is_some_and(|a| a.status == AgentStatus::Active)
        })
        .await;
    // It stops printing: active to quiet. The shell's first prompt in the new terminal can
    // still arrive after the clock moved and stamp that later time, which makes the agent
    // active again; so the clock keeps moving until the state is quiet.
    let quiet = |c: &Client| {
        c.view.as_ref().unwrap().projects[0].panels[1].terminals[0]
            .agent
            .as_ref()
            .is_some_and(|a| a.status == AgentStatus::Quiet)
    };
    let deadline = std::time::Instant::now() + support::TIMEOUT;
    while !quiet(&client) {
        assert!(std::time::Instant::now() < deadline, "never quiet");
        clock.advance(Duration::from_secs(3));
        client.recv(Duration::from_millis(100)).await;
    }
    pause().await;
    client.recv(Duration::from_millis(50)).await;
    assert!(alerts(&client).is_empty() && !any_unseen(&client));
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn a_command_makes_its_terminal_busy_and_its_end_marks_the_panel_unseen() {
    let clock = Arc::new(ManualClock::new());
    // `(own, argv)`: the shell itself, or a command it started.
    let foreground = Arc::new(Mutex::new(Some((true, vec!["zsh".to_string()]))));
    let seen = Arc::clone(&foreground);
    let options = Options {
        clock: clock.clone(),
        agent_poll: Duration::from_millis(20),
        foreground_query: ForegroundQuery::with_owner(move |_| seen.lock().unwrap().clone()),
        ..Options::default()
    };
    let daemon = TestDaemon::start_with(support::config(), options).await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let (panel, terminal) = inactive_panel(&mut client).await;
    let busy = |c: &Client| {
        c.view
            .as_ref()
            .unwrap()
            .projects
            .iter()
            .flat_map(|p| &p.panels)
            .flat_map(|p| &p.terminals)
            .any(|t| t.id == terminal && t.busy)
    };
    *foreground.lock().unwrap() = Some((false, vec!["cargo".into(), "build".into()]));
    // Not before it has run for a second.
    pause().await;
    drain(&mut client).await;
    assert!(!busy(&client));
    clock.advance(Duration::from_secs(1));
    client.wait(busy).await;
    // It ends while nobody looks at its panel.
    *foreground.lock().unwrap() = Some((true, vec!["zsh".to_string()]));
    client.wait(|c| !busy(c)).await;
    client.wait(|c| unseen(c, panel)).await;
    assert!(alerts(&client).is_empty());
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn interactive_and_full_screen_programs_are_never_busy() {
    let clock = Arc::new(ManualClock::new());
    let foreground = Arc::new(Mutex::new(Some((
        false,
        vec!["ssh".to_string(), "host".into()],
    ))));
    let seen = Arc::clone(&foreground);
    let options = Options {
        clock: clock.clone(),
        agent_poll: Duration::from_millis(20),
        foreground_query: ForegroundQuery::with_owner(move |_| seen.lock().unwrap().clone()),
        ..Options::default()
    };
    let daemon = TestDaemon::start_with(support::config(), options).await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let terminal = client.view.as_ref().unwrap().projects[0].panels[0].terminals[0].id;
    let busy = |c: &Client| c.view.as_ref().unwrap().projects[0].panels[0].terminals[0].busy;
    for _ in 0..5 {
        clock.advance(Duration::from_secs(1));
        pause().await;
    }
    drain(&mut client).await;
    // `ssh` is a session the user works in.
    assert!(!busy(&client));
    // Then a full-screen program: the shell's terminal switches to the alternate screen.
    *foreground.lock().unwrap() = Some((true, vec!["zsh".to_string()]));
    client
        .send(&ClientMsg::Input {
            terminal,
            bytes: b"printf '\\033[?1049h'; echo on-alt; sleep 100\r".to_vec(),
        })
        .await;
    // On the alternate screen before `sleep` shows in the foreground: a fixed pause is not
    // always long enough, and `sleep` on the main screen is a command.
    client
        .wait(|c| {
            c.screens
                .get(&terminal)
                .is_some_and(|rows| rows.iter().any(|r| r.trim() == "on-alt"))
        })
        .await;
    *foreground.lock().unwrap() = Some((false, vec!["sleep".to_string(), "100".into()]));
    for _ in 0..5 {
        clock.advance(Duration::from_secs(1));
        pause().await;
    }
    drain(&mut client).await;
    assert!(!busy(&client));
    daemon.stop().await.unwrap();
}
