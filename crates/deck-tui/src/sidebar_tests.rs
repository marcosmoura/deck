//! The sidebar, prompts, confirmations and help, through the client's state machine and
//! the drawing, on the mockup session.

use std::path::PathBuf;
use std::time::Instant;

use deck_core::action::{Action as Act, ActionError};
use deck_core::config::Config;
use deck_core::model::{PanelId, ProjectId, SessionView, TerminalId};
use deck_core::protocol::{ClientMsg, ServerMsg};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event as ct;
use ratatui::style::{Color, Modifier};

use crate::app::{App, Out};
use crate::draw::draw;
use crate::mirror::tests::{frame, mockup, no_sidebar, panel, project};
use crate::modal::Modal;
use crate::sidebar::Sel;

fn app_at(config: &Config, size: (u16, u16), view: SessionView) -> App {
    let mut app = App::new(config, size, &[])
        .with_dirs(PathBuf::from("/work/proj"), Some(PathBuf::from("/home/me")));
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

fn key(code: ct::KeyCode) -> ct::Event {
    ct::Event::Key(ct::KeyEvent::new(code, ct::KeyModifiers::NONE))
}

fn ch(c: char) -> ct::Event {
    let mods = if c.is_uppercase() {
        ct::KeyModifiers::SHIFT
    } else {
        ct::KeyModifiers::NONE
    };
    ct::Event::Key(ct::KeyEvent::new(ct::KeyCode::Char(c), mods))
}

fn prefix() -> ct::Event {
    ct::Event::Key(ct::KeyEvent::new(
        ct::KeyCode::Char(' '),
        ct::KeyModifiers::ALT,
    ))
}

/// Runs the events and returns everything they asked for.
fn run(a: &mut App, events: &[ct::Event]) -> Vec<Out> {
    let now = Instant::now();
    events
        .iter()
        .flat_map(|e| a.on_event(e.clone(), now))
        .collect()
}

fn typed(a: &mut App, text: &str) -> Vec<Out> {
    let events: Vec<_> = text.chars().map(ch).collect();
    run(a, &events)
}

/// The prefix, then `c`.
fn pfx(a: &mut App, c: char) -> Vec<Out> {
    run(a, &[prefix(), ch(c)])
}

fn action(a: Act) -> Out {
    Out::Send(ClientMsg::Action(a))
}

fn mouse(kind: ct::MouseEventKind, column: u16, row: u16) -> ct::Event {
    ct::Event::Mouse(ct::MouseEvent {
        kind,
        column,
        row,
        modifiers: ct::KeyModifiers::NONE,
    })
}

fn click(column: u16, row: u16) -> ct::Event {
    mouse(ct::MouseEventKind::Down(ct::MouseButton::Left), column, row)
}

fn screen(a: &App) -> (Terminal<TestBackend>, Vec<String>) {
    let mut t = Terminal::new(TestBackend::new(a.size.0, a.size.1)).unwrap();
    t.draw(|f| draw(f, a)).unwrap();
    let buf = t.backend().buffer();
    let rows = (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect();
    (t, rows)
}

fn pad(text: &str, width: usize) -> String {
    format!("{text}{}", " ".repeat(width - text.chars().count()))
}

fn prefix_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌥Space"
    } else {
        "M-Space"
    }
}

