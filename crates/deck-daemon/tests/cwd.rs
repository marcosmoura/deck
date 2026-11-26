//! Where each shell is: OSC 7, the operating system's answer, precedence, saving, restoring
//! and the directory of new terminals.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use deck_core::action::Action;
use deck_core::model::TerminalId;
use deck_core::state::StateFile;
use deck_daemon::{CwdQuery, Options, process_cwd};
use support::{Attach, Client, TestDaemon};

const POLL: Duration = Duration::from_millis(100);

fn workdir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("pw")
        .tempdir_in("/tmp")
        .unwrap()
}

fn canon(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap()
}

fn polling() -> Options {
    Options {
        cwd_poll: POLL,
        ..Options::default()
    }
}

fn counting(answer: Option<PathBuf>) -> (Options, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&calls);
    let options = Options {
        cwd_poll: POLL,
        cwd_query: CwdQuery::new(move |_| {
            seen.fetch_add(1, Ordering::SeqCst);
            answer.clone()
        }),
        ..Options::default()
    };
    (options, calls)
}

fn view_cwd(client: &Client, terminal: TerminalId) -> Option<PathBuf> {
    client
        .view
        .as_ref()?
        .projects
        .iter()
        .flat_map(|p| &p.panels)
        .flat_map(|p| &p.terminals)
        .find(|t| t.id == terminal)?
        .cwd
        .clone()
}

async fn wait_view_cwd(client: &mut Client, terminal: TerminalId, expected: &Path) {
    client
        .wait(|c| view_cwd(c, terminal).as_deref() == Some(expected))
        .await;
}

fn read_state(daemon: &TestDaemon) -> Option<StateFile> {
    serde_json::from_slice(&fs::read(daemon.paths.state_dir.join("state.json")).ok()?).ok()
}

