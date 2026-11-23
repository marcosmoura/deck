//! The Restore button and dialog, end to end: the real `deck` in a PTY, fixture trees in a
//! temp `CLAUDE_CONFIG_DIR` and `CODEX_HOME`, and fake `claude`, `codex` and `opencode`
//! `#!/bin/sh` scripts on a `PATH` that holds only them. Every
//! test has its own `HOME`; no real agent runs and nothing of the user's is read.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::thread::sleep;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use support::{Harness, TIMEOUT, wait_until};

const SIZE: (u16, u16) = (100, 24);
const PREFIX: &[u8] = b"\x1b ";
const UUID_A: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const UUID_B: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const UUID_X: &str = "11111111-1111-4111-8111-111111111111";
/// The bar row of the 24-row window, 0-based.
const BAR_ROW: u16 = 22;
/// A column on the `Restore` chip of the 28-column sidebar.
const CHIP: u16 = 20;

/// The sandbox of one test: a home, the agents' directories and the fake agents.
struct Sandbox {
    root: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let root = tempfile::Builder::new()
            .prefix("rt")
            .tempdir_in("/tmp")
            .unwrap();
        let root = Self { root };
        for dir in ["home", "claude", "codex", "bin"] {
            fs::create_dir_all(root.path(dir)).unwrap();
        }
        root
    }

    /// Paths are canonical: the project roots the daemon sees are.
    fn path(&self, name: &str) -> PathBuf {
        fs::canonicalize(self.root.path()).unwrap().join(name)
    }

    fn builder(&self, size: (u16, u16)) -> support::Builder {
        // Only the fakes: a real `claude` of the machine can never run, and nothing else is found.
        let path = self.path("bin").display().to_string();
        Harness::builder(size)
            .config_unset("sidebar")
            .env("HOME", &self.path("home").to_string_lossy())
            .env("CLAUDE_CONFIG_DIR", &self.path("claude").to_string_lossy())
            .env("CODEX_HOME", &self.path("codex").to_string_lossy())
            .env("XDG_DATA_HOME", &self.path("home/data").to_string_lossy())
            .env(
                "XDG_CONFIG_HOME",
                &self.path("home/config").to_string_lossy(),
            )
            .env("PATH", &path)
    }

    /// A Claude transcript `age` seconds old.
    fn claude(&self, id: &str, title: &str, cwd: &str, age: u64) {
        let dir = self.path("claude/projects/p");
        fs::create_dir_all(&dir).unwrap();
        let line = format!(
            "{{\"type\":\"user\",\"cwd\":\"{cwd}\",\"message\":{{\"content\":\"{title}\"}}}}\n"
        );
        age_file(&dir.join(format!("{id}.jsonl")), &line, age);
    }

    /// A Codex rollout `age` seconds old.
    fn codex(&self, id: &str, title: &str, cwd: &str, age: u64) {
        let dir = self.path("codex/sessions/2026/11/22");
        fs::create_dir_all(&dir).unwrap();
        let meta = format!(
            "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"{cwd}\"}}}}\n"
        );
        let msg = format!(
            "{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"user_message\",\"message\":\"{title}\"}}}}\n"
        );
        age_file(
            &dir.join(format!("rollout-2026-11-22T10-00-00-{id}.jsonl")),
            &(meta + &msg),
            age,
        );
    }

    fn script(&self, name: &str, body: &str) {
        let path = self.path("bin").join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A fake `claude` or `codex`: records `<directory>|<arguments>` and waits for input.
    fn agent(&self, name: &str) {
        let rec = self.path(&format!("bin/{name}.rec"));
        self.script(
            name,
            &format!(
                "printf '%s|%s\\n' \"$(pwd -P)\" \"$*\" >> '{}'\nread line\n",
                rec.display()
            ),
        );
    }

    /// The recorded starts of a fake agent.
    fn starts(&self, name: &str) -> Vec<String> {
        fs::read_to_string(self.path(&format!("bin/{name}.rec")))
            .map(|t| t.lines().map(str::to_string).collect())
            .unwrap_or_default()
    }

    fn wait_start(&self, name: &str) -> String {
        wait_until(&format!("a start of {name}"), TIMEOUT, || {
            !self.starts(name).is_empty()
        });
        self.starts(name).remove(0)
    }

    /// A fake `opencode` whose `session list` prints `sessions` after `delay` seconds, or
    /// fails.
    fn opencode(&self, sessions: &[(&str, &str, &str, u64)], delay: u32, fail: bool) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let items: Vec<String> = sessions
            .iter()
            .map(|(id, title, dir, age)| {
                format!(
                    "{{\"id\":\"{id}\",\"title\":\"{title}\",\"updated\":{},\"created\":{},\"projectId\":\"p\",\"directory\":\"{dir}\"}}",
                    now - age,
                    now - age
                )
            })
            .collect();
        let sleep = if delay > 0 {
            format!("/bin/sleep {delay}\n")
        } else {
            String::new()
        };
        let body = if fail {
            "exit 1\n".to_string()
        } else {
            format!("printf '%s\\n' '[{}]'\n", items.join(","))
        };
        self.script(
            "opencode",
            &format!("if [ \"$1\" = session ] && [ \"$2\" = list ]; then\n{sleep}{body}exit 0\nfi\nread line\n"),
        );
    }
}

