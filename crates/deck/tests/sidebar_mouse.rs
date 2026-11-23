//! Resizing the sidebar with the mouse, end to end: the real `deck` in a PTY. Mouse events
//! are written to the outer terminal as the SGR sequences a terminal sends (1-based cells).

#![allow(
    clippy::unwrap_used,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant};

use support::{Harness, TIMEOUT, stdout, wait_until};

const SIZE: (u16, u16) = (100, 24);
const PREFIX: &[u8] = b"\x1b ";
const ENTER: &str = "\r";

fn attached_with(size: (u16, u16), config: &[(&str, &str)]) -> Harness {
    let mut builder = Harness::builder(size).config_unset("sidebar");
    for (key, value) in config {
        builder = builder.config(key, value);
    }
    let h = builder.deck();
    h.wait_for_text("│$", TIMEOUT);
    h
}

fn attached() -> Harness {
    attached_with(SIZE, &[])
}

fn prefixed(h: &mut Harness, key: &str) {
    h.send_bytes(PREFIX);
    h.send_keys(key);
}

fn rows(h: &Harness) -> Vec<String> {
    h.screen_text().lines().map(str::to_string).collect()
}

/// What the shell reports for `stty size`, once it answers. Every ask has its own tag, so an
/// earlier answer still on the screen is never taken for this one. The ask is repeated every
/// second: crossterm can leave typed bytes unread after a window resize until the next ones
/// arrive (its poll drops the TTY's readiness when the SIGWINCH is handled first).
fn stty(h: &mut Harness) -> String {
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    let mut tags = Vec::new();
    let deadline = Instant::now() + TIMEOUT;
    let mut asked = None;
    loop {
        if asked.is_none_or(|at: Instant| at.elapsed() >= Duration::from_secs(1)) {
            let tag = format!("Q{}Q", CALLS.fetch_add(1, Ordering::Relaxed));
            h.send_keys(&format!("echo {tag}$(stty size)E\r"));
            tags.push(tag);
            asked = Some(Instant::now());
        }
        let text = h.screen_text();
        for tag in &tags {
            for line in text.lines() {
                // The echoed command line has `$(stty` after the tag, not numbers.
                for rest in line.split(tag.as_str()).skip(1) {
                    if rest.starts_with(|c: char| c.is_ascii_digit())
                        && let Some(end) = rest.find('E')
                    {
                        return rest[..end].to_string();
                    }
                }
            }
        }
        assert!(Instant::now() < deadline, "no size:\n{text}");
        sleep(Duration::from_millis(20));
    }
}

fn wait_stty(h: &mut Harness, want: &str) {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let got = stty(h);
        if got == want {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "wanted {want}, got {got}:\n{}\n{}",
            h.screen_text(),
            std::fs::read_to_string(h.home().log_file()).unwrap_or_default()
        );
    }
}

/// A left press at the 0-based `column` of row 4.
fn press(h: &mut Harness, column: u16) {
    h.send_keys(&format!("\x1b[<0;{};5M", column + 1));
}

/// Motion with the left button held, to the 0-based `column`.
fn motion(h: &mut Harness, column: u16) {
    h.send_keys(&format!("\x1b[<32;{};5M", column + 1));
    sleep(Duration::from_millis(50));
}

fn release(h: &mut Harness, column: u16) {
    h.send_keys(&format!("\x1b[<0;{};5m", column + 1));
}

/// A whole drag of the separator from `from` to `to`.
fn drag(h: &mut Harness, from: u16, to: u16) {
    press(h, from);
    sleep(Duration::from_millis(50));
    motion(h, to);
    release(h, to);
}

fn wait_gone(h: &Harness, text: &str) {
    wait_until(&format!("{text:?} to go"), TIMEOUT, || {
        !h.screen_text().contains(text)
    });
}

fn switch_to(h: &mut Harness, name: &str) {
    prefixed(h, "s");
    h.wait_for_text("Sessions", TIMEOUT);
    h.send_keys(name);
    h.send_keys(ENTER);
    // The status bar starts with the session name (the sidebar may be hidden).
    wait_until(&format!("the session {name}"), TIMEOUT, || {
        rows(h)
            .last()
            .is_some_and(|r| r.trim_start().starts_with(name))
    });
}

