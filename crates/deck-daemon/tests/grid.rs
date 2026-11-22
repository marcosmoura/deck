//! Several terminals per panel: sizing, count changes, focus and the room refusal.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use deck_core::action::{Action, ActionError};
use deck_core::layout::{self, Direction, Ratios, Rect};
use deck_core::model::{PanelView, TerminalId};
use deck_core::protocol::ClientMsg;
use support::{Attach, Client, TestDaemon};

async fn attach(daemon: &TestDaemon, cols: u16, rows: u16) -> Client {
    Client::attach(
        &daemon.paths.socket,
        Attach {
            cols,
            rows,
            ..Attach::default()
        },
    )
    .await
}

fn panel(c: &Client) -> PanelView {
    let view = c.view.as_ref().expect("a view");
    view.projects
        .iter()
        .flat_map(|p| &p.panels)
        .find(|p| Some(p.id) == view.active_panel)
        .expect("an active panel")
        .clone()
}

fn ids(c: &Client) -> Vec<TerminalId> {
    panel(c).terminals.iter().map(|t| t.id).collect()
}

async fn new_terminal(c: &mut Client) -> Result<(), ActionError> {
    c.action(Action::NewTerminal { panel: None }).await
}

/// The panel with `n` terminals, once the view shows them.
async fn grow(c: &mut Client, n: usize) {
    while ids(c).len() < n {
        let before = ids(c).len();
        new_terminal(c).await.unwrap();
        c.wait(|c| ids(c).len() > before).await;
    }
}

/// What `stty size` says in a terminal, as (columns, rows).
async fn stty(c: &mut Client, terminal: TerminalId, tag: &str) -> (u16, u16) {
    c.send(&ClientMsg::Input {
        terminal,
        bytes: format!("echo {tag}=$(stty size)\n").into_bytes(),
    })
    .await;
    let marker = format!("{tag}=");
    let find = move |c: &Client| -> Option<(u16, u16)> {
        c.screens.get(&terminal)?.iter().find_map(|row| {
            let rest = row.trim_start().strip_prefix(&marker)?;
            let (rows, cols) = rest.trim().split_once(' ')?;
            Some((cols.parse().ok()?, rows.parse().ok()?))
        })
    };
    c.wait(|c| find(c).is_some()).await;
    find(c).unwrap()
}

/// The inner sizes the layout gives each terminal of the active panel in `viewport`.
fn expected(c: &Client, viewport: (u16, u16)) -> Vec<(u16, u16)> {
    let p = panel(c);
    let n = p.terminals.len();
    layout::panel_rects(Rect::new(0, 0, viewport.0, viewport.1), n, &p.ratios)
        .into_iter()
        .map(|r| layout::inner(r, n))
        .collect()
}

async fn assert_sizes(c: &mut Client, viewport: (u16, u16), tag: &str) {
    let want = expected(c, viewport);
    for (i, id) in ids(c).into_iter().enumerate() {
        // The PTYs are resized one after the other, so ask again until this one has been.
        let mut attempt = 0;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let got = loop {
            let got = stty(c, id, &format!("{tag}{i}r{attempt}")).await;
            attempt += 1;
            if got == want[i] || std::time::Instant::now() > deadline {
                break got;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        };
        assert_eq!(got, want[i], "terminal {} of {}", i + 1, want.len());
    }
}

/// A program that changes its own window size (`stty cols 50`) is put back to the layout's.
#[tokio::test(flavor = "multi_thread")]
async fn a_pty_changed_behind_the_daemon_returns_to_the_layout_size_at_the_next_pass() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon, 100, 25).await;
    // Past the rechecks that follow the sizing at attach.
    tokio::time::sleep(std::time::Duration::from_millis(1300)).await;
    let id = ids(&c)[0];
    c.send(&ClientMsg::Input {
        terminal: id,
        bytes: b"stty cols 50\n".to_vec(),
    })
    .await;
    // The emulator still says 100 columns, the PTY now has 50.
    let mut got = (0, 0);
    for n in 0..100 {
        got = stty(&mut c, id, &format!("S{n}")).await;
        if got.0 == 50 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(got.0, 50, "the program resized its PTY");
    // A pass that changes no size (a rename) still compares the real PTY size.
    let p = panel(&c).id;
    c.action(Action::RenamePanel {
        panel: p,
        name: "renamed".into(),
    })
    .await
    .unwrap();
    assert_sizes(&mut c, (100, 24), "T").await;
    daemon.stop().await.unwrap();
}