async fn wait_saved_cwd(daemon: &TestDaemon, expected: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let saved = read_state(daemon)
            .and_then(|f| f.sessions[0].projects[0].panels[0].terminals[0].cwd.clone());
        if saved.as_deref() == Some(expected) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{expected:?} was not saved, saved {saved:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// The shell reports its directory with OSC 7 from its "prompt".
async fn report(client: &mut Client) {
    client
        .type_line(r#"printf '\033]7;file://localhost%s\007' "$PWD""#)
        .await;
}

async fn reattach(daemon: &TestDaemon) -> Client {
    Client::attach(
        &daemon.paths.socket,
        Attach {
            session: Some("main".into()),
            create: false,
            ..Attach::default()
        },
    )
    .await
}

/// Checks the shell's directory without printing it: a long path wraps in a narrow grid.
async fn expect_pwd(client: &mut Client, expected: &Path) {
    let expected = canon(expected);
    client
        .type_line(&format!(
            r#"[ "$(pwd -P)" = "{}" ] && echo in-place-$((6*7))"#,
            expected.display()
        ))
        .await;
    client.wait_row("in-place-42").await;
}

#[test]
fn process_cwd_of_a_child_started_in_a_directory() {
    let dir = workdir();
    let mut child = Command::new("sleep")
        .arg("5")
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let found = process_cwd(child.id());
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(found, Some(canon(dir.path())));
    assert_eq!(process_cwd(i32::MAX as u32), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn osc7_sets_the_cwd_wins_over_the_os_and_is_restored() {
    let dir = workdir();
    let reported = dir.path().to_path_buf();
    let (options, calls) = counting(Some(PathBuf::from("/os-says-this")));
    let daemon = TestDaemon::start_with(support::config(), options).await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let tid = client.terminal();

    // The OS answer comes first, and OSC 7 replaces it.
    wait_view_cwd(&mut client, tid, Path::new("/os-says-this")).await;
    client
        .type_line(&format!("cd {} && true", reported.display()))
        .await;
    report(&mut client).await;
    wait_view_cwd(&mut client, tid, &reported).await;
    wait_saved_cwd(&daemon, &reported).await;

    // The OS is still asked, and its answer, which does not change, never replaces the report.
    let before = calls.load(Ordering::SeqCst);
    let deadline = Instant::now() + Duration::from_secs(5);
    while calls.load(Ordering::SeqCst) < before + 6 {
        assert!(Instant::now() < deadline, "the OS was not asked again");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(view_cwd(&client, tid).as_deref(), Some(reported.as_path()));
    wait_saved_cwd(&daemon, &reported).await;
    drop(client);

    let home = daemon.stop_keep().await;
    let daemon = TestDaemon::start_on(home).await;
    let mut client = reattach(&daemon).await;
    expect_pwd(&mut client, &reported).await;
    daemon.stop().await.unwrap();
}

/// A daemon whose directory lookup answers what the test last put in the returned cell.
fn answering(first: &str) -> (Options, Arc<Mutex<PathBuf>>) {
    let answer = Arc::new(Mutex::new(PathBuf::from(first)));
    let asked = Arc::clone(&answer);
    let options = Options {
        cwd_poll: POLL,
        cwd_query: CwdQuery::new(move |_| Some(asked.lock().unwrap().clone())),
        ..Options::default()
    };
    (options, answer)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_report_no_shell_sent_holds_only_until_the_shell_moves() {
    let (options, os) = answering("/os-first");
    let daemon = TestDaemon::start_with(support::config(), options).await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let tid = client.terminal();
    wait_view_cwd(&mut client, tid, Path::new("/os-first")).await;

    // Some output carries an OSC 7: it is believed while the shell stays where it is.
    client
        .type_line(r"printf '\033]7;file://localhost/not-the-shell\007'")
        .await;
    wait_view_cwd(&mut client, tid, Path::new("/not-the-shell")).await;
    tokio::time::sleep(POLL * 5).await;
    assert_eq!(
        view_cwd(&client, tid).as_deref(),
        Some(Path::new("/not-the-shell"))
    );

    // The shell moves and reports nothing: the report was not its own, and the operating
    // system is believed again, also for the moves that follow.
    *os.lock().unwrap() = PathBuf::from("/os-second");
    wait_view_cwd(&mut client, tid, Path::new("/os-second")).await;
    *os.lock().unwrap() = PathBuf::from("/os-third");
    wait_view_cwd(&mut client, tid, Path::new("/os-third")).await;

    // A shell that reports each move keeps its own spelling of the directory.
    *os.lock().unwrap() = PathBuf::from("/private/os-fourth");
    client
        .type_line(r"printf '\033]7;file://localhost/os-fourth\007'")
        .await;
    wait_view_cwd(&mut client, tid, Path::new("/os-fourth")).await;
    tokio::time::sleep(POLL * 5).await;
    assert_eq!(
        view_cwd(&client, tid).as_deref(),
        Some(Path::new("/os-fourth"))
    );
    drop(client);
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_burst_of_reports_sends_few_views() {
    let options = Options {
        cwd_query: CwdQuery::new(|_| None),
        ..Options::default()
    };
    let daemon = TestDaemon::start_with(support::config(), options).await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let tid = client.terminal();
    client.wait(|c| c.view.is_some()).await;
    let before = client.views().count();

    // 200 directories in one go, far inside one interval of the view throttle.
    client
        .type_line(
            r"i=0; while [ $i -lt 200 ]; do printf '\033]7;file://localhost/burst/%s\007' $i; i=$((i+1)); done",
        )
        .await;
    wait_view_cwd(&mut client, tid, Path::new("/burst/199")).await;
    let views = client.views().count() - before;
    assert!(views <= 20, "{views} views for one burst");
    drop(client);
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_os_poll_finds_a_cd_ignores_subshells_and_is_restored() {
    let dir = workdir();
    let daemon = TestDaemon::start_with(support::config(), polling()).await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let tid = client.terminal();

    client
        .type_line(&format!("cd {}", dir.path().display()))
        .await;
    let expected = canon(dir.path());
    wait_view_cwd(&mut client, tid, &expected).await;
    wait_saved_cwd(&daemon, &expected).await;

    // A foreground job in another directory is not where the shell is.
    client
        .type_line("(cd /var && sleep 1); echo ran-$((1+1))")
        .await;
    client.wait_row("ran-2").await;
    assert_eq!(view_cwd(&client, tid), Some(expected.clone()));
    drop(client);

    let home = daemon.stop_keep().await;
    let daemon = TestDaemon::start_on(home).await;
    let mut client = reattach(&daemon).await;
    expect_pwd(&mut client, &expected).await;
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tick_without_a_change_does_not_save() {
    let dir = workdir();
    let answer = canon(dir.path());
    let (options, calls) = counting(Some(answer.clone()));
    let daemon = TestDaemon::start_with(support::config(), options).await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let tid = client.terminal();
    wait_view_cwd(&mut client, tid, &answer).await;
    wait_saved_cwd(&daemon, &answer).await;

    let file = daemon.paths.state_dir.join("state.json");
    let stamp = |f: &Path| {
        let m = fs::metadata(f).unwrap();
        (m.ino(), m.mtime(), m.mtime_nsec())
    };
    let before = stamp(&file);
    let ticks = calls.load(Ordering::SeqCst) + 4;
    while calls.load(Ordering::SeqCst) < ticks {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // Longer than the save window: a save caused by a tick would have happened.
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(stamp(&file), before, "the file was not rewritten");
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn frames_keep_coming_while_a_query_is_slow() {
    let entered = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let (e, r) = (Arc::clone(&entered), Arc::clone(&release));
    let options = Options {
        cwd_poll: POLL,
        cwd_query: CwdQuery::new(move |_| {
            e.store(true, Ordering::SeqCst);
            while !r.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(10));
            }
            None
        }),
        ..Options::default()
    };
    let daemon = TestDaemon::start_with(support::config(), options).await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    while !entered.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    client.type_line("echo during-$((20+22))").await;
    client.wait_row("during-42").await;
    assert!(
        !release.load(Ordering::SeqCst),
        "the query was still blocked"
    );
    release.store(true, Ordering::SeqCst);
    drop(client);
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_terminal_starts_where_the_focused_one_is() {
    let dir = workdir();
    let daemon = TestDaemon::start_with(support::config(), polling()).await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let first = client.terminal();
    client
        .type_line(&format!("cd {}", dir.path().display()))
        .await;
    wait_view_cwd(&mut client, first, &canon(dir.path())).await;

    client
        .action(Action::NewTerminal { panel: None })
        .await
        .unwrap();
    client.wait(|c| c.active_terminal() != first).await;
    expect_pwd(&mut client, dir.path()).await;
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_terminal_falls_back_when_the_directory_is_gone() {
    let dir = workdir();
    let gone = dir.path().join("gone");
    fs::create_dir(&gone).unwrap();
    let daemon = TestDaemon::start_with(support::config(), polling()).await;
    let root = std::env::temp_dir();
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let first = client.terminal();
    client.type_line(&format!("cd {}", gone.display())).await;
    wait_view_cwd(&mut client, first, &canon(&gone)).await;
    fs::remove_dir(&gone).unwrap();

    client
        .action(Action::NewTerminal { panel: None })
        .await
        .unwrap();
    client.wait(|c| c.active_terminal() != first).await;
    expect_pwd(&mut client, &root).await;
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_terminal_falls_back_to_the_launch_directory_before_the_root() {
    // The project root is the attach directory; the first terminal starts elsewhere.
    let root = workdir();
    let launch = workdir();
    let home = TestDaemon::new_home();
    let state = format!(
        r#"{{"version":1,"next_id":100,"last_attached":[],"sessions":[{{"id":1,"name":"main","active_project":2,"projects":[{{"id":2,"name":"p","root":{:?},"collapsed":false,"active_panel":3,"panels":[{{"id":3,"name":"1","name_source":"Default","focused_terminal":4,"ratios":{{"main":null,"cols":[1000],"rows":[1000]}},"terminals":[{{"id":4,"program":null,"args":[],"env":[],"cwd":{:?}}}]}}]}}]}}]}}"#,
        root.path().display().to_string(),
        launch.path().display().to_string()
    );
    fs::create_dir_all(home.path().join("state")).unwrap();
    fs::write(home.path().join("state/state.json"), state).unwrap();
    // No lookup answers, so the terminal's live cwd stays unknown.
    let options = Options {
        cwd_query: CwdQuery::new(|_| None),
        ..Options::default()
    };
    let daemon = TestDaemon::start_on_with(home, options).await;
    let mut client = reattach(&daemon).await;
    let first = client.terminal();
    expect_pwd(&mut client, launch.path()).await;
    client
        .action(Action::NewTerminal { panel: None })
        .await
        .unwrap();
    client.wait(|c| c.active_terminal() != first).await;
    expect_pwd(&mut client, launch.path()).await;
    daemon.stop().await.unwrap();
}