/// Writes `text` to `path` and sets its modification time `age` seconds back.
fn age_file(path: &std::path::Path, text: &str, age: u64) {
    fs::write(path, text).unwrap();
    let file = fs::OpenOptions::new().write(true).open(path).unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(age))
        .unwrap();
}

/// The fixtures most tests use: two Claude sessions, one OpenCode, one Codex.
fn fixtures(sb: &Sandbox) {
    let root = sb.path("home").display().to_string();
    sb.claude(UUID_A, "Fix the login redirect", &root, 2 * 3600);
    sb.claude(UUID_B, "Add the rate limit", &root, 5 * 86_400);
    sb.codex(UUID_X, "Write the parser", &root, 60);
    sb.opencode(
        &[("ses_one", "Rework login form", &root, 3 * 86_400)],
        0,
        false,
    );
    sb.agent("claude");
    sb.agent("codex");
}

fn start(sb: &Sandbox) -> Harness {
    let h = sb.builder(SIZE).deck();
    h.wait_for_text("│", TIMEOUT);
    wait_until("the bottom bar", TIMEOUT, || {
        rows(&h)
            .get(usize::from(BAR_ROW))
            .is_some_and(|r| r.contains("Restore"))
    });
    h
}

fn rows(h: &Harness) -> Vec<String> {
    h.screen_text().lines().map(str::to_string).collect()
}

fn prefixed(h: &mut Harness, key: &str) {
    h.send_bytes(PREFIX);
    h.send_keys(key);
}

fn click_at(h: &mut Harness, column: u16, row: u16) {
    h.send_keys(&format!("\x1b[<0;{};{}M", column + 1, row + 1));
    h.send_keys(&format!("\x1b[<0;{};{}m", column + 1, row + 1));
}

fn hover_at(h: &mut Harness, column: u16, row: u16) {
    h.send_keys(&format!("\x1b[<35;{};{}M", column + 1, row + 1));
}

fn wheel_down(h: &mut Harness, column: u16, row: u16) {
    h.send_keys(&format!("\x1b[<65;{};{}M", column + 1, row + 1));
}

const RIGHT: &str = "\x1b[C";
const DOWN: &str = "\x1b[B";
const ESC: &str = "\x1b";

fn open_by_click(h: &mut Harness) {
    click_at(h, CHIP, BAR_ROW);
    h.wait_for_text("Restore agent session", TIMEOUT);
}

fn wait_gone(h: &Harness, text: &str) {
    wait_until(&format!("{text:?} to go"), TIMEOUT, || {
        !h.screen_text().contains(text)
    });
}