#[test]
fn the_sidebar_shows_the_session_projects_and_panels() {
    let a = app();
    let (t, rows) = screen(&a);
    let side: Vec<String> = rows.iter().map(|r| r.chars().take(29).collect()).collect();
    assert_eq!(
        side[0],
        format!("{}│", pad(" main          + New space  ", 28))
    );
    assert_eq!(side[1], format!("{}│", pad("", 28)));
    assert_eq!(side[2], format!("{}│", pad(" ▾ terminal-multiplexer", 28)));
    assert_eq!(side[3], format!("{}│", pad("   ❯  editor", 28)));
    assert_eq!(side[4], format!("{}│", pad("   ❯  claude", 28)));
    assert_eq!(side[5], format!("{}│", pad("   ❯  tests", 28)));
    assert_eq!(side[6], format!("{}│", pad(" ▾ budget", 28)));
    assert_eq!(side[7], format!("{}│", pad("   ❯  api", 28)));
    assert_eq!(side[8], format!("{}│", pad(" ▾ empty", 28)));
    assert_eq!(side[9], format!("{}│", pad("   no panels", 28)));
    assert_eq!(
        side[10],
        format!("{}│", pad(" ▸ dotfiles               2 ", 28))
    );
    assert_eq!(side[11], format!("{}│", pad("", 28)));
    // The panel area is the rest: the terminal from column 29 on, with no border.
    assert_eq!(rows[0].chars().count(), 100);
    assert!(!rows.iter().any(|r| r.contains('╭') || r.contains('╰')));
    // The last row is the sidebar's bottom bar and the terminal's.
    assert!(rows[23].starts_with("  + New "), "{}", rows[23]);
    assert!(!rows[23].contains("detach"), "{}", rows[23]);
    let buf = t.backend().buffer();
    // Styles: header and the active project bold, the active panel a bold band of the
    // selection colours (reversed here), `no panels` and the count muted.
    assert!(buf[(1, 0)].modifier.contains(Modifier::BOLD));
    assert_eq!(buf[(1, 0)].fg, Color::Blue);
    assert!(buf[(3, 2)].modifier.contains(Modifier::BOLD));
    assert!(!buf[(3, 6)].modifier.contains(Modifier::BOLD));
    for x in [0, 3, 27] {
        assert!(buf[(x, 3)].modifier.contains(Modifier::REVERSED), "{x}");
    }
    assert!(buf[(3, 3)].modifier.contains(Modifier::BOLD));
    assert!(!buf[(28, 3)].modifier.contains(Modifier::REVERSED));
    assert!(!buf[(3, 4)].modifier.contains(Modifier::REVERSED));
    assert_eq!(buf[(3, 4)].fg, Color::Reset);
    assert_eq!(buf[(3, 9)].fg, Color::DarkGray);
    assert_eq!(buf[(26, 10)].fg, Color::DarkGray);
    assert_eq!(buf[(28, 5)].symbol(), "│");
    // Nothing is reversed outside the sidebar focus.
    assert!(!buf[(3, 4)].modifier.contains(Modifier::REVERSED));
}

#[test]
fn long_names_end_in_an_ellipsis_and_a_count_keeps_its_place() {
    let mut view = mockup();
    view.projects[0].name = "a-very-long-project-name-indeed".into();
    view.projects[0].panels[1].name = "a-very-long-panel-name-indeed-too".into();
    let extra = view.projects[0].panels[0].terminals[0].clone();
    view.projects[0].panels[1].terminals.push(extra);
    let a = app_at(&Config::default(), (100, 24), view);
    let (_, rows) = screen(&a);
    assert_eq!(
        rows[2].chars().take(29).collect::<String>(),
        format!("{}│", pad(" ▾ a-very-long-project-nam…", 28))
    );
    // The terminal count of a panel with two terminals is at the right.
    assert_eq!(
        rows[4].chars().take(29).collect::<String>(),
        format!("{}│", pad("   ❯  a-very-long-panel-… 2 ", 28))
    );
}

#[test]
fn sidebar_focus_selects_with_reversed_and_shows_the_hints() {
    let mut a2 = app();
    run(&mut a2, &[prefix(), key(ct::KeyCode::Tab)]);
    assert!(a2.sidebar_focus);
    assert_eq!(a2.selection, Some(Sel::Panel(ProjectId(1), PanelId(3))));
    run(&mut a2, &[ch('j')]);
    assert_eq!(a2.selection, Some(Sel::Panel(ProjectId(1), PanelId(4))));
    let (t, rows) = screen(&a2);
    let buf = t.backend().buffer();
    assert!(buf[(5, 4)].modifier.contains(Modifier::REVERSED));
    assert!(buf[(28 - 1, 4)].modifier.contains(Modifier::REVERSED));
    assert!(!buf[(5, 3)].modifier.contains(Modifier::REVERSED));
    // The floating bar, centered in the panel area, cuts the hints to fit.
    assert!(rows[23].contains(" SIDEBAR  ↑↓ move"), "{}", rows[23]);
    assert!(rows[23].contains("…"), "{}", rows[23]);
    // Keys do not reach the terminal while the sidebar has them.
    let outs = run(&mut a2, &[ch('x'), ch('z')]);
    assert!(
        outs.iter()
            .all(|o| !matches!(o, Out::Send(ClientMsg::Input { .. })))
    );
}

