//! The daemon reads and watches `config.toml` and pushes it to its clients.

mod support;

use std::fs;
use std::time::Duration;

use deck_core::config::Config;
use deck_core::protocol::{ClientMsg, NoticeLevel, ServerMsg};
use support::{Attach, Client, TestDaemon};

const POLL: Duration = Duration::from_millis(20);
const SHELL: &str = "shell = \"/bin/sh\"\nshell_args = []\n";

/// Writes the config the way editors save: a temp file renamed over it, so the daemon's poll
/// never sees a half-written (empty, hence valid) file.
#[allow(
    clippy::unwrap_used,
    reason = "integration-test helpers fail the test on error"
)]
fn save(path: &std::path::Path, contents: String) {
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, contents).unwrap();
    fs::rename(&tmp, path).unwrap();
}

fn text(extra: &str) -> String {
    format!("{SHELL}{extra}")
}

async fn watching(extra: &str) -> TestDaemon {
    TestDaemon::start_watching(Some(&text(extra)), POLL).await
}

#[expect(clippy::panic, reason = "a missing Welcome fails the integration test")]
fn welcome_config(client: &Client) -> &Config {
    let Some(ServerMsg::Welcome { config, .. }) = client.log.first() else {
        panic!("Welcome first: {:?}", client.log.first())
    };
    config
}