/// The row of the screen that holds `text`.
fn row_of(h: &Harness, text: &str) -> usize {
    rows(h)
        .iter()
        .position(|r| r.contains(text))
        .unwrap_or_else(|| panic!("{text:?} not on the screen:\n{}", h.screen_text()))
}

/// The sidebar's projects and their panel rows, from its 28 columns.
fn projects(h: &Harness) -> Vec<(String, Vec<String>)> {
    let mut found: Vec<(String, Vec<String>)> = Vec::new();
    for row in rows(h).iter().take(usize::from(BAR_ROW) - 1).skip(2) {
        let side: String = row.chars().take(28).collect();
        let trimmed = side.trim();
        if trimmed.is_empty() {
            continue;
        }
        if side.starts_with(" ▾") || side.starts_with(" ▸") {
            found.push((
                side[side.find(' ').unwrap_or(0)..].trim().to_string(),
                Vec::new(),
            ));
        } else if let Some((_, panels)) = found.last_mut() {
            panels.push(side.trim_end().to_string());
        }
    }
    found
}

fn panel_names(h: &Harness, project: &str) -> Vec<String> {
    projects(h)
        .into_iter()
        .find(|(name, _)| name.contains(project))
        .map(|(_, panels)| panels)
        .unwrap_or_default()
}

#[test]
fn clicking_restore_opens_three_columns_newest_first_with_title_directory_and_age() {
    let sb = Sandbox::new();
    fixtures(&sb);
    let mut h = start(&sb);
    open_by_click(&mut h);
    for title in [
        "Fix the login redirect",
        "Add the rate limit",
        "Rework login form",
        "Write the parser",
    ] {
        h.wait_for_text(title, TIMEOUT);
    }
    let screen = h.screen_text();
    assert!(screen.contains("Claude Code (2)"), "{screen}");
    assert!(screen.contains("OpenCode (1)") && screen.contains("Codex (1)"));
    assert!(screen.contains("2h ago") && screen.contains("5d ago") && screen.contains("3d ago"));
    assert!(screen.contains("1m ago"));
    // The newest Claude session is above the older one; the directory line is below a title.
    assert!(row_of(&h, "Fix the login redirect") < row_of(&h, "Add the rate limit"));
    assert!(rows(&h)[row_of(&h, "Fix the login redirect") + 1].contains("2h ago"));
    assert!(screen.contains("⏎ resume  ←→ column  ↑↓ row  type to filter  esc close"));
    h.capture("restore");
}

#[test]
fn the_button_sits_at_the_right_of_the_bar_and_highlights_on_hover() {
    let sb = Sandbox::new();
    fixtures(&sb);
    let mut h = start(&sb);
    let bar = rows(&h)[usize::from(BAR_ROW)].clone();
    let side: String = bar.chars().take(28).collect();
    assert_eq!(side.trim_end(), "  + New project    Restore", "{bar:?}");
    hover_at(&mut h, CHIP, BAR_ROW);
    // The hovered chip is drawn in the selection colours: its cells change colour.
    let idle = h.cell(usize::from(BAR_ROW), usize::from(CHIP));
    wait_until("the hover colours", TIMEOUT, || {
        let hot = h.cell(usize::from(BAR_ROW), usize::from(CHIP));
        hot.inverse() || hot.bg != idle.bg || hot.fg != idle.fg
    });
    h.capture("restore-button");
    hover_at(&mut h, 60, 5);
    wait_until("the hover to end", TIMEOUT, || {
        let cold = h.cell(usize::from(BAR_ROW), usize::from(CHIP));
        cold.fg == idle.fg && cold.bg == idle.bg && cold.inverse() == idle.inverse()
    });
}

