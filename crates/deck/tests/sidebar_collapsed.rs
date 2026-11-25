//! The collapsed sidebar, end to end: the real `deck` in a PTY. Mouse events are written to
//! the outer terminal as the SGR sequences a terminal sends (1-based cells).

#![allow(
    clippy::unwrap_used,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant};

use support::{Harness, TIMEOUT, wait_until};

const SIZE: (u16, u16) = (100, 24);
const PREFIX: &[u8] = b"\x1b ";

fn attached_with(config: &[(&str, &str)]) -> Harness {
    let mut builder = Harness::builder(SIZE).config_unset("sidebar");
    for (key, value) in config {
        builder = builder.config(key, value);
    }
    let h = builder.deck();
    h.wait_for_text("│$", TIMEOUT);
    h
}

fn prefixed(h: &mut Harness, key: &str) {
    h.send_bytes(PREFIX);
    h.send_keys(key);
}

fn rows(h: &Harness) -> Vec<String> {
    h.screen_text().lines().map(str::to_string).collect()
}

fn row(h: &Harness, y: usize) -> String {
    rows(h).get(y).cloned().unwrap_or_default()
}

fn click_at(h: &mut Harness, column: u16, row: u16) {
    h.send_keys(&format!("\x1b[<0;{};{}M", column + 1, row + 1));
    h.send_keys(&format!("\x1b[<0;{};{}m", column + 1, row + 1));
}

/// What the shell reports for `stty size`, asked again every second until it answers; every
/// ask has its own tag, so an earlier answer on the screen is never taken for this one.
fn stty(h: &mut Harness) -> String {
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    let mut tags = Vec::new();
    let deadline = Instant::now() + TIMEOUT;
    let mut asked = None;
    loop {
        if asked.is_none_or(|at: Instant| at.elapsed() >= Duration::from_secs(1)) {
            let tag = format!("Q{}Q", CALLS.fetch_add(1, Ordering::Relaxed) + 9000);
            h.send_keys(&format!("echo {tag}$(stty size)E\r"));
            tags.push(tag);
            asked = Some(Instant::now());
        }
        let text = h.screen_text();
        for tag in &tags {
            for line in text.lines() {
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
            "wanted {want}, got {got}:\n{}",
            h.screen_text()
        );
    }
}

#[test]
fn the_chips_collapse_and_expand_the_sidebar_and_the_terminals_follow() {
    let mut h = attached_with(&[]);
    wait_until("the header chip", TIMEOUT, || {
        row(&h, 0).starts_with(" «  main")
    });
    wait_stty(&mut h, "24 71");
    h.capture("expanded");

    click_at(&mut h, 1, 0);
    wait_until("the collapsed sidebar", TIMEOUT, || {
        row(&h, 0).starts_with("   »    │")
    });
    wait_until("the entry", TIMEOUT, || row(&h, 2).starts_with(" ❯  1   │"));
    wait_until("the New chip", TIMEOUT, || {
        row(&h, 23).starts_with("   +    │")
    });
    // The focused terminal of the active panel has the band.
    assert!((0..8).all(|x| h.cell(2, x).inverse()));
    wait_stty(&mut h, "24 91");
    h.capture("collapsed");

    click_at(&mut h, 3, 0);
    wait_until("the full sidebar", TIMEOUT, || {
        row(&h, 0).starts_with(" «  main")
    });
    wait_stty(&mut h, "24 71");
    h.capture("expanded-again");
}

#[test]
fn a_click_on_an_entry_activates_its_panel() {
    let mut h = attached_with(&[]);
    h.wait_for_text("▾", TIMEOUT);
    prefixed(&mut h, "c");
    // Two panels, the second active: its row has the band.
    wait_until("two panels", TIMEOUT, || h.is_active_row(4));
    click_at(&mut h, 1, 0);
    wait_until("two entries", TIMEOUT, || {
        row(&h, 3).starts_with(" ❯  2   │") && row(&h, 2).starts_with(" ❯  1   │")
    });
    wait_until("the band on entry 2", TIMEOUT, || {
        h.is_active_row(3) && !h.is_active_row(2)
    });
    h.capture("collapsed-two-panels");
    click_at(&mut h, 2, 2);
    wait_until("the band on entry 1", TIMEOUT, || {
        h.is_active_row(2) && !h.is_active_row(3)
    });
    h.capture("collapsed-first-entry-clicked");
    click_at(&mut h, 3, 0);
    wait_until("the first panel active", TIMEOUT, || h.is_active_row(3));
}

#[test]
fn it_starts_collapsed_and_the_bound_key_expands_it() {
    let mut h = attached_with(&[
        ("sidebar_collapsed", "true"),
        ("keys", "{ collapse_sidebar = \"B\" }"),
    ]);
    wait_until("collapsed at start", TIMEOUT, || {
        row(&h, 0).starts_with("   »    │")
    });
    // The daemon sized the terminal for the strip: no reflow after attach.
    wait_stty(&mut h, "24 91");
    prefixed(&mut h, "B");
    wait_until("the full sidebar", TIMEOUT, || {
        row(&h, 0).starts_with(" «  main")
    });
    wait_stty(&mut h, "24 71");
    prefixed(&mut h, "B");
    wait_until("collapsed again", TIMEOUT, || {
        row(&h, 0).starts_with("   »    │")
    });
    wait_stty(&mut h, "24 91");
}