#[test]
fn sidebar_keys_navigate_open_fold_and_act() {
    let mut a = app();
    run(&mut a, &[prefix(), key(ct::KeyCode::Tab)]);
    // Up/Down and k/j clamp at the ends.
    run(&mut a, &[key(ct::KeyCode::Up), ch('k'), ch('k'), ch('k')]);
    assert_eq!(a.selection, Some(Sel::Project(ProjectId(1))));
    run(&mut a, &[key(ct::KeyCode::Down), ch('j'), ch('j')]);
    assert_eq!(a.selection, Some(Sel::Panel(ProjectId(1), PanelId(5))));
    // Enter on a panel focuses it and hands the keyboard back.
    assert_eq!(
        run(&mut a, &[key(ct::KeyCode::Enter)]),
        [action(Act::FocusPanel { panel: PanelId(5) })]
    );
    assert!(!a.sidebar_focus);

    // Collapse and expand: `h` and `l` on a panel act on its project and move the selection.
    run(&mut a, &[prefix(), key(ct::KeyCode::Tab)]);
    assert_eq!(
        run(&mut a, &[ch('h')]),
        [action(Act::SetProjectCollapsed {
            project: ProjectId(1),
            collapsed: true
        })]
    );
    assert_eq!(a.selection, Some(Sel::Project(ProjectId(1))));
    // `l` on an expanded project and `h` on a collapsed one change nothing.
    assert!(run(&mut a, &[ch('l')]).is_empty());
    typed(&mut a, "jjjjjjj");
    assert_eq!(a.selection, Some(Sel::Project(ProjectId(8))));
    assert!(run(&mut a, &[key(ct::KeyCode::Left)]).is_empty());
    assert_eq!(
        run(&mut a, &[key(ct::KeyCode::Right)]),
        [action(Act::SetProjectCollapsed {
            project: ProjectId(8),
            collapsed: false
        })]
    );
    // Enter on a project row toggles its collapse.
    assert_eq!(
        run(&mut a, &[key(ct::KeyCode::Enter)]),
        [action(Act::SetProjectCollapsed {
            project: ProjectId(8),
            collapsed: false
        })]
    );
    // `c` is a new panel in the selected project; `P` a new project.
    assert_eq!(
        run(&mut a, &[ch('c')]),
        [action(Act::NewPanel {
            project: ProjectId(8),
            name: None
        })]
    );
    run(&mut a, &[ch('P')]);
    assert!(matches!(&a.modal, Some(Modal::Prompt(p)) if p.label() == "Directory:"));
    run(&mut a, &[key(ct::KeyCode::Esc)]);
    assert!(a.modal.is_none() && a.sidebar_focus);
    // Esc and Tab hand the keyboard back.
    run(&mut a, &[key(ct::KeyCode::Esc)]);
    assert!(!a.sidebar_focus);
    run(
        &mut a,
        &[prefix(), key(ct::KeyCode::Tab), key(ct::KeyCode::Tab)],
    );
    assert!(!a.sidebar_focus);
}

#[test]
fn sidebar_rename_and_close_act_on_the_selected_row() {
    let mut a = app();
    run(&mut a, &[prefix(), key(ct::KeyCode::Tab), ch('j')]);
    run(&mut a, &[ch('r')]);
    let Some(Modal::Prompt(p)) = &a.modal else {
        panic!("{:?}", a.modal)
    };
    assert_eq!(
        (p.label(), p.editor.text()),
        ("Rename panel:", "claude".to_string())
    );
    run(&mut a, &[key(ct::KeyCode::Esc), ch('x')]);
    let Some(Modal::Confirm(c)) = &a.modal else {
        panic!("{:?}", a.modal)
    };
    assert_eq!(c.question, "Close panel \"claude\" and its terminal?");
    run(&mut a, &[ch('n'), ch('k'), ch('k'), ch('r')]);
    let Some(Modal::Prompt(p)) = &a.modal else {
        panic!("{:?}", a.modal)
    };
    assert_eq!(
        (p.label(), p.editor.text()),
        ("Rename space:", "terminal-multiplexer".to_string())
    );
}

#[test]
fn prefix_bindings_drive_panels() {
    let mut a = app();
    assert_eq!(
        pfx(&mut a, 'c'),
        [action(Act::NewPanel {
            project: ProjectId(1),
            name: None
        })]
    );
    assert_eq!(pfx(&mut a, 'n'), [action(Act::CyclePanel { delta: 1 })]);
    assert_eq!(pfx(&mut a, 'p'), [action(Act::CyclePanel { delta: -1 })]);
    assert_eq!(
        pfx(&mut a, '2'),
        [action(Act::FocusPanel { panel: PanelId(4) })]
    );
    assert!(pfx(&mut a, '9').is_empty());
}

