//! The sidebar, panels and projects, end to end: the real `deck` in a PTY, through the harness.

mod support;

use std::path::PathBuf;
use std::time::Duration;

use support::{Harness, TIMEOUT, wait_until};

const SIZE: (u16, u16) = (100, 24);
const PREFIX: &[u8] = b"\x1b ";
const CTRL_U: &str = "\x15";
const ENTER: &str = "\r";
const ESC: &str = "\x1b";
const TAB: &str = "\t";

/// `deck` with the sidebar of the product's defaults, once its first prompt is up.
fn attached() -> Harness {
    attached_with(SIZE, &[])
}

fn attached_with(size: (u16, u16), config: &[(&str, &str)]) -> Harness {
    let mut builder = Harness::builder(size).config_unset("sidebar");
    for (key, value) in config {
        builder = builder.config(key, value);
    }
    let h = builder.deck();
    h.wait_for_text("│$", TIMEOUT);
    h
}

/// `deck` whose first panel is named `1` by hand, so panels can be told apart by number.
fn numbered() -> Harness {
    let mut h = attached();
    h.wait_for_text("▌  ❯  sh", TIMEOUT);
    prefixed(&mut h, ",");
    h.wait_for_text("Rename panel: sh", TIMEOUT);
    h.send_keys(CTRL_U);
    h.send_keys("1");
    h.send_keys(ENTER);
    wait_for_row(&h, "▌  ❯  1");
    h
}

/// The prefix, then `key`.
fn prefixed(h: &mut Harness, key: &str) {
    h.send_bytes(PREFIX);
    h.send_keys(key);
}

fn rows(h: &Harness) -> Vec<String> {
    h.screen_text().lines().map(str::to_string).collect()
}

/// A new panel, which is named after its shell at once, then named `name` by hand so the
/// tests can tell the panels apart.
fn new_panel(h: &mut Harness, name: &str) {
    prefixed(h, "c");
    wait_for_row(h, "▌  ❯  sh");
    prefixed(h, ",");
    h.wait_for_text("Rename panel: sh", TIMEOUT);
    h.send_keys(CTRL_U);
    h.send_keys(name);
    h.send_keys(ENTER);
    wait_for_row(h, &format!("▌  ❯  {name}"));
}

/// Waits until some screen row starts with `prefix`.
fn wait_for_row(h: &Harness, prefix: &str) {
    wait_until(&format!("a row starting {prefix:?}"), TIMEOUT, || {
        rows(h).iter().any(|r| r.starts_with(prefix))
    });
}

fn wait_gone(h: &Harness, text: &str) {
    wait_until(&format!("{text:?} to go"), TIMEOUT, || {
        !h.screen_text().contains(text)
    });
}

fn second_client(first: &Harness, size: (u16, u16)) -> Harness {
    let path = first.home().path().to_string_lossy().into_owned();
    Harness::builder(size).env("DECK_HOME", &path).deck()
}

