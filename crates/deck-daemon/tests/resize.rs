//! `SetRatios`: validation, sizing, and the stored ratios across count and viewport changes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use deck_core::action::{Action, ActionError};
use deck_core::layout::{self, Ratios, Rect};
use deck_core::model::{PanelId, PanelView, TerminalId};
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

async fn grow(c: &mut Client, n: usize) {
    while ids(c).len() < n {
        let before = ids(c).len();
        c.action(Action::NewTerminal { panel: None }).await.unwrap();
        c.wait(|c| ids(c).len() > before).await;
    }
}

async fn set(c: &mut Client, ratios: Ratios) -> Result<(), ActionError> {
    let p = panel(c);
    c.action(Action::SetRatios {
        panel: p.id,
        n: p.terminals.len(),
        ratios,
    })
    .await
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
            // Anywhere in the row: dash prints the answer to a line typed ahead after its prompt.
            let (_, rest) = row.split_once(&marker)?;
            let (rows, cols) = rest.trim().split_once(' ')?;
            Some((cols.parse().ok()?, rows.parse().ok()?))
        })
    };
    c.wait(|c| find(c).is_some()).await;
    find(c).unwrap()
}

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

fn ratios(main: Option<u16>, cols: &[u16], rows: &[u16]) -> Ratios {
    Ratios {
        main,
        cols: cols.to_vec(),
        rows: rows.to_vec(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn set_ratios_stores_them_and_resizes_the_ptys() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon, 100, 24).await;
    grow(&mut c, 3).await;
    let want = ratios(Some(700), &[1000], &[400, 600]);
    set(&mut c, want.clone()).await.unwrap();
    c.wait(|c| panel(c).ratios == want).await;
    // 100 columns: 70 for the main terminal, 30 for the others, 2 of each for the border.
    assert_eq!(expected(&c, (100, 24))[0].0, 68);
    c.wait(|c| c.screens[&ids(c)[0]].len() == 22).await;
    assert_sizes(&mut c, (100, 24), "A").await;
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_length_or_a_stale_count_is_refused() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon, 100, 24).await;
    grow(&mut c, 3).await;
    let id = panel(&c).id;
    let before = panel(&c).ratios;

    // Valid for 2 terminals, not for 3.
    let wrong = Ratios::default_for(2);
    let err = c
        .action(Action::SetRatios {
            panel: id,
            n: 3,
            ratios: wrong.clone(),
        })
        .await
        .unwrap_err();
    assert_eq!(err, ActionError::InvalidRatios);
    assert_eq!(err.to_string(), "invalid layout ratios");
    // Shares that do not add up to 1000.
    let err = set(&mut c, ratios(Some(500), &[900], &[400, 400]))
        .await
        .unwrap_err();
    assert_eq!(err, ActionError::InvalidRatios);

    // A count the client computed for, which is no longer the panel's.
    let err = c
        .action(Action::SetRatios {
            panel: id,
            n: 2,
            ratios: wrong,
        })
        .await
        .unwrap_err();
    assert_eq!(err, ActionError::LayoutChanged);
    assert_eq!(err.to_string(), "layout changed, try again");
    assert_eq!(
        c.action(Action::SetRatios {
            panel: PanelId(9999),
            n: 3,
            ratios: Ratios::default_for(3),
        })
        .await,
        Err(ActionError::NoSuchPanel(PanelId(9999)))
    );
    assert_eq!(panel(&c).ratios, before, "nothing was stored");
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn custom_ratios_survive_count_changes() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon, 100, 24).await;
    grow(&mut c, 2).await;
    set(&mut c, ratios(None, &[600, 400], &[1000]))
        .await
        .unwrap();
    c.wait(|c| panel(c).ratios.cols == [600, 400]).await;

    grow(&mut c, 3).await;
    assert_eq!(panel(&c).ratios.main, Some(600), "the big one keeps 60%");
    assert_eq!(expected(&c, (100, 24))[0].0, 58);

    let last = *ids(&c).last().unwrap();
    c.action(Action::CloseTerminal { terminal: last })
        .await
        .unwrap();
    c.wait(|c| ids(c).len() == 2).await;
    assert_eq!(panel(&c).ratios, ratios(None, &[600, 400], &[1000]));
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn untouched_defaults_follow_the_shape_rule() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon, 100, 24).await;
    grow(&mut c, 4).await;
    assert_eq!(panel(&c).ratios, Ratios::default_for(4));
    grow(&mut c, 5).await;
    assert_eq!(panel(&c).ratios, Ratios::default_for(5));
    assert_eq!(expected(&c, (100, 24))[0].0, 48, "exactly half the width");
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_narrow_window_gets_a_clamped_copy_and_the_exact_sizes_return() {
    let daemon = TestDaemon::start().await;
    let mut c = attach(&daemon, 200, 24).await;
    grow(&mut c, 2).await;
    let lopsided = ratios(None, &[900, 100], &[1000]);
    set(&mut c, lopsided.clone()).await.unwrap();
    c.wait(|c| panel(c).ratios == lopsided).await;
    assert_sizes(&mut c, (200, 24), "B").await;
    let wide = expected(&c, (200, 24));
    assert_eq!(wide[1].0, 18, "10% of 200 columns, less the border");

    c.send(&ClientMsg::Viewport { cols: 40, rows: 24 }).await;
    c.wait(|c| c.view.as_ref().unwrap().layout_viewport == Some((40, 24)))
        .await;
    assert_eq!(panel(&c).ratios, lopsided, "never overwritten");
    let narrow = expected(&c, (40, 24));
    assert!(narrow[1].0 >= 10, "the clamped copy keeps the minimum");
    let t2 = ids(&c)[1];
    c.wait(|c| {
        c.frames()
            .filter(|f| f.terminal == t2)
            .last()
            .is_some_and(|f| f.size == narrow[1])
    })
    .await;

    c.send(&ClientMsg::Viewport {
        cols: 200,
        rows: 24,
    })
    .await;
    c.wait(|c| c.view.as_ref().unwrap().layout_viewport == Some((200, 24)))
        .await;
    assert_eq!(panel(&c).ratios, lopsided);
    assert_eq!(expected(&c, (200, 24)), wide);
    assert_sizes(&mut c, (200, 24), "C").await;
    daemon.stop().await.unwrap();
}