#[test]
fn renaming_a_panel_prompts_prefilled_and_an_error_reopens_the_prompt() {
    let mut a = app();
    assert!(pfx(&mut a, ',').is_empty());
    let (_, rows) = screen(&a);
    assert!(rows[9].contains("╭ Rename panel "), "{}", rows[9]);
    assert!(rows[10].contains("│ editor█"), "{}", rows[10]);
    assert!(rows[12].contains("│ ⏎ save  esc cancel"), "{}", rows[12]);
    // Ctrl+u clears; Enter submits an empty name; the daemon refuses.
    let ctrl_u = ct::Event::Key(ct::KeyEvent::new(
        ct::KeyCode::Char('u'),
        ct::KeyModifiers::CONTROL,
    ));
    let outs = run(&mut a, &[ctrl_u, key(ct::KeyCode::Enter)]);
    assert_eq!(
        outs,
        [action(Act::RenamePanel {
            panel: PanelId(3),
            name: String::new()
        })]
    );
    assert!(a.modal.is_none());
    a.on_server(ServerMsg::ActionResult(Err(ActionError::EmptyName)));
    assert_eq!(a.flash.as_ref().unwrap().0, "name must not be empty");
    assert!(a.flash_error);
    let (t, rows) = screen(&a);
    assert!(rows[10].contains("│ █"), "{}", rows[10]);
    assert!(
        rows[12].contains("│ name must not be empty"),
        "{}",
        rows[12]
    );
    let at = rows[12].split("name must").next().unwrap().chars().count();
    let red = t.backend().buffer()[(u16::try_from(at).unwrap(), 12)].fg;
    assert_eq!(red, Color::Red);
    // The floating bar does not repeat it.
    assert!(!rows[23].contains("name must not"), "{}", rows[23]);
    // Typing again and saving: success closes it for good.
    typed(&mut a, "docs");
    let outs = run(&mut a, &[key(ct::KeyCode::Enter)]);
    assert_eq!(
        outs,
        [action(Act::RenamePanel {
            panel: PanelId(3),
            name: "docs".into()
        })]
    );
    a.on_server(ServerMsg::ActionResult(Ok(())));
    assert!(a.modal.is_none());
    assert!(a.pending.is_empty());
}

#[test]
fn the_prompt_edits_pastes_and_takes_the_prefix_as_text() {
    let mut a = app();
    pfx(&mut a, ',');
    run(
        &mut a,
        &[
            key(ct::KeyCode::Home),
            key(ct::KeyCode::Delete),
            key(ct::KeyCode::Right),
        ],
    );
    typed(&mut a, "X");
    run(
        &mut a,
        &[key(ct::KeyCode::Left), key(ct::KeyCode::Backspace)],
    );
    // The prefix inserts nothing and arms nothing.
    assert!(run(&mut a, &[prefix()]).is_empty());
    assert!(!a.armed);
    a.on_event(ct::Event::Paste("a\nb".into()), Instant::now());
    let Some(Modal::Prompt(p)) = &a.modal else {
        panic!()
    };
    assert_eq!(p.editor.text(), "abXitor");
}

#[test]
fn a_new_project_asks_the_directory_then_the_name() {
    let mut a = app();
    pfx(&mut a, 'P');
    let Some(Modal::Prompt(p)) = &a.modal else {
        panic!()
    };
    assert_eq!(
        (p.label(), p.editor.text()),
        ("Directory:", "/work/proj".to_string())
    );
    // `~` proposes `~`.
    run(&mut a, &[ctrl('u')]);
    typed(&mut a, "~");
    run(&mut a, &[key(ct::KeyCode::Enter)]);
    let Some(Modal::Prompt(p)) = &a.modal else {
        panic!()
    };
    assert_eq!(
        (p.label(), p.editor.text()),
        ("Space name:", "~".to_string())
    );
    let outs = run(&mut a, &[key(ct::KeyCode::Enter)]);
    assert_eq!(
        outs,
        [action(Act::NewProject {
            name: "~".into(),
            root: "/home/me".into()
        })]
    );

    // A relative directory resolves against the client's start directory.
    a.on_server(ServerMsg::ActionResult(Ok(())));
    pfx(&mut a, 'P');
    run(&mut a, &[ctrl('u')]);
    typed(&mut a, "../other");
    run(&mut a, &[key(ct::KeyCode::Enter)]);
    typed(&mut a, "2");
    let outs = run(&mut a, &[key(ct::KeyCode::Enter)]);
    assert_eq!(
        outs,
        [action(Act::NewProject {
            name: "other2".into(),
            root: "/work/other".into()
        })]
    );

    // A refused name reopens the name prompt with the text; a refused directory the first one.
    a.on_server(ServerMsg::ActionResult(Err(ActionError::ProjectExists(
        "other2".into(),
    ))));
    let Some(Modal::Prompt(p)) = &a.modal else {
        panic!()
    };
    assert_eq!(
        (p.label(), p.editor.text()),
        ("Space name:", "other2".to_string())
    );
    run(&mut a, &[key(ct::KeyCode::Enter)]);
    a.on_server(ServerMsg::ActionResult(Err(ActionError::NotADirectory(
        "/work/other".into(),
    ))));
    let Some(Modal::Prompt(p)) = &a.modal else {
        panic!()
    };
    assert_eq!(
        (p.label(), p.editor.text()),
        ("Directory:", "../other".to_string())
    );
    assert_eq!(
        a.flash.as_ref().unwrap().0,
        "\"/work/other\" is not a directory"
    );
}

fn ctrl(c: char) -> ct::Event {
    ct::Event::Key(ct::KeyEvent::new(
        ct::KeyCode::Char(c),
        ct::KeyModifiers::CONTROL,
    ))
}

