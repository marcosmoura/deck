//! Dragging space and panel rows to reorder them: the click on release, the drag's state
//! machine, the preview and the actions it sends.

use std::time::Instant;

use deck_core::action::{Action as Act, ActionError};
use deck_core::config::Config;
use deck_core::model::{PanelId, ProjectId, SessionView};
use deck_core::protocol::{ClientMsg, ServerMsg};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event as ct;
use ratatui::style::Modifier;

use crate::app::{App, Out};
use crate::draw::{color, draw};
use crate::mirror::tests::{frame, mockup, panel, project};
use crate::sidebar::{Row, Sel};

// The mockup's rows (no gap): 2 terminal-multiplexer, 3 editor (active), 4 claude, 5 tests,
// 6 budget, 7 api, 8 empty, 9 `no panels`, 10 dotfiles (folded).

fn app_at(config: &Config, size: (u16, u16), view: SessionView) -> App {
    let mut app = App::new(config, size, &[]);
    app.on_server(ServerMsg::SessionView(view));
    if let Some(t) = app.mirror.focused_id() {
        let mut f = frame(true, (40, 10), vec![]);
        f.terminal = t;
        app.on_server(ServerMsg::Frame(f));
    }
    app
}

fn app() -> App {
    app_at(&Config::default(), (100, 24), mockup())
}

fn mouse(kind: ct::MouseEventKind, column: u16, row: u16) -> ct::Event {
    ct::Event::Mouse(ct::MouseEvent {
        kind,
        column,
        row,
        modifiers: ct::KeyModifiers::NONE,
    })
}

const LEFT: ct::MouseButton = ct::MouseButton::Left;

fn down(a: &mut App, column: u16, row: u16) -> Vec<Out> {
    a.on_event(
        mouse(ct::MouseEventKind::Down(LEFT), column, row),
        Instant::now(),
    )
}

fn drag(a: &mut App, column: u16, row: u16) -> Vec<Out> {
    a.on_event(
        mouse(ct::MouseEventKind::Drag(LEFT), column, row),
        Instant::now(),
    )
}

fn up(a: &mut App, column: u16, row: u16) -> Vec<Out> {
    a.on_event(
        mouse(ct::MouseEventKind::Up(LEFT), column, row),
        Instant::now(),
    )
}

fn click(a: &mut App, column: u16, row: u16) -> Vec<Out> {
    down(a, column, row);
    up(a, column, row)
}

fn key(a: &mut App, code: ct::KeyCode) -> Vec<Out> {
    a.on_event(
        ct::Event::Key(ct::KeyEvent::new(code, ct::KeyModifiers::NONE)),
        Instant::now(),
    )
}

fn send(act: Act) -> Out {
    Out::Send(ClientMsg::Action(act))
}

fn move_panel(panel: u64, to: usize) -> Out {
    send(Act::MovePanel {
        panel: PanelId(panel),
        to,
    })
}

fn move_project(project: u64, to: usize) -> Out {
    send(Act::MoveProject {
        project: ProjectId(project),
        to,
    })
}

fn to(a: &App) -> Option<usize> {
    a.row_drag.map(|d| d.to)
}