#[test]
fn a_narrow_sidebar_shows_the_button_as_a_glyph() {
    let sb = Sandbox::new();
    fixtures(&sb);
    let h = sb.builder(SIZE).config("sidebar_width", "20").deck();
    wait_until("the glyph", TIMEOUT, || {
        rows(&h)
            .get(usize::from(BAR_ROW))
            .is_some_and(|r| r.chars().take(20).collect::<String>().contains('↻'))
    });
    let bar = rows(&h)[usize::from(BAR_ROW)].clone();
    assert_eq!(
        bar.chars().take(20).collect::<String>().trim_end(),
        "  +              ↻",
        "{bar:?}"
    );
}

#[test]
fn a_slow_opencode_leaves_its_column_loading_while_the_others_fill() {
    let sb = Sandbox::new();
    fixtures(&sb);
    sb.opencode(&[("ses_one", "Rework login form", "/tmp", 60)], 3, false);
    let mut h = start(&sb);
    open_by_click(&mut h);
    wait_until("the other columns and the loading one", TIMEOUT, || {
        let s = h.screen_text();
        s.contains("Fix the login redirect")
            && s.contains("Write the parser")
            && s.contains("loading…")
    });
    assert!(!h.screen_text().contains("Rework login form"));
    // It joins in once its list is done.
    h.wait_for_text("Rework login form", Duration::from_secs(8));
    assert!(!h.screen_text().contains("loading…"));
}

#[test]
fn a_failing_opencode_shows_its_error_in_the_column() {
    let sb = Sandbox::new();
    fixtures(&sb);
    sb.opencode(&[], 0, true);
    let mut h = start(&sb);
    open_by_click(&mut h);
    h.wait_for_text("could not run opencode", TIMEOUT);
    assert!(h.screen_text().contains("Fix the login redirect"));
}

#[test]
fn typing_filters_all_columns_escape_clears_and_a_second_escape_closes() {
    let sb = Sandbox::new();
    fixtures(&sb);
    let mut h = start(&sb);
    open_by_click(&mut h);
    h.wait_for_text("Write the parser", TIMEOUT);
    h.wait_for_text("Rework login form", TIMEOUT);
    h.send_keys("login");
    wait_until("the filter", TIMEOUT, || {
        let s = h.screen_text();
        s.contains("> login█")
            && !s.contains("Add the rate limit")
            && !s.contains("Write the parser")
    });
    let s = h.screen_text();
    assert!(s.contains("Fix the login redirect") && s.contains("Rework login form"));
    assert!(s.contains("Claude Code (1)") && s.contains("OpenCode (1)") && s.contains("Codex (0)"));
    h.send_keys(ESC);
    h.wait_for_text("Write the parser", TIMEOUT);
    assert!(h.screen_text().contains("> █"));
    h.send_keys(ESC);
    wait_gone(&h, "Restore agent session");
}

#[test]
fn right_down_enter_on_the_codex_column_resumes_that_session_in_a_new_active_panel() {
    let sb = Sandbox::new();
    fixtures(&sb);
    // No OpenCode sessions: the arrow skips the empty column.
    sb.opencode(&[], 0, false);
    let mut h = start(&sb);
    open_by_click(&mut h);
    h.wait_for_text("Write the parser", TIMEOUT);
    h.wait_for_text("Fix the login redirect", TIMEOUT);
    // The dialog's OpenCode column must be answered before the arrow skips it.
    h.wait_for_text("OpenCode (0)", TIMEOUT);
    h.send_keys(RIGHT);
    h.send_keys(DOWN);
    h.send_keys("\r");
    let start = sb.wait_start("codex");
    assert_eq!(
        start,
        format!("{}|resume {UUID_X}", sb.path("home").display())
    );
    wait_gone(&h, "Restore agent session");
    wait_until("the new panel active", TIMEOUT, || {
        rows(&h)
            .iter()
            .any(|r| r.starts_with('▌') && r.chars().take(28).collect::<String>().contains("codex"))
    });
    assert!(sb.starts("claude").is_empty());
}