#[test]
fn closing_asks_first_and_only_y_confirms() {
    let mut a = app();
    pfx(&mut a, 'X');
    let (t, rows) = screen(&a);
    assert!(
        rows[10].contains("│ Close panel \"editor\" and its terminal?"),
        "{}",
        rows[10]
    );
    assert!(rows[12].contains("│ y yes  n no"), "{}", rows[12]);
    let at = rows[10].split("Close").next().unwrap().chars().count();
    assert_eq!(
        t.backend().buffer()[(u16::try_from(at).unwrap(), 10)].fg,
        Color::Yellow
    );
    // Other keys, the prefix and pastes are ignored.
    assert!(
        run(
            &mut a,
            &[ch('z'), key(ct::KeyCode::Enter), prefix(), ch('d')]
        )
        .is_empty()
    );
    assert!(matches!(a.modal, Some(Modal::Confirm(_))));
    assert!(run(&mut a, &[ch('n')]).is_empty());
    assert!(a.modal.is_none());
    pfx(&mut a, 'X');
    assert!(run(&mut a, &[key(ct::KeyCode::Esc)]).is_empty());
    assert!(a.modal.is_none());
    pfx(&mut a, 'X');
    assert_eq!(
        run(&mut a, &[ch('y')]),
        [action(Act::ClosePanel { panel: PanelId(3) })]
    );
    assert!(a.modal.is_none());

    pfx(&mut a, 'Q');
    let Some(Modal::Confirm(c)) = &a.modal else {
        panic!()
    };
    assert_eq!(
        c.question,
        "Close space \"terminal-multiplexer\" and its 3 terminals?"
    );
    assert_eq!(
        run(&mut a, &[ch('y')]),
        [action(Act::CloseProject {
            project: ProjectId(1)
        })]
    );
}

#[test]
fn a_dialog_whose_target_goes_away_closes_with_a_flash() {
    let mut a = app();
    pfx(&mut a, ',');
    let mut v = mockup();
    v.projects[0].panels.remove(0);
    v.projects[0].active_panel = Some(PanelId(4));
    v.active_panel = Some(PanelId(4));
    a.on_server(ServerMsg::SessionView(v));
    assert!(a.modal.is_none());
    assert_eq!(a.flash.as_ref().unwrap().0, "panel \"editor\" was closed");

    let mut a = app();
    pfx(&mut a, 'Q');
    let mut v = mockup();
    v.projects.remove(0);
    v.active_project = Some(ProjectId(2));
    v.active_panel = Some(PanelId(6));
    a.on_server(ServerMsg::SessionView(v));
    assert!(a.modal.is_none());
    assert_eq!(
        a.flash.as_ref().unwrap().0,
        "space \"terminal-multiplexer\" was closed"
    );

    // A dialog about something that is still there stays open.
    let mut a = app();
    pfx(&mut a, ',');
    a.on_server(ServerMsg::SessionView(mockup()));
    assert!(a.modal.is_some());
}

#[test]
fn empty_states_and_typing_that_goes_nowhere() {
    let mut view = mockup();
    view.projects = vec![project(7, "empty", false, vec![])];
    view.active_project = Some(ProjectId(7));
    view.active_panel = None;
    let mut a = app_at(&Config::default(), (100, 24), view);
    let (_, rows) = screen(&a);
    let text = format!("No panels in empty — {} c to create one", prefix_label());
    assert!(rows[12].contains(&text), "{rows:#?}");
    assert!(!rows.iter().any(|r| r.contains('╭')));
    // Typing and pasting go nowhere, and are not kept for later either.
    assert!(run(&mut a, &[ch('a'), key(ct::KeyCode::Enter)]).is_empty());
    assert!(
        a.on_event(ct::Event::Paste("p".into()), Instant::now())
            .is_empty()
    );

    let mut view = mockup();
    view.projects.clear();
    view.active_project = None;
    view.active_panel = None;
    let a = app_at(&Config::default(), (100, 24), view);
    let (_, rows) = screen(&a);
    let text = format!("No spaces — {} P to add one", prefix_label());
    assert!(rows[12].contains(&text), "{rows:#?}");
}