/// The preview as kind letters and ids: `P1 p3 p4 ...`.
fn preview(a: &App) -> String {
    a.preview_rows()
        .iter()
        .map(|r| match r {
            Row::Project(p) => format!("P{}", p.0),
            Row::Panel(_, p) => format!("p{}", p.0),
            Row::Terminal(_, _, t) => format!("t{}", t.0),
            Row::NoPanels => "n".into(),
            Row::Gap => "_".into(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn buffer(a: &App) -> Buffer {
    let mut t = Terminal::new(TestBackend::new(a.size.0, a.size.1)).unwrap();
    t.draw(|f| draw(f, a)).unwrap();
    t.backend().buffer().clone()
}

fn text(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
}

#[test]
fn a_click_acts_on_release_only() {
    let mut a = app();
    assert!(down(&mut a, 5, 4).is_empty());
    assert!(a.row_press.is_some());
    assert_eq!(
        up(&mut a, 5, 4),
        [send(Act::FocusPanel { panel: PanelId(4) })]
    );
    assert_eq!(
        (down(&mut a, 5, 2), up(&mut a, 5, 2)),
        (
            vec![],
            vec![send(Act::SetProjectCollapsed {
                project: ProjectId(1),
                collapsed: true
            })]
        )
    );
    // A release elsewhere does nothing, and motion inside the pressed row starts no drag.
    down(&mut a, 5, 7);
    assert!(drag(&mut a, 9, 7).is_empty());
    assert!(a.row_drag.is_none());
    assert!(up(&mut a, 9, 6).is_empty());
    assert!(a.row_press.is_none());
    // A release on the same row after motion inside it still clicks.
    down(&mut a, 5, 7);
    drag(&mut a, 9, 7);
    assert_eq!(
        up(&mut a, 9, 7),
        [send(Act::FocusPanel { panel: PanelId(6) })]
    );
}

#[test]
fn a_panel_drag_previews_and_drops_inside_its_space() {
    let mut a = app();
    down(&mut a, 5, 4);
    assert!(drag(&mut a, 5, 3).is_empty());
    assert_eq!(to(&a), Some(0));
    // The list stays still while dragging.
    assert_eq!(preview(&a), "P1 p3 p4 p5 P2 p6 P7 n P8");
    // Over the space row: the first place; over other spaces: still inside its own.
    drag(&mut a, 5, 5);
    assert_eq!(to(&a), Some(2));
    drag(&mut a, 5, 2);
    assert_eq!(to(&a), Some(0));
    drag(&mut a, 5, 7);
    assert_eq!(to(&a), Some(2));
    drag(&mut a, 5, 3);
    assert_eq!(up(&mut a, 5, 3), [move_panel(4, 0)]);
    assert!(a.row_drag.is_none());
    // The preview stays until the move's own answer, whatever views come before it.
    assert_eq!(preview(&a), "P1 p4 p3 p5 P2 p6 P7 n P8");
    a.on_server(ServerMsg::SessionView(mockup()));
    assert_eq!(preview(&a), "P1 p4 p3 p5 P2 p6 P7 n P8");
    let mut moved = mockup();
    moved.projects[0].panels.swap(0, 1);
    a.on_server(ServerMsg::SessionView(moved));
    a.on_server(ServerMsg::ActionResult(Ok(())));
    assert_eq!(preview(&a), "P1 p4 p3 p5 P2 p6 P7 n P8");
    assert!(a.row_drop.is_none());
}

#[test]
fn the_preview_waits_for_the_answer_of_its_own_move() {
    let mut a = app();
    // An earlier action is still unanswered.
    click(&mut a, 5, 2);
    down(&mut a, 5, 4);
    drag(&mut a, 5, 3);
    up(&mut a, 5, 3);
    a.on_server(ServerMsg::ActionResult(Ok(())));
    assert!(a.row_drop.is_some());
    a.on_server(ServerMsg::ActionResult(Ok(())));
    assert!(a.row_drop.is_none());
}

#[test]
fn a_refused_move_falls_back_to_the_daemons_order() {
    let mut a = app();
    down(&mut a, 5, 4);
    drag(&mut a, 5, 3);
    up(&mut a, 5, 3);
    a.on_server(ServerMsg::ActionResult(Err(ActionError::NoSuchPanel(
        PanelId(4),
    ))));
    assert_eq!(preview(&a), "P1 p3 p4 p5 P2 p6 P7 n P8");
    assert!(a.flash.is_some() && a.flash_error);
}

#[test]
fn a_space_drag_targets_the_space_under_the_pointer() {
    let mut a = app();
    down(&mut a, 5, 2);
    // Over a space's block, its panels or `no panels`.
    drag(&mut a, 5, 7);
    assert_eq!(to(&a), Some(1));
    assert_eq!(preview(&a), "P1 p3 p4 p5 P2 p6 P7 n P8");
    drag(&mut a, 5, 9);
    assert_eq!(to(&a), Some(2));
    drag(&mut a, 5, 10);
    assert_eq!(to(&a), Some(3));
    assert_eq!(up(&mut a, 5, 10), [move_project(1, 3)]);

    // Dragging up.
    let mut a = app();
    down(&mut a, 5, 10);
    drag(&mut a, 5, 6);
    assert_eq!(up(&mut a, 5, 6), [move_project(8, 1)]);
}

#[test]
fn gap_rows_take_the_space_above() {
    let config = Config::parse("sidebar_space_gap = 1").unwrap().config;
    let mut a = app_at(&config, (100, 24), mockup());
    // Rows: 2 P1, 3-5 panels, 6 gap, 7 P2, 8 api, 9 gap, 10 P7.
    down(&mut a, 5, 10);
    drag(&mut a, 5, 6);
    assert_eq!(to(&a), Some(0));
    drag(&mut a, 5, 9);
    assert_eq!(to(&a), Some(1));
    drag(&mut a, 5, 8);
    assert_eq!(to(&a), Some(1));
}

#[test]
fn the_target_follows_the_daemons_rows() {
    let mut a = app();
    down(&mut a, 5, 2);
    // The pointer stays on a row: the target never depends on what is drawn there.
    for _ in 0..3 {
        drag(&mut a, 5, 7);
        assert_eq!(to(&a), Some(1));
    }
    drag(&mut a, 5, 6);
    assert_eq!(to(&a), Some(1));
}

#[test]
fn dropping_on_the_own_place_sends_nothing() {
    let mut a = app();
    down(&mut a, 5, 4);
    drag(&mut a, 5, 5);
    drag(&mut a, 5, 4);
    assert!(a.row_drag.is_some());
    assert!(up(&mut a, 5, 4).is_empty());
    assert!(a.pending.is_empty());
    assert_eq!(preview(&a), "P1 p3 p4 p5 P2 p6 P7 n P8");
}

#[test]
fn escape_cancels_and_goes_no_further() {
    let mut a = app();
    a.sidebar_focus = true;
    down(&mut a, 5, 4);
    drag(&mut a, 5, 5);
    assert!(key(&mut a, ct::KeyCode::Esc).is_empty());
    assert!(a.row_drag.is_none());
    assert!(a.sidebar_focus, "esc does not leave the sidebar");
    assert!(a.pending.is_empty());
    // The rest of the gesture is nobody's.
    assert!(drag(&mut a, 5, 6).is_empty());
    assert!(up(&mut a, 5, 6).is_empty());
    assert!(a.row_drag.is_none());
}

#[test]
fn another_key_cancels_the_drag_and_acts() {
    let mut a = app();
    a.sidebar_focus = true;
    a.selection = Some(Sel::Panel(ProjectId(1), PanelId(3)));
    down(&mut a, 5, 4);
    drag(&mut a, 5, 5);
    key(&mut a, ct::KeyCode::Down);
    assert!(a.row_drag.is_none());
    assert_eq!(a.selection, Some(Sel::Panel(ProjectId(1), PanelId(4))));
    assert!(up(&mut a, 5, 5).is_empty());
}

#[test]
fn a_release_over_the_panel_area_cancels_and_the_header_and_bar_keep_the_target() {
    let mut a = app();
    down(&mut a, 5, 4);
    drag(&mut a, 5, 5);
    assert_eq!(to(&a), Some(2));
    // Outside the list the target stays.
    drag(&mut a, 5, 0);
    drag(&mut a, 5, 23);
    drag(&mut a, 60, 3);
    assert_eq!(to(&a), Some(2));
    assert!(up(&mut a, 60, 3).is_empty(), "cancelled");

    let mut a = app();
    down(&mut a, 5, 4);
    drag(&mut a, 5, 5);
    drag(&mut a, 5, 0);
    assert_eq!(up(&mut a, 5, 0), [move_panel(4, 2)]);
}

#[test]
fn a_press_that_leaves_the_sidebar_sends_nothing_to_a_program() {
    let mut a = app();
    down(&mut a, 5, 4);
    assert!(drag(&mut a, 60, 5).is_empty());
    assert!(up(&mut a, 60, 5).is_empty());
    // Nor does it swallow the next click's release.
    down(&mut a, 5, 4);
    assert_eq!(
        up(&mut a, 5, 4),
        [send(Act::FocusPanel { panel: PanelId(4) })]
    );
}

#[test]
fn the_dragged_item_closed_or_hidden_cancels() {
    let mut a = app();
    down(&mut a, 5, 4);
    drag(&mut a, 5, 5);
    let mut view = mockup();
    view.projects[0].panels.remove(1);
    a.on_server(ServerMsg::SessionView(view));
    assert!(a.row_drag.is_none());
    assert!(up(&mut a, 5, 5).is_empty());

    // Another client folds the dragged panel's space.
    let mut a = app();
    down(&mut a, 5, 4);
    drag(&mut a, 5, 5);
    let mut view = mockup();
    view.projects[0].collapsed = true;
    a.on_server(ServerMsg::SessionView(view));
    assert!(a.row_drag.is_none());

    // A pending press is dropped too.
    let mut a = app();
    down(&mut a, 5, 4);
    let mut view = mockup();
    view.projects[0].panels.remove(1);
    a.on_server(ServerMsg::SessionView(view));
    assert!(a.row_press.is_none());
    assert!(up(&mut a, 5, 4).is_empty());
}

#[test]
fn focus_loss_a_tiny_window_and_an_overlay_cancel() {
    let mut a = app();
    down(&mut a, 5, 4);
    drag(&mut a, 5, 5);
    a.on_event(ct::Event::FocusLost, Instant::now());
    assert!(a.row_drag.is_none());
    assert!(up(&mut a, 5, 5).is_empty());

    let mut a = app();
    down(&mut a, 5, 4);
    drag(&mut a, 5, 5);
    a.on_event(ct::Event::Resize(10, 4), Instant::now());
    assert!(a.row_drag.is_none());

    let mut a = app();
    down(&mut a, 5, 4);
    drag(&mut a, 5, 5);
    a.open_new_project();
    drag(&mut a, 5, 6);
    assert!(a.row_drag.is_none());
    assert!(a.pending.is_empty());

    // A config that hides the sidebar.
    let mut a = app();
    down(&mut a, 5, 4);
    drag(&mut a, 5, 5);
    let config = Config::parse("sidebar = false").unwrap().config;
    a.apply_config(config);
    assert!(a.row_drag.is_none());
}

#[test]
fn there_is_no_hover_while_dragging() {
    let mut a = app();
    a.on_event(mouse(ct::MouseEventKind::Moved, 5, 4), Instant::now());
    assert!(a.hover.is_some());
    down(&mut a, 5, 4);
    drag(&mut a, 5, 5);
    assert!(a.hover.is_none());
    a.on_server(ServerMsg::SessionView(mockup()));
    assert!(a.hover.is_none());
}

#[test]
fn the_dragged_row_is_dimmed_over_the_band_and_the_bar_shows_move() {
    let mut a = app();
    // `editor` (row 3) is the active panel: it has the band until it is dragged.
    down(&mut a, 5, 3);
    drag(&mut a, 5, 4);
    let buf = buffer(&a);
    // Nothing moved: `claude` is the target on row 4, `editor` is dimmed on row 3.
    assert!(text(&buf, 3).contains("editor") && text(&buf, 4).contains("claude"));
    assert!(is_target(&a, 4));
    assert!(!is_target(&a, 3));
    assert!(dimmed(&a, 3), "the banded dragged row is dimmed");
    assert!(
        text(&buf, 23).contains(" MOVE  esc cancel"),
        "{}",
        text(&buf, 23)
    );
}

#[test]
fn the_edges_of_the_list_scroll_it_while_dragging() {
    let mut view = mockup();
    view.projects = vec![project(
        1,
        "big",
        false,
        (1..=30).map(|n| panel(n, &format!("p{n}"), 1)).collect(),
    )];
    view.active_project = Some(ProjectId(1));
    view.active_panel = Some(PanelId(1));
    let mut a = app_at(&Config::default(), (100, 24), view);
    // The list is 20 rows tall: rows 2..=21.
    down(&mut a, 5, 4);
    drag(&mut a, 5, 5);
    assert_eq!(a.sidebar_offset, 0);
    drag(&mut a, 5, 21);
    drag(&mut a, 5, 21);
    assert_eq!(a.sidebar_offset, 2);
    assert_eq!(to(&a), Some(19));
    // The header scrolls back up, keeping the target.
    drag(&mut a, 5, 0);
    assert_eq!(a.sidebar_offset, 1);
    assert_eq!(to(&a), Some(19));
}

#[test]
fn the_selection_stays_on_the_moved_row() {
    let mut a = app();
    a.sidebar_focus = true;
    a.selection = Some(Sel::Panel(ProjectId(1), PanelId(4)));
    down(&mut a, 5, 4);
    drag(&mut a, 5, 3);
    up(&mut a, 5, 3);
    assert_eq!(a.selection, Some(Sel::Panel(ProjectId(1), PanelId(4))));
}

#[test]
fn a_cancelled_gesture_does_not_swallow_the_next_terminal_click() {
    // An overlay opens during the press: its release is not swallowed afterwards.
    let mut a = app();
    down(&mut a, 5, 4);
    a.open_new_project();
    up(&mut a, 5, 4);
    assert!(!a.swallow_release);

    // Focus is lost during the press.
    let mut a = app();
    down(&mut a, 5, 4);
    a.on_event(ct::Event::FocusLost, Instant::now());
    assert!(a.row_press.is_none());
    assert!(!a.swallow_release);
    assert!(up(&mut a, 60, 5).is_empty());
    assert!(!a.swallow_release);
}

/// Whether row `y` of the sidebar is drawn as the drop target: the caret in column 0 in accent,
/// bold, then a space.
fn is_target(a: &App, y: u16) -> bool {
    let buf = buffer(a);
    let caret = &buf[(0, y)];
    caret.symbol() == crate::sidebar::caret_glyph(a.tier)
        && caret.fg == color(a.theme.accent)
        && caret.modifier.contains(Modifier::BOLD)
        && buf[(1, y)].symbol() == " "
}

fn target_rows(a: &App) -> usize {
    let buf = buffer(a);
    (0..a.size.1)
        .filter(|y| buf[(0, *y)].symbol() == crate::sidebar::caret_glyph(a.tier))
        .count()
}

/// Whether the text of row `y` is dimmed: muted, not bold, no band.
fn dimmed(a: &App, y: u16) -> bool {
    let buf = buffer(a);
    let c = &buf[(5, y)];
    c.fg == color(a.theme.muted)
        && !c.modifier.contains(Modifier::BOLD)
        && !c.modifier.contains(Modifier::REVERSED)
        && c.bg == color(a.theme.sidebar_bg)
}

fn screen_rows(a: &App) -> Vec<String> {
    (0..a.size.1).map(|y| text(&buffer(a), y)).collect()
}

#[test]
fn the_target_row_has_a_caret_and_the_dragged_row_is_dimmed() {
    let still = screen_rows(&app());
    let names = |rows: &[String]| -> Vec<String> {
        rows.iter().map(|r| r.chars().skip(1).collect()).collect()
    };

    // A panel dragged up: the target is the panel above.
    let mut a = app();
    down(&mut a, 5, 5);
    assert_eq!(target_rows(&a), 0, "a pending press marks nothing");
    drag(&mut a, 5, 4);
    assert!(is_target(&a, 4) && dimmed(&a, 5));
    assert_eq!(target_rows(&a), 1);
    // The list did not move: the other rows are the same, the target's content is two
    // columns to the right.
    let now = screen_rows(&a);
    for y in (0..22).filter(|y| *y != 4) {
        assert_eq!(names(&now)[y], names(&still)[y], "row {y}");
    }
    assert!(now[4].starts_with("\u{27a4} "), "{}", now[4]);
    let shifted: String = now[4].chars().skip(2).take(10).collect();
    let normal: String = still[4].chars().take(10).collect();
    assert_eq!(shifted, normal);
    // Down: the target is the panel below.
    let mut a = app();
    down(&mut a, 5, 4);
    drag(&mut a, 5, 5);
    assert!(is_target(&a, 5) && dimmed(&a, 4));
    // Clamped inside the space, from another space's row.
    drag(&mut a, 5, 7);
    assert!(is_target(&a, 5));
    // On its own place: no highlight, only the dimmed dragged row.
    drag(&mut a, 5, 4);
    assert_eq!(target_rows(&a), 0);
    assert!(dimmed(&a, 4));
    // Dropped and answered, or cancelled: nothing.
    drag(&mut a, 5, 5);
    up(&mut a, 5, 5);
    assert_eq!(target_rows(&a), 0);
    assert!(!dimmed(&a, 4));
    // The post-drop preview shows until the answer.
    assert_eq!(preview(&a), "P1 p3 p5 p4 P2 p6 P7 n P8");
    a.on_server(ServerMsg::ActionResult(Ok(())));
    assert_eq!(preview(&a), "P1 p3 p4 p5 P2 p6 P7 n P8");
    down(&mut a, 5, 4);
    drag(&mut a, 5, 5);
    assert_eq!(target_rows(&a), 1);
    key(&mut a, ct::KeyCode::Esc);
    assert_eq!(target_rows(&a), 0);
    assert!(!dimmed(&a, 4));

    // A space: the target is the space's own row, never its panels.
    let mut a = app();
    down(&mut a, 5, 2);
    drag(&mut a, 5, 7);
    assert!(is_target(&a, 6) && dimmed(&a, 2));
    assert_eq!(target_rows(&a), 1);
    assert!(text(&buffer(&a), 6).contains("budget"));
}

#[test]
fn the_shifted_row_keeps_its_content_inside_the_sidebar() {
    let mut view = mockup();
    view.projects = vec![project(
        1,
        "big",
        false,
        vec![
            panel(3, "short", 1),
            panel(
                4,
                "a very long panel name that cannot fit in the sidebar",
                2,
            ),
        ],
    )];
    view.active_project = Some(ProjectId(1));
    view.active_panel = Some(PanelId(3));
    let mut a = app_at(&Config::default(), (100, 24), view);
    let normal = text(&buffer(&a), 4);
    down(&mut a, 5, 3);
    drag(&mut a, 5, 4);
    let buf = buffer(&a);
    let row: Vec<&str> = (0..28).map(|x| buf[(x, 4)].symbol()).collect();
    assert_eq!(&row[..2], ["\u{27a4}", " "]);
    // The name is cut with `…` and stays inside the sidebar.
    let line: String = row.concat();
    assert!(line.contains('…'), "{line}");
    assert!(line.trim_end().ends_with('…'), "{line}");
    assert_eq!(buf[(28, 4)].symbol(), "│");
    assert!(normal.contains('…'));
    // A space drag shifts the space row, its glyph included.
    let mut a = app();
    down(&mut a, 5, 2);
    drag(&mut a, 5, 7);
    let row = text(&buffer(&a), 6);
    assert!(row.starts_with("\u{27a4}  budget"), "{row}");
}

#[test]
fn the_caret_follows_the_tier() {
    use crate::tier::Tier;
    for (tier, glyph) in [
        (Tier::Unicode, "\u{27a4}"),
        (Tier::Nerd, "\u{f0da}"),
        (Tier::Image, "\u{f0da}"),
    ] {
        let mut a = app_at(&Config::default(), (100, 24), mockup());
        a.tier = tier;
        down(&mut a, 5, 4);
        drag(&mut a, 5, 5);
        let buf = buffer(&a);
        assert_eq!(buf[(0, 5)].symbol(), glyph, "{tier:?}");
        assert_eq!(buf[(0, 5)].fg, color(a.theme.accent));
        assert!(buf[(0, 5)].modifier.contains(Modifier::BOLD));
        assert_eq!(buf[(1, 5)].symbol(), " ");
    }
}

#[test]
fn the_caret_is_drawn_outside_the_band_of_a_banded_target() {
    let mut a = app();
    // `editor` (row 3) is the active panel, with the band.
    down(&mut a, 5, 4);
    drag(&mut a, 5, 3);
    let buf = buffer(&a);
    assert!(is_target(&a, 3));
    for x in [0, 1] {
        assert!(
            !buf[(x, 3)].modifier.contains(Modifier::REVERSED),
            "col {x}"
        );
    }
    assert!(buf[(2, 3)].modifier.contains(Modifier::REVERSED));
    assert!(buf[(27, 3)].modifier.contains(Modifier::REVERSED));
}