#[test]
fn a_session_opens_in_the_project_that_holds_its_directory_else_in_the_active_one() {
    let sb = Sandbox::new();
    let site = sb.path("home/site");
    fs::create_dir_all(site.join("pkg")).unwrap();
    fs::create_dir_all(sb.path("home/elsewhere")).unwrap();
    let inside = site.join("pkg").display().to_string();
    let outside = sb.path("home/elsewhere").display().to_string();
    sb.claude(UUID_A, "Fix the login redirect", &inside, 600);
    sb.codex(UUID_X, "Write the parser", &outside, 1200);
    sb.opencode(&[], 0, false);
    sb.agent("claude");
    sb.agent("codex");
    let mut h = start(&sb);
    let out = h.run_deck(&["new-project", "zeta-site", "--dir", &site.to_string_lossy()]);
    assert!(out.status.success(), "{}", support::stderr(&out));
    wait_until("both projects", TIMEOUT, || projects(&h).len() == 2);
    // Make the first project active: click its panel.
    click_at(&mut h, 10, 3);
    wait_until("the first project active", TIMEOUT, || {
        let p = projects(&h);
        p[0].1.iter().any(|r| r.starts_with('▌'))
    });
    open_by_click(&mut h);
    h.wait_for_text("Fix the login redirect", TIMEOUT);
    h.wait_for_text("Write the parser", TIMEOUT);
    // The Claude session is inside zeta-site's root.
    h.send_keys("\r");
    let start = sb.wait_start("claude");
    assert_eq!(start, format!("{inside}|--resume {UUID_A}"));
    wait_until("the panel in zeta-site", TIMEOUT, || {
        panel_names(&h, "zeta-site")
            .iter()
            .any(|r| r.contains("claude"))
    });
    // The first project, the one that was active, got no panel from the Claude session.
    assert!(!projects(&h)[0].1.iter().any(|r| r.contains("claude")));

    // The Codex session is in neither project's root... but the first project holds the home
    // directory, so the session goes where the longest root says: the first project.
    click_at(&mut h, 10, 3);
    open_by_click(&mut h);
    h.wait_for_text("Write the parser", TIMEOUT);
    h.send_keys(RIGHT);
    h.send_keys(RIGHT);
    h.send_keys("\r");
    sb.wait_start("codex");
    wait_until("the panel in the active project", TIMEOUT, || {
        projects(&h)[0].1.iter().any(|r| r.contains("codex"))
    });
}