#[test]
fn dragging_the_separator_resizes_the_shell_and_back() {
    let mut h = attached();
    assert_eq!(stty(&mut h), "23 71");
    press(&mut h, 28);
    sleep(Duration::from_millis(50));
    motion(&mut h, 40);
    h.wait_for_text("SIDEBAR 40", TIMEOUT);
    let accent = h.cell(0, 9).fg;
    wait_until("the accent separator", TIMEOUT, || {
        h.cell(5, 40).ch == '┃' && h.cell(5, 40).fg == accent
    });
    h.capture("sidebar-dragged");
    release(&mut h, 40);
    wait_gone(&h, "SIDEBAR 40");
    wait_stty(&mut h, "23 59");
    assert_eq!(h.cell(5, 40).ch, '│');
    assert_ne!(h.cell(5, 40).fg, accent);

    drag(&mut h, 40, 28);
    wait_stty(&mut h, "23 71");
}

#[test]
fn the_width_stops_at_the_limits() {
    let mut h = attached();
    drag(&mut h, 28, 80);
    wait_stty(&mut h, "23 39");
    drag(&mut h, 60, 3);
    wait_stty(&mut h, "23 79");
}

#[test]
fn the_panel_area_keeps_twenty_columns_in_a_narrow_window() {
    let mut h = attached_with((60, 24), &[]);
    drag(&mut h, 28, 58);
    wait_stty(&mut h, "23 20");
}

#[test]
fn a_switch_keeps_the_dragged_width_and_a_hidden_sidebar() {
    let mut h = attached();
    let out = h.run_deck(&["new", "other", "-d"]);
    assert!(out.status.success());
    drag(&mut h, 28, 40);
    wait_stty(&mut h, "23 59");
    switch_to(&mut h, "other");
    wait_stty(&mut h, "23 59");
    // Hidden before the switch: the other session's shell has the full width.
    prefixed(&mut h, "b");
    wait_stty(&mut h, "23 100");
    switch_to(&mut h, "main");
    wait_stty(&mut h, "23 100");
}

#[test]
fn a_shrinking_window_shrinks_a_wide_sidebar_and_a_growing_one_does_not_widen_it() {
    let mut h = attached_with((81, 24), &[("sidebar_width", "60")]);
    h.wait_for_text("▾", TIMEOUT);
    h.resize(80, 24);
    let separator = |h: &Harness| {
        rows(h)
            .get(5)
            .and_then(|r| r.chars().position(|c| c == '│'))
    };
    wait_until("the sidebar at 59", TIMEOUT, || separator(&h) == Some(59));
    wait_stty(&mut h, "23 20");
    h.resize(100, 24);
    wait_stty(&mut h, "23 40");
    assert_eq!(separator(&h), Some(59));
}

#[test]
fn the_drag_never_reaches_a_program_that_asked_for_the_mouse() {
    let mut h = attached();
    h.send_keys(
        "set +m; printf '\\033[?1000h\\033[?1002h\\033[?1006h'; stty raw -echo; printf 'REA%s\\r\\n' DY; \
         x=$(dd bs=1 count=9 2>/dev/null | od -An -tx1 -v | tr -d ' \\n'); \
         stty sane; printf 'GOT:%s\\r\\n' \"$x\"; sleep 30\r",
    );
    h.wait_for_text("READY", TIMEOUT);
    drag(&mut h, 28, 40);
    sleep(Duration::from_millis(300));
    assert!(!h.screen_text().contains("GOT:1b"), "{}", h.screen_text());
    // A click inside the terminal (now at column 41) still reaches it: cell (4, 3).
    h.send_keys("\x1b[<0;46;4M");
    h.wait_for_text("GOT:1b5b3c303b353b344d", TIMEOUT);
}

#[test]
fn the_dragged_width_survives_hiding_and_showing_the_sidebar() {
    let mut h = attached();
    drag(&mut h, 28, 40);
    wait_stty(&mut h, "23 59");
    prefixed(&mut h, "b");
    wait_stty(&mut h, "23 100");
    prefixed(&mut h, "b");
    wait_stty(&mut h, "23 59");
}

#[test]
fn a_press_on_the_separator_with_the_sidebar_hidden_does_nothing() {
    let mut h = attached();
    prefixed(&mut h, "b");
    wait_stty(&mut h, "23 100");
    drag(&mut h, 28, 40);
    sleep(Duration::from_millis(200));
    assert!(!h.screen_text().contains("SIDEBAR"));
    prefixed(&mut h, "b");
    wait_stty(&mut h, "23 71");
}