#[test]
fn the_sidebar_hides_when_the_window_is_narrow_and_toggles_by_key() {
    let wide = |width| app_at(&Config::default(), (width, 24), mockup());
    // The sidebar narrows with the window (to the 20-column limit); under 41 columns it hides.
    assert_eq!(wide(49).viewport(), (20, 24));
    assert!(wide(48).sidebar_visible());
    assert_eq!(wide(48).sidebar_width, 27);
    assert_eq!(wide(48).viewport(), (20, 24));
    assert!(wide(41).sidebar_visible());
    assert!(!wide(40).sidebar_visible());
    assert_eq!(wide(40).viewport(), (40, 24));
    let wider = Config {
        sidebar_width: 60,
        ..Config::default()
    };
    assert!(app_at(&wider, (81, 24), mockup()).sidebar_visible());
    assert!(app_at(&wider, (80, 24), mockup()).sidebar_visible());

    let mut a = app();
    assert_eq!(a.viewport(), (71, 24));
    assert_eq!(
        pfx(&mut a, 'b'),
        [Out::Send(ClientMsg::Viewport {
            cols: 100,
            rows: 24
        })]
    );
    let (_, rows) = screen(&a);
    assert!(!rows[0].contains('│'), "{}", rows[0]);
    assert_eq!(
        pfx(&mut a, 'b'),
        [Out::Send(ClientMsg::Viewport { cols: 71, rows: 24 })]
    );
    // It reappears when the window grows again, and resizing sends the panel area.
    let mut a = app_at(&Config::default(), (100, 24), mockup());
    // Under 41 columns it hides; it comes back at the lower limit, and does not grow.
    let outs = run(&mut a, &[ct::Event::Resize(40, 24)]);
    assert_eq!(
        outs,
        [Out::Send(ClientMsg::Viewport { cols: 40, rows: 24 })]
    );
    assert!(!a.sidebar_visible());
    let outs = run(&mut a, &[ct::Event::Resize(100, 24)]);
    assert_eq!(
        outs,
        [Out::Send(ClientMsg::Viewport { cols: 79, rows: 24 })]
    );
    assert_eq!(a.sidebar_width, 20);
    // A hidden sidebar stays hidden when the window grows.
    let mut a = app_at(&Config::default(), (100, 24), mockup());
    pfx(&mut a, 'b');
    let outs = run(&mut a, &[ct::Event::Resize(120, 30)]);
    assert_eq!(
        outs,
        [Out::Send(ClientMsg::Viewport {
            cols: 120,
            rows: 30
        })]
    );
    // Hidden from the config.
    let a = app_at(&no_sidebar(), (100, 24), mockup());
    assert!(!a.sidebar_visible());
}

#[test]
fn clicks_focus_panels_and_fold_projects_and_the_wheel_scrolls() {
    let mut a = app();
    // Rows: header 0, blank 1, project 2, editor 3, claude 4.
    assert_eq!(
        run(&mut a, &[click(5, 4)]),
        [action(Act::FocusPanel { panel: PanelId(4) })]
    );
    assert_eq!(
        run(&mut a, &[click(5, 2)]),
        [action(Act::SetProjectCollapsed {
            project: ProjectId(1),
            collapsed: true
        })]
    );
    assert_eq!(
        run(&mut a, &[click(5, 12)]),
        [] as [Out; 0],
        "past the last row"
    );
    // The collapsed project of the mockup opens on a click.
    assert_eq!(
        run(&mut a, &[click(5, 10)]),
        [action(Act::SetProjectCollapsed {
            project: ProjectId(8),
            collapsed: false
        })]
    );
    // The separator, the header, the blank row and `no panels` do nothing.
    assert!(
        run(
            &mut a,
            &[click(28, 4), click(5, 0), click(5, 1), click(5, 9)]
        )
        .is_empty()
    );
    // Clicks are not sent to the terminal (it did not ask for the mouse anyway).
    assert!(run(&mut a, &[click(40, 4)]).is_empty());
}

fn many_panels() -> SessionView {
    let mut view = mockup();
    view.projects = vec![project(
        1,
        "big",
        false,
        (1..=30).map(|n| panel(n, &format!("p{n}"), 1)).collect(),
    )];
    view.active_project = Some(ProjectId(1));
    view.active_panel = Some(PanelId(1));
    view
}

#[test]
fn the_sidebar_scrolls_to_keep_the_active_panel_and_the_selection_visible() {
    let mut a = app_at(&Config::default(), (100, 24), many_panels());
    let (_, rows) = screen(&a);
    assert!(rows[3].starts_with("   ❯  p1"), "{}", rows[3]);
    // The active panel moves down the list.
    let mut view = many_panels();
    view.projects[0].active_panel = Some(PanelId(30));
    view.active_panel = Some(PanelId(30));
    a.on_server(ServerMsg::SessionView(view));
    let (_, rows) = screen(&a);
    let at = rows
        .iter()
        .position(|r| r.starts_with("   ❯  p30"))
        .unwrap_or_else(|| panic!("{rows:#?}"));
    assert_eq!(at, 21, "the last row of the list");
    assert!(rows[2].starts_with("   ❯  p11"), "{}", rows[2]);
    // The selection scrolls it back up.
    run(&mut a, &[prefix(), key(ct::KeyCode::Tab)]);
    for _ in 0..28 {
        run(&mut a, &[ch('k')]);
    }
    let (_, rows) = screen(&a);
    assert!(rows.iter().any(|r| r.starts_with("   ❯  p2 ")), "{rows:#?}");
    // The wheel scrolls by three rows and stops at the ends.
    run(&mut a, &[mouse(ct::MouseEventKind::ScrollDown, 5, 5)]);
    let first = a.sidebar_offset;
    run(&mut a, &[mouse(ct::MouseEventKind::ScrollDown, 5, 5)]);
    assert_eq!(a.sidebar_offset, first + 3);
    for _ in 0..20 {
        run(&mut a, &[mouse(ct::MouseEventKind::ScrollDown, 5, 5)]);
    }
    assert_eq!(a.sidebar_offset, 31 - 20);
    for _ in 0..20 {
        run(&mut a, &[mouse(ct::MouseEventKind::ScrollUp, 5, 5)]);
    }
    assert_eq!(a.sidebar_offset, 0);
}

