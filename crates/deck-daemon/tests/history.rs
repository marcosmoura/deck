//! Row ids, `ScrollTo::Row`, `Request::Search` and `Request::Text` through the protocol.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use std::path::PathBuf;

use deck_core::config::Config;
use deck_core::model::{LaunchSpec, TerminalId};
use deck_core::protocol::{ClientMsg, Frame, Pos, Request, Response, ScrollTo, WaitFor, Waited};
use deck_daemon::Options;
use support::{Attach, Client, TestDaemon, config};

fn last_frame(c: &Client) -> Frame {
    c.frames().last().expect("a frame").clone()
}

async fn scroll(c: &mut Client, to: ScrollTo) {
    let terminal = c.terminal();
    c.send(&ClientMsg::Scroll { terminal, to }).await;
}

async fn daemon_with(scrollback: usize) -> TestDaemon {
    TestDaemon::start_with(
        Config {
            scrollback_lines: scrollback,
            ..config()
        },
        Options::default(),
    )
    .await
}

fn launch(script: &str) -> Option<LaunchSpec> {
    Some(LaunchSpec {
        program: Some("/bin/sh".into()),
        args: vec!["-c".into(), script.into()],
        cwd: PathBuf::new(),
        env: Vec::new(),
    })
}

/// A terminal that runs `script` and then sleeps, once its last line `done` shows.
async fn printing(c: &mut Client, script: &str) -> TerminalId {
    let panel = c.view.as_ref().unwrap().projects[0].panels[0].id;
    let id = match c
        .request(Request::NewTerminal {
            panel,
            launch: launch(&format!("{script}; echo done; sleep 60")),
            keep: false,
            focus: false,
        })
        .await
    {
        Response::Created { id } => TerminalId(id),
        other => panic!("expected an id, got {other:?}"),
    };
    let waited = c
        .request(Request::Wait {
            terminal: id,
            until: WaitFor::Text("done".into()),
            timeout_ms: Some(20_000),
        })
        .await;
    assert!(
        matches!(waited, Response::Waited(Waited::Text(_))),
        "{waited:?}"
    );
    id
}

/// The end of the history: before the first row of any search going up.
const BOTTOM: Pos = Pos {
    row: u64::MAX,
    col: 0,
};

async fn search(
    c: &mut Client,
    terminal: TerminalId,
    query: &str,
    from: Pos,
    up: bool,
) -> Response {
    c.request(Request::Search {
        terminal,
        query: query.into(),
        from,
        up,
    })
    .await
}

