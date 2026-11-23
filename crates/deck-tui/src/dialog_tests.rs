//! The prompt and confirmation dialogs: their box, their rows, their flash.

use std::time::Instant;

use deck_core::model::{PanelId, TerminalId};
use deck_core::protocol::ServerMsg;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::{Color, Modifier};

use crate::app::App;
use crate::draw::draw;
use crate::mirror::tests::{mockup, no_sidebar};
use crate::modal::{Confirm, Modal, PromptKind, Target};
use crate::picker::Picker;

fn app(size: (u16, u16)) -> App {
    let mut a = App::new(&no_sidebar(), size, &[]);
    a.on_server(ServerMsg::SessionView(mockup()));
    a
}

fn screen(a: &App) -> (Terminal<TestBackend>, Vec<String>) {
    let mut t = Terminal::new(TestBackend::new(a.size.0, a.size.1)).unwrap();
    t.draw(|f| draw(f, a)).unwrap();
    let buf = t.backend().buffer();
    let rows = (0..buf.area.height)
        .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect())
        .collect();
    (t, rows)
}

/// Columns `from..to` of a row.
fn cols(row: &str, from: usize, to: usize) -> String {
    row.chars()
        .skip(from)
        .take(to.saturating_sub(from))
        .collect()
}

fn rename(a: &mut App, text: &str) {
    a.open_prompt(
        PromptKind::RenamePanel(PanelId(3)),
        text,
        Some(Target::Panel(PanelId(3), text.into())),
    );
}

fn ask(a: &mut App, question: &str) {
    a.modal = Some(Modal::Confirm(Confirm {
        target: Target::Terminal(TerminalId(1), 1),
        question: question.into(),
    }));
}

#[test]
fn a_prompt_is_a_centered_rounded_box_with_title_editor_and_hint() {
    let mut a = app((80, 24));
    rename(&mut a, "editor");
    let (t, rows) = screen(&a);
    // 40 columns at 20, 5 rows from row 9 (24 - 5 = 19, half rounded down).
    assert_eq!(
        cols(&rows[9], 20, 60),
        "╭ Rename panel ────────────────────────╮"
    );
    assert_eq!(cols(&rows[9], 0, 20).trim(), "");
    assert_eq!(
        cols(&rows[10], 20, 60),
        "│ editor█                              │"
    );
    assert_eq!(
        cols(&rows[11], 20, 60),
        "│                                      │"
    );
    assert_eq!(
        cols(&rows[12], 20, 60),
        "│ ⏎ save  esc cancel                   │"
    );
    assert_eq!(
        cols(&rows[13], 20, 60),
        "╰──────────────────────────────────────╯"
    );
    let buf = t.backend().buffer();
    assert_eq!(buf[(22, 9)].fg, Color::Blue);
    assert!(buf[(22, 9)].modifier.contains(Modifier::BOLD));
    assert!(!buf[(22, 12)].modifier.contains(Modifier::BOLD));
}

#[test]
fn a_confirmation_has_no_title_and_the_question_in_warning() {
    let mut a = app((80, 24));
    ask(&mut a, "Close panel \"zsh\" and its terminal?");
    let (t, rows) = screen(&a);
    assert_eq!(
        cols(&rows[9], 20, 60),
        "╭──────────────────────────────────────╮"
    );
    assert_eq!(
        cols(&rows[10], 20, 60),
        "│ Close panel \"zsh\" and its terminal?  │"
    );
    assert_eq!(
        cols(&rows[12], 20, 60),
        "│ y yes  n no                          │"
    );
    assert_eq!(t.backend().buffer()[(22, 10)].fg, Color::Yellow);
}

#[test]
fn the_box_grows_with_a_long_question_but_not_with_the_editor() {
    let mut a = app((100, 24));
    let q = "Close space \"a-rather-long-space-name-indeed\" and its 12 terminals?";
    ask(&mut a, q);
    let (_, rows) = screen(&a);
    let w = q.chars().count() + 4;
    let left = (100 - w) / 2;
    assert!(rows[10].contains(&format!("│ {q} │")), "{}", rows[10]);
    assert_eq!(rows[10].chars().position(|c| c == '│').unwrap(), left);

    rename(&mut a, &"x".repeat(80));
    let (_, rows) = screen(&a);
    assert_eq!(rows[9].chars().position(|c| c == '╭').unwrap(), 30);
    assert!(
        rows[10].contains("│ xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx█ │"),
        "{}",
        rows[10]
    );
}

#[test]
fn the_editor_keeps_the_cursor_in_sight_in_the_middle_of_the_text() {
    let mut a = app((80, 24));
    rename(&mut a, "abcdef");
    let Some(Modal::Prompt(p)) = &mut a.modal else {
        panic!()
    };
    p.editor.on_key(&deck_core::keys::KeyEvent::new(
        deck_core::keys::KeyCode::Home,
        deck_core::keys::Modifiers::default(),
    ));
    let (t, rows) = screen(&a);
    assert!(rows[10].contains("│ abcdef "), "{}", rows[10]);
    assert!(
        t.backend().buffer()[(22, 10)]
            .modifier
            .contains(Modifier::REVERSED)
    );
}

