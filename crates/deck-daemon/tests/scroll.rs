//! Scrolling a terminal's display through the protocol.

mod support;

use std::time::Duration;

use deck_core::model::TerminalId;
use deck_core::protocol::{ClientMsg, Frame, Modes, ScrollTo};
use support::{Attach, Client, TestDaemon};

fn last_frame(c: &Client) -> Frame {
    c.frames().last().expect("a frame").clone()
}

async fn scroll(c: &mut Client, to: ScrollTo) {
    let terminal = c.terminal();
    c.send(&ClientMsg::Scroll { terminal, to }).await;
}

/// A client whose terminal has printed 1 to 100.
async fn with_history(daemon: &TestDaemon) -> Client {
    let mut c = Client::attach(&daemon.paths.socket, Attach::default()).await;
    c.type_line("seq 1 100").await;
    c.wait_row("100").await;
    c
}

#[tokio::test(flavor = "multi_thread")]
async fn scrolling_is_clamped_and_every_answer_carries_the_offset_and_history() {
    let daemon = TestDaemon::start().await;
    let mut c = with_history(&daemon).await;
    let live = last_frame(&c);
    assert_eq!(live.display_offset, 0);
    assert!(live.history > 70, "{}", live.history);

    scroll(&mut c, ScrollTo::Lines(5)).await;
    c.wait(|c| last_frame(c).display_offset == 5).await;
    let f = last_frame(&c);
    assert!(f.full);
    assert_eq!(f.history, live.history);

    scroll(&mut c, ScrollTo::Pages(1)).await;
    c.wait(|c| last_frame(c).display_offset == 5 + 24).await;

    scroll(&mut c, ScrollTo::Lines(i32::MAX)).await;
    c.wait(|c| last_frame(c).display_offset == live.history)
        .await;
    scroll(&mut c, ScrollTo::Lines(-5)).await;
    c.wait(|c| last_frame(c).display_offset == live.history - 5)
        .await;
    scroll(&mut c, ScrollTo::Lines(i32::MIN)).await;
    c.wait(|c| last_frame(c).display_offset == 0).await;
    scroll(&mut c, ScrollTo::Top).await;
    c.wait(|c| last_frame(c).display_offset == live.history)
        .await;
    scroll(&mut c, ScrollTo::Bottom).await;
    c.wait(|c| last_frame(c).display_offset == 0).await;
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn input_returns_a_scrolled_terminal_to_the_bottom() {
    let daemon = TestDaemon::start().await;
    let mut c = with_history(&daemon).await;
    scroll(&mut c, ScrollTo::Top).await;
    c.wait(|c| last_frame(c).display_offset > 0).await;
    c.type_line("echo back").await;
    c.wait(|c| last_frame(c).display_offset == 0).await;
    c.wait_row("back").await;
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn output_while_scrolled_back_grows_the_history_in_a_frame() {
    let daemon = TestDaemon::start().await;
    let mut c = with_history(&daemon).await;
    c.type_line("(sleep 1; seq 1 50) &").await;
    c.wait_text("[1]").await;
    scroll(&mut c, ScrollTo::Lines(10)).await;
    c.wait(|c| last_frame(c).display_offset == 10).await;
    let before = last_frame(&c);
    c.wait(|c| {
        let f = last_frame(c);
        f.history > before.history && f.display_offset > 0
    })
    .await;
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_alternate_scroll_flag_follows_the_program() {
    let daemon = TestDaemon::start().await;
    let mut c = Client::attach(&daemon.paths.socket, Attach::default()).await;
    assert!(last_frame(&c).modes.has(Modes::ALTERNATE_SCROLL));
    c.type_line("printf '\\033[?1007l'").await;
    c.wait(|c| !last_frame(c).modes.has(Modes::ALTERNATE_SCROLL))
        .await;
    c.type_line("printf '\\033[?1007h'").await;
    c.wait(|c| last_frame(c).modes.has(Modes::ALTERNATE_SCROLL))
        .await;
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn scrolling_a_terminal_outside_the_clients_session_is_ignored() {
    let daemon = TestDaemon::start().await;
    let mut a = with_history(&daemon).await;
    let mut b = Client::attach(
        &daemon.paths.socket,
        Attach {
            session: Some("other".into()),
            ..Attach::default()
        },
    )
    .await;
    let theirs = a.terminal();
    assert_ne!(theirs, b.terminal());

    // The other session's client tries to scroll `a`'s terminal, and a terminal that
    // does not exist; `a`'s terminal does not move.
    b.send(&ClientMsg::Scroll {
        terminal: theirs,
        to: ScrollTo::Top,
    })
    .await;
    b.send(&ClientMsg::Scroll {
        terminal: TerminalId(9999),
        to: ScrollTo::Top,
    })
    .await;
    // The messages of one connection are handled in order: this input is after them.
    b.type_line("echo marker").await;
    b.wait_row("marker").await;
    while a.recv(Duration::from_millis(300)).await.is_some() {}
    assert_eq!(last_frame(&a).display_offset, 0);
    assert!(a.frames().all(|f| f.display_offset == 0));
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn focus_reports_do_not_return_a_scrolled_terminal_to_the_bottom() {
    let daemon = TestDaemon::start().await;
    let mut c = with_history(&daemon).await;
    scroll(&mut c, ScrollTo::Lines(10)).await;
    c.wait(|c| last_frame(c).display_offset == 10).await;
    let seen = c.log.len();
    let terminal = c.terminal();
    for bytes in [b"\x1b[I", b"\x1b[O"] {
        c.send(&ClientMsg::Input {
            terminal,
            bytes: bytes.to_vec(),
        })
        .await;
    }
    // The scroll is answered after the inputs were handled, in order.
    scroll(&mut c, ScrollTo::Lines(0)).await;
    c.wait(|c| c.log.len() > seen).await;
    while c.recv(Duration::from_millis(300)).await.is_some() {}
    assert!(
        c.frames()
            .skip_while(|f| f.display_offset != 10)
            .all(|f| f.display_offset == 10)
    );
    assert_eq!(last_frame(&c).display_offset, 10);
    daemon.stop().await.unwrap();
}
