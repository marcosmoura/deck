//! What the emulator makes of program output: characters, colour queries, the clipboard.

mod support;

use std::time::Duration;

use deck_core::protocol::{OuterColors, ServerMsg};
use support::{Attach, Client, TestDaemon};

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

const READ_COLOR: &str = "stty raw -echo; printf '\\033]11;?\\007'; \
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
    let copies: Vec<_> = newer
        .log
        .iter()
        .filter_map(|m| match m {
            ServerMsg::Clipboard { selection, text } => Some((selection.as_str(), text.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(copies, [("c", "hi")]);

    // A read request gets no answer: nothing arrives on the program's input.
    newer
        .type_line(
            "printf '\\033]52;c;?\\007'; stty -icanon min 0 time 5; \
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