#[test]
fn the_help_overlay_lists_every_binding_and_closes() {
    // Tall enough for every group.
    let mut a = app_at(&Config::default(), (100, 40), mockup());
    pfx(&mut a, '?');
    let (t, rows) = screen(&a);
    let all = rows.join("\n");
    let p = prefix_label();
    for line in [
        "Session",
        "Panels",
        "Spaces",
        "Terminal",
        "Sidebar",
        "Help",
        &format!("{p} d"),
        &format!("{p} c"),
        &format!("{p} n"),
        &format!("{p} p"),
        &format!("{p} 1-9"),
        &format!("{p} ,"),
        &format!("{p} X"),
        &format!("{p} P"),
        &format!("{p} ."),
        &format!("{p} Q"),
        &format!("{p} ["),
        &format!("{p} b"),
        &format!("{p} Tab"),
        &format!("{p} ?"),
        "Detach from the session",
        "New panel",
    ] {
        assert!(all.contains(line), "{line:?} missing:\n{all}");
    }
    // A rounded box in the middle; the groups are in the documented order (the last match:
    // the header names the session).
    assert!(all.contains('╭') && all.contains('╰'));
    let order = ["Session", "Panels", "Spaces", "Terminal", "Sidebar"].map(|g| {
        rows.iter()
            .rposition(|r| r.contains(&format!(" {g}")))
            .unwrap()
    });
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
    // The terminal's cursor is hidden under it.
    let mut t = t;
    assert!(t.get_cursor_position().is_ok());
    // Typing is swallowed; Esc, q and ? close it.
    assert!(run(&mut a, &[ch('x'), prefix()]).is_empty());
    assert!(matches!(a.modal, Some(Modal::Help)));
    for close in [key(ct::KeyCode::Esc), ch('q'), ch('?')] {
        a.modal = Some(Modal::Help);
        assert!(run(&mut a, &[close]).is_empty());
        assert!(a.modal.is_none());
    }
}

#[test]
fn the_help_shows_overrides() {
    let config = Config::parse("[keys]\nnew_panel = \"t\"\npanel_1 = \"F1\"")
        .unwrap()
        .config;
    let mut a = app_at(&config, (100, 24), mockup());
    pfx(&mut a, '?');
    let (_, rows) = screen(&a);
    let all = rows.join("\n");
    assert!(all.contains(&format!("{} t", prefix_label())), "{all}");
    assert!(!all.contains(&format!("{} c ", prefix_label())), "{all}");
    assert!(all.contains("F1 2 3 4 5 6 7 8 9"), "{all}");
}

#[test]
fn detach_works_in_sidebar_focus_and_scroll_mode_and_a_dialog_blocks_it() {
    let mut a = app();
    run(&mut a, &[prefix(), key(ct::KeyCode::Tab)]);
    let outs = pfx(&mut a, 'd');
    assert!(outs.contains(&Out::Send(ClientMsg::Detach)));

    let mut a = app();
    pfx(&mut a, '[');
    assert_eq!(a.scroll_mode, Some(TerminalId(31)));
    let outs = pfx(&mut a, 'd');
    assert!(outs.contains(&Out::Send(ClientMsg::Detach)));

    // Entering the sidebar ends scroll mode.
    let mut a = app();
    pfx(&mut a, '[');
    run(&mut a, &[prefix(), key(ct::KeyCode::Tab)]);
    assert_eq!(a.scroll_mode, None);
    assert!(a.sidebar_focus);
}

#[test]
fn sidebar_focus_ends_when_a_resize_hides_the_sidebar() {
    let mut a = app();
    run(&mut a, &[prefix(), key(ct::KeyCode::Tab)]);
    assert!(a.sidebar_focus);
    run(&mut a, &[ct::Event::Resize(40, 24)]);
    assert!(!a.sidebar_focus);
    run(&mut a, &[ct::Event::Resize(100, 24)]);
    assert!(!a.sidebar_focus);
}