#[test]
fn a_flash_takes_the_hint_row_and_the_bar_does_not_show_it() {
    let mut a = app((80, 24));
    rename(&mut a, "editor");
    a.set_flash("a space named \"x\" exists".into(), Instant::now(), true);
    let (t, rows) = screen(&a);
    assert!(
        rows[12].contains("│ a space named \"x\" exists"),
        "{}",
        rows[12]
    );
    assert!(!rows[12].contains("save"));
    assert_eq!(t.backend().buffer()[(22, 12)].fg, Color::Red);
    assert!(!rows[23].contains("exists"), "{}", rows[23]);
    // The editor keeps its text.
    assert!(rows[10].contains("editor█"));
    // A flash with no dialog shows in the floating bar.
    a.modal = None;
    assert!(screen(&a).1[23].contains("exists"));
}

#[test]
fn a_long_title_and_a_long_flash_are_cut_with_an_ellipsis() {
    let mut a = app((24, 10));
    let q = "Close terminal 1 of a panel with a very long name?";
    ask(&mut a, q);
    let (_, rows) = screen(&a);
    // 24 - 2 = 22 columns from column 1; the question is cut to 18.
    assert_eq!(cols(&rows[3], 1, 23), "│ Close terminal 1 … │");
    a.modal = None;
    a.open_prompt(PromptKind::NewSession, "", None);
    a.set_flash("x".repeat(60), Instant::now(), true);
    let (_, rows) = screen(&a);
    assert!(rows[5].contains("… │"), "{}", rows[5]);
}

#[test]
fn the_dialog_fits_the_smallest_window_and_is_not_drawn_in_a_smaller_one() {
    let mut a = app((20, 5));
    rename(&mut a, "editor");
    let (_, rows) = screen(&a);
    assert_eq!(rows[0], " ╭ Rename panel ──╮ ");
    assert_eq!(rows[1], " │ editor█        │ ");
    assert_eq!(rows[3], " │ ⏎ save  esc c… │ ");
    assert_eq!(rows[4], " ╰────────────────╯ ");
    // Too small: the message, no dialog.
    let mut a = app((20, 4));
    rename(&mut a, "editor");
    let (_, rows) = screen(&a);
    assert!(rows.iter().all(|r| !r.contains('╭')));
}

#[test]
fn the_terminal_cursor_is_hidden_and_the_dialog_is_over_the_picker() {
    let mut a = app((80, 24));
    a.picker = Some(Picker::default());
    ask(&mut a, "Kill session \"work\" and its 2 terminals?");
    let (mut t, rows) = screen(&a);
    assert!(
        rows[10].contains("│ Kill session \"work\" and its 2 terminals?"),
        "{}",
        rows[10]
    );
    assert!(rows.iter().any(|r| r.contains("Sessions")), "{rows:#?}");
    assert!(rows[12].contains("y yes  n no"), "{}", rows[12]);
    t.backend_mut()
        .assert_cursor_position(ratatui::layout::Position::new(0, 0));
}

#[test]
fn a_prompt_alone_hides_the_cursor_a_focused_terminal_would_show() {
    use crate::mirror::tests::frame;
    use ratatui::layout::Position;

    let mut a = app((80, 24));
    let id = a.mirror.focused_id().unwrap();
    let mut f = frame(true, (40, 10), vec![]);
    f.terminal = id;
    a.on_server(ServerMsg::Frame(f));
    // Without a dialog the cursor is at the terminal's cursor, not at (0, 0).
    let (mut t, _) = screen(&a);
    assert_ne!(t.get_cursor_position().unwrap(), Position::new(0, 0));
    rename(&mut a, "editor");
    let (mut t, _) = screen(&a);
    t.backend_mut().assert_cursor_position(Position::new(0, 0));
}

#[test]
fn a_live_flash_widens_the_dialog_up_to_the_window_minus_two() {
    let mut a = app((80, 24));
    rename(&mut a, "editor");
    let text = "a session named \"main\" already exists";
    a.set_flash(text.into(), Instant::now(), true);
    let (_, rows) = screen(&a);
    let w = text.chars().count() + 4;
    assert!(rows[12].contains(&format!("│ {text} │")), "{}", rows[12]);
    assert_eq!(
        rows[12].chars().position(|c| c == '│').unwrap(),
        (80 - w) / 2
    );
    a.set_flash("x".repeat(200), Instant::now(), true);
    let (_, rows) = screen(&a);
    assert_eq!(rows[12].chars().position(|c| c == '│').unwrap(), 1);
    assert!(rows[12].contains("… │"), "{}", rows[12]);
}
