//! The stress and soak suite: the daemon under heavy load and for long, measured with
//! `deck stats --json`. Every scenario is ignored; `just stress` runs them in a release
//! build, one at a time. Each writes `target/stress/<scenario>.json` and prints a summary.
//!
//! Bounds follow `docs/specs/2026-11-23-daemon-stats/00-overview.md`: time bounds 3 to 5
//! times the measured value, memory bounds twice, descriptor and thread checks the
//! baseline plus 2. Each bound has the value it came from next to it (macOS, see
//! `docs/features/stats.md` for the machine).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use std::fmt::Write as _;
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde_json::{Value, json};
use support::{Harness, Home, stderr, stdout};

const MIB: f64 = 1024.0 * 1024.0;

fn mib(bytes: u64) -> f64 {
    bytes as f64 / MIB
}

// ---------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------

fn stats_json(home: &Home) -> Value {
    let out = home.run_deck(&["stats", "--json"]);
    assert!(out.status.success(), "{}", stderr(&out));
    serde_json::from_str(&stdout(&out)).unwrap()
}

/// The figures the checks compare: (rss, fds, threads).
fn figures(home: &Home) -> (u64, u64, u64) {
    let j = stats_json(home);
    (
        j["rss_bytes"].as_u64().unwrap(),
        j["fds"].as_u64().unwrap(),
        j["threads"].as_u64().unwrap(),
    )
}

fn new_terminal(home: &Home, panel: Option<&str>, focus: bool, program: &[&str]) -> String {
    let mut args = vec!["new-terminal"];
    if let Some(panel) = panel {
        args.extend(["--panel", panel]);
    }
    if focus {
        args.push("--focus");
    }
    if !program.is_empty() {
        args.push("--");
        args.extend_from_slice(program);
    }
    let out = home.run_deck(&args);
    assert!(out.status.success(), "{}", stderr(&out));
    stdout(&out).trim().to_string()
}

fn new_panel(home: &Home, program: &[&str]) -> String {
    let mut args = vec!["new-panel", "--"];
    args.extend_from_slice(program);
    let out = home.run_deck(&args);
    assert!(out.status.success(), "{}", stderr(&out));
    stdout(&out).trim().to_string()
}

