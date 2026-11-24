//! The client's cell size in pixels reaching the PTY and the size reports of its terminals.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use deck_core::model::TerminalId;
use deck_core::protocol::{CellPx, ClientMsg};
use support::{Attach, Client, TestDaemon};

const CELL: CellPx = CellPx {
    width: 9,
    height: 18,
};

fn first_terminal(c: &Client) -> TerminalId {
    let view = c.view.as_ref().expect("a view");
    view.projects
        .iter()
        .flat_map(|p| &p.panels)
        .find(|p| Some(p.id) == view.active_panel)
        .and_then(|p| p.terminals.first())
        .expect("a terminal")
        .id
}

/// Types `line` and waits for a screen row that `find` reads a value from.
async fn ask<T>(
    c: &mut Client,
    terminal: TerminalId,
    line: String,
    find: impl Fn(&str) -> Option<T>,
) -> T {
    c.send(&ClientMsg::Input {
        terminal,
        bytes: format!("\x15{line}\n").into_bytes(),
    })
    .await;
    let read = |c: &Client| c.screens.get(&terminal)?.iter().find_map(|r| find(r));
    c.wait(|c| read(c).is_some()).await;
    read(c).unwrap()
}

/// `stty size`: (columns, rows).
async fn cells(c: &mut Client, t: TerminalId, tag: &str) -> (u16, u16) {
    let marker = format!("{tag}=");
    ask(c, t, format!("echo {tag}=$(stty size)"), move |row| {
        let (_, rest) = row.split_once(&marker)?;
        let (rows, cols) = rest.trim().split_once(' ')?;
        Some((cols.parse().ok()?, rows.parse().ok()?))
    })
    .await
}

/// The PTY's pixel size from `TIOCGWINSZ`, as (width, height).
async fn pixels(c: &mut Client, t: TerminalId, tag: &str) -> (u16, u16) {
    let script = format!(
        "import fcntl,termios,struct;r,c,x,y=struct.unpack('HHHH',fcntl.ioctl(0,termios.TIOCGWINSZ,bytes(8)));print('{tag}:%d,%d:'%(x,y))"
    );
    let marker = format!("{tag}:");
    ask(c, t, format!("python3 -c \"{script}\""), move |row| {
        let rest = row.strip_prefix(&marker)?;
        let (w, rest) = rest.split_once(',')?;
        let (h, _) = rest.split_once(':')?;
        Some((w.parse().ok()?, h.parse().ok()?))
    })
    .await
}

/// What the PTY's line discipline echoes of the answer to `query` (`ESC` shows as `^[`), `None`
/// when nothing was echoed before `tag` was printed.
async fn answer(c: &mut Client, t: TerminalId, query: &str, tag: &str) -> Option<String> {
    // Ctrl-U first: the program does not read an answer, so part of the last one may still
    // wait in the line.
    c.send(&ClientMsg::Input {
        terminal: t,
        bytes: format!("\x15printf '\\033[{query}'; sleep 0.3; echo {tag}\n").into_bytes(),
    })
    .await;
    let printed = |row: &String| !row.contains("echo") && row.ends_with(tag);
    c.wait(|c| {
        c.screens
            .get(&t)
            .is_some_and(|rows| rows.iter().any(printed))
    })
    .await;
    let row = c
        .screens
        .get(&t)
        .unwrap()
        .iter()
        .find(|r| printed(r))?
        .clone();
    let echoed = row.strip_suffix(tag)?;
    (!echoed.is_empty()).then(|| echoed.to_string())
}

fn have_python() -> bool {
    std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_known_cell_size_reaches_the_pty_and_the_size_reports() {
    let daemon = TestDaemon::start().await;
    let mut c = Client::attach(
        &daemon.paths.socket,
        Attach {
            cell: Some(CELL),
            ..Attach::default()
        },
    )
    .await;
    let t = first_terminal(&c);
    let (cols, rows) = cells(&mut c, t, "A").await;
    if have_python() {
        let got = pixels(&mut c, t, "PX").await;
        assert_eq!(got, (cols * 9, rows * 18), "{cols}x{rows} cells");
    } else {
        eprintln!("python3 is missing: the pixel size of the PTY is not checked");
    }
    // `CSI 16 t`: the cell size; `CSI 14 t`: the window in pixels.
    assert_eq!(
        answer(&mut c, t, "16t", "D1").await.as_deref(),
        Some("^[[6;18;9t")
    );
    assert_eq!(
        answer(&mut c, t, "14t", "D2").await.as_deref(),
        Some(format!("^[[4;{};{}t", rows * 18, cols * 9).as_str())
    );
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_cell_size_the_pty_has_no_pixels_and_the_old_answers_stay() {
    let daemon = TestDaemon::start().await;
    let mut c = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let t = first_terminal(&c);
    let (cols, rows) = cells(&mut c, t, "A").await;
    if have_python() {
        assert_eq!(pixels(&mut c, t, "PX").await, (0, 0));
    }
    assert_eq!(answer(&mut c, t, "16t", "D1").await, None);
    assert_eq!(
        answer(&mut c, t, "14t", "D2").await.as_deref(),
        Some(format!("^[[4;{};{}t", rows * 16, cols * 8).as_str())
    );
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cell_size_change_alone_reaches_the_emulator_and_the_pty() {
    let daemon = TestDaemon::start().await;
    let mut c = Client::attach(
        &daemon.paths.socket,
        Attach {
            cell: Some(CELL),
            ..Attach::default()
        },
    )
    .await;
    let t = first_terminal(&c);
    let (cols, rows) = cells(&mut c, t, "A").await;
    let layout = c.view.as_ref().unwrap().layout_viewport.expect("viewport");
    // The same size with another cell size: a font zoom.
    c.send(&ClientMsg::Viewport {
        cols: layout.0,
        rows: layout.1,
        cell: Some(CellPx {
            width: 10,
            height: 20,
        }),
    })
    .await;
    assert_eq!(cells(&mut c, t, "B").await, (cols, rows));
    if have_python() {
        // The PTYs are resized one after the other: ask again until this one has been.
        let want = (cols * 10, rows * 20);
        let mut got = pixels(&mut c, t, "P1").await;
        for i in 0..50 {
            if got == want {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            got = pixels(&mut c, t, &format!("Q{i}")).await;
        }
        assert_eq!(got, want);
    }
    assert_eq!(
        answer(&mut c, t, "16t", "D1").await.as_deref(),
        Some("^[[6;20;10t")
    );
    daemon.stop().await.unwrap();
}