#[test]
fn a_press_on_a_sidebar_row_still_focuses_it() {
    let mut h = attached();
    h.wait_for_text("▌  ❯  sh", TIMEOUT);
    // The project row (row 3, 1-based 3): folding it hides its panel.
    h.send_keys("\x1b[<0;5;3M");
    h.send_keys("\x1b[<0;5;3m");
    wait_gone(&h, "▌  ❯  sh");
}

#[test]
fn a_drag_keeps_the_ratios_of_the_terminals() {
    let mut h = attached();
    prefixed(&mut h, "t");
    let corner = |h: &Harness| {
        rows(h)
            .iter()
            .find_map(|row| row.find("╭─ 2 · sh").map(|i| row[..i].chars().count()))
    };
    wait_until("two boxes", TIMEOUT, || corner(&h).is_some());
    // Terminal 2 is focused; grow it to the right side's 60%: the left one to 40%.
    prefixed(&mut h, "\x1b[D");
    prefixed(&mut h, "L");
    h.send_keys("L");
    // The left terminal's share of the panel area, in thousandths.
    let share = |at: usize, sidebar: usize, panel: usize| (at - sidebar) * 1000 / panel;
    let wait_share =
        |h: &Harness, what: &str, sidebar: usize, panel: usize, ok: &dyn Fn(usize) -> bool| {
            wait_until(what, TIMEOUT, || {
                corner(h).is_some_and(|c| c > sidebar && c < 100 && ok(share(c, sidebar, panel)))
            });
            corner(h).unwrap()
        };
    // Two resize steps move the split past 520 (the settled layout is 535); the layout
    // passes through smaller shares on the way.
    let before = wait_share(&h, "the split moved by both steps", 29, 71, &|a| a >= 520);
    drag(&mut h, 28, 40);
    let a = share(before, 29, 71);
    let after = wait_share(
        &h,
        &format!("the share after the drag to be within 20 of {a}"),
        41,
        59,
        &|b| a.abs_diff(b) <= 20,
    );
    let b = share(after, 41, 59);
    assert!(a.abs_diff(b) <= 20, "{a} vs {b}");
    assert!(a > 510, "the split was moved: {a}");
}

/// crossterm drops a SIGWINCH that becomes ready together with typed input, so the client
/// also watches the signal itself. Typing while the window changes size must leave the
/// shell at the last size. (Without that, this fails in about half the runs.)
#[test]
fn a_resize_during_typing_is_never_lost() {
    let mut h = attached();
    for i in 0..12 {
        h.send_keys("echo hi\r");
        h.resize(if i % 2 == 0 { 90 } else { 100 }, 24);
    }
    h.resize(100, 24);
    wait_stty(&mut h, "23 71");
    h.resize(90, 24);
    h.send_keys("echo hi\r");
    wait_stty(&mut h, "23 61");
}

/// The bar row of the 24-row window: 0-based row 22.
const BAR_ROW: usize = 22;

/// Pointer motion with no button held, at the 0-based `column` and `row`.
fn hover_at(h: &mut Harness, column: u16, row: u16) {
    h.send_keys(&format!("\x1b[<35;{};{}M", column + 1, row + 1));
}

fn wait_bar(h: &Harness) {
    wait_until("the bottom bar", TIMEOUT, || {
        rows(h)
            .get(BAR_ROW)
            .is_some_and(|r| r.starts_with("  + New "))
    });
}

#[test]
fn the_bottom_bar_shows_the_new_chip_and_the_restore_chip_at_the_right() {
    let h = attached();
    wait_bar(&h);
    let screen = rows(&h);
    let bar = &screen[BAR_ROW];
    let sidebar: String = bar.chars().take(28).collect();
    assert_eq!(sidebar.trim_end(), "  + New            Restore", "{bar:?}");
    assert!(screen[BAR_ROW - 1].chars().take(28).all(|c| c == ' '));
    h.capture("bottom-bar");
}