/// The same, with the change landing after a resize: the rechecks within a second undo it.
#[tokio::test(flavor = "multi_thread")]
async fn a_pty_changed_just_after_a_resize_is_put_back_by_the_rechecks() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon, 100, 25).await;
    let id = ids(&c)[0];
    c.send(&ClientMsg::Input {
        terminal: id,
        bytes: b"sleep 0.4; stty cols 50\n".to_vec(),
    })
    .await;
    c.send(&ClientMsg::Viewport { cols: 90, rows: 24 }).await;
    c.wait(|c| c.view.as_ref().unwrap().layout_viewport == Some((90, 24)))
        .await;
    // The change lands at 400 ms; the recheck at 1 s sets the layout size again.
    tokio::time::sleep(std::time::Duration::from_millis(1300)).await;
    assert_sizes(&mut c, (90, 24), "U").await;
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn adding_terminals_sizes_every_pty_from_its_rect() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon, 100, 25).await;
    assert_eq!(panel(&c).ratios, Ratios::default_for(1));
    assert_sizes(&mut c, (100, 24), "A").await;

    grow(&mut c, 3).await;
    let p = panel(&c);
    assert_eq!(p.focused_terminal, Some(p.terminals[2].id), "the new one");
    assert_eq!(p.ratios, Ratios::default_for(3));
    let want = expected(&c, (100, 24));
    assert_eq!(want, [(48, 22), (48, 10), (48, 10)]);
    assert_sizes(&mut c, (100, 24), "B").await;
    // The frames follow the sizes.
    for (id, (_, rows)) in ids(&c).into_iter().zip(&want) {
        assert_eq!(c.screens[&id].len(), usize::from(*rows));
    }

    grow(&mut c, 4).await;
    assert_sizes(&mut c, (100, 24), "C").await;
    grow(&mut c, 5).await;
    assert_sizes(&mut c, (100, 24), "D").await;
    assert_eq!(expected(&c, (100, 24))[0], (48, 22), "main + 2x2");
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_terminal_starts_in_the_projects_root() {
    let daemon = TestDaemon::start().await;
    let dir = tempfile::Builder::new()
        .prefix("grid")
        .tempdir_in("/tmp")
        .unwrap();
    let root = dir.path().canonicalize().unwrap();
    let mut c = Client::attach(
        &daemon.paths.socket,
        Attach {
            cwd: root.clone(),
            ..Attach::default()
        },
    )
    .await;
    grow(&mut c, 2).await;
    let t = ids(&c)[1];
    c.send(&ClientMsg::Input {
        terminal: t,
        bytes: b"echo CWD=$(pwd)\n".to_vec(),
    })
    .await;
    let want = format!("CWD={}", root.display());
    c.wait(|c| {
        c.screens
            .get(&t)
            .is_some_and(|s| s.iter().any(|r| r.trim_end() == want))
    })
    .await;
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn focus_moves_by_direction_and_typing_reaches_only_the_focused_terminal() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon, 100, 25).await;
    grow(&mut c, 3).await;
    let t = ids(&c);
    let focus = |c: &Client| panel(c).focused_terminal.unwrap();
    assert_eq!(focus(&c), t[2]);

    let step = async |c: &mut Client, dir: Direction, to: TerminalId| {
        c.action(Action::FocusDirection { dir }).await.unwrap();
        c.wait(|c| focus(c) == to).await;
    };
    step(&mut c, Direction::Left, t[0]).await;
    step(&mut c, Direction::Right, t[1]).await;
    step(&mut c, Direction::Down, t[2]).await;
    step(&mut c, Direction::Down, t[2]).await;
    step(&mut c, Direction::Up, t[1]).await;
    step(&mut c, Direction::Up, t[1]).await;
    step(&mut c, Direction::Left, t[0]).await;
    step(&mut c, Direction::Left, t[0]).await;
    step(&mut c, Direction::Right, t[1]).await;

    c.type_line("echo ONLY-HERE").await;
    c.wait(|c| c.screens[&t[1]].iter().any(|r| r.trim_end() == "ONLY-HERE"))
        .await;
    for other in [t[0], t[2]] {
        assert!(!c.screens[&other].iter().any(|r| r.contains("ONLY-HERE")));
    }

    c.action(Action::FocusTerminal { terminal: t[0] })
        .await
        .unwrap();
    c.wait(|c| focus(c) == t[0]).await;
    assert_eq!(
        c.action(Action::FocusTerminal {
            terminal: TerminalId(999)
        })
        .await,
        Err(ActionError::NoSuchTerminal(TerminalId(999)))
    );
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn closing_a_terminal_reflows_the_rest_and_moves_focus() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon, 100, 25).await;
    grow(&mut c, 3).await;
    let t = ids(&c);
    c.action(Action::FocusTerminal { terminal: t[1] })
        .await
        .unwrap();
    c.wait(|c| panel(c).focused_terminal == Some(t[1])).await;

    c.action(Action::CloseTerminal { terminal: t[1] })
        .await
        .unwrap();
    c.wait(|c| ids(c).len() == 2).await;
    assert_eq!(ids(&c), [t[0], t[2]]);
    assert_eq!(panel(&c).focused_terminal, Some(t[2]), "took its position");
    assert_eq!(panel(&c).ratios, Ratios::default_for(2));
    c.wait(|c| c.screens[&t[0]].len() == 22).await;
    assert_sizes(&mut c, (100, 24), "E").await;

    // The last of two, focused: focus goes to the previous one.
    c.action(Action::CloseTerminal { terminal: t[2] })
        .await
        .unwrap();
    c.wait(|c| ids(c).len() == 1).await;
    assert_eq!(panel(&c).focused_terminal, Some(t[0]));
    c.wait(|c| c.screens[&t[0]].len() == 24).await;
    assert_eq!(
        stty(&mut c, t[0], "F").await,
        (100, 24),
        "a single terminal fills the panel"
    );
    assert_eq!(
        c.action(Action::CloseTerminal { terminal: t[2] }).await,
        Err(ActionError::NoSuchTerminal(t[2]))
    );
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn exit_in_one_of_three_reflows_to_two_side_by_side() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon, 100, 25).await;
    grow(&mut c, 3).await;
    let t = ids(&c);
    c.send(&ClientMsg::Input {
        terminal: t[2],
        bytes: b"exit\n".to_vec(),
    })
    .await;
    c.wait(|c| ids(c).len() == 2).await;
    assert_eq!(ids(&c), [t[0], t[1]]);
    assert_eq!(panel(&c).focused_terminal, Some(t[1]), "the previous one");
    c.wait(|c| c.screens[&t[0]].len() == 22 && c.screens[&t[0]].len() == c.screens[&t[1]].len())
        .await;
    assert_eq!(expected(&c, (100, 24)), [(48, 22), (48, 22)]);
    assert_sizes(&mut c, (100, 24), "G").await;
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_window_too_small_for_another_terminal_refuses_it() {
    let daemon = TestDaemon::start().await;
    // A 40x12 window: the panel area is 40x11.
    let mut c = attach(&daemon, 40, 12).await;
    grow(&mut c, 4).await;
    let err = new_terminal(&mut c).await.unwrap_err();
    assert_eq!(err, ActionError::NoRoom);
    assert_eq!(err.to_string(), "not enough room for another terminal");
    assert_eq!(ids(&c).len(), 4);
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_clamped_copy_follows_the_window_and_the_stored_ratios_stay() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon, 100, 25).await;
    grow(&mut c, 5).await;
    let stored = panel(&c).ratios;
    assert_eq!(stored, Ratios::default_for(5));

    c.send(&ClientMsg::Viewport { cols: 30, rows: 24 }).await;
    c.wait(|c| c.view.as_ref().unwrap().layout_viewport == Some((30, 24)))
        .await;
    assert_eq!(panel(&c).ratios, stored, "never overwritten");
    let small = expected(&c, (30, 24));
    // Too narrow to read `stty` output: the last frame of each terminal has its size.
    let want = small.clone();
    c.wait(|c| {
        ids(c).iter().zip(&want).all(|(id, size)| {
            c.frames()
                .filter(|f| f.terminal == *id)
                .last()
                .is_some_and(|f| f.size == *size)
        })
    })
    .await;
    assert_ne!(small, expected_default(100));

    c.send(&ClientMsg::Viewport {
        cols: 100,
        rows: 24,
    })
    .await;
    c.wait(|c| c.view.as_ref().unwrap().layout_viewport == Some((100, 24)))
        .await;
    assert_eq!(panel(&c).ratios, stored);
    assert_sizes(&mut c, (100, 24), "I").await;
    assert_eq!(expected(&c, (100, 24)), expected_default(100));
    daemon.stop().await.unwrap();
}