#[test]
fn a_session_outside_every_project_opens_in_the_active_project() {
    let sb = Sandbox::new();
    let elsewhere = sb.path("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    sb.codex(UUID_X, "Write the parser", &elsewhere.to_string_lossy(), 60);
    sb.opencode(&[], 0, false);
    sb.agent("codex");
    let mut h = start(&sb);
    open_by_click(&mut h);
    h.wait_for_text("Write the parser", TIMEOUT);
    h.send_keys("\r");
    let start = sb.wait_start("codex");
    assert_eq!(start, format!("{}|resume {UUID_X}", elsewhere.display()));
    wait_until("the panel in the only project", TIMEOUT, || {
        projects(&h)[0].1.iter().any(|r| r.contains("codex"))
    });
}

#[test]
fn a_directory_that_is_gone_starts_in_the_project_root_with_the_notice() {
    let sb = Sandbox::new();
    let gone = sb.path("home/gone");
    sb.claude(
        UUID_A,
        "Fix the login redirect",
        &gone.to_string_lossy(),
        60,
    );
    sb.opencode(&[], 0, false);
    sb.agent("claude");
    let mut h = start(&sb);
    open_by_click(&mut h);
    h.wait_for_text("Fix the login redirect", TIMEOUT);
    h.send_keys("\r");
    let start = sb.wait_start("claude");
    // The project root is where deck was started: the directory of its home.
    let root = fs::canonicalize(h.home().path()).unwrap();
    assert_eq!(
        start,
        format!("{}|--resume {UUID_A}", root.display()),
        "the session's own directory is gone"
    );
    h.wait_for_text("no longer exists; started in", TIMEOUT);
}

#[test]
fn mouse_selects_resumes_closes_and_the_wheel_scrolls_one_column() {
    let sb = Sandbox::new();
    let root = sb.path("home").display().to_string();
    for i in 0..30u64 {
        let id = format!("{i:08}-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
        sb.claude(&id, &format!("Session {i:02} of many"), &root, 60 * (i + 1));
    }
    sb.codex(UUID_X, "Write the parser", &root, 60);
    sb.opencode(&[("ses_one", "Rework login form", &root, 120)], 0, false);
    sb.agent("claude");
    let mut h = start(&sb);
    open_by_click(&mut h);
    h.wait_for_text("Write the parser", TIMEOUT);
    h.wait_for_text("Rework login form", TIMEOUT);
    h.wait_for_text("Session 00 of many", TIMEOUT);
    // The wheel over the Claude column scrolls only it: the top session goes.
    for _ in 0..3 {
        wheel_down(&mut h, 12, 10);
    }
    wait_gone(&h, "Session 00 of many");
    assert!(h.screen_text().contains("Rework login form"));
    assert!(h.screen_text().contains("Write the parser"));
    // A click selects the second visible row, a second click resumes it.
    let top = row_of(&h, "Session 06 of many");
    let second = u16::try_from(top).unwrap() + 2;
    click_at(&mut h, 12, second);
    sleep(Duration::from_millis(200));
    assert!(sb.starts("claude").is_empty(), "one click only selects");
    click_at(&mut h, 12, second);
    let start = sb.wait_start("claude");
    assert!(
        start.contains("--resume 00000007-aaaa-4aaa-8aaa-aaaaaaaaaaaa"),
        "{start}"
    );
    wait_gone(&h, "Restore agent session");

    // A click outside closes.
    open_by_click(&mut h);
    h.wait_for_text("Restore agent session", TIMEOUT);
    click_at(&mut h, 1, 1);
    wait_gone(&h, "Restore agent session");
    assert_eq!(sb.starts("claude").len(), 1);
}

#[test]
fn fifty_sessions_scroll_with_the_selection_and_end_reaches_the_last() {
    let sb = Sandbox::new();
    let root = sb.path("home").display().to_string();
    for i in 0..50u64 {
        let id = format!("{i:08}-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
        sb.claude(&id, &format!("Session {i:02} of many"), &root, 60 * (i + 1));
    }
    sb.opencode(&[], 0, false);
    sb.agent("claude");
    let mut h = start(&sb);
    open_by_click(&mut h);
    h.wait_for_text("Claude Code (50)", TIMEOUT);
    for _ in 0..20 {
        h.send_keys(DOWN);
    }
    h.wait_for_text("Session 20 of many", TIMEOUT);
    h.send_keys("\x1b[F");
    h.wait_for_text("Session 49 of many", TIMEOUT);
    h.send_keys("\r");
    let start = sb.wait_start("claude");
    assert!(
        start.ends_with("--resume 00000049-aaaa-4aaa-8aaa-aaaaaaaaaaaa"),
        "{start}"
    );
}

#[test]
fn the_prefix_binding_and_sidebar_focus_r_open_the_dialog() {
    let sb = Sandbox::new();
    fixtures(&sb);
    let mut h = start(&sb);
    prefixed(&mut h, "R");
    h.wait_for_text("Restore agent session", TIMEOUT);
    h.send_keys(ESC);
    wait_gone(&h, "Restore agent session");
    // Sidebar focus: Tab, then R.
    prefixed(&mut h, "\t");
    h.wait_for_text("c new", TIMEOUT);
    h.send_keys("R");
    h.wait_for_text("Restore agent session", TIMEOUT);
}

#[test]
fn without_a_project_the_binding_and_the_button_flash_and_open_nothing() {
    let sb = Sandbox::new();
    fixtures(&sb);
    let mut h = start(&sb);
    prefixed(&mut h, "Q");
    h.wait_for_text("(y/n)", TIMEOUT);
    h.send_keys("y");
    wait_until("no project", TIMEOUT, || projects(&h).is_empty());
    prefixed(&mut h, "R");
    h.wait_for_text("no project to restore into", TIMEOUT);
    assert!(!h.screen_text().contains("Restore agent session"));
    wait_gone(&h, "no project to restore into");
    click_at(&mut h, CHIP, BAR_ROW);
    h.wait_for_text("no project to restore into", TIMEOUT);
    assert!(!h.screen_text().contains("Restore agent session"));
}

#[test]
fn a_missing_claude_flashes_and_creates_no_panel() {
    let sb = Sandbox::new();
    fixtures(&sb);
    fs::remove_file(sb.path("bin/claude")).unwrap();
    let mut h = start(&sb);
    open_by_click(&mut h);
    h.wait_for_text("Fix the login redirect", TIMEOUT);
    h.send_keys("\r");
    h.wait_for_text("not found in PATH", TIMEOUT);
    wait_gone(&h, "Restore agent session");
    sleep(Duration::from_millis(200));
    assert!(projects(&h)[0].1.len() == 1, "{}", h.screen_text());
}

#[test]
fn a_narrow_window_shows_one_column_and_tab_switches() {
    let sb = Sandbox::new();
    fixtures(&sb);
    let mut h = sb.builder((50, 24)).deck();
    h.wait_for_text("│", TIMEOUT);
    prefixed(&mut h, "R");
    h.wait_for_text("‹ Claude Code (2) ›", TIMEOUT);
    h.wait_for_text("Fix the login redirect", TIMEOUT);
    assert!(!h.screen_text().contains("Write the parser"));
    h.send_keys("\t");
    h.wait_for_text("‹ OpenCode (1) ›", TIMEOUT);
    h.wait_for_text("Rework login form", TIMEOUT);
    assert!(!h.screen_text().contains("Fix the login redirect"));
    h.capture("restore-narrow");
}

#[test]
fn closing_with_answers_pending_drops_the_late_ones() {
    let sb = Sandbox::new();
    fixtures(&sb);
    sb.opencode(&[("ses_one", "Rework login form", "/tmp", 60)], 2, false);
    let mut h = start(&sb);
    open_by_click(&mut h);
    h.wait_for_text("loading…", TIMEOUT);
    h.send_keys(ESC);
    wait_gone(&h, "Restore agent session");
    // The slow answer arrives after the dialog is gone, and changes nothing.
    sleep(Duration::from_millis(2800));
    let s = h.screen_text();
    assert!(
        !s.contains("Rework login form") && !s.contains("Restore agent session"),
        "{s}"
    );
    // The keyboard is the terminal's again.
    h.send_keys("echo ok$((1+2))\r");
    h.wait_for_text("ok3", TIMEOUT);
}

#[test]
fn a_restore_key_override_moves_the_binding_and_shows_in_the_help() {
    let sb = Sandbox::new();
    fixtures(&sb);
    let mut h = sb.builder(SIZE).config("keys.restore", "\"Z\"").deck();
    h.wait_for_text("│", TIMEOUT);
    prefixed(&mut h, "R");
    h.wait_for_text("no binding for R", TIMEOUT);
    assert!(!h.screen_text().contains("Restore agent session"));
    prefixed(&mut h, "Z");
    h.wait_for_text("Restore agent session", TIMEOUT);
    h.send_keys(ESC);
    wait_gone(&h, "Restore agent session");
    prefixed(&mut h, "?");
    h.wait_for_text("Help", TIMEOUT);
    let found = |h: &Harness| {
        rows(h)
            .iter()
            .any(|r| r.contains("Restore an agent session") && r.contains(" Z "))
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