#[test]
fn the_header_reads_workspaces_and_holds_the_new_project_chip() {
    let h = attached();
    wait_bar(&h);
    let header = rows(&h)[0].clone();
    let sidebar: String = header.chars().take(28).collect();
    assert!(sidebar.starts_with(" Workspaces "), "{header:?}");
    assert!(sidebar.trim_end().ends_with("+ New project"), "{header:?}");
    h.capture("header");
}

#[test]
fn clicking_the_header_chip_adds_a_project_like_the_prefix_flow() {
    let mut h = attached();
    wait_bar(&h);
    h.send_keys("\x1b[<0;16;1M");
    h.send_keys("\x1b[<0;16;1m");
    h.wait_for_text("Directory:", TIMEOUT);
    h.send_keys("\x15/tmp");
    h.send_keys(ENTER);
    h.wait_for_text("Project name:", TIMEOUT);
    h.send_keys("\x15barchip");
    h.send_keys(ENTER);
    h.wait_for_text("barchip", TIMEOUT);
    wait_bar(&h);
}

#[test]
fn clicking_the_header_chip_with_a_prompt_open_does_nothing() {
    let mut h = attached();
    wait_bar(&h);
    prefixed(&mut h, "P");
    h.wait_for_text("Directory:", TIMEOUT);
    h.send_keys("\x1b[<0;16;1M");
    h.send_keys("\x1b[<0;16;1m");
    sleep(Duration::from_millis(300));
    let screen = h.screen_text();
    assert_eq!(screen.matches("Directory:").count(), 1, "{screen}");
    assert!(!screen.contains("Project name:"), "{screen}");
}

#[test]
fn the_chip_is_drawn_in_the_selection_colours_under_the_pointer() {
    let mut h = attached();
    wait_bar(&h);
    let idle = h.cell(BAR_ROW, 3);
    hover_at(&mut h, 5, 22);
    wait_until("the chip to highlight", TIMEOUT, || {
        let c = h.cell(BAR_ROW, 3);
        c.fg != idle.fg || c.bg != idle.bg || c.flags != idle.flags
    });
    h.capture("bottom-bar-hover");
    hover_at(&mut h, 40, 5);
    wait_until("the chip to settle", TIMEOUT, || {
        let c = h.cell(BAR_ROW, 3);
        c.fg == idle.fg && c.bg == idle.bg && c.flags == idle.flags
    });
}

#[test]
fn motion_that_keeps_the_hover_target_draws_nothing() {
    let mut h = attached();
    wait_bar(&h);
    let idle = h.cell(BAR_ROW, 3);
    hover_at(&mut h, 3, 22);
    wait_until("the chip to highlight", TIMEOUT, || {
        let c = h.cell(BAR_ROW, 3);
        c.fg != idle.fg || c.bg != idle.bg || c.flags != idle.flags
    });
    sleep(Duration::from_millis(300));
    let settled = h.raw_output().len();
    for i in 1..100u16 {
        hover_at(&mut h, 1 + i % 7, 22);
    }
    sleep(Duration::from_millis(500));
    assert_eq!(h.raw_output().len(), settled, "the screen was redrawn");
}

#[test]
fn motion_over_the_sidebar_never_reaches_a_program() {
    let mut h = attached();
    h.send_keys(
        "set +m; printf '\\033[?1000h\\033[?1003h\\033[?1006h'; stty raw -echo; printf 'REA%s\\r\\n' DY; \
         x=$(dd bs=1 count=9 2>/dev/null | od -An -tx1 -v | tr -d ' \\n'); \
         stty sane; printf 'GOT:%s\\r\\n' \"$x\"; sleep 30\r",
    );
    h.wait_for_text("READY", TIMEOUT);
    hover_at(&mut h, 5, 4);
    hover_at(&mut h, 3, 22);
    sleep(Duration::from_millis(300));
    assert!(!h.screen_text().contains("GOT:1b"), "{}", h.screen_text());
    // Motion over its terminal (cell 4,3 at column 33, row 4) is delivered: `ESC [ < 3 5 ; 5 ; 4`, the first nine bytes.
    hover_at(&mut h, 33, 3);
    h.wait_for_text("GOT:1b5b3c33353b353b34", TIMEOUT);
}

#[test]
fn a_seven_row_window_has_no_bar() {
    let h = attached_with((100, 8), &[]);
    sleep(Duration::from_millis(300));
    assert!(!h.screen_text().contains("Restore"), "{}", h.screen_text());
}