#[test]
fn a_wheel_scrolled_list_stays_put_on_a_view_update_until_the_active_panel_moves() {
    let mut a = app_at(&Config::default(), (100, 24), many_panels());
    run(&mut a, &[mouse(ct::MouseEventKind::ScrollDown, 5, 5)]);
    assert_eq!(a.sidebar_offset, 3);
    a.on_server(ServerMsg::SessionView(many_panels()));
    assert_eq!(a.sidebar_offset, 3);
    let mut view = many_panels();
    view.projects[0].active_panel = Some(PanelId(30));
    view.active_panel = Some(PanelId(30));
    a.on_server(ServerMsg::SessionView(view));
    assert_eq!(a.sidebar_offset, 11);
}

#[test]
fn a_click_reaches_the_program_with_the_sidebar_taken_off() {
    use deck_core::protocol::Modes;
    let mut a = app();
    let mut f = frame(true, (40, 10), vec![]);
    f.terminal = a.mirror.focused_id().unwrap();
    f.modes = Modes(Modes::MOUSE_CLICK | Modes::SGR_MOUSE);
    a.on_server(ServerMsg::Frame(f));
    // Column 29 is the first of the panel area: terminal column 0, 1-based 1.
    let outs = run(&mut a, &[click(29, 0)]);
    let [Out::Send(ClientMsg::Input { bytes, .. })] = outs.as_slice() else {
        panic!("{outs:?}")
    };
    assert_eq!(bytes, b"\x1b[<0;1;1M");
    // The separator column and the sidebar are not the program's.
    assert!(run(&mut a, &[click(28, 1), click(10, 1)]).len() <= 1);
    assert!(
        !run(&mut a, &[click(28, 1)])
            .iter()
            .any(|o| matches!(o, Out::Send(ClientMsg::Input { .. })))
    );
}

#[test]
fn toggling_a_sidebar_that_is_too_narrow_to_show_says_so() {
    let mut a = app_at(&Config::default(), (40, 24), mockup());
    a.sidebar_hidden = true;
    pfx(&mut a, 'b');
    assert!(!a.sidebar_hidden);
    assert_eq!(
        a.flash.as_ref().unwrap().0,
        "sidebar hidden (window too narrow)"
    );
}

#[test]
fn a_help_taller_than_the_window_scrolls_and_says_how_much_is_hidden() {
    let screen = |a: &App| {
        let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
        t.draw(|f| draw(f, a)).unwrap();
        let buf = t.backend().buffer().clone();
        (0..24u16)
            .map(|y| (0..80u16).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
    };
    let mut a = app_at(&Config::default(), (80, 24), mockup());
    run(&mut a, &[prefix(), ch('?')]);
    let rows = screen(&a);
    assert!(!rows.iter().any(|r| r.contains("This help")), "{rows:#?}");
    let hidden = rows
        .iter()
        .find_map(|r| r.split("↓ ").nth(1))
        .and_then(|t| t.split(' ').next()?.parse::<usize>().ok())
        .expect("a hidden count on the bottom border");
    assert!(
        rows.iter()
            .any(|r| r.contains("↓ ") && r.contains("esc close"))
    );
    assert!(!rows.iter().any(|r| r.contains('↑')));

    // Down scrolls one line at a time; End shows the last row.
    run(&mut a, &[key(ct::KeyCode::Down)]);
    assert!(screen(&a).iter().any(|r| r.contains("↑ 1 more")));
    run(&mut a, &[key(ct::KeyCode::End)]);
    let rows = screen(&a);
    assert!(rows.iter().any(|r| r.contains("This help")), "{rows:#?}");
    assert!(rows.iter().any(|r| r.contains(&format!("↑ {hidden} more"))));
    assert!(!rows.iter().any(|r| r.contains('↓')));
    // It stops at the end, and Home, PageUp and k go back.
    run(&mut a, &[ch('j'), key(ct::KeyCode::PageDown)]);
    assert!(screen(&a).iter().any(|r| r.contains("This help")));
    run(&mut a, &[ch('k')]);
    let rows = screen(&a);
    assert!(rows.iter().any(|r| r.contains("↓ 1 more")), "{rows:#?}");
    assert_eq!(rows.iter().any(|r| r.contains('↑')), hidden > 1);
    run(&mut a, &[key(ct::KeyCode::Home)]);
    assert!(!screen(&a).iter().any(|r| r.contains('↑')));
    run(
        &mut a,
        &[key(ct::KeyCode::PageDown), key(ct::KeyCode::PageUp)],
    );
    assert!(!screen(&a).iter().any(|r| r.contains('↑')));
    // Opening it again starts at the top.
    run(
        &mut a,
        &[
            key(ct::KeyCode::End),
            key(ct::KeyCode::Esc),
            prefix(),
            ch('?'),
        ],
    );
    assert!(!screen(&a).iter().any(|r| r.contains('↑')));
}