fn expected_default(cols: u16) -> Vec<(u16, u16)> {
    layout::layout(Rect::new(0, 0, cols, 24), 5, &Ratios::default_for(5))
        .into_iter()
        .map(|r| layout::inner(r, 5))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn two_clients_share_focus_and_a_smaller_one_leaves_the_sizes_alone_until_it_types() {
    let daemon = TestDaemon::start().await;
    let mut a = attach(&daemon, 100, 25).await;
    grow(&mut a, 3).await;
    let mut b = Client::attach(
        &daemon.paths.socket,
        Attach {
            session: None,
            create: false,
            cols: 60,
            rows: 15,
            ..Attach::default()
        },
    )
    .await;
    // The attach is a claim on the size, like input: the PTYs follow the newest client.
    b.wait(|c| c.view.as_ref().unwrap().layout_viewport == Some((60, 14)))
        .await;
    assert_eq!(panel(&b).focused_terminal, panel(&a).focused_terminal);
    assert_eq!(panel(&b).ratios, panel(&a).ratios);

    // The first client types: the layout returns to its size.
    a.type_line("echo hi").await;
    a.wait(|c| c.view.as_ref().unwrap().layout_viewport == Some((100, 24)))
        .await;
    b.wait(|c| c.view.as_ref().unwrap().layout_viewport == Some((100, 24)))
        .await;
    assert_sizes(&mut a, (100, 24), "J").await;
    daemon.stop().await.unwrap();
}