// The New button and menu.

/// A directory with fake `claude`, `opencode` and `codex` programs: each prints where it runs
/// and with which arguments, then waits for a line.
fn fake_agents() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .prefix("ag")
        .tempdir_in("/tmp")
        .unwrap();
    for name in ["claude", "opencode", "codex"] {
        let script = dir.path().join(name);
        fs::write(
            &script,
            "#!/bin/sh\necho \"FAKE-$(basename \"$0\")-ready cwd=$(basename \"$PWD\") args=$*\"\nread line\n",
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    }
    dir
}

/// `deck` whose shell finds the fake agents first, and nothing else that could be an agent.
fn with_agents(bin: &tempfile::TempDir, size: (u16, u16), config: &[(&str, &str)]) -> Harness {
    let mut builder = Harness::builder(size)
        .config_unset("sidebar")
        .env("PATH", &format!("{}:/usr/bin:/bin", bin.path().display()));
    for (key, value) in config {
        builder = builder.config(key, value);
    }
    let h = builder.deck();
    h.wait_for_text("│$", TIMEOUT);
    h
}

fn home_name(h: &Harness) -> String {
    h.home()
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

/// The sidebar's 28 columns, one string per row.
fn side(h: &Harness) -> Vec<String> {
    rows(h)
        .iter()
        .map(|r| {
            r.chars()
                .take(28)
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

fn click_at(h: &mut Harness, column: u16, row: u16) {
    h.send_keys(&format!("\x1b[<0;{};{}M", column + 1, row + 1));
    h.send_keys(&format!("\x1b[<0;{};{}m", column + 1, row + 1));
}

/// Clicks the `+ New` chip of the bottom bar: the menu opens for the active project.
fn open_menu(h: &mut Harness) {
    click_at(h, 3, 22);
    h.wait_for_text("New in", TIMEOUT);
}

/// Clicks the menu item `index` of a menu opened above the bar of a 24-row window: the menu's
/// top border is on row 15, the items follow.
fn pick(h: &mut Harness, index: u16) {
    click_at(h, 12, 16 + index);
}

fn panels_in_sidebar(h: &Harness) -> usize {
    let side = side(h);
    // Not the status bar, which names the active panel too.
    side.iter()
        .rev()
        .skip(1)
        .filter(|r| r.contains("❯  sh"))
        .count()
}

fn new_project(h: &Harness, name: &str) -> String {
    let out = h.run_deck(&["new-project", name, "--dir", "/tmp"]);
    assert!(out.status.success(), "{}", support::stderr(&out));
    stdout(&out).trim().to_string()
}

#[test]
fn project_rows_show_no_new_button_under_the_pointer() {
    let mut h = attached();
    new_project(&h, "api");
    h.wait_for_text("api", TIMEOUT);
    hover_at(&mut h, 10, 2);
    hover_at(&mut h, 10, 4);
    sleep(Duration::from_millis(300));
    assert!(!h.screen_text().contains("[ New"), "{}", h.screen_text());
}

#[test]
fn a_click_on_new_opens_the_menu_above_the_bar_and_a_click_on_the_name_folds() {
    let mut h = attached();
    open_menu(&mut h);
    let items: Vec<&str> = ["Terminal", "Claude", "OpenCode", "Codex"].into();
    for (i, item) in items.iter().enumerate() {
        let row = rows(&h)[16 + i].clone();
        assert!(row.starts_with("│ "), "{row:?}");
        assert!(row.contains(item), "{row:?}");
        assert!(row.contains(&format!("{} ", i + 1)), "{row:?}");
    }
    assert!(rows(&h)[15].contains("╭─ New in"));
    assert!(rows(&h)[21].starts_with('╰'));
    h.capture("new-menu");
    h.send_keys("\x1b");
    wait_gone(&h, "New in");
    // The name of the project still folds it.
    click_at(&mut h, 6, 2);
    wait_gone(&h, "▌  ❯  sh");
}

#[test]
fn the_terminal_item_adds_a_panel_with_a_shell_in_the_project_root() {
    let mut h = attached();
    open_menu(&mut h);
    pick(&mut h, 0);
    wait_until("two panels", TIMEOUT, || panels_in_sidebar(&h) == 2);
    let side = side(&h);
    // The old panel keeps one terminal (no count), the new one is active.
    assert!(side[3].ends_with("sh"), "{side:?}");
    assert!(side[4].starts_with('▌'), "{side:?}");
    wait_until("the new shell", TIMEOUT, || h.screen_text().contains("│$"));
    h.send_keys("echo cwd=$(basename $PWD)\r");
    h.wait_for_text(&format!("cwd={}", home_name(&h)), TIMEOUT);
}

#[test]
fn choosing_on_a_folded_project_unfolds_it_and_shows_the_active_new_panel() {
    let mut h = attached();
    click_at(&mut h, 6, 2);
    wait_gone(&h, "▌  ❯  sh");
    open_menu(&mut h);
    pick(&mut h, 0);
    wait_until("two panels, the new one active", TIMEOUT, || {
        let side = side(&h);
        panels_in_sidebar(&h) == 2 && side.get(4).is_some_and(|r| r.starts_with('▌'))
    });
}

#[test]
fn the_agent_items_add_panels_running_the_fake_agents_in_the_project_root() {
    let bin = fake_agents();
    let mut h = with_agents(&bin, SIZE, &[]);
    let home = home_name(&h);
    for (i, name) in ["claude", "opencode", "codex"].into_iter().enumerate() {
        open_menu(&mut h);
        pick(&mut h, u16::try_from(i).unwrap() + 1);
        h.wait_for_text(&format!("FAKE-{name}-ready cwd={home} args="), TIMEOUT);
    }
    wait_until("the three panels", TIMEOUT, || {
        let side = side(&h);
        ["claude", "opencode", "codex"]
            .iter()
            .all(|n| side.iter().any(|r| r.contains(n)))
    });
}

#[test]
fn a_missing_program_flashes_closes_the_menu_and_adds_no_panel() {
    let bin = fake_agents();
    fs::remove_file(bin.path().join("opencode")).unwrap();
    let mut h = with_agents(&bin, SIZE, &[]);
    open_menu(&mut h);
    h.send_keys("3");
    h.wait_for_text("\"opencode\" not found in PATH", TIMEOUT);
    wait_gone(&h, "New in");
    sleep(Duration::from_millis(200));
    assert_eq!(panels_in_sidebar(&h), 1, "{}", h.screen_text());
    assert!(!side(&h).iter().any(|r| r.contains("opencode")));
}

#[test]
fn a_configured_agent_command_is_honoured() {
    let bin = fake_agents();
    let mut h = with_agents(&bin, SIZE, &[("agents.codex.command", "\"codex --fast\"")]);
    open_menu(&mut h);
    h.send_keys("4");
    h.wait_for_text("args=--fast", TIMEOUT);
}

#[test]
fn the_keyboard_reaches_the_menu() {
    let bin = fake_agents();
    let mut h = with_agents(&bin, SIZE, &[]);
    prefixed(&mut h, "N");
    h.wait_for_text("New in", TIMEOUT);
    h.send_keys("\x1b");
    wait_gone(&h, "New in");
    // Sidebar focus: Tab, j, n.
    prefixed(&mut h, "\t");
    h.wait_for_text("n menu", TIMEOUT);
    h.send_keys("j");
    h.send_keys("n");
    h.wait_for_text("New in", TIMEOUT);
    h.send_keys("3");
    h.wait_for_text(
        &format!("FAKE-opencode-ready cwd={}", home_name(&h)),
        TIMEOUT,
    );
}

#[test]
fn the_sidebar_hints_list_the_menu() {
    let mut h = attached();
    prefixed(&mut h, "\t");
    h.wait_for_text(
        "↑↓ move  ⏎ open  ←→ fold  c new  n menu  r rename  x close  esc back",
        TIMEOUT,
    );
}

#[test]
fn a_click_outside_closes_the_menu_without_folding_the_row_under_it() {
    let mut h = attached();
    open_menu(&mut h);
    // The project row: a click there would fold the project.
    click_at(&mut h, 3, 2);
    wait_gone(&h, "New in");
    sleep(Duration::from_millis(200));
    assert_eq!(panels_in_sidebar(&h), 1, "{}", h.screen_text());
}

#[test]
fn the_menu_sits_right_above_the_bar_in_a_short_window() {
    let mut h = attached_with((100, 12), &[]);
    click_at(&mut h, 3, 10);
    h.wait_for_text("New in", TIMEOUT);
    let screen = rows(&h);
    let top = screen.iter().position(|r| r.contains("╭─ New in")).unwrap();
    let bottom = screen.iter().rposition(|r| r.starts_with('╰')).unwrap();
    assert_eq!((top, bottom), (3, 9), "{}", h.screen_text());
    assert!(screen[10].starts_with("  + New"), "{}", h.screen_text());
}

#[test]
fn the_menu_stays_inside_a_forty_by_twelve_window() {
    // The window is too narrow for the sidebar: the menu is at the top right.
    let mut h = Harness::builder((40, 12)).config_unset("sidebar").deck();
    h.wait_for_text("$", TIMEOUT);
    prefixed(&mut h, "N");
    h.wait_for_text("New in", TIMEOUT);
    let screen = rows(&h);
    assert!(screen.iter().any(|r| r.contains('╭')), "{screen:?}");
    assert!(screen.iter().any(|r| r.contains('╰')), "{screen:?}");
    assert!(screen.iter().any(|r| r.contains("Codex")), "{screen:?}");
    assert!(screen.iter().any(|r| r.contains("esc close")), "{screen:?}");
}

#[test]
fn a_project_closed_by_another_client_closes_the_menu_with_a_flash() {
    let mut h = attached();
    let id = new_project(&h, "api");
    h.wait_for_text("api", TIMEOUT);
    // Sidebar focus on `api`, then the bar's New button.
    prefixed(&mut h, "\t");
    h.wait_for_text("n menu", TIMEOUT);
    h.send_keys("jj");
    click_at(&mut h, 3, 22);
    h.wait_for_text("New in api", TIMEOUT);
    let out = h.run_deck(&["close", &id]);
    assert!(out.status.success(), "{}", support::stderr(&out));
    h.wait_for_text("project \"api\" was closed", TIMEOUT);
    wait_gone(&h, "New in");
}

#[test]
fn a_new_menu_key_override_moves_the_binding_and_shows_in_the_help() {
    let mut h = attached_with(SIZE, &[("keys.new_menu", "\"M\"")]);
    prefixed(&mut h, "N");
    sleep(Duration::from_millis(300));
    assert!(!h.screen_text().contains("New in"));
    prefixed(&mut h, "M");
    h.wait_for_text("New in", TIMEOUT);
    h.send_keys("\x1b");
    wait_gone(&h, "New in");
    prefixed(&mut h, "?");
    h.wait_for_text("Help", TIMEOUT);
    let found = |h: &Harness| {
        rows(h)
            .iter()
            .any(|r| r.contains("New panel menu") && r.contains(" M "))
    };
    for _ in 0..8 {
        if found(&h) {
            break;
        }
        h.send_keys("\x1b[6~");
        sleep(Duration::from_millis(100));
    }
    wait_until("the help row", TIMEOUT, || found(&h));
}

#[test]
fn hovering_within_a_row_draws_no_extra_frames() {
    let mut h = attached();
    hover_at(&mut h, 3, 2);
    sleep(Duration::from_millis(300));
    let settled = h.raw_output().len();
    for i in 1..100u16 {
        hover_at(&mut h, 1 + i % 14, 2);
    }
    sleep(Duration::from_millis(500));
    assert_eq!(h.raw_output().len(), settled, "the screen was redrawn");
}

#[test]
fn the_new_button_adds_a_terminal_panel_to_the_selected_project() {
    let mut h = attached();
    new_project(&h, "api");
    h.wait_for_text("api", TIMEOUT);
    // Sidebar focus on `api` (rows: project, its panel, api), then `+ New`.
    prefixed(&mut h, "\t");
    h.wait_for_text("n menu", TIMEOUT);
    h.send_keys("jj");
    click_at(&mut h, 3, 22);
    h.wait_for_text("New in api", TIMEOUT);
    // `api` holds its own first panel.
    let api_panels = |h: &Harness| {
        side(h)
            .iter()
            .skip_while(|r| !r.starts_with(" ▾ api"))
            .skip(1)
            .take_while(|r| r.contains('❯'))
            .count()
    };
    assert_eq!(api_panels(&h), 1, "{}", h.screen_text());
    h.send_keys("1");
    wait_until("a second panel in api", TIMEOUT, || api_panels(&h) == 2);
}