/// The id of the first panel of the first session.
fn first_panel(home: &Home) -> String {
    let out = home.run_deck(&["ls", "--json"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let json: Value = serde_json::from_str(&stdout(&out)).unwrap();
    json["sessions"][0]["spaces"][0]["panels"][0]["id"]
        .as_u64()
        .unwrap()
        .to_string()
}

/// A home with a daemon and one session (one shell, no client).
fn probed(config: &[(&str, &str)], env: &[(&str, Option<&str>)]) -> Home {
    let home = Home::with(config, env);
    let out = home.run_deck(&["_probe"]);
    assert!(out.status.success(), "{}", stderr(&out));
    home
}

fn wait_for(what: &str, limit: Duration, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + limit;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Polls for up to 15 s (tokio's blocking pool keeps a thread for 10 s) until the
/// descriptors and threads are back within 2 of the baseline; the error says what is left.
fn settle(home: &Home, baseline: (u64, u64, u64), threads_too: bool) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let (_, fds, threads) = figures(home);
        let fds_ok = fds.abs_diff(baseline.1) <= 2;
        let threads_ok = !threads_too || threads.abs_diff(baseline.2) <= 2;
        if fds_ok && threads_ok {
            return Ok(());
        }
        if Instant::now() >= deadline {
            // Starts with the figure that is off, so a test can tell which check failed.
            return Err(if fds_ok {
                format!("threads {threads} (baseline {})", baseline.2)
            } else {
                format!("fds {fds} (baseline {})", baseline.1)
            });
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn record(name: &str, value: &Value, summary: &str) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("stress");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join(format!("{name}.json")),
        serde_json::to_string_pretty(value).unwrap() + "\n",
    )
    .unwrap();
    eprintln!("stress {name}: {summary}");
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    let at = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[at]
}

/// A second client in its own PTY on the home, for attach and detach. It answers no
/// terminal queries, so every attach waits out the client's 200 ms colour probe.
struct Client {
    child: Box<dyn Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    _master: Box<dyn MasterPty + Send>,
}

impl Client {
    fn attach(home: &Home) -> Self {
        let pty = native_pty_system()
            .openpty(PtySize {
                rows: 30,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("open a PTY");
        let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_deck"));
        cmd.env_clear();
        for (k, v) in home.env() {
            cmd.env(k, v);
        }
        cmd.cwd(home.path());
        let child = pty.slave.spawn_command(cmd).expect("spawn deck");
        drop(pty.slave);
        let mut reader = pty.master.try_clone_reader().expect("PTY reader");
        std::thread::spawn(move || {
            let mut buf = [0u8; 16 * 1024];
            while matches!(reader.read(&mut buf), Ok(n) if n > 0) {}
        });
        let writer = pty.master.take_writer().expect("PTY writer");
        Self {
            child,
            writer,
            _master: pty.master,
        }
    }

    /// Presses the prefix and `d`, and waits for the client to exit. The client must be
    /// attached already (`deck stats` counts it), or the keys would be taken as answers
    /// to its start-up queries.
    fn detach(mut self) {
        self.writer.write_all(b"\x1b d").expect("write the keys");
        self.writer.flush().expect("flush the keys");
        let deadline = Instant::now() + Duration::from_secs(10);
        while self
            .child
            .try_wait()
            .expect("wait for the client")
            .is_none()
        {
            assert!(Instant::now() < deadline, "the client did not detach");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn wait_clients(home: &Home, clients: u64) {
    wait_for(
        &format!("{clients} client(s) in deck stats"),
        Duration::from_secs(10),
        || stats_json(home)["clients"] == clients,
    );
}

// ---------------------------------------------------------------------------------------
// flood
// ---------------------------------------------------------------------------------------

/// Measured: 12.8 s (4 times it is 51 s; 60 s leaves room for a slower runner).
const FLOOD_MAX_SECS: f64 = 60.0;
/// Measured: 34 MiB at the peak, sampled every 250 ms or so (`deck stats` answered in at
/// most 9 ms during the flood, so the sampling is not held up by the flooding terminal).
const FLOOD_MAX_RSS_MIB: f64 = 70.0;
/// Measured: 7 ms at the median, 20 ms at most, over about 60 keys. 5 times the maximum.
const FLOOD_MAX_ECHO_SECS: f64 = 0.1;

#[test]
#[ignore = "stress: run with `just stress`"]
fn flood() {
    let mut h = Harness::builder((120, 30)).deck();
    h.wait_for_text("$", Duration::from_secs(10));
    let home_panel = first_panel(h.home());
    let baseline = stats_json(h.home());

    // The flood, then a shell next to it in the same panel, focused for the typing.
    let flood_cmd = "head -c 209715200 /dev/urandom | base64 | fold -w 100";
    let started = Instant::now();
    let flood_id: u64 = new_terminal(h.home(), Some(&home_panel), false, &["sh", "-c", flood_cmd])
        .parse()
        .unwrap();
    // The terminal closes when the program ends: that is the end of the flood.
    let _shell = new_terminal(h.home(), Some(&home_panel), true, &[]);

    // The shell is up and has the focus when its title shows; `#` is not in base64.
    h.wait_for_text("3 · sh", Duration::from_secs(10));
    std::thread::sleep(Duration::from_millis(500));
    let marks = h.screen_text().matches('#').count();

    let mut peak_rss = 0u64;
    let mut stats_secs: Vec<f64> = Vec::new();
    let mut echo_secs: Vec<f64> = Vec::new();
    let mut typed = 0usize;
    let mut bytes_read = 0u64;
    let mut failure: Option<String> = None;
    let finished;
    'flood: loop {
        let t = Instant::now();
        let j = stats_json(h.home());
        stats_secs.push(t.elapsed().as_secs_f64());
        peak_rss = peak_rss.max(j["rss_bytes"].as_u64().unwrap());
        let flood = j["terminal_list"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"] == flood_id);
        match flood {
            Some(t) => bytes_read = t["bytes_read"].as_u64().unwrap(),
            None => {
                finished = started.elapsed();
                break;
            }
        }
        if started.elapsed() >= Duration::from_secs_f64(FLOOD_MAX_SECS) {
            failure = Some(format!(
                "the flood did not finish in {FLOOD_MAX_SECS} s ({:.0} MiB read)",
                mib(bytes_read)
            ));
            finished = started.elapsed();
            break;
        }

        // Responsiveness: a key typed into the other terminal echoes on screen.
        typed += 1;
        let t = Instant::now();
        h.send_keys("#");
        let deadline = Instant::now() + Duration::from_secs(10);
        while h.screen_text().matches('#').count() < typed + marks {
            if Instant::now() >= deadline {
                failure = Some("the key typed during the flood did not echo in 10 s".into());
                finished = started.elapsed();
                break 'flood;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        echo_secs.push(t.elapsed().as_secs_f64());
        std::thread::sleep(Duration::from_millis(200));
    }

    let after = stats_json(h.home());
    let rss_after = after["rss_bytes"].as_u64().unwrap();
    echo_secs.sort_by(f64::total_cmp);
    stats_secs.sort_by(f64::total_cmp);
    let echo_max = echo_secs.last().copied().unwrap_or(0.0);
    let echo_p50 = if echo_secs.is_empty() {
        0.0
    } else {
        percentile(&echo_secs, 0.5)
    };
    let stats_max = *stats_secs.last().unwrap();
    let secs = finished.as_secs_f64();
    let rss_base = baseline["rss_bytes"].as_u64().unwrap();
    record(
        "flood",
        &json!({
            "secs": secs,
            "peak_rss_mib": mib(peak_rss),
            "baseline_rss_mib": mib(rss_base),
            "rss_after_mib": mib(rss_after),
            "echo_probes": echo_secs.len(),
            "echo_p50_secs": echo_p50,
            "echo_max_secs": echo_max,
            "stats_max_secs": stats_max,
            "bytes_read_last_seen_mib": mib(bytes_read),
            "failure": failure,
        }),
        &format!(
            "{secs:.1} s, peak rss {:.0} MiB, echo p50 {:.0} ms max {:.0} ms, stats max {:.0} ms",
            mib(peak_rss),
            echo_p50 * 1000.0,
            echo_max * 1000.0,
            stats_max * 1000.0
        ),
    );
    assert!(failure.is_none(), "{}", failure.unwrap_or_default());
    assert!(secs < FLOOD_MAX_SECS, "flood took {secs:.1} s");
    assert!(
        mib(peak_rss) < FLOOD_MAX_RSS_MIB,
        "peak rss {:.0} MiB",
        mib(peak_rss)
    );
    assert!(
        echo_max < FLOOD_MAX_ECHO_SECS,
        "a key took {echo_max:.2} s to echo during the flood"
    );
}

// ---------------------------------------------------------------------------------------
// many terminals
// ---------------------------------------------------------------------------------------

/// Measured: 3 ms for `deck ls` (a process start and one request); 0.25 s is far above
/// the rule's 3 to 5 times because a runner's process start dominates.
const MANY_MAX_LS_SECS: f64 = 0.25;
/// Measured: 276 MiB for 65 terminals, each holding 10,000 lines of history.
const MANY_MAX_RSS_MIB: f64 = 560.0;
/// Measured: 4.1 MiB per terminal above the baseline.
const MANY_MAX_RSS_PER_TERMINAL_MIB: f64 = 8.5;

#[test]
#[ignore = "stress: run with `just stress`"]
fn many_terminals() {
    let home = probed(&[], &[]);
    let baseline = stats_json(&home);
    let print = ["sh", "-c", "seq 1 10000; sleep 3600"];
    let mut panels = vec![first_panel(&home)];
    let mut printers: Vec<String> = Vec::new();
    for _ in 1..16 {
        let panel = new_panel(&home, &print);
        panels.push(panel);
    }
    // 15 printers came with the panels; 49 more make 64, spread over the 16 panels.
    for i in 0..49 {
        printers.push(new_terminal(&home, Some(&panels[i % 16]), false, &print));
    }
    let want = 64;
    let started = Instant::now();
    // `seq 1 10000` is 48,894 bytes. A timeout is recorded, then fails the test.
    let deadline = Instant::now() + Duration::from_secs(120);
    let printed = loop {
        let j = stats_json(&home);
        let printed = j["terminal_list"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|t| t["bytes_read"].as_u64().unwrap() >= 48_894)
            .count();
        if printed >= want || Instant::now() >= deadline {
            break printed;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let print_secs = started.elapsed().as_secs_f64();

    let j = stats_json(&home);
    let terminals = j["terminals"].as_u64().unwrap();
    let rss = j["rss_bytes"].as_u64().unwrap();
    let rss_base = baseline["rss_bytes"].as_u64().unwrap();
    let per_terminal = mib(rss.saturating_sub(rss_base)) / terminals as f64;
    let mut ls_secs: Vec<f64> = (0..5)
        .map(|_| {
            let t = Instant::now();
            let out = home.run_deck(&["ls"]);
            assert!(out.status.success(), "{}", stderr(&out));
            t.elapsed().as_secs_f64()
        })
        .collect();
    ls_secs.sort_by(f64::total_cmp);
    let ls_max = *ls_secs.last().unwrap();
    record(
        "many_terminals",
        &json!({
            "terminals": terminals,
            "panels": panels.len(),
            "print_secs": print_secs,
            "rss_mib": mib(rss),
            "baseline_rss_mib": mib(rss_base),
            "rss_per_terminal_mib": per_terminal,
            "fds": j["fds"],
            "threads": j["threads"],
            "ls_max_secs": ls_max,
            "printed_all": printed,
        }),
        &format!(
            "{terminals} terminals, rss {:.0} MiB ({per_terminal:.1} MiB each), ls {:.0} ms",
            mib(rss),
            ls_max * 1000.0
        ),
    );
    assert!(
        printed >= want,
        "only {printed} of {want} terminals printed"
    );
    assert!(terminals >= 64, "{terminals} terminals");
    assert!(mib(rss) < MANY_MAX_RSS_MIB, "rss {:.0} MiB", mib(rss));
    assert!(
        per_terminal < MANY_MAX_RSS_PER_TERMINAL_MIB,
        "{per_terminal:.1} MiB per terminal"
    );
    assert!(ls_max < MANY_MAX_LS_SECS, "deck ls took {ls_max:.2} s");
}

// ---------------------------------------------------------------------------------------
// churn
// ---------------------------------------------------------------------------------------

const CHURN_CYCLES: u32 = 500;
/// Measured: +0.7 MiB over the baseline after 500 terminals. Twice that is inside the
/// allocator's noise, so the bound is 16 MiB: a leak of even 32 KiB per terminal shows.
const CHURN_MAX_RSS_GROWTH_MIB: f64 = 16.0;

/// Opens and closes `cycles` terminals one after the other; an error says what did not
/// come back.
fn churn(home: &Home, cycles: u32, name: &str) -> Result<(), String> {
    let panel = first_panel(home);
    // Warm up: the first terminals start threads the daemon keeps for good (measured: two
    // or three), so the baseline is taken once they exist and the blocking pool (10 s of
    // keep-alive) has let its idle ones go.
    for _ in 0..3 {
        let id = new_terminal(home, Some(&panel), false, &["sleep", "600"]);
        assert!(home.run_deck(&["close", &id]).status.success());
    }
    std::thread::sleep(Duration::from_secs(12));
    let base = figures(home);
    let started = Instant::now();
    for _ in 0..cycles {
        let id = new_terminal(home, Some(&panel), false, &["sleep", "600"]);
        let out = home.run_deck(&["close", &id]);
        assert!(out.status.success(), "{}", stderr(&out));
    }
    let secs = started.elapsed().as_secs_f64();
    let (rss_peak, fds_peak, threads_peak) = figures(home);
    let settled = settle(home, base, true).map_err(|e| format!("churn left {e}"));
    let (rss, fds, threads) = figures(home);
    let growth = mib(rss.saturating_sub(base.0));
    let failure = settled.err().or_else(|| {
        (growth > CHURN_MAX_RSS_GROWTH_MIB)
            .then(|| format!("churn left rss {growth:.0} MiB above the baseline"))
    });
    // Recorded before it can fail, so a failing run leaves its numbers.
    let report = json!({
        "cycles": cycles,
        "secs": secs,
        "baseline": { "rss_mib": mib(base.0), "fds": base.1, "threads": base.2 },
        "right_after": { "rss_mib": mib(rss_peak), "fds": fds_peak, "threads": threads_peak },
        "settled": { "rss_mib": mib(rss), "fds": fds, "threads": threads },
        "rss_growth_mib": growth,
        "failure": failure,
    });
    record(
        name,
        &report,
        &format!(
            "{cycles} cycles in {secs:.1} s, fds {} -> {fds}, threads {} -> {threads}, rss {growth:+.1} MiB{}",
            base.1,
            base.2,
            failure
                .as_ref()
                .map_or(String::new(), |f| format!(" ({f})"))
        ),
    );
    failure.map_or(Ok(()), Err)
}

#[test]
#[ignore = "stress: run with `just stress`"]
fn churn_500_terminals() {
    let home = probed(&[], &[]);
    churn(&home, CHURN_CYCLES, "churn").unwrap_or_else(|e| panic!("{e}"));
}

/// The check has teeth: a daemon that leaks a descriptor per terminal fails churn.
#[test]
#[ignore = "stress: run with `just stress`"]
fn churn_catches_a_leak() {
    let home = probed(&[], &[("DECK_TEST_LEAK_FD", Some("1"))]);
    let err = churn(&home, 30, "churn_catches_a_leak")
        .expect_err("churn must notice the leaked descriptors");
    // The descriptor check failed, not only the thread check.
    assert!(err.starts_with("churn left fds "), "{err}");
    eprintln!("stress churn_catches_a_leak: caught: {err}");
}

// ---------------------------------------------------------------------------------------
// attach loop
// ---------------------------------------------------------------------------------------

const ATTACH_CYCLES: u32 = 300;

#[test]
#[ignore = "stress: run with `just stress`"]
fn attach_loop() {
    let home = probed(&[], &[]);
    let base = figures(&home);
    let started = Instant::now();
    let mut peak_fds = base.1;
    for i in 0..ATTACH_CYCLES {
        let client = Client::attach(&home);
        wait_clients(&home, 1);
        if i % 50 == 0 {
            peak_fds = peak_fds.max(figures(&home).1);
        }
        client.detach();
    }
    let secs = started.elapsed().as_secs_f64();
    wait_clients(&home, 0);
    let settled = settle(&home, base, false);
    let (rss, fds, threads) = figures(&home);
    record(
        "attach_loop",
        &json!({
            "cycles": ATTACH_CYCLES,
            "secs": secs,
            "baseline": { "rss_mib": mib(base.0), "fds": base.1, "threads": base.2 },
            "peak_fds_attached": peak_fds,
            "settled": { "rss_mib": mib(rss), "fds": fds, "threads": threads },
        }),
        &format!(
            "{ATTACH_CYCLES} attaches in {secs:.1} s, fds {} -> {fds}, rss {:.1} -> {:.1} MiB",
            base.1,
            mib(base.0),
            mib(rss)
        ),
    );
    settled.unwrap_or_else(|e| panic!("attach loop left {e}"));
}

// ---------------------------------------------------------------------------------------
// control loop
// ---------------------------------------------------------------------------------------

const CONTROL_CYCLES: u32 = 2000;

#[test]
#[ignore = "stress: run with `just stress`"]
fn control_loop() {
    let home = probed(&[], &[]);
    let panel = first_panel(&home);
    let id = new_terminal(&home, Some(&panel), false, &[]);
    let base = figures(&home);
    let started = Instant::now();
    let mut peak_fds = base.1;
    for i in 0..CONTROL_CYCLES {
        let text = format!("echo {i}");
        let out = home.run_deck(&["send", "--terminal", &id, "--enter", &text]);
        assert!(out.status.success(), "send {i}: {}", stderr(&out));
        let out = home.run_deck(&["capture", "--terminal", &id]);
        assert!(out.status.success(), "capture {i}: {}", stderr(&out));
        if i % 250 == 0 {
            peak_fds = peak_fds.max(figures(&home).1);
        }
    }
    let secs = started.elapsed().as_secs_f64();
    let settled = settle(&home, base, false);
    let (rss, fds, threads) = figures(&home);
    record(
        "control_loop",
        &json!({
            "cycles": CONTROL_CYCLES,
            "secs": secs,
            "baseline": { "rss_mib": mib(base.0), "fds": base.1, "threads": base.2 },
            "peak_fds": peak_fds,
            "settled": { "rss_mib": mib(rss), "fds": fds, "threads": threads },
        }),
        &format!(
            "{CONTROL_CYCLES} send+capture in {secs:.1} s, fds {} -> {fds}, rss {:.1} -> {:.1} MiB",
            base.1,
            mib(base.0),
            mib(rss)
        ),
    );
    settled.unwrap_or_else(|e| panic!("control loop left {e}"));
}

// ---------------------------------------------------------------------------------------
// soak
// ---------------------------------------------------------------------------------------

const SOAK_WARMUP_SECS: f64 = 120.0;
const SOAK_MIN_SAMPLES: usize = 30;
/// Measured over 10 minutes: -1.6 and -11.4 MiB/hour in two runs (the heap settles and
/// shrinks; a 49-sample window is noisy by about 10 MiB/hour). 50 is far above that noise
/// and far below a real leak, which grows by hundreds of MiB per hour under this load.
const SOAK_MAX_RSS_MIB_PER_HOUR: f64 = 50.0;
/// Measured: 0.0 per hour (20 descriptors in every sample). The spec's bound.
const SOAK_MAX_FDS_PER_HOUR: f64 = 1.0;
/// Measured: 0.03 CPU seconds in the quiet minute. About 7 times that: a runner's timers
/// cost more, and a daemon that spins costs seconds.
const SOAK_MAX_IDLE_CPU_SECS_PER_MINUTE: f64 = 0.2;

/// The least-squares slope of `y` against `x`.
fn slope(points: &[(f64, f64)]) -> f64 {
    let n = points.len() as f64;
    let (sx, sy) = points
        .iter()
        .fold((0.0, 0.0), |(sx, sy), (x, y)| (sx + x, sy + y));
    let (mx, my) = (sx / n, sy / n);
    let num: f64 = points.iter().map(|(x, y)| (x - mx) * (y - my)).sum();
    let den: f64 = points.iter().map(|(x, _)| (x - mx) * (x - mx)).sum();
    num / den
}

fn cpu_secs(j: &Value) -> f64 {
    j["cpu_user_secs"].as_f64().unwrap() + j["cpu_system_secs"].as_f64().unwrap()
}

#[test]
#[ignore = "stress: run with `just stress`"]
fn soak() {
    let secs: u64 = std::env::var("DECK_SOAK_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(600);
    let home = probed(&[("scrollback_lines", "1000")], &[]);
    let panel = first_panel(&home);
    let shell = stats_json(&home)["terminal_list"][0]["id"].clone();
    // One line every 10 ms.
    new_terminal(
        &home,
        Some(&panel),
        false,
        &[
            "sh",
            "-c",
            "i=0; while :; do i=$((i+1)); echo line $i; sleep 0.01; done",
        ],
    );

    let csv_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("stress");
    fs::create_dir_all(&csv_path).unwrap();
    let csv_path = csv_path.join("soak.csv");
    let mut csv = String::from(
        "elapsed_secs,rss_bytes,fds,threads,cpu_user_secs,cpu_system_secs,terminals,clients\n",
    );
    let mut samples: Vec<(f64, f64, f64)> = Vec::new();

    let started = Instant::now();
    let mut tick = 0u64;
    while tick <= secs {
        // The load. Terminals and clients come and go within one tick of the 10 s
        // samples' phase, so every sample sees the same quiet state and the slopes are
        // not drowned by the sawtooth.
        if tick % 30 == 11 {
            let client = Client::attach(&home);
            wait_clients(&home, 1);
            std::thread::sleep(Duration::from_secs(1));
            client.detach();
        }
        if tick % 5 == 2 {
            let id = new_terminal(
                &home,
                Some(&panel),
                false,
                &["sh", "-c", "seq 1 2000; sleep 600"],
            );
            std::thread::sleep(Duration::from_secs(1));
            let out = home.run_deck(&["close", &id]);
            assert!(out.status.success(), "{}", stderr(&out));
        }
        if tick.is_multiple_of(10) {
            let j = stats_json(&home);
            let at = started.elapsed().as_secs_f64();
            let rss = j["rss_bytes"].as_u64().unwrap();
            let fds = j["fds"].as_u64().unwrap();
            let _ = writeln!(
                csv,
                "{at:.1},{rss},{fds},{},{},{},{},{}",
                j["threads"],
                j["cpu_user_secs"],
                j["cpu_system_secs"],
                j["terminals"],
                j["clients"]
            );
            fs::write(&csv_path, &csv).unwrap();
            if at >= SOAK_WARMUP_SECS {
                samples.push((at / 3600.0, mib(rss), fds as f64));
            }
        }
        tick += 1;
        let next = started + Duration::from_secs(tick);
        std::thread::sleep(next.saturating_duration_since(Instant::now()));
    }

    // A quiet minute: the load stops, and what is left is the daemon's own upkeep.
    let ids: Vec<Value> = {
        let j = stats_json(&home);
        j["terminal_list"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].clone())
            .collect()
    };
    for id in ids.iter().filter(|id| **id != shell) {
        let out = home.run_deck(&["close", &id.to_string()]);
        assert!(out.status.success(), "{}", stderr(&out));
    }
    assert_eq!(stats_json(&home)["terminals"], 1, "only the shell is left");
    std::thread::sleep(Duration::from_secs(15));
    let before = stats_json(&home);
    std::thread::sleep(Duration::from_secs(60));
    let after = stats_json(&home);
    let idle_cpu = cpu_secs(&after) - cpu_secs(&before);

    let rss_slope = (samples.len() >= SOAK_MIN_SAMPLES)
        .then(|| slope(&samples.iter().map(|(x, y, _)| (*x, *y)).collect::<Vec<_>>()));
    let fds_slope = (samples.len() >= SOAK_MIN_SAMPLES)
        .then(|| slope(&samples.iter().map(|(x, _, z)| (*x, *z)).collect::<Vec<_>>()));
    record(
        "soak",
        &json!({
            "secs": secs,
            "samples_after_warmup": samples.len(),
            "rss_slope_mib_per_hour": rss_slope,
            "fds_slope_per_hour": fds_slope,
            "idle_cpu_secs_per_minute": idle_cpu,
            "csv": "target/stress/soak.csv",
        }),
        &format!(
            "{secs} s, {} samples, rss {:?} MiB/h, fds {:?}/h, idle cpu {idle_cpu:.3} s/min",
            samples.len(),
            rss_slope.map(|s| (s * 10.0).round() / 10.0),
            fds_slope.map(|s| (s * 100.0).round() / 100.0)
        ),
    );
    if let (Some(rss_slope), Some(fds_slope)) = (rss_slope, fds_slope) {
        assert!(
            rss_slope <= SOAK_MAX_RSS_MIB_PER_HOUR,
            "rss grows {rss_slope:.1} MiB/hour"
        );
        assert!(
            fds_slope <= SOAK_MAX_FDS_PER_HOUR,
            "fds grow {fds_slope:.2}/hour"
        );
    }
    assert!(
        idle_cpu <= SOAK_MAX_IDLE_CPU_SECS_PER_MINUTE,
        "idle daemon used {idle_cpu:.3} CPU s in a minute"
    );
}