fn found(response: &Response) -> (Pos, Pos, bool) {
    match response {
        Response::Found {
            start,
            end,
            wrapped,
        } => (*start, *end, *wrapped),
        other => panic!("expected a match, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn frames_carry_row_ids_a_generation_and_the_wrap_mark() {
    let daemon = daemon_with(100).await;
    let mut c = Client::attach(&daemon.paths.socket, Attach::default()).await;
    // Nothing prints after the marker, so the screen is settled once it shows.
    c.type_line("seq 1 500; echo marker-''done; exec sleep 60")
        .await;
    c.wait_row("marker-done").await;
    let live = last_frame(&c);
    assert_eq!(live.history, 100);
    scroll(&mut c, ScrollTo::Top).await;
    c.wait(|c| last_frame(c).display_offset == 100).await;
    let top = last_frame(&c);
    // The oldest row held is exactly the cap below the live screen's top, and above 0.
    assert_eq!(top.top_row, live.top_row - 100);
    assert!(top.top_row > 0);
    assert_eq!(top.numbering, live.numbering);
    scroll(&mut c, ScrollTo::Bottom).await;
    c.wait(|c| last_frame(c).display_offset == 0).await;

    // Rows that left the history are an error.
    let Response::Error(e) = c
        .request(Request::Text {
            terminal: c.terminal(),
            from: Pos { row: 0, col: 0 },
            to: Pos { row: 2, col: 0 },
            lines: false,
        })
        .await
    else {
        panic!("expected an error");
    };
    assert_eq!(e, "the selection is no longer in the history");
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_soft_wrapped_row_is_marked_and_a_newline_ends_the_mark() {
    let daemon = daemon_with(100).await;
    let mut c = Client::attach(&daemon.paths.socket, Attach::default()).await;
    // A line longer than the window wraps; the row ended by a newline does not.
    c.type_line("clear; printf '%0200d\\nshort\\n' 0").await;
    c.wait_row("short").await;
    let f = last_frame(&c);
    let wrapped: Vec<bool> = f.lines.iter().map(|(_, l)| l.wrapped).collect();
    assert!(wrapped.iter().any(|w| *w), "{wrapped:?}");
    let short = f
        .lines
        .iter()
        .find(|(_, l)| l.runs.iter().any(|r| r.text.starts_with("short")))
        .unwrap();
    assert!(!short.1.wrapped);
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_resize_that_reflows_changes_the_generation() {
    let daemon = daemon_with(100).await;
    let mut c = Client::attach(&daemon.paths.socket, Attach::default()).await;
    c.type_line("seq 1 50").await;
    c.wait_row("50").await;
    let before = last_frame(&c);
    c.send(&ClientMsg::Viewport {
        cols: 60,
        rows: 24,
        cell: None,
    })
    .await;
    c.wait(|c| last_frame(c).numbering > before.numbering).await;
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_to_a_row_shows_it_centred_and_a_dropped_row_goes_to_the_top() {
    let daemon = daemon_with(1_000).await;
    let mut c = Client::attach(&daemon.paths.socket, Attach::default()).await;
    c.type_line("seq 1 600; echo marker-''done; exec sleep 60")
        .await;
    c.wait_row("marker-done").await;
    let live = last_frame(&c);
    let rows = u64::from(live.size.1);

    // A visible row: nothing moves.
    scroll(&mut c, ScrollTo::Row(live.top_row + 3)).await;
    scroll(&mut c, ScrollTo::Lines(1)).await;
    c.wait(|c| last_frame(c).display_offset == 1).await;
    scroll(&mut c, ScrollTo::Row(live.top_row + 3)).await;
    scroll(&mut c, ScrollTo::Lines(1)).await;
    c.wait(|c| last_frame(c).display_offset == 2).await;
    scroll(&mut c, ScrollTo::Bottom).await;
    c.wait(|c| last_frame(c).display_offset == 0).await;

    // 300 rows back: in the middle of the screen.
    let target = live.top_row - 300;
    scroll(&mut c, ScrollTo::Row(target)).await;
    c.wait(|c| last_frame(c).display_offset > 0).await;
    let f = last_frame(&c);
    assert_eq!(f.top_row, target - rows / 2);
    assert_eq!(u64::from(f.display_offset), 300 + rows / 2);

    // A row that is gone: the top of the history.
    scroll(&mut c, ScrollTo::Bottom).await;
    c.wait(|c| last_frame(c).display_offset == 0).await;
    scroll(&mut c, ScrollTo::Row(0)).await;
    c.wait(|c| last_frame(c).display_offset == last_frame(c).history && last_frame(c).history > 0)
        .await;
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn search_finds_steps_wraps_and_respects_smart_case() {
    let daemon = daemon_with(30_000).await;
    let mut c = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let script = "awk 'BEGIN { for (i = 1; i <= 6000; i++) { \
         w = (i == 1000 || i == 1100) ? \"needle\" : \"hay\"; \
         if (i == 3000) w = \"Needle\"; \
         if (i == 4000) w = \"日本語\"; \
         print w \" \" i } }'";
    let t = printing(&mut c, script).await;

    // Five thousand lines up from the bottom.
    let (start, end, wrapped) = found(&search(&mut c, t, "needle", BOTTOM, true).await);
    assert!(!wrapped);
    // Smart case: lower case also matches the capitalised one, which is nearer.
    assert_eq!((start.col, end.col), (0, 5));
    let line_3000 = start;
    let (next, _, wrapped) = found(&search(&mut c, t, "needle", line_3000, true).await);
    assert!(!wrapped);
    assert_eq!(line_3000.row - next.row, 1_900);
    let (older, ..) = found(&search(&mut c, t, "needle", next, true).await);
    assert_eq!(next.row - older.row, 100);
    // Past the oldest one it goes around, to the newest.
    let (around, _, wrapped) = found(&search(&mut c, t, "needle", older, true).await);
    assert!(wrapped);
    assert_eq!(around, line_3000);
    // Going down it steps back.
    let (down, _, wrapped) = found(&search(&mut c, t, "needle", older, false).await);
    assert_eq!((down, wrapped), (next, false));

    // With a capital only the capitalised word matches.
    let (capital, ..) = found(&search(&mut c, t, "Needle", BOTTOM, true).await);
    assert_eq!(capital, line_3000);
    let (again, _, wrapped) = found(&search(&mut c, t, "Needle", capital, true).await);
    assert_eq!((again, wrapped), (capital, true));

    // CJK, with the columns of its wide cells.
    let (cjk, cjk_end, _) = found(&search(&mut c, t, "本語", BOTTOM, true).await);
    assert_eq!((cjk.col, cjk_end.col), (2, 5));
    assert_eq!(cjk.row - line_3000.row, 1_000);

    assert_eq!(
        search(&mut c, t, "absent", BOTTOM, true).await,
        Response::NotFound
    );
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn text_between_positions_is_the_captures_rows_with_wrapped_ones_joined() {
    let daemon = daemon_with(30_000).await;
    let mut c = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let script = "awk 'BEGIN { for (i = 1; i <= 300; i++) { \
         pad = \"\"; \
         if (i == 290) for (j = 0; j < 200; j++) pad = pad \"x\"; \
         print \"row \" i pad } }'";
    let t = printing(&mut c, script).await;
    let (from, ..) = found(&search(&mut c, t, "row 250", BOTTOM, true).await);
    let (done, done_end, _) = found(&search(&mut c, t, "done", BOTTOM, true).await);
    let Response::Text { text, truncated } = c
        .request(Request::Text {
            terminal: t,
            from: done_end,
            to: from,
            lines: false,
        })
        .await
    else {
        panic!("expected text");
    };
    assert!(!truncated);
    let shot = match c
        .request(Request::Capture {
            terminal: t,
            lines: None,
            history: true,
            ansi: false,
        })
        .await
    {
        Response::Captured(shot) => shot,
        other => panic!("expected a capture, got {other:?}"),
    };
    let first = shot.lines.iter().position(|l| l == "row 250").unwrap();
    let last = shot.lines.iter().rposition(|l| l == "done").unwrap();
    // The long row takes three rows of the terminal; the text has it as one line.
    let long = format!("row 290{}", "x".repeat(200));
    assert!(text.contains(&long), "{text}");
    let width = usize::from(shot.size.0);
    let wrapped =
        |l: &str| l.len() == width && (l.starts_with("row 290") || l.bytes().all(|b| b == b'x'));
    let mut expected = String::new();
    for (i, line) in shot.lines[first..=last].iter().enumerate() {
        if i > 0 && !wrapped(&shot.lines[first + i - 1]) {
            expected.push('\n');
        }
        expected.push_str(line);
    }
    assert_eq!(text, expected);
    assert_eq!(done.row, done_end.row);

    // Whole rows ignore the columns.
    let Response::Text { text: rows, .. } = c
        .request(Request::Text {
            terminal: t,
            from: Pos { col: 3, ..from },
            to: Pos { col: 1, ..from },
            lines: true,
        })
        .await
    else {
        panic!("expected text");
    };
    assert_eq!(rows, "row 250");
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_attached_client_reaches_only_the_terminals_of_its_own_session() {
    let daemon = daemon_with(100).await;
    let mut a = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let b = Client::attach(
        &daemon.paths.socket,
        Attach {
            session: Some("other".into()),
            ..Attach::default()
        },
    )
    .await;
    let theirs = b.terminal();
    assert_ne!(a.terminal(), theirs);
    let ask = Request::Search {
        terminal: theirs,
        query: "x".into(),
        from: BOTTOM,
        up: true,
    };
    let text = Request::Text {
        terminal: theirs,
        from: Pos { row: 0, col: 0 },
        to: Pos { row: 0, col: 1 },
        lines: false,
    };
    for request in [ask.clone(), text.clone()] {
        let Response::Error(e) = a.request(request).await else {
            panic!("a terminal of another session must be refused");
        };
        assert_eq!(e, format!("no terminal {theirs}"));
    }
    // Their own terminal answers, and so does any terminal for a control client.
    let own = a.terminal();
    assert_eq!(
        search(&mut a, own, "zzzz", BOTTOM, true).await,
        Response::NotFound
    );
    let mut control = Client::control(&daemon.paths.socket).await;
    assert_eq!(control.request(ask).await, Response::NotFound);
    assert!(matches!(
        control.request(text).await,
        Response::Text { .. } | Response::Error(_)
    ));
    drop(b);
    daemon.stop().await.unwrap();
}