#[tokio::test(flavor = "multi_thread")]
async fn the_welcome_carries_the_daemons_config_and_the_first_shell_is_sized_for_it() {
    let daemon = watching("sidebar_width = 40\n").await;
    // A client in another config home gets the daemon's config all the same.
    let mut env = support::hello_env();
    env.push(("XDG_CONFIG_HOME".into(), "/nonexistent/xdg".into()));
    let mut client = Client::attach(
        &daemon.paths.socket,
        Attach {
            cols: 100,
            rows: 25,
            env,
            ..Attach::default()
        },
    )
    .await;
    assert_eq!(welcome_config(&client).sidebar_width, 40);
    // Outer 100 - (40 + 1) columns, and the status-bar row off the rows.
    assert_eq!(
        client.view.as_ref().unwrap().layout_viewport,
        Some((59, 24))
    );
    assert!(client.frames().all(|f| f.size == (59, 24)), "one size only");
    client.type_line("stty size").await;
    client.wait_row("24 59").await;
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_written_file_reaches_every_attached_client_with_a_notice() {
    let daemon = watching("").await;
    let mut a = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let mut b = Client::attach(&daemon.paths.socket, Attach::default()).await;
    fs::write(
        daemon.config_file(),
        text("sidebar_width = 33\n[theme]\naccent = \"red\"\n"),
    )
    .unwrap();
    for client in [&mut a, &mut b] {
        client.wait(|c| c.has_notice("config reloaded")).await;
        let [config] = client.configs()[..] else {
            panic!("one config: {:?}", client.configs().len())
        };
        assert_eq!(config.sidebar_width, 33);
        assert_eq!(
            config.theme.accent,
            Config::parse("[theme]\naccent = \"red\"")
                .unwrap()
                .config
                .theme
                .accent
        );
        assert_eq!(client.notices()[0].0, NoticeLevel::Info);
    }
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_save_through_a_rename_and_a_deletion_are_seen() {
    let daemon = watching("").await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let tmp = daemon.home.path().join("config.toml.swp");
    fs::write(&tmp, text("sidebar_width = 31\n")).unwrap();
    fs::rename(&tmp, daemon.config_file()).unwrap();
    client
        .wait(|c| c.configs().last().is_some_and(|c| c.sidebar_width == 31))
        .await;
    fs::remove_file(daemon.config_file()).unwrap();
    client
        .wait(|c| c.configs().last().is_some_and(|c| **c == Config::default()))
        .await;
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_invalid_file_keeps_the_config_and_names_the_line() {
    let daemon = watching("").await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    save(
        &daemon.config_file(),
        text("[theme]\naccent = \"purple\"\n"),
    );
    client.wait(|c| !c.notices().is_empty()).await;
    let [(level, notice)] = client.notices()[..] else {
        panic!("{:?}", client.notices())
    };
    assert_eq!(level, NoticeLevel::Error);
    assert!(
        notice.starts_with("config.toml:4:10: ") && notice.ends_with("(kept the previous config)"),
        "{notice}"
    );
    assert!(client.configs().is_empty(), "nothing changed");
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn two_user_keys_on_one_key_are_an_error_and_a_replaced_default_a_warning() {
    let daemon = watching("").await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    save(
        &daemon.config_file(),
        text("[keys]\ndetach = \"q\"\nscroll = \"q\"\n"),
    );
    client.wait(|c| !c.notices().is_empty()).await;
    assert_eq!(client.notices()[0].0, NoticeLevel::Error);
    assert!(client.configs().is_empty());

    save(&daemon.config_file(), text("[keys]\ndetach = \"n\"\n"));
    client.wait(|c| c.has_notice("replaces")).await;
    let warning = client
        .notices()
        .into_iter()
        .find(|(l, _)| *l == NoticeLevel::Warning)
        .unwrap()
        .1;
    assert!(
        warning.contains("detach") && warning.contains("next_panel"),
        "{warning}"
    );
    assert_eq!(client.configs().len(), 1);
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_invalid_file_at_start_gives_the_defaults_and_the_first_client_the_notice() {
    let daemon = TestDaemon::start_watching(Some("sidebar_width = \"x\"\n"), POLL).await;
    let client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    assert_eq!(*welcome_config(&client), Config::default());
    // The notice follows the Welcome.
    let mut client = client;
    client.wait(|c| !c.notices().is_empty()).await;
    let (level, text) = client.notices()[0];
    assert_eq!(level, NoticeLevel::Error);
    assert!(
        text.starts_with("config.toml:1:17: ") && text.ends_with("(using the defaults)"),
        "{text}"
    );
    // Delivered once: the next client gets none.
    let other = Client::attach(&daemon.paths.socket, Attach::default()).await;
    assert!(other.notices().is_empty());
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn notices_wait_for_the_next_client_the_last_twenty_are_kept_and_repeats_collapse() {
    // One warning at start, then 22 reloads (each with a warning of its own) while nobody
    // is attached.
    let daemon = watching("colour = 1\n").await;
    for i in 0..22 {
        fs::write(
            daemon.config_file(),
            text(&format!("colour{i} = 1\nscrollback_lines = {}\n", 100 + i)),
        )
        .unwrap();
        tokio::time::sleep(POLL * 4).await;
    }
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    client.wait(|c| c.notices().len() >= 20).await;
    tokio::time::sleep(POLL * 4).await;
    let notices = client.notices();
    assert_eq!(notices.len(), 20, "{notices:?}");
    let reloaded = notices
        .iter()
        .filter(|(_, t)| *t == "config reloaded")
        .count();
    assert_eq!(reloaded, 1, "repeats collapse to the newest: {notices:?}");
    assert!(
        !notices.iter().any(|(_, t)| t.contains("`colour`")),
        "the oldest was dropped"
    );
    assert_eq!(welcome_config(&client).scrollback_lines, 121);
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reload_moves_no_pty_to_a_client_that_was_not_active() {
    let daemon = watching("").await;
    let attach = |cols| Attach {
        cols,
        rows: 25,
        ..Attach::default()
    };
    let mut a = Client::attach(&daemon.paths.socket, attach(100)).await;
    let mut b = Client::attach(&daemon.paths.socket, attach(120)).await;
    // b attached last: the shell follows it (120 - 29 columns).
    b.type_line("stty size").await;
    b.wait_row("24 91").await;

    fs::write(daemon.config_file(), text("sidebar_width = 40\n")).unwrap();
    for client in [&mut a, &mut b] {
        client.wait(|c| !c.configs().is_empty()).await;
    }
    // Both apply the new width and say so with a reload viewport.
    a.send(&ClientMsg::ReloadViewport { cols: 59, rows: 24 })
        .await;
    b.type_line("stty size").await;
    b.wait_row("24 91").await;
    assert!(!b.has_row("24 59"), "a did not take the shell over");

    b.send(&ClientMsg::ReloadViewport { cols: 79, rows: 24 })
        .await;
    b.type_line("stty size").await;
    b.wait_row("24 79").await;
    daemon.stop().await.unwrap();
}
