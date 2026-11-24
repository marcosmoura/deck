//! What the emulator makes of program output: characters, colour queries, the clipboard.

mod support;

use std::sync::Arc;
use std::time::Duration;

use deck_core::protocol::{OuterColors, ServerMsg};
use deck_daemon::{ManualClock, Options};
use support::{Attach, Client, TestDaemon, config};

#[tokio::test(flavor = "multi_thread")]
async fn wide_and_combining_characters_come_out_as_runs_at_the_right_columns() {
    let daemon = TestDaemon::start().await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    // 日本 🙂 e + combining acute, x (octal escapes keep the shell portable).
    client
        .type_line("printf '\\346\\227\\245\\346\\234\\254 \\360\\237\\231\\202 e\\314\\201x\\n'")
        .await;
    client
        .wait(|c| {
            c.frames().any(|f| {
                f.lines.iter().any(|(_, l)| {
                    l.runs
                        .iter()
                        .any(|r| r.col == 0 && r.text == "日本 🙂 e\u{301}x")
                })
            })
        })
        .await;
    // Columns: 日 0-1, 本 2-3, space 4, 🙂 5-6, space 7, é 8, x 9. The next prompt's cursor
    // shows where the line ended.
    daemon.stop().await.unwrap();
}

const READ_COLOR: &str = "set +m; stty raw -echo; printf '\\033]11;?\\007'; \
    x=$(dd bs=1 count=24 2>/dev/null | tr -d '\\033\\007'); stty sane; echo \"GOT:$x\"";

#[tokio::test(flavor = "multi_thread")]
async fn a_background_query_is_answered_with_the_clients_colours_or_a_fallback() {
    let daemon = TestDaemon::start().await;
    let mut client = Client::attach(
        &daemon.paths.socket,
        Attach {
            colors: Some(OuterColors {
                fg: (1, 2, 3),
                bg: (16, 32, 48),
            }),
            ..Attach::default()
        },
    )
    .await;
    client.type_line(READ_COLOR).await;
    client.wait_row("GOT:]11;rgb:1010/2020/3030").await;

    let mut plain = Client::attach(
        &daemon.paths.socket,
        Attach {
            session: Some("plain".into()),
            colors: None,
            ..Attach::default()
        },
    )
    .await;
    plain.type_line(READ_COLOR).await;
    plain.wait_row("GOT:]11;rgb:0000/0000/0000").await;
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_clipboard_copy_reaches_only_the_most_recently_active_client() {
    let daemon = TestDaemon::start().await;
    let mut older = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let mut newer = Client::attach(&daemon.paths.socket, Attach::default()).await;
    // `newer` types, so it is the most recently active.
    newer
        .type_line("printf '\\033]52;c;aGk=\\007'; echo COPIED")
        .await;
    newer.wait_row("COPIED").await;
    let copies = |c: &Client| -> Vec<(String, String)> {
        c.log
            .iter()
            .filter_map(|m| match m {
                ServerMsg::Clipboard { selection, text } => Some((selection.clone(), text.clone())),
                _ => None,
            })
            .collect()
    };
    // The frame that shows `COPIED` can come before the copy.
    newer.wait(|c| !copies(c).is_empty()).await;
    assert_eq!(copies(&newer), [("c".to_string(), "hi".to_string())]);

    // A read request gets no answer: nothing arrives on the program's input.
    newer
        .type_line(
            "set +m; printf '\\033]52;c;?\\007'; stty -icanon min 0 time 5; \
             n=$(dd bs=1 count=1 2>/dev/null | wc -c); stty sane; echo N=$((n))",
        )
        .await;
    newer.wait_row("N=0").await;

    while older.recv(Duration::from_millis(100)).await.is_some() {}
    assert!(
        !older
            .log
            .iter()
            .any(|m| matches!(m, ServerMsg::Clipboard { .. })),
        "the other client got the copy"
    );
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn synchronized_output_holds_the_frame_until_its_timeout_on_the_daemons_clock() {
    let clock = Arc::new(ManualClock::new());
    let daemon = TestDaemon::start_with(
        config(),
        Options {
            clock: clock.clone(),
            ..Options::default()
        },
    )
    .await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    client.wait_text("$").await;
    // Set the mode and print a number; the echoed command line has no such number.
    client
        .type_line("printf '\\033[?2026h%s\\n' $((40+2)); sleep 30")
        .await;
    // No frame carries the output while the mode is set: the clock does not move.
    let quiet = tokio::time::Instant::now() + Duration::from_millis(600);
    while let Some(left) = quiet.checked_duration_since(tokio::time::Instant::now()) {
        client.recv(left).await;
    }
    assert!(!client.has_row("42"), "{:?}", client.screen());
    // 149 ms later it is still held; at 150 ms the frame goes out with the output.
    clock.advance(Duration::from_millis(149));
    let quiet = tokio::time::Instant::now() + Duration::from_millis(300);
    while let Some(left) = quiet.checked_duration_since(tokio::time::Instant::now()) {
        client.recv(left).await;
    }
    assert!(!client.has_row("42"), "{:?}", client.screen());
    clock.advance(Duration::from_millis(1));
    client.wait_row("42").await;
    daemon.stop().await.unwrap();
}

/// Asks XTVERSION and reads the answer (19 bytes for `ghostty 1.2.0`), waiting at most 1 s.
const READ_VERSION: &str = "set +m; stty raw -echo min 0 time 10; printf '\\033[>q'; \
    x=$(dd bs=1 count=19 2>/dev/null | tr -d '\\033'); stty sane; echo \"N=${#x}:$x\"";

#[tokio::test(flavor = "multi_thread")]
async fn xtversion_is_answered_with_the_name_of_the_followed_client_when_it_shows_images() {
    let daemon = TestDaemon::start().await;
    let mut shows = Client::attach(
        &daemon.paths.socket,
        Attach {
            images: true,
            outer_name: Some("ghostty 1.2.0".into()),
            ..Attach::default()
        },
    )
    .await;
    shows.type_line(READ_VERSION).await;
    shows.wait_row("N=17:P>|ghostty 1.2.0\\").await;

    // Another client of the session, without images, types and is the one followed: no answer.
    let mut plain = Client::attach(
        &daemon.paths.socket,
        Attach {
            outer_name: Some("ghostty 1.2.0".into()),
            ..Attach::default()
        },
    )
    .await;
    plain.type_line(READ_VERSION).await;
    plain.wait_row("N=0:").await;

    // The first client types again: it is followed again.
    shows.type_line(READ_VERSION).await;
    shows
        .wait(|c| {
            c.screen()
                .iter()
                .filter(|r| r.contains("N=17:P>|ghostty 1.2.0\\"))
                .count()
                >= 2
        })
        .await;

    // An image client without a name: no answer either.
    let mut nameless = Client::attach(
        &daemon.paths.socket,
        Attach {
            images: true,
            ..Attach::default()
        },
    )
    .await;
    nameless.type_line(READ_VERSION).await;
    nameless.wait_row("N=0:").await;
    daemon.stop().await.unwrap();
}