/// What the shell in the panel area reports for `stty size`, once it answers.
fn stty(h: &mut Harness) -> String {
    h.send_keys("clear; echo S$(stty size)E\r");
    let deadline = std::time::Instant::now() + TIMEOUT;
    loop {
        let text = h.screen_text();
        if let Some(a) = text.find("S2").or_else(|| text.find("S1"))
            && let Some(b) = text[a..].find('E')
        {
            return text[a + 1..a + b].to_string();
        }
        assert!(std::time::Instant::now() < deadline, "no size:\n{text}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn hint_prefix() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌥Space"
    } else {
        "M-Space"
    }
}

#[test]
fn the_sidebar_shows_the_session_project_and_first_panel() {
    let mut h = attached();
    h.wait_for_text("▌  ❯  sh", TIMEOUT);
    let path = h.capture("sidebar");
    assert!(path.is_file());
    let rows = rows(&h);
    assert!(rows[0].starts_with(" Spaces "), "{}", rows[0]);
    assert!(rows[2].starts_with(" ▾ dk"), "{}", rows[2]);
    assert!(rows[3].starts_with("▌  ❯  sh"), "{}", rows[3]);
    assert_eq!(rows[0].chars().nth(28), Some('│'));
    // The terminal starts right after the separator: no border, no title.
    assert!(
        rows[0]
            .chars()
            .skip(29)
            .collect::<String>()
            .starts_with('$'),
        "{}",
        rows[0]
    );
    assert!(rows[23].starts_with(" main  dk"), "{}", rows[23]);
    assert!(rows[23].contains("› ❯  sh"), "{}", rows[23]);
    assert!(rows[23].ends_with(&format!("{} ? help  d detach", hint_prefix())));
    // The panel area is the window less the sidebar and its separator: 71 columns.
    assert_eq!(stty(&mut h), "23 71");
}

#[test]
fn a_new_panel_is_added_focused_and_switching_shows_the_right_shell() {
    let mut h = numbered();
    h.send_keys("echo ONE\r");
    h.wait_for_text("│ONE", TIMEOUT);
    new_panel(&mut h, "2");
    h.wait_for_text("› ❯  2", TIMEOUT);
    wait_gone(&h, "│ONE");
    h.send_keys("echo TWO\r");
    h.wait_for_text("│TWO", TIMEOUT);
    // `p` goes back, `n` forward, `1` and `2` by number.
    prefixed(&mut h, "p");
    h.wait_for_text("│ONE", TIMEOUT);
    wait_for_row(&h, "▌  ❯  1");
    assert!(!h.screen_text().contains("│TWO"));
    prefixed(&mut h, "n");
    h.wait_for_text("│TWO", TIMEOUT);
    prefixed(&mut h, "1");
    h.wait_for_text("│ONE", TIMEOUT);
    wait_for_row(&h, "▌  ❯  1");
    prefixed(&mut h, "2");
    h.wait_for_text("│TWO", TIMEOUT);
    h.capture("two-panels");
}

#[test]
fn renaming_a_panel_and_an_empty_name_shows_the_error_and_reopens_the_prompt() {
    let mut h = attached();
    prefixed(&mut h, ",");
    h.wait_for_text("Rename panel: sh", TIMEOUT);
    h.send_keys(CTRL_U);
    h.send_keys(ENTER);
    h.wait_for_text("name must not be empty", TIMEOUT);
    h.wait_for_text("Rename panel:", TIMEOUT);
    h.capture("rename-error");
    h.send_keys("work");
    h.send_keys(ENTER);
    wait_for_row(&h, "▌  ❯  work");
    h.wait_for_text("› ❯  work", TIMEOUT);
    assert!(!h.screen_text().contains("Rename panel"));
}

#[test]
fn a_new_project_takes_a_directory_and_a_name() {
    let mut h = attached();
    let home = std::env::var("HOME").expect("HOME");
    let home = std::fs::canonicalize(home).expect("the home directory");
    prefixed(&mut h, "P");
    h.wait_for_text("Directory:", TIMEOUT);
    h.send_keys(CTRL_U);
    h.send_keys("~");
    h.send_keys(ENTER);
    // The home directory proposes `~`.
    h.wait_for_text("Space name: ~", TIMEOUT);
    h.send_keys(ENTER);
    wait_for_row(&h, " ▾ ~");
    h.wait_for_text("› ❯  sh", TIMEOUT);
    wait_until("the new shell", TIMEOUT, || {
        rows(&h).iter().any(|r| r.contains("│$"))
    });
    h.send_keys("echo P:$(pwd -P):E\r");
    h.wait_for_text(&format!("P:{}:E", home.display()), TIMEOUT);

    // A second project in a typed name; a duplicate shows the error and reopens the prompt.
    prefixed(&mut h, "P");
    h.wait_for_text("Directory:", TIMEOUT);
    h.send_keys(CTRL_U);
    h.send_keys("~");
    h.send_keys(ENTER);
    h.wait_for_text("Space name: ~", TIMEOUT);
    h.send_keys(ENTER);
    h.wait_for_text("a space named \"~\" already exists", TIMEOUT);
    h.wait_for_text("Space name: ~", TIMEOUT);
    h.send_keys(CTRL_U);
    h.send_keys("other");
    h.send_keys(ENTER);
    wait_for_row(&h, " ▾ other");
    h.capture("projects");
    wait_until("the new shell", TIMEOUT, || {
        rows(&h).iter().any(|r| r.contains("│$"))
    });
    h.send_keys("echo O:$(pwd -P):E\r");
    h.wait_for_text(&format!("O:{}:E", home.display()), TIMEOUT);
}

#[test]
fn a_relative_directory_resolves_against_the_start_directory() {
    let mut h = attached();
    prefixed(&mut h, "P");
    h.wait_for_text("Directory:", TIMEOUT);
    h.send_keys(CTRL_U);
    // The harness starts deck in /tmp/dk…; `..` is /tmp.
    h.send_keys("..");
    h.send_keys(ENTER);
    h.wait_for_text("Space name: tmp", TIMEOUT);
    h.send_keys(ENTER);
    wait_for_row(&h, " ▾ tmp");
    wait_until("the new shell", TIMEOUT, || {
        rows(&h).iter().any(|r| r.contains("│$"))
    });
    h.send_keys("echo D:$(pwd -P):E\r");
    let tmp = std::fs::canonicalize("/tmp").unwrap_or_else(|_| PathBuf::from("/tmp"));
    h.wait_for_text(&format!("D:{}:E", tmp.display()), TIMEOUT);
}

#[test]
fn closing_a_panel_asks_first_and_focus_moves_on() {
    let mut h = numbered();
    new_panel(&mut h, "2");
    prefixed(&mut h, "X");
    h.wait_for_text("Close panel \"2\" and its terminal? (y/n)", TIMEOUT);
    h.capture("confirm-close");
    h.send_keys("n");
    wait_gone(&h, "Close panel");
    assert!(rows(&h).iter().any(|r| r.starts_with("▌  ❯  2")));
    prefixed(&mut h, "X");
    h.wait_for_text("(y/n)", TIMEOUT);
    h.send_keys("y");
    wait_for_row(&h, "▌  ❯  1");
    wait_gone(&h, "   ❯  2");
}

#[test]
fn closing_the_last_panel_shows_the_empty_state_and_a_new_panel_recovers() {
    let mut h = attached();
    prefixed(&mut h, "X");
    h.wait_for_text("(y/n)", TIMEOUT);
    h.send_keys("y");
    h.wait_for_text("No panels in dk", TIMEOUT);
    h.wait_for_text(&format!("{} c to create one", hint_prefix()), TIMEOUT);
    wait_gone(&h, "│$");
    h.capture("empty-project");
    assert!(!h.screen_text().contains('╭'));
    // Typing goes nowhere.
    h.send_keys("echo lost\r");
    prefixed(&mut h, "c");
    wait_for_row(&h, "▌  ❯  sh");
    h.wait_for_text("│$", TIMEOUT);
    assert!(!h.screen_text().contains("lost"), "{}", h.screen_text());
}

#[test]
fn sidebar_focus_opens_a_panel_and_folds_a_project() {
    let mut h = numbered();
    new_panel(&mut h, "2");
    h.send_keys("echo TWO\r");
    h.wait_for_text("│TWO", TIMEOUT);
    prefixed(&mut h, "1");
    wait_for_row(&h, "▌  ❯  1");
    prefixed(&mut h, TAB);
    h.wait_for_text("SIDEBAR", TIMEOUT);
    h.send_keys("j");
    h.wait_for_text("↑↓ move", TIMEOUT);
    h.capture("sidebar-focus-selected");
    h.send_keys(ENTER);
    wait_for_row(&h, "▌  ❯  2");
    h.wait_for_text("│TWO", TIMEOUT);
    wait_gone(&h, "SIDEBAR");
    // Collapse: the panels go. The project row keeps its panel count.
    prefixed(&mut h, TAB);
    h.wait_for_text("SIDEBAR", TIMEOUT);
    h.send_keys("h");
    wait_until("the collapsed project", TIMEOUT, || {
        rows(&h)
            .iter()
            .any(|r| r.starts_with(" ▸ dk") && r.contains("2 │"))
    });
    assert!(!rows(&h).iter().any(|r| r.starts_with("▌  ❯  2")));
    h.capture("sidebar-focus");
    h.send_keys(ESC);
    // Esc must be handled before the next prefix, or the two merge into one key.
    wait_gone(&h, "SIDEBAR");
    wait_until("the panel count", TIMEOUT, || {
        rows(&h)
            .iter()
            .any(|r| r.starts_with(" ▸ dk") && r.contains("2 │"))
    });
    prefixed(&mut h, TAB);
    h.wait_for_text("SIDEBAR", TIMEOUT);
    h.send_keys("l");
    wait_for_row(&h, "▌  ❯  2");
    // Esc hands the keyboard back to the shell.
    h.send_keys(ESC);
    wait_gone(&h, "SIDEBAR");
    h.send_keys("echo BACK\r");
    h.wait_for_text("│BACK", TIMEOUT);
}

#[test]
fn the_mouse_focuses_panels_and_folds_projects() {
    let mut h = numbered();
    new_panel(&mut h, "2");
    // Panel 1 is on row 4 (1-based), the project on row 3.
    h.send_keys("\x1b[<0;6;4M\x1b[<0;6;4m");
    wait_for_row(&h, "▌  ❯  1");
    h.send_keys("\x1b[<0;6;3M\x1b[<0;6;3m");
    wait_until("the collapsed project", TIMEOUT, || {
        rows(&h).iter().any(|r| r.starts_with(" ▸ dk"))
    });
    h.send_keys("\x1b[<0;6;3M\x1b[<0;6;3m");
    wait_until("the open project", TIMEOUT, || {
        rows(&h).iter().any(|r| r.starts_with(" ▾ dk"))
    });
}

#[test]
fn toggling_the_sidebar_resizes_the_shell_by_its_width_and_separator() {
    let mut h = attached();
    assert_eq!(stty(&mut h), "23 71");
    prefixed(&mut h, "b");
    wait_gone(&h, "▾");
    wait_until("the wider shell", TIMEOUT, || stty(&mut h) == "23 100");
    prefixed(&mut h, "b");
    h.wait_for_text("▾", TIMEOUT);
    wait_until("the narrower shell", TIMEOUT, || stty(&mut h) == "23 71");
}

#[test]
fn the_sidebar_hides_in_a_narrow_window_and_comes_back() {
    // Under 41 columns there is no room for the 20-column sidebar and a 20-column panel area.
    let mut h = attached_with((49, 24), &[]);
    h.wait_for_text("▾", TIMEOUT);
    h.resize(40, 24);
    wait_gone(&h, "▾");
    h.resize(49, 24);
    h.wait_for_text("▾", TIMEOUT);

    // A wide sidebar shrinks with the window instead of hiding (see sidebar_mouse.rs).
    let mut h = attached_with((81, 24), &[("sidebar_width", "60")]);
    h.wait_for_text("▾", TIMEOUT);
    h.resize(80, 24);
    h.resize(40, 24);
    wait_gone(&h, "▾");
}

#[test]
fn the_help_overlay_lists_the_bindings_and_shows_overrides() {
    // Tall enough for every group.
    let mut h = attached_with((100, 40), &[]);
    prefixed(&mut h, "?");
    h.wait_for_text("This help", TIMEOUT);
    h.capture("help");
    let screen = h.screen_text();
    let p = hint_prefix();
    for text in [
        "Session",
        "Panels",
        "Spaces",
        "Terminal",
        "Sidebar",
        "New panel",
        "Next panel",
        "Previous panel",
        "Close the space",
        "Show or hide the sidebar",
        &format!("{p} c"),
        &format!("{p} 1-9"),
        &format!("{p} Tab"),
        &format!("{p} ?"),
    ] {
        assert!(screen.contains(text), "{text:?} missing:\n{screen}");
    }
    h.send_keys(ESC);
    wait_gone(&h, "Detach from the session");

    let mut h = attached_with(SIZE, &[("keys", "{ new_panel = \"t\" }")]);
    prefixed(&mut h, "?");
    h.wait_for_text(&format!("{} t", hint_prefix()), TIMEOUT);
    assert!(!h.screen_text().contains(&format!("{} c ", hint_prefix())));
    h.send_keys("q");
    wait_gone(&h, "Detach from the session");
}

#[test]
fn thirty_panels_scroll_the_sidebar_to_the_active_one() {
    let mut h = numbered();
    for n in 2..=30 {
        new_panel(&mut h, &n.to_string());
    }
    wait_for_row(&h, "▌  ❯  30");
    h.capture("sidebar-scrolled");
    assert!(!rows(&h).iter().any(|r| r.starts_with("   ❯  1 ")));
    prefixed(&mut h, "1");
    wait_for_row(&h, "▌  ❯  1");
}

#[test]
fn detach_works_in_sidebar_focus_scroll_mode_and_the_prefix_is_text_in_a_prompt() {
    let mut h = attached();
    prefixed(&mut h, TAB);
    h.wait_for_text("SIDEBAR", TIMEOUT);
    prefixed(&mut h, "d");
    assert!(h.wait_for_exit(TIMEOUT).success());

    let mut h = attached();
    prefixed(&mut h, "[");
    h.wait_for_text("SCROLL", TIMEOUT);
    prefixed(&mut h, "d");
    assert!(h.wait_for_exit(TIMEOUT).success());

    let mut h = attached();
    prefixed(&mut h, ",");
    h.wait_for_text("Rename panel: sh", TIMEOUT);
    h.send_bytes(PREFIX);
    h.send_keys("Z");
    h.wait_for_text("Rename panel: shZ", TIMEOUT);
    let screen = h.screen_text();
    assert!(
        !screen.contains("PREFIX") && !screen.contains("no binding"),
        "{screen}"
    );
}

#[test]
fn two_clients_share_the_active_panel_but_not_focus_prompts_or_overlays() {
    let mut a = numbered();
    new_panel(&mut a, "2");
    let mut b = second_client(&a, SIZE);
    wait_for_row(&b, "▌  ❯  2");
    // `n` in A moves the active panel in both.
    prefixed(&mut a, "n");
    wait_for_row(&a, "▌  ❯  1");
    wait_for_row(&b, "▌  ❯  1");
    // Sidebar focus and prompts are local.
    prefixed(&mut a, TAB);
    a.wait_for_text("SIDEBAR", TIMEOUT);
    prefixed(&mut b, ",");
    b.wait_for_text("Rename panel: 1", TIMEOUT);
    assert!(!b.screen_text().contains("SIDEBAR"));
    assert!(!a.screen_text().contains("Rename panel"));
    // A closes the panel B is renaming: B's prompt closes with the flash.
    a.send_keys(ESC);
    wait_gone(&a, "SIDEBAR");
    prefixed(&mut a, "X");
    a.wait_for_text("(y/n)", TIMEOUT);
    a.send_keys("y");
    b.wait_for_text("panel \"1\" was closed", TIMEOUT);
    assert!(!b.screen_text().contains("Rename panel"));
    b.capture("closed-target");
}
