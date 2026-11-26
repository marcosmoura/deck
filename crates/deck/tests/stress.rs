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
    clippy::panic,
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

use deck_daemon::Marker;

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde_json::{Value, json};
use support::{Harness, Home, stderr, stdout, sys};

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

// ---------------------------------------------------------------------------------------
// Review measurements (docs/reviews/2026-11-25-performance.md)
//
// Each records its numbers before it asserts anything, and its bounds are generous: a missed
// target of the review (5 ms at rest, 20 ms during a flood, 0.5% idle CPU) is a finding in
// the report, not a red run. Figures in the comments are from the report's machine.
// ---------------------------------------------------------------------------------------

/// Min, p50, p90, p99, max and every sample, in milliseconds.
fn summary_ms(secs: &[f64]) -> Value {
    if secs.is_empty() {
        return json!({ "n": 0 });
    }
    let mut sorted: Vec<f64> = secs.iter().map(|s| s * 1000.0).collect();
    sorted.sort_by(f64::total_cmp);
    json!({
        "n": sorted.len(),
        "min": sorted[0],
        "p50": percentile(&sorted, 0.5),
        "p90": percentile(&sorted, 0.9),
        "p99": percentile(&sorted, 0.99),
        "max": sorted[sorted.len() - 1],
        "samples": secs.iter().map(|s| s * 1000.0).collect::<Vec<_>>(),
    })
}

fn p50_ms(secs: &[f64]) -> f64 {
    let mut sorted = secs.to_vec();
    sorted.sort_by(f64::total_cmp);
    if sorted.is_empty() {
        f64::NAN
    } else {
        percentile(&sorted, 0.5) * 1000.0
    }
}

fn max_ms(secs: &[f64]) -> f64 {
    secs.iter().copied().fold(0.0, f64::max) * 1000.0
}

/// A flood of lines that all differ: `cat` over and over of the file `throughput` writes
/// (a constant line would redraw almost nothing on the client).
fn flood_command(file: &std::path::Path) -> String {
    format!("while :; do cat {}; done", file.display())
}

fn flood_program(file: &std::path::Path) -> Vec<String> {
    vec!["sh".into(), "-c".into(), flood_command(file)]
}

fn spawn_program(home: &Home, panel: Option<&str>, program: &[String]) -> String {
    let refs: Vec<&str> = program.iter().map(String::as_str).collect();
    new_terminal(home, panel, false, &refs)
}

/// Types `x` into the focused terminal `count` times, `gap` apart, with Enter after every
/// 40; returns, per key, the seconds until the echo cell showed and until the first output
/// after the key (output before the echo is not the echo: the client's no-op draw).
fn type_keys(h: &mut Harness, count: usize, gap: Duration) -> (Vec<f64>, Vec<f64>, usize) {
    let (mut echo, mut first) = (Vec::new(), Vec::new());
    let mut lost = 0;
    for i in 0..count {
        if i > 0 && i % 40 == 0 {
            h.send_keys("\r");
            std::thread::sleep(gap);
        }
        let (row, col, _) = h.cursor();
        if h.cell(row, col).ch == 'x' {
            lost += 1;
            continue;
        }
        let watch = h.watch_cell(row, col, 'x');
        let sent = Instant::now();
        h.send_keys("x");
        match watch.wait(Duration::from_secs(5)) {
            Some(seen) => {
                echo.push(seen.shown.duration_since(sent).as_secs_f64());
                first.push(seen.first_output.duration_since(sent).as_secs_f64());
            }
            None => lost += 1,
        }
        std::thread::sleep(gap);
    }
    (echo, first, lost)
}

fn load_average() -> String {
    fs::read_to_string("/proc/loadavg").unwrap_or_else(|_| {
        let out = std::process::Command::new("sysctl")
            .args(["-n", "vm.loadavg"])
            .output();
        out.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    })
}

/// `cat` in the shell of the harness, so every typed key is echoed by the terminal line
/// discipline (the least a keystroke can cost) at the cursor.
fn start_cat(h: &mut Harness) {
    h.send_keys("cat\r");
    h.wait_for_text("cat", Duration::from_secs(10));
    std::thread::sleep(Duration::from_millis(500));
}

const LATENCY_SAMPLES: usize = 200;
/// Bounds: p50 16 ms at rest and 55 ms in a flood, 2 s at most for any key or prompt.
/// The report's figures are far below them; the review's targets are 5 and 20 ms.
const LATENCY_MAX_P50_REST_MS: f64 = 16.0;
const LATENCY_MAX_P50_FLOOD_MS: f64 = 55.0;
const LATENCY_MAX_MS: f64 = 2000.0;

#[test]
#[ignore = "stress: run with `just stress`"]
fn latency() {
    let mut h = Harness::builder((200, 60)).config_unset("sidebar").deck();
    h.wait_for_text("$", Duration::from_secs(10));
    let home_panel = first_panel(h.home());
    start_cat(&mut h);
    let load_start = load_average();
    let file = throughput_file(false);

    let gap = Duration::from_millis(300);
    // Warm up: the first keys pay for lazily initialised paths.
    type_keys(&mut h, 20, Duration::from_millis(100));
    let (rest, rest_first, rest_lost) = type_keys(&mut h, LATENCY_SAMPLES, gap);
    let load_rest_end = load_average();

    // A flood in a second terminal of the same panel; the typing stays in the first.
    let flood = spawn_program(h.home(), Some(&home_panel), &flood_program(&file));
    std::thread::sleep(Duration::from_secs(2));
    let (busy, busy_first, busy_lost) = type_keys(&mut h, LATENCY_SAMPLES, gap);
    let busy_stats = stats_json(h.home());
    let load_flood_end = load_average();
    assert!(
        h.home().run_deck(&["close", &flood]).status.success(),
        "close the flood"
    );
    std::thread::sleep(Duration::from_millis(500));
    // Leave `cat`, then flood the same terminal and interrupt it.
    h.send_keys("\x04");
    h.wait_for_text("$", Duration::from_secs(10));
    std::thread::sleep(Duration::from_millis(500));
    let flood_cmd = format!("{}\r", flood_command(&file));
    let mut ctrl_c = Vec::new();
    let mut ctrl_c_first = Vec::new();
    for _ in 0..10 {
        h.send_keys(&flood_cmd);
        std::thread::sleep(Duration::from_millis(1500));
        let watch = h.watch(|s| {
            let row = s.row_text(s.cursor_row());
            row.contains('$') && !row.contains("000000")
        });
        let sent = Instant::now();
        h.send_bytes(b"\x03");
        match watch.wait(Duration::from_secs(10)) {
            Some(seen) => {
                ctrl_c.push(seen.shown.duration_since(sent).as_secs_f64());
                ctrl_c_first.push(seen.first_output.duration_since(sent).as_secs_f64());
            }
            None => panic!(
                "no prompt after Ctrl-C; the screen is:\n{}",
                h.screen_text()
            ),
        }
        std::thread::sleep(Duration::from_millis(1000));
    }
    let load_end = load_average();

    record(
        "latency",
        &json!({
            "size": "200x60, sidebar on, cat in the focused terminal",
            "load_average_start": load_start,
            "load_average_end": load_end,
            "load_average_after_rest_samples": load_rest_end,
            "load_average_after_flood_samples": load_flood_end,
            "flood": "while :; do cat <256 MiB file of distinct 100-column lines>; done",
            "at_rest": { "echo_ms": summary_ms(&rest), "first_output_ms": summary_ms(&rest_first), "lost": rest_lost },
            "flood_in_other_terminal": {
                "echo_ms": summary_ms(&busy),
                "first_output_ms": summary_ms(&busy_first),
                "lost": busy_lost,
                "flood_bytes_read": busy_stats["terminal_list"].as_array().map(|l| l.iter().filter_map(|t| t["bytes_read"].as_u64()).max()),
            },
            "ctrl_c_into_flood_same_terminal": {
                "prompt_ms": summary_ms(&ctrl_c),
                "first_output_ms": summary_ms(&ctrl_c_first),
            },
        }),
        &format!(
            "echo p50 {:.1} ms at rest (first output {:.1}), {:.1} ms in a flood, ctrl-c to prompt p50 {:.0} ms max {:.0} ms",
            p50_ms(&rest),
            p50_ms(&rest_first),
            p50_ms(&busy),
            p50_ms(&ctrl_c),
            max_ms(&ctrl_c)
        ),
    );
    assert!(rest.len() >= LATENCY_SAMPLES / 2, "{} samples", rest.len());
    assert!(busy.len() >= LATENCY_SAMPLES / 2, "{} samples", busy.len());
    assert!(
        p50_ms(&rest) < LATENCY_MAX_P50_REST_MS,
        "{:.1} ms",
        p50_ms(&rest)
    );
    assert!(
        p50_ms(&busy) < LATENCY_MAX_P50_FLOOD_MS,
        "{:.1} ms",
        p50_ms(&busy)
    );
    assert!(max_ms(&rest) < LATENCY_MAX_MS && max_ms(&busy) < LATENCY_MAX_MS);
    assert!(
        max_ms(&ctrl_c) < 5.0 * LATENCY_MAX_MS,
        "ctrl-c {:.0} ms",
        max_ms(&ctrl_c)
    );
}

// ---------------------------------------------------------------------------------------
// stall
// ---------------------------------------------------------------------------------------

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = (data.len() as u32).to_be_bytes().to_vec();
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
    out
}

/// A PNG that declares `width` × `height` RGBA pixels and holds a few bytes of data.
fn tiny_png(width: u32, height: u32) -> Vec<u8> {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = width.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    png.extend(png_chunk(b"IHDR", &ihdr));
    png.extend(png_chunk(
        b"IDAT",
        &[0x78, 0x01, 0x01, 0x00, 0x00, 0xff, 0xff],
    ));
    png.extend(png_chunk(b"IEND", &[]));
    png
}

fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The kitty command that transmits (without displaying) a PNG.
fn png_command(id: u32, png: &[u8]) -> Vec<u8> {
    format!("\x1b_Ga=t,f=100,i={id},q=2;{}\x1b\\", base64(png)).into_bytes()
}

/// The slow sequence: `a` then CSI 65535 b (repeat the character 65535 times), in 64 KiB
/// pieces, over and over.
fn slow_file(home: &Home) -> PathBuf {
    let unit = b"a\x1b[65535b";
    let mut piece = Vec::new();
    while piece.len() + unit.len() <= 64 * 1024 {
        piece.extend_from_slice(unit);
    }
    let path = home.path().join("slow.bin");
    fs::write(&path, piece).unwrap();
    path
}

/// The PNG commands: 200 of them in a file, each `a=t` with a PNG of 4096 × 4096 RGBA
/// pixels (exactly 64 MiB, what the daemon accepts) whose data is a few bytes.
fn png_program(home: &Home) -> Vec<String> {
    let mut file = Vec::new();
    for id in 1..=200 {
        file.extend(png_command(id, &tiny_png(4096, 4096)));
    }
    let path = home.path().join("tiny-png.bin");
    fs::write(&path, file).unwrap();
    slow_program(&path)
}

fn slow_program(file: &std::path::Path) -> Vec<String> {
    vec![
        "sh".into(),
        "-c".into(),
        format!("while :; do cat {}; done", file.display()),
    ]
}

const STALL_KEYS: usize = 60;

/// Presses Enter and waits until the cursor is on a blank cell: a line of 30 keys per
/// variant would otherwise wrap, and a key typed before the cursor moved is not told apart.
fn fresh_line(h: &mut Harness) {
    h.send_keys("\r");
    wait_for("a fresh line", Duration::from_secs(30), || {
        let (row, col, _) = h.cursor();
        h.cell(row, col).ch == ' ' && (col == 0 || h.cell(row, col - 1).ch != 'x')
    });
    std::thread::sleep(Duration::from_millis(100));
}
/// Generous: the figures that matter are in the report.
const STALL_MAX_MS: f64 = 10_000.0;

#[test]
#[ignore = "stress: run with `just stress`"]
fn stall() {
    let mut h = Harness::builder((200, 60)).config_unset("sidebar").deck();
    h.wait_for_text("$", Duration::from_secs(10));
    let panel = first_panel(h.home());
    start_cat(&mut h);
    let load_start = load_average();
    // Two inputs for terminal A: the repeat sequence, and small PNG commands that declare a
    // 64 MiB image each (the daemon allocates and zeroes the declared size before it looks
    // at the data).
    let inputs = [
        ("", slow_program(&slow_file(h.home()))),
        ("png_", png_program(h.home())),
    ];
    let other = h.home().run_deck(&["new", "work", "-d"]);
    assert!(other.status.success(), "{}", stderr(&other));

    let mut report = serde_json::Map::new();
    let mut worst_key = 0.0f64;
    let mut worst_ls = 0.0f64;
    for (prefix, program) in &inputs {
        for variant in [
            "none",
            "active_panel",
            "background_panel",
            "no_client_session",
        ] {
            if !prefix.is_empty() && variant == "none" {
                continue;
            }
            let id = match variant {
                "none" => None,
                "active_panel" => Some(spawn_program(h.home(), Some(&panel), program)),
                "background_panel" => {
                    let mut args = vec!["new-panel", "--"];
                    args.extend(program.iter().map(String::as_str));
                    let out = h.home().run_deck(&args);
                    assert!(out.status.success(), "{}", stderr(&out));
                    // Its terminal's id is the highest one.
                    let list = stats_json(h.home());
                    let terminal = list["terminal_list"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter_map(|t| t["id"].as_u64())
                        .max()
                        .unwrap();
                    Some(terminal.to_string())
                }
                _ => {
                    let mut args = vec!["new-terminal", "--session", "work", "--"];
                    args.extend(program.iter().map(String::as_str));
                    let out = h.home().run_deck(&args);
                    assert!(out.status.success(), "{}", stderr(&out));
                    Some(stdout(&out).trim().to_string())
                }
            };
            std::thread::sleep(Duration::from_secs(2));
            let daemon_pid = h.home().daemon_pid().unwrap();
            let cpu0 = sys::usage(daemon_pid).unwrap();
            let window = Instant::now();
            let mut ls = Vec::new();
            let (mut keys, mut first, mut lost) = (Vec::new(), Vec::new(), 0);
            for n in 0..STALL_KEYS {
                if n % 30 == 0 {
                    fresh_line(&mut h);
                }
                let t = Instant::now();
                let out = h.home().run_deck(&["ls"]);
                assert!(out.status.success(), "{}", stderr(&out));
                ls.push(t.elapsed().as_secs_f64());
                let (k, f, l) = type_keys(&mut h, 1, Duration::from_millis(250));
                keys.extend(k);
                first.extend(f);
                lost += l;
            }
            let cpu =
                (sys::usage(daemon_pid).unwrap() - cpu0).cpu_secs / window.elapsed().as_secs_f64();
            let bytes = id.as_ref().map(|id| {
                stats_json(h.home())["terminal_list"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|t| t["id"] == id.parse::<u64>().unwrap())
                    .map_or(0, |t| t["bytes_read"].as_u64().unwrap_or(0))
            });
            worst_key = worst_key.max(max_ms(&keys));
            worst_ls = worst_ls.max(max_ms(&ls));
            eprintln!(
                "stress stall {prefix}{variant}: key p50 {:.1} ms max {:.0} ms ({} lost), ls p50 {:.1} ms max {:.0} ms",
                p50_ms(&keys),
                max_ms(&keys),
                lost,
                p50_ms(&ls),
                max_ms(&ls)
            );
            report.insert(
                format!("{prefix}{variant}"),
                json!({
                    "key_echo_ms": summary_ms(&keys),
                    "first_output_ms": summary_ms(&first),
                    "keys_lost": lost,
                    "ls_ms": summary_ms(&ls),
                    "slow_terminal_bytes_read": bytes,
                    "daemon_cpu_cores": cpu,
                }),
            );
            if let Some(id) = id {
                let _ = h.home().run_deck(&["close", &id]);
                std::thread::sleep(Duration::from_secs(1));
            }
        }
    }
    report.insert("load_average_start".into(), json!(load_start));
    report.insert("load_average_end".into(), json!(load_average()));
    record("stall", &Value::Object(report), "see the file");
    assert!(worst_key < STALL_MAX_MS, "a key took {worst_key:.0} ms");
    assert!(worst_ls < STALL_MAX_MS, "deck ls took {worst_ls:.0} ms");
}

// ---------------------------------------------------------------------------------------
// throughput
// ---------------------------------------------------------------------------------------

const THROUGHPUT_BYTES: usize = 256 * 1024 * 1024;

/// A file of lines of 100 columns, `styled` with an SGR colour per word; it is reused
/// across runs.
fn throughput_file(styled: bool) -> PathBuf {
    let dir = std::env::temp_dir().join("deck-stress");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(if styled {
        "styled-256m.txt"
    } else {
        "plain-256m.txt"
    });
    if fs::metadata(&path).is_ok_and(|m| m.len() as usize >= THROUGHPUT_BYTES) {
        return path;
    }
    let mut chunk = String::new();
    let mut n = 0;
    while chunk.len() < 1024 * 1024 {
        n += 1;
        if styled {
            // About 100 visible columns: 16 words of 5 letters and a space.
            for w in 0..16 {
                let _ = write!(
                    chunk,
                    "\x1b[3{}mw{:04}\x1b[0m ",
                    (n + w) % 7 + 1,
                    (n * 7 + w) % 10_000
                );
            }
            chunk.push('\n');
        } else {
            let _ = writeln!(chunk, "{:099}", n);
        }
    }
    let mut out = Vec::with_capacity(THROUGHPUT_BYTES + chunk.len());
    while out.len() < THROUGHPUT_BYTES {
        out.extend_from_slice(chunk.as_bytes());
    }
    fs::write(&path, out).unwrap();
    path
}

/// `cat file` in a bare PTY whose reader discards; the file's size in MiB over the time.
fn bare_pty_mibs(file: &std::path::Path) -> f64 {
    let pty = native_pty_system()
        .openpty(PtySize {
            rows: 60,
            cols: 200,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("open a PTY");
    let mut cmd = CommandBuilder::new("cat");
    cmd.arg(file);
    let mut child = pty.slave.spawn_command(cmd).expect("spawn cat");
    drop(pty.slave);
    let mut reader = pty.master.try_clone_reader().unwrap();
    let started = Instant::now();
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0usize;
    while let Ok(n) = reader.read(&mut buf) {
        if n == 0 {
            break;
        }
        total += n;
    }
    let secs = started.elapsed().as_secs_f64();
    let _ = child.wait();
    mib(total as u64) / secs
}

/// Runs `cat file` as a terminal of `home` and returns (MiB/s overall, the peak MiB/s
/// between two samples 250 ms apart). The terminal closes when `cat` ends.
fn deck_mibs(home: &Home, panel: &str, focus: bool, file: &std::path::Path) -> (f64, f64) {
    let started = Instant::now();
    let mut args = vec!["new-terminal", "--panel", panel];
    if focus {
        args.push("--focus");
    }
    let file = file.display().to_string();
    args.extend(["--", "cat", &file]);
    let out = home.run_deck(&args);
    assert!(out.status.success(), "{}", stderr(&out));
    let id: u64 = stdout(&out).trim().parse().unwrap();
    let mut peak = 0.0f64;
    let mut last = (started, 0u64);
    let finished = loop {
        std::thread::sleep(Duration::from_millis(250));
        let j = stats_json(home);
        let Some(t) = j["terminal_list"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"] == id)
        else {
            break started.elapsed();
        };
        let bytes = t["bytes_read"].as_u64().unwrap();
        let now = Instant::now();
        peak = peak.max(mib(bytes - last.1) / now.duration_since(last.0).as_secs_f64());
        last = (now, bytes);
        assert!(
            started.elapsed() < Duration::from_secs(300),
            "cat did not finish"
        );
    };
    (mib(THROUGHPUT_BYTES as u64) / finished.as_secs_f64(), peak)
}

#[test]
#[ignore = "stress: run with `just stress`"]
fn throughput() {
    let load_start = load_average();
    let plain = throughput_file(false);
    let styled = throughput_file(true);
    let baseline_plain = bare_pty_mibs(&plain);
    let baseline_styled = bare_pty_mibs(&styled);

    // A client attached with the terminal in its active panel.
    let h = Harness::builder((200, 60)).config_unset("sidebar").deck();
    h.wait_for_text("$", Duration::from_secs(10));
    let panel = first_panel(h.home());
    let (attached, attached_peak) = deck_mibs(h.home(), &panel, true, &plain);
    let (attached_styled, attached_styled_peak) = deck_mibs(h.home(), &panel, true, &styled);
    drop(h);

    // No client at all.
    let home = probed(&[], &[]);
    let panel = first_panel(&home);
    let (bare, bare_peak) = deck_mibs(&home, &panel, false, &plain);
    let (bare_styled, _) = deck_mibs(&home, &panel, false, &styled);
    let load = load_average();

    record(
        "throughput",
        &json!({
            "file_mib": mib(THROUGHPUT_BYTES as u64),
            "load_average_start": load_start,
            "load_average_end": load,
            "baseline_bare_pty_mib_s": { "plain": baseline_plain, "styled": baseline_styled },
            "client_attached_active_mib_s": { "plain": attached, "plain_peak": attached_peak, "styled": attached_styled, "styled_peak": attached_styled_peak },
            "no_client_mib_s": { "plain": bare, "plain_peak": bare_peak, "styled": bare_styled },
            "ratio_to_baseline": {
                "attached_plain": attached / baseline_plain,
                "attached_styled": attached_styled / baseline_styled,
                "no_client_plain": bare / baseline_plain,
                "no_client_styled": bare_styled / baseline_styled,
            },
        }),
        &format!(
            "bare PTY {baseline_plain:.0}/{baseline_styled:.0} MiB/s (plain/styled); deck attached {attached:.0}/{attached_styled:.0}, no client {bare:.0}/{bare_styled:.0}"
        ),
    );
    // Bound: a fifth of the slowest measured rate (47 MiB/s, after the reader batches its reads).
    for (name, v) in [
        ("attached", attached),
        ("attached styled", attached_styled),
        ("no client", bare),
        ("no client styled", bare_styled),
    ] {
        assert!(v > 10.0, "{name}: {v:.1} MiB/s");
    }
}

// ---------------------------------------------------------------------------------------
// idle
// ---------------------------------------------------------------------------------------

/// A window of an idle system: what the daemon and the client cost over `secs`.
fn idle_window(h: &Harness, secs: u64) -> Value {
    let daemon_pid = h.home().daemon_pid().unwrap();
    let client_pid = i32::try_from(h.pid()).unwrap();
    let (d0, c0) = (
        sys::usage(daemon_pid).unwrap(),
        sys::usage(client_pid).unwrap(),
    );
    let stats0 = stats_json(h.home());
    let out0 = h.output_bytes();
    let started = Instant::now();
    std::thread::sleep(Duration::from_secs(secs));
    let elapsed = started.elapsed().as_secs_f64();
    let (d1, c1) = (
        sys::usage(daemon_pid).unwrap(),
        sys::usage(client_pid).unwrap(),
    );
    let stats1 = stats_json(h.home());
    let out1 = h.output_bytes();
    let (d, c) = (d1 - d0, c1 - c0);
    json!({
        "secs": elapsed,
        "daemon": {
            "cpu_secs": d.cpu_secs,
            "cpu_percent_of_a_core": d.cpu_secs / elapsed * 100.0,
            "wakeups_per_sec": d.wakeups as f64 / elapsed,
            "cpu_secs_by_deck_stats": cpu_secs(&stats1) - cpu_secs(&stats0),
            "threads": stats1["threads"],
        },
        "client": {
            "cpu_secs": c.cpu_secs,
            "cpu_percent_of_a_core": c.cpu_secs / elapsed * 100.0,
            "wakeups_per_sec": c.wakeups as f64 / elapsed,
            "output_bytes": out1 - out0,
            "output_bytes_per_sec": (out1 - out0) as f64 / elapsed,
        },
    })
}

/// Whether the recipe's filter is exactly `name` (an unfiltered `just stress` skips the
/// 10-minute scenarios).
fn asked_for(name: &str) -> bool {
    std::env::var("DECK_STRESS_FILTER").is_ok_and(|f| f.split_whitespace().any(|w| w == name))
}

#[test]
#[ignore = "stress: 10 minutes; run with `just stress idle`"]
fn idle() {
    if !asked_for("idle") {
        eprintln!("stress idle: skipped; run `just stress idle`");
        return;
    }
    let secs: u64 = std::env::var("DECK_IDLE_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(600);
    let h = Harness::builder((200, 60)).config_unset("sidebar").deck();
    h.wait_for_text("$", Duration::from_secs(10));
    for _ in 0..19 {
        new_panel(h.home(), &["sh"]);
    }
    wait_for("20 terminals", Duration::from_secs(30), || {
        stats_json(h.home())["terminals"] == 20
    });
    // Let start-up settle: the colour answers window (3 s) and the first draws.
    std::thread::sleep(Duration::from_secs(10));
    let load_start = load_average();
    let window = idle_window(&h, secs);
    let load_end = load_average();
    let cpu = window["daemon"]["cpu_percent_of_a_core"].as_f64().unwrap();
    record(
        "idle",
        &json!({
            "terminals": 20, "load_average_start": load_start, "load_average_end": load_end,
            "window": window,
        }),
        &format!(
            "{secs} s: daemon {cpu:.3}% of a core, {:.0} wake-ups/s; client {:.3}% of a core, {:.0} wake-ups/s, {} bytes written",
            window["daemon"]["wakeups_per_sec"].as_f64().unwrap(),
            window["client"]["cpu_percent_of_a_core"].as_f64().unwrap(),
            window["client"]["wakeups_per_sec"].as_f64().unwrap(),
            window["client"]["output_bytes"],
        ),
    );
    // Bounds: five times what was measured (0.38% of a core and 26 wake-ups a second for 20
    // terminals, against 1.59% and 475 before the timers went).
    assert!(cpu < 2.0, "the idle daemon used {cpu:.2}% of a core");
    let wakeups = window["daemon"]["wakeups_per_sec"].as_f64().unwrap();
    // Wake-ups are measured on macOS only (`ri_interrupt_wkups`); on Linux the count is a
    // context-switch proxy that nobody has measured for this bound, so it is not asserted.
    #[cfg(target_os = "macos")]
    assert!(
        wakeups < 130.0,
        "the idle daemon woke {wakeups:.0} times a second"
    );
    #[cfg(not(target_os = "macos"))]
    let _ = wakeups;
}

// ---------------------------------------------------------------------------------------
// idle with a working agent
// ---------------------------------------------------------------------------------------

/// A fake `claude` on `PATH` that stays running (see `agents.rs`).
fn fake_claude() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::Builder::new()
        .prefix("ag")
        .tempdir_in("/tmp")
        .unwrap();
    let script = dir.path().join("claude");
    fs::write(&script, "#!/bin/sh\necho agent-ready\nread line\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    dir
}

/// Tells the daemon that the agent of `terminal` works, as its hook does.
fn agent_working(home: &Home, terminal: u64) {
    use deck_core::agents::{AgentKind, AgentStatus};
    use deck_core::model::TerminalId;
    use deck_core::protocol::{
        ClientMsg, FrameReader, PROTOCOL_VERSION, Request, Role, ServerMsg, encode,
    };
    use std::os::unix::net::UnixStream;
    let mut stream = UnixStream::connect(home.socket()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let hello = ClientMsg::Hello {
        version: PROTOCOL_VERSION,
        role: Role::Control,
        cwd: std::env::temp_dir(),
        env: Vec::new(),
        colors: None,
    };
    let request = ClientMsg::Request(Request::AgentEvent {
        terminal: TerminalId(terminal),
        kind: AgentKind::Claude,
        status: Some(AgentStatus::Working),
        session_id: Some("s1".into()),
        prompt: Some("refactor the layout engine".into()),
        raw: None,
    });
    stream.write_all(&encode(&hello).unwrap()).unwrap();
    stream.write_all(&encode(&request).unwrap()).unwrap();
    let mut reader = FrameReader::new();
    let mut buf = [0u8; 16 * 1024];
    loop {
        while let Some(msg) = reader.next_message::<ServerMsg>().unwrap() {
            if matches!(msg, ServerMsg::Response(_)) {
                return;
            }
        }
        match stream.read(&mut buf).unwrap() {
            0 => panic!("the daemon closed the connection"),
            n => reader.push(&buf[..n]),
        }
    }
}

const SPINNER_SECS: u64 = 60;

#[test]
#[ignore = "stress: run with `just stress`"]
fn idle_spinner() {
    let bin = fake_claude();
    let path = format!(
        "{}:{}",
        bin.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut report = serde_json::Map::new();
    let mut worst = 0.0f64;
    let mut idle_wakeups = 0.0f64;
    for tier in ["text", "image"] {
        let mut builder = Harness::builder((200, 60))
            .config_unset("sidebar")
            .env("PATH", &path);
        if tier == "image" {
            builder = builder.kitty().xtversion("ghostty 1.2.0");
        }
        let mut h = builder.deck();
        h.wait_for_text("$", Duration::from_secs(10));
        h.send_keys("claude\r");
        h.wait_for_text("agent-ready", Duration::from_secs(10));
        let terminal = stats_json(h.home())["terminal_list"][0]["id"]
            .as_u64()
            .unwrap();
        std::thread::sleep(Duration::from_secs(5));
        // Without the agent working, then with it: the same client, so the figures compare.
        let without = idle_window(&h, 20);
        idle_wakeups = idle_wakeups.max(without["client"]["wakeups_per_sec"].as_f64().unwrap());
        agent_working(h.home(), terminal);
        std::thread::sleep(Duration::from_secs(2));
        let load_start = load_average();
        let working = idle_window(&h, SPINNER_SECS);
        let cpu = working["client"]["cpu_percent_of_a_core"].as_f64().unwrap();
        worst = worst.max(cpu);
        eprintln!(
            "stress idle_spinner {tier}: client {:.2}% of a core, {:.0} bytes/s with the agent working; {:.2}%, {:.0} bytes/s without",
            cpu,
            working["client"]["output_bytes_per_sec"].as_f64().unwrap(),
            without["client"]["cpu_percent_of_a_core"].as_f64().unwrap(),
            without["client"]["output_bytes_per_sec"].as_f64().unwrap(),
        );
        report.insert(
            tier.into(),
            json!({ "agent_not_working": without, "agent_working": working, "load_average_start": load_start, "load_average_end": load_average() }),
        );
        drop(h);
    }
    record("idle_spinner", &Value::Object(report), "see the file");
    // Bounds: five times what was measured at 10 frames a second, before the rate became 8 (1.2% of a core working, 3 wake-ups a second idle).
    assert!(worst < 6.0, "the client used {worst:.1}% of a core");
    // macOS only; Linux counts context switches, which this bound has not been measured for.
    #[cfg(target_os = "macos")]
    assert!(
        idle_wakeups < 15.0,
        "an idle client woke {idle_wakeups:.0} times a second"
    );
    #[cfg(not(target_os = "macos"))]
    let _ = idle_wakeups;
}

// ---------------------------------------------------------------------------------------
// memory
// ---------------------------------------------------------------------------------------

fn rss_mib(home: &Home) -> f64 {
    mib(stats_json(home)["rss_bytes"].as_u64().unwrap())
}

/// Opens `count` panels (each focused, so the client sizes it) that print `lines` lines
/// of `cols` columns; returns the memory each added and the size the terminals had.
fn history_cost(cols: u16, lines: u64, count: usize) -> Value {
    let h = Harness::builder((cols + 2, 40)).deck();
    h.wait_for_text("$", Duration::from_secs(10));
    let home = h.home();
    std::thread::sleep(Duration::from_secs(2));
    let before = rss_mib(home);
    let line = "m".repeat(usize::from(cols) - 4);
    let program = format!("yes {line} | head -n {lines}; sleep 3600");
    for _ in 0..count {
        let out = home.run_deck(&["new-panel", "--focus", "--", "sh", "-c", &program]);
        assert!(out.status.success(), "{}", stderr(&out));
        std::thread::sleep(Duration::from_millis(300));
        let want = (line.len() as u64 + 1) * lines;
        wait_for("the lines to print", Duration::from_secs(300), || {
            let j = stats_json(home);
            let list = j["terminal_list"].as_array().unwrap();
            let done = list
                .iter()
                .filter(|t| t["bytes_read"].as_u64().unwrap() >= want)
                .count();
            done > 0 && done == list.len() - 1
        });
    }
    std::thread::sleep(Duration::from_secs(3));
    let after = rss_mib(home);
    let j = stats_json(home);
    let sizes: Vec<Value> = j["terminal_list"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| json!([t["cols"], t["rows"]]))
        .collect();
    let history: Vec<Value> = j["terminal_list"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["history"].clone())
        .collect();
    json!({
        "columns": cols, "lines": lines, "terminals": count,
        "mib_per_terminal": (after - before) / count as f64,
        "rss_before_mib": before, "rss_after_mib": after,
        "terminal_sizes": sizes, "history_lines": history,
    })
}

#[test]
#[ignore = "stress: run with `just stress`"]
fn memory() {
    let load_start = load_average();
    // 20 idle terminals on a daemon with no client.
    let home = probed(&[], &[]);
    std::thread::sleep(Duration::from_secs(2));
    let before = stats_json(&home);
    for _ in 0..20 {
        new_panel(&home, &["sh"]);
    }
    wait_for("21 terminals", Duration::from_secs(30), || {
        stats_json(&home)["terminals"] == 21
    });
    std::thread::sleep(Duration::from_secs(3));
    let after = stats_json(&home);
    let idle_mib =
        mib(after["rss_bytes"].as_u64().unwrap() - before["rss_bytes"].as_u64().unwrap()) / 20.0;
    let idle_threads =
        (after["threads"].as_u64().unwrap() - before["threads"].as_u64().unwrap()) as f64 / 20.0;
    drop(home);

    // Per attached client: five, each in its own PTY.
    let home = probed(&[], &[]);
    std::thread::sleep(Duration::from_secs(2));
    let base = rss_mib(&home);
    let clients: Vec<Client> = (0..5)
        .map(|i| {
            let c = Client::attach(&home);
            wait_clients(&home, i + 1);
            c
        })
        .collect();
    std::thread::sleep(Duration::from_secs(3));
    let client_mib = (rss_mib(&home) - base) / 5.0;
    drop(clients);
    drop(home);

    let h80 = history_cost(80, 10_000, 5);
    let h250 = history_cost(250, 10_000, 5);
    let f80 = history_cost(80, 100_000, 3);
    let f250 = history_cost(250, 100_000, 3);
    record(
        "memory",
        &json!({
            "load_average_start": load_start,
            "load_average_end": load_average(),
            "mib_per_idle_terminal": idle_mib,
            "threads_per_idle_terminal": idle_threads,
            "mib_per_attached_client": client_mib,
            "history_10000_lines_80_columns": h80,
            "history_10000_lines_250_columns": h250,
            "history_100000_lines_80_columns": f80,
            "history_100000_lines_250_columns": f250,
        }),
        &format!(
            "idle terminal {idle_mib:.2} MiB, client {client_mib:.2} MiB; 10k lines {:.1} MiB at 80 / {:.1} at 250 columns; 100k lines {:.1} / {:.1}",
            h80["mib_per_terminal"].as_f64().unwrap(),
            h250["mib_per_terminal"].as_f64().unwrap(),
            f80["mib_per_terminal"].as_f64().unwrap(),
            f250["mib_per_terminal"].as_f64().unwrap(),
        ),
    );
    // Bounds: twice what a daemon of this design should need.
    assert!(idle_mib < 10.0, "{idle_mib:.1} MiB per idle terminal");
    assert!(client_mib < 20.0, "{client_mib:.1} MiB per client");
    for v in [&f80, &f250] {
        assert!(v["mib_per_terminal"].as_f64().unwrap() < 100.0);
    }
}

// ---------------------------------------------------------------------------------------
// Second round of the performance review (sub-spec 05)
// ---------------------------------------------------------------------------------------

/// `deck stats --json`, or `None` while no daemon answers.
fn stats_try(home: &Home) -> Option<Value> {
    let out = home.run_deck(&["stats", "--json"]);
    if !out.status.success() {
        return None;
    }
    serde_json::from_str(&stdout(&out)).ok()
}

/// Stops the daemon (keeping the saved state unless `forget`) and waits for its lock to go.
fn stop_daemon(home: &Home, forget: bool) {
    let args: &[&str] = if forget {
        &["kill-server", "--forget"]
    } else {
        &["kill-server"]
    };
    let _ = home.run_deck(args);
    wait_for("the daemon to stop", Duration::from_secs(30), || {
        !home.daemon_running()
    });
}

fn median_ms(secs: &[f64]) -> f64 {
    p50_ms(secs)
}

// ---- startup --------------------------------------------------------------------------

const STARTUP_RUNS: usize = 20;
/// Generous: a start-up over 5 s (warm) is broken.
const STARTUP_MAX_WARM_MS: f64 = 150.0;

struct Started {
    first_frame: f64,
    prompt: f64,
    live: Option<f64>,
}

/// One start of the client on `home` (the daemon may or may not run): seconds from the spawn to
/// the alternate screen and to the prompt on the screen, and, with `live`, from just before
/// the spawn until `deck stats` counts that many terminals.
fn start_client(home: &Home, mute: bool, live: Option<u64>) -> Started {
    let builder = Harness::builder((200, 60));
    let builder = if mute { builder.mute() } else { builder };
    let t0 = Instant::now();
    let h = builder.deck_in(home.alias());
    wait_for("the prompt", Duration::from_secs(60), || {
        h.screen_entries().first().is_some_and(|e| e.1.is_some())
    });
    let (alt, prompt) = h.screen_entries()[0];
    let spawned = h.spawned_at();
    let live = live.map(|n| {
        wait_for("the terminals", Duration::from_secs(60), || {
            stats_try(home).is_some_and(|j| j["terminals"] == n)
        });
        t0.elapsed().as_secs_f64()
    });
    drop(h);
    Started {
        first_frame: alt.duration_since(spawned).as_secs_f64(),
        prompt: prompt.unwrap().duration_since(spawned).as_secs_f64(),
        live,
    }
}

/// `STARTUP_RUNS` starts; `cold` stops the daemon before each (`Some(true)` also forgets the
/// state).
fn startup_case(home: &Home, mute: bool, cold: Option<bool>, live: Option<u64>) -> Value {
    // One start nobody times: the first pays for page cache and lazy initialisation.
    if let Some(forget) = cold {
        stop_daemon(home, forget);
    }
    start_client(home, mute, live);
    let (mut first, mut prompt, mut lives) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..STARTUP_RUNS {
        if let Some(forget) = cold {
            stop_daemon(home, forget);
        }
        let s = start_client(home, mute, live);
        first.push(s.first_frame);
        prompt.push(s.prompt);
        lives.extend(s.live);
        if cold.is_none() {
            wait_clients(home, 0);
        }
    }
    json!({
        "runs": STARTUP_RUNS,
        "spawn_to_alternate_screen_ms": summary_ms(&first),
        "spawn_to_prompt_ms": summary_ms(&prompt),
        "until_all_terminals_live_ms": if lives.is_empty() { Value::Null } else { summary_ms(&lives) },
    })
}

#[test]
#[ignore = "stress: run with `just stress`"]
fn startup() {
    let load_start = load_average();
    // The product's sidebar is on.
    let config = [("sidebar", "true")];
    let mut report = serde_json::Map::new();

    let home = probed(&config, &[]);
    report.insert(
        "warm_1_terminal".into(),
        startup_case(&home, false, None, None),
    );
    report.insert(
        "warm_1_terminal_outer_terminal_answers_nothing".into(),
        startup_case(&home, true, None, None),
    );
    report.insert(
        "cold_no_saved_state".into(),
        startup_case(&home, false, Some(true), None),
    );
    stop_daemon(&home, true);
    drop(home);

    let home = probed(&config, &[]);
    for _ in 0..29 {
        new_panel(&home, &["sh"]);
    }
    wait_for("30 terminals", Duration::from_secs(30), || {
        stats_json(&home)["terminals"] == 30
    });
    report.insert(
        "warm_30_terminals".into(),
        startup_case(&home, false, None, None),
    );
    // 200 × `deck ls` against 30 terminals, no client.
    let mut ls = Vec::new();
    for _ in 0..200 {
        let t = Instant::now();
        let out = home.run_deck(&["ls"]);
        assert!(out.status.success(), "{}", stderr(&out));
        ls.push(t.elapsed().as_secs_f64());
    }
    report.insert("deck_ls_x200_with_30_terminals".into(), summary_ms(&ls));
    // The saved layout comes back: 30 terminals start again.
    std::thread::sleep(Duration::from_secs(1));
    report.insert(
        "cold_30_restored_terminals".into(),
        startup_case(&home, false, Some(false), Some(30)),
    );
    report.insert("load_average_start".into(), json!(load_start));
    report.insert("load_average_end".into(), json!(load_average()));
    let warm = report["warm_1_terminal"]["spawn_to_prompt_ms"]["p50"]
        .as_f64()
        .unwrap();
    record(
        "startup",
        &Value::Object(report.clone()),
        &format!(
            "warm to prompt p50 {warm:.0} ms; 30 restored terminals live p50 {:.0} ms; deck ls p50 {:.1} ms",
            report["cold_30_restored_terminals"]["until_all_terminals_live_ms"]["p50"]
                .as_f64()
                .unwrap_or(f64::NAN),
            median_ms(&ls)
        ),
    );
    assert!(warm < STARTUP_MAX_WARM_MS, "{warm:.0} ms");
}

// ---- live upgrade ---------------------------------------------------------------------

#[test]
#[ignore = "stress: run with `just stress`"]
fn upgrade() {
    let load_start = load_average();
    let mut h = Harness::builder((200, 60)).deck();
    h.wait_for_text("$", Duration::from_secs(10));
    let line = "m".repeat(99);
    let program = format!("yes {line} | head -n 10000; echo full; exec sleep 3600");
    h.send_keys(&format!("yes {line} | head -n 10000; echo full\r"));
    let home = h.home();
    let first = first_panel(home);
    for _ in 0..29 {
        let out = home.run_deck(&["new-panel", "--focus", "--", "sh", "-c", &program]);
        assert!(out.status.success(), "{}", stderr(&out));
        std::thread::sleep(Duration::from_millis(200));
    }
    let want = 100 * 10_000;
    wait_for("the lines to print", Duration::from_secs(300), || {
        let j = stats_json(home);
        let list = j["terminal_list"].as_array().unwrap();
        list.len() == 30
            && list
                .iter()
                .all(|t| t["bytes_read"].as_u64().unwrap() >= want)
    });
    assert!(home.run_deck(&["focus", &first]).status.success());
    std::thread::sleep(Duration::from_secs(3));
    let before = stats_json(home);
    let daemon_pid = home.daemon_pid().unwrap();
    let marker_path = home.path().join("run").join("deck.handoff");
    let entries = h.screen_entries().len();
    let output_before = h.output_bytes();

    let t0 = Instant::now();
    let mut up = home
        .deck_command(&["upgrade"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let mut ls: Option<(std::process::Child, Instant)> = None;
    let (mut started, mut ls_done, mut adopted, mut up_done) = (None, None, None, None);
    let mut peak = 0u64;
    while up_done.is_none() || ls_done.is_none() || adopted.is_none() {
        let now = Instant::now();
        assert!(
            now.duration_since(t0) < Duration::from_secs(120),
            "the upgrade took over 120 s"
        );
        if let Some(m) = sys::usage(daemon_pid) {
            peak = peak.max(m.memory);
        }
        if started.is_none() && Marker::read(&marker_path).is_some_and(|m| m.upgrading) {
            started = Some(now);
            let child = home
                .deck_command(&["ls"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            ls = Some((child, now));
        }
        if let Some((child, since)) = ls.as_mut()
            && ls_done.is_none()
            && child.try_wait().unwrap().is_some()
        {
            ls_done = Some(now.duration_since(*since));
        }
        if up_done.is_none() && up.try_wait().unwrap().is_some() {
            up_done = Some(now.duration_since(t0));
        }
        if adopted.is_none()
            && fs::read_to_string(home.log_file()).is_ok_and(|l| l.contains("adopted "))
        {
            adopted = Some(now);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let _ = up.wait();
    if let Some((child, _)) = ls.as_mut() {
        let _ = child.wait();
    }
    let adopted_at = adopted.unwrap().duration_since(t0);
    wait_for(
        "the client to attach again",
        Duration::from_secs(60),
        || {
            let e = h.screen_entries();
            e.len() > entries && e[e.len() - 1].1.is_some()
        },
    );
    let e = h.screen_entries();
    let (alt, prompt) = e[e.len() - 1];
    std::thread::sleep(Duration::from_secs(3));
    let after = stats_json(home);
    let log = fs::read_to_string(home.log_file()).unwrap_or_default();
    let lines: Vec<&str> = log
        .lines()
        .filter(|l| l.contains("adopted ") || l.contains("upgrad"))
        .collect();
    let report = json!({
        "terminals": 30, "lines_each": 10_000, "columns": 100,
        "load_average_start": load_start, "load_average_end": load_average(),
        "deck_upgrade_wall_ms": up_done.unwrap().as_secs_f64() * 1e3,
        "marker_says_upgrading_after_ms": started.unwrap().duration_since(t0).as_secs_f64() * 1e3,
        "adopted_line_in_log_after_ms": adopted_at.as_secs_f64() * 1e3,
        "handoff_ms_marker_to_adopted_line": adopted.unwrap().duration_since(started.unwrap()).as_secs_f64() * 1e3,
        "deck_ls_started_during_the_handoff_answered_after_ms": ls_done.unwrap().as_secs_f64() * 1e3,
        "client_alternate_screen_again_after_ms": alt.duration_since(t0).as_secs_f64() * 1e3,
        "client_prompt_on_screen_again_after_ms": prompt.unwrap().duration_since(t0).as_secs_f64() * 1e3,
        "client_bytes_written_during_and_after": h.output_bytes() - output_before,
        "daemon_rss_mib_before": mib(before["rss_bytes"].as_u64().unwrap()),
        "daemon_rss_mib_after": mib(after["rss_bytes"].as_u64().unwrap()),
        "daemon_footprint_peak_mib_sampled_every_2ms": mib(peak),
        "terminals_after": after["terminals"],
        "log_lines": lines,
    });
    record(
        "upgrade",
        &report,
        &format!(
            "deck upgrade {:.0} ms, adopted after {:.0} ms, a deck ls during it answered after {:.0} ms, client back after {:.0} ms; rss {:.0} -> {:.0} MiB, peak {:.0} MiB",
            up_done.unwrap().as_secs_f64() * 1e3,
            adopted_at.as_secs_f64() * 1e3,
            ls_done.unwrap().as_secs_f64() * 1e3,
            alt.duration_since(t0).as_secs_f64() * 1e3,
            mib(before["rss_bytes"].as_u64().unwrap()),
            mib(after["rss_bytes"].as_u64().unwrap()),
            mib(peak)
        ),
    );
    assert_eq!(after["terminals"], 30);
    // Measured 0.42 s; five times that.
    assert!(up_done.unwrap() < Duration::from_millis(2100));
}

// ---- control connections --------------------------------------------------------------

const HELLO_RUNS: usize = 500;

/// `deck <args>` with nothing in its environment but `DECK_HOME`, a terminal id when given,
/// and `pad_kib` KiB of padding in variables of 2 KiB.
fn control_command(
    home: &Home,
    args: &[&str],
    pad_kib: usize,
    terminal: Option<&str>,
) -> std::process::Command {
    let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_deck"));
    c.args(args)
        .env_clear()
        .env("DECK_HOME", home.path())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if let Some(terminal) = terminal {
        c.env("DECK_TERMINAL_ID", terminal);
    }
    let mut left = pad_kib * 1024;
    let mut i = 0;
    while left > 0 {
        let n = left.min(2048);
        c.env(format!("PAD{i:04}"), "x".repeat(n.saturating_sub(8)));
        left -= n;
        i += 1;
    }
    c
}

/// `kill-server` while a handoff is held for 1.5 s: seconds until it returns, and the log
/// lines after the handoff.
fn kill_during_handoff(pad_kib: usize) -> Value {
    let home = Home::with(
        &[],
        &[
            ("DECK_TEST_PREFREEZE_DELAY_MS", Some("0")),
            ("DECK_TEST_HANDOFF_DELAY_MS", Some("1500")),
        ],
    );
    assert!(home.run_deck(&["new", "main", "-d"]).status.success());
    let marker_path = home.path().join("run").join("deck.handoff");
    let mut up = home
        .deck_command(&["upgrade"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    wait_for("the handoff to start", Duration::from_secs(20), || {
        Marker::read(&marker_path).is_some_and(|m| m.upgrading)
    });
    let t = Instant::now();
    let status = control_command(&home, &["kill-server"], pad_kib, None)
        .status()
        .unwrap();
    let took = t.elapsed().as_secs_f64();
    let _ = up.wait();
    wait_for("the daemon to stop", Duration::from_secs(30), || {
        !home.daemon_running()
    });
    let log = fs::read_to_string(home.log_file()).unwrap_or_default();
    let lines: Vec<&str> = log
        .lines()
        .filter(|l| l.contains("shutting down") || l.contains("adopted "))
        .collect();
    json!({
        "environment_padding_kib": pad_kib, "kill_server_returned_after_ms": took * 1e3,
        "exit_ok": status.success(), "log_lines": lines,
    })
}

#[test]
#[ignore = "stress: run with `just stress`"]
fn hello() {
    let load_start = load_average();
    let home = probed(&[], &[]);
    let terminal = stats_json(&home)["terminal_list"][0]["id"]
        .as_u64()
        .unwrap()
        .to_string();
    let mut report = serde_json::Map::new();
    for (name, args, ids) in [
        ("deck_ls", vec!["ls"], None),
        (
            "deck_agent_event_claude_Stop",
            vec!["agent-event", "--agent", "claude", "--event", "Stop"],
            Some(terminal.as_str()),
        ),
        // Without a terminal id the hook returns before it connects: the cost of the process.
        (
            "deck_agent_event_outside_deck",
            vec!["agent-event", "--agent", "claude", "--event", "Stop"],
            None,
        ),
    ] {
        let mut by_pad = serde_json::Map::new();
        for pad in [0usize, 8, 32, 128] {
            // Warm up, then time.
            for _ in 0..10 {
                assert!(
                    control_command(&home, &args, pad, ids)
                        .status()
                        .unwrap()
                        .success()
                );
            }
            let mut times = Vec::new();
            for _ in 0..HELLO_RUNS {
                let t = Instant::now();
                assert!(
                    control_command(&home, &args, pad, ids)
                        .status()
                        .unwrap()
                        .success()
                );
                times.push(t.elapsed().as_secs_f64());
            }
            eprintln!(
                "stress hello {name} +{pad} KiB: p50 {:.2} ms max {:.1} ms",
                median_ms(&times),
                max_ms(&times)
            );
            let mut s = summary_ms(&times);
            s.as_object_mut().unwrap().remove("samples");
            by_pad.insert(format!("environment_padded_{pad}_kib"), s);
        }
        report.insert(name.into(), Value::Object(by_pad));
    }
    drop(home);
    report.insert(
        "kill_server_during_a_1500_ms_handoff_minimal_environment".into(),
        kill_during_handoff(0),
    );
    report.insert(
        "kill_server_during_a_1500_ms_handoff_32_kib_environment".into(),
        kill_during_handoff(32),
    );
    report.insert("load_average_start".into(), json!(load_start));
    report.insert("load_average_end".into(), json!(load_average()));
    record("hello", &Value::Object(report.clone()), "see the file");
    for key in [
        "kill_server_during_a_1500_ms_handoff_minimal_environment",
        "kill_server_during_a_1500_ms_handoff_32_kib_environment",
    ] {
        assert_eq!(report[key]["exit_ok"], true, "{key}");
    }
}

// ---- paste ----------------------------------------------------------------------------

/// `bytes` of text in lines of 100 bytes, each `L<number>` then filler.
fn paste_text(bytes: usize) -> (Vec<u8>, usize) {
    let lines = bytes / 100;
    let mut out = Vec::with_capacity(lines * 100);
    for n in 0..lines {
        let mut line = format!("L{n:06} ");
        while line.len() < 99 {
            line.push('p');
        }
        line.push('\n');
        out.extend_from_slice(line.as_bytes());
    }
    (out, lines)
}

/// Waits until `path` holds `bytes` bytes (the file a `cat` writes), without looking at the
/// terminal: the tty may drop echo when its output queue is full. Returns the instant it did,
/// or the size it had reached at the limit.
fn wait_size(path: &std::path::Path, bytes: usize, limit: Duration) -> Result<Instant, u64> {
    let deadline = Instant::now() + limit;
    loop {
        let size = std::fs::metadata(path).map_or(0, |m| m.len());
        if size >= bytes as u64 {
            return Ok(Instant::now());
        }
        if Instant::now() >= deadline {
            return Err(size);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

const PASTE_LIMIT: Duration = Duration::from_secs(120);

#[test]
#[ignore = "stress: run with `just stress`"]
fn paste() {
    let load_start = load_average();
    let mut h = Harness::builder((200, 60)).deck();
    h.wait_for_text("$", Duration::from_secs(10));
    let mut report = serde_json::Map::new();
    let mut worst = 0.0f64;
    let mut incomplete = Vec::new();
    let mut burst = 0.0f64;
    // A real paste (the text wrapped in the bracketed-paste markers, one `Input` for the
    // client) at three sizes, and the same 64 KiB as a burst of typed keys (no markers: the
    // client sees one key per byte).
    for (name, size, bracketed) in [
        ("paste_64_kib", 64 * 1024, true),
        ("paste_256_kib", 256 * 1024, true),
        ("paste_1_mib", 1 << 20, true),
        ("typed_burst_64_kib", 64 * 1024, false),
    ] {
        let (text, lines) = paste_text(size);
        let target = h.home().path().join("pasted.txt");
        let (mut first, mut done) = (Vec::new(), Vec::new());
        for _ in 0..3 {
            let _ = std::fs::remove_file(&target);
            h.send_keys(&format!("clear; cat > '{}'\r", target.display()));
            std::thread::sleep(Duration::from_millis(600));
            // The watch notes when the first line shows (the first echo of the text).
            let first_seen = std::sync::Arc::new(std::sync::Mutex::new(None::<Instant>));
            let noted = std::sync::Arc::clone(&first_seen);
            let watch = h.watch(move |s| {
                if (0..60).any(|r| s.row_text(r).contains("L000000 ")) {
                    noted.lock().unwrap().get_or_insert_with(Instant::now);
                }
                false
            });
            let sent = Instant::now();
            if bracketed {
                let mut body = b"\x1b[200~".to_vec();
                body.extend_from_slice(&text);
                body.extend_from_slice(b"\x1b[201~");
                h.send_bytes(&body);
            } else if !h.send_bytes_within(&text, PASTE_LIMIT) {
                // The client stopped reading its input: nothing after this can be trusted.
                incomplete.push(format!("{name}: the input was not accepted"));
                drop(watch);
                break;
            }
            let finished = wait_size(&target, lines * 100, PASTE_LIMIT);
            drop(watch);
            let echo = *first_seen.lock().unwrap();
            if let Some(echo) = echo {
                first.push(echo.duration_since(sent).as_secs_f64());
            }
            match finished {
                Ok(at) => done.push(at.duration_since(sent).as_secs_f64()),
                Err(reached) => {
                    incomplete.push(format!("{name}: {reached} of {} bytes", lines * 100))
                }
            }
            std::thread::sleep(Duration::from_millis(300));
            h.send_keys("\x04");
            std::thread::sleep(Duration::from_millis(500));
        }
        if done.is_empty() {
            report.insert(
                name.into(),
                json!({ "bytes": lines * 100, "complete": null }),
            );
            continue;
        }
        worst = worst.max(max_ms(&done));
        if name == "typed_burst_64_kib" {
            burst = median_ms(&done);
        }
        eprintln!(
            "stress paste {name}: complete p50 {:.0} ms ({} of 3 runs)",
            median_ms(&done),
            done.len()
        );
        report.insert(
            name.into(),
            json!({
                "bytes": lines * 100, "runs_complete": done.len(),
                "first_echo_ms": if first.is_empty() { Value::Null } else { summary_ms(&first) },
                "complete_ms": summary_ms(&done),
            }),
        );
    }
    // The same paste into `cat` in a PTY with no deck in between: the line discipline's own
    // rate.
    for (name, size) in [
        ("bare_pty_64_kib", 64 * 1024),
        ("bare_pty_256_kib", 256 * 1024),
    ] {
        let (text, lines) = paste_text(size);
        let target = h.home().path().join("bare.txt");
        let _ = std::fs::remove_file(&target);
        let mut bare = Harness::spawn(
            (200, 60),
            &["sh", "-c", &format!("cat > '{}'", target.display())],
        );
        std::thread::sleep(Duration::from_millis(600));
        let sent = Instant::now();
        bare.send_bytes(&text);
        match wait_size(&target, lines * 100, Duration::from_secs(60)) {
            Ok(at) => {
                let took = at.duration_since(sent).as_secs_f64();
                eprintln!("stress paste {name}: complete {:.0} ms", took * 1e3);
                report.insert(
                    name.into(),
                    json!({ "bytes": lines * 100, "complete_ms": took * 1e3 }),
                );
            }
            Err(reached) => incomplete.push(format!("{name}: {reached} bytes")),
        }
    }
    // The same text through `deck send -`: the daemon's path to the PTY without the client.
    for (name, size) in [
        ("deck_send_64_kib", 64 * 1024),
        ("deck_send_256_kib", 256 * 1024),
    ] {
        let (text, lines) = paste_text(size);
        let target = h.home().path().join(format!("{name}.txt"));
        let id = new_terminal(
            h.home(),
            None,
            false,
            &["sh", "-c", &format!("cat > '{}'", target.display())],
        );
        std::thread::sleep(Duration::from_millis(600));
        let sent = Instant::now();
        let mut child = h
            .home()
            .deck_command(&["send", "--terminal", &id, "-"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let body = text.clone();
        let feeder = std::thread::spawn(move || stdin.write_all(&body));
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut returned = None;
        let mut exit_ok = None;
        while Instant::now() < deadline {
            if let Some(status) = child.try_wait().unwrap() {
                returned = Some(sent.elapsed().as_secs_f64() * 1e3);
                exit_ok = Some(status.success());
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let finished = wait_size(&target, lines * 100, Duration::from_secs(30));
        if returned.is_none() {
            let _ = child.kill();
        }
        let _ = child.wait();
        let _ = feeder.join();
        let mut said = String::new();
        if let Some(mut err) = child.stderr.take() {
            let _ = err.read_to_string(&mut said);
        }
        let complete = finished
            .ok()
            .map(|at| at.duration_since(sent).as_secs_f64() * 1e3);
        if complete.is_none() {
            incomplete.push(format!(
                "{name}: file at {} bytes",
                std::fs::metadata(&target).map_or(0, |m| m.len())
            ));
        }
        eprintln!("stress paste {name}: send returned {returned:?} ms, complete {complete:?} ms");
        report.insert(
            name.into(),
            json!({
                "bytes": lines * 100, "send_returned_ms": returned, "exit_ok": exit_ok,
                "stderr": said.trim(), "complete_ms": complete, "limit_s": 30,
            }),
        );
    }
    report.insert("incomplete".into(), json!(incomplete));
    report.insert("load_average_start".into(), json!(load_start));
    report.insert("load_average_end".into(), json!(load_average()));
    record("paste", &Value::Object(report), "see the file");
    assert!(incomplete.is_empty(), "did not complete: {incomplete:?}");
    assert!(worst < 80_000.0, "a paste took {worst:.0} ms");
    // Measured 74 ms; it was 980 ms when every key waited for the emulator.
    assert!(burst < 400.0, "64 KiB of typed keys took {burst:.0} ms");
}

// ---- images ---------------------------------------------------------------------------

const PLACEHOLDER: char = '\u{10EEEE}';

/// A file that a program prints to show an RGBA image of `width` × `height` pixels over
/// `cols` × `rows` cells at the cursor: kitty `a=T,f=32` in chunks of 4096 base64 bytes.
fn kitty_image_file(
    path: &std::path::Path,
    id: u32,
    (width, height): (u32, u32),
    (cols, rows): (u16, u16),
) {
    let mut data = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            data.extend_from_slice(&[x as u8, y as u8, (x ^ y) as u8, 255]);
        }
    }
    let chunks: Vec<&[u8]> = data.chunks(3072).collect();
    let mut out = Vec::new();
    for (n, chunk) in chunks.iter().enumerate() {
        let more = u8::from(n + 1 < chunks.len());
        if n == 0 {
            write!(
                out_string(&mut out),
                "\x1b_Ga=T,f=32,s={width},v={height},i={id},c={cols},r={rows},t=d,q=2,m={more};"
            )
            .unwrap();
        } else {
            write!(out_string(&mut out), "\x1b_Gm={more},q=2;").unwrap();
        }
        out.extend_from_slice(base64(chunk).as_bytes());
        out.extend_from_slice(b"\x1b\\");
    }
    fs::write(path, out).unwrap();
}

/// A `fmt::Write` over a byte buffer.
struct OutString<'a>(&'a mut Vec<u8>);

impl std::fmt::Write for OutString<'_> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.extend_from_slice(s.as_bytes());
        Ok(())
    }
}

fn out_string(out: &mut Vec<u8>) -> OutString<'_> {
    OutString(out)
}

fn image_harness() -> Harness {
    let h = Harness::builder((200, 60))
        .kitty()
        .xtversion("ghostty 1.2.0")
        .deck();
    h.wait_for_text("$", Duration::from_secs(10));
    h
}

fn placeholders_on(h: &Harness) -> usize {
    let (cols, rows) = h.size();
    let mut n = 0;
    for row in 0..usize::from(rows) {
        for col in 0..usize::from(cols) {
            if h.cell(row, col).ch == PLACEHOLDER {
                n += 1;
            }
        }
    }
    n
}

fn wait_quiet(h: &Harness, quiet: Duration) {
    let mut last = (h.output_bytes(), Instant::now());
    loop {
        std::thread::sleep(Duration::from_millis(50));
        let now = h.output_bytes();
        if now != last.0 {
            last = (now, Instant::now());
        } else if last.1.elapsed() >= quiet {
            return;
        }
    }
}

#[test]
#[ignore = "stress: run with `just stress`"]
fn images() {
    let load_start = load_average();
    let mut h = image_harness();
    let first = first_panel(h.home());
    let dir = h.home().path().to_path_buf();
    let small = dir.join("small.bin");
    kitty_image_file(&small, 1, (2, 2), (4, 2));
    let big = dir.join("img-1.bin");
    kitty_image_file(&big, 2, (1024, 1024), (40, 20));

    // Time from a program's image to the placeholders on the screen, in a new panel.
    let placement = |file: &std::path::Path| -> Vec<f64> {
        let mut times = Vec::new();
        for _ in 0..5 {
            let cmd = format!("cat {}; exec sleep 3600", file.display());
            let (cols, rows) = h.size();
            let watch = h.watch(move |s| {
                (0..usize::from(rows))
                    .any(|r| (0..usize::from(cols)).any(|c| s.char_at(r, c) == PLACEHOLDER))
            });
            let t = Instant::now();
            let out = h
                .home()
                .run_deck(&["new-panel", "--focus", "--", "sh", "-c", &cmd]);
            assert!(out.status.success(), "{}", stderr(&out));
            let seen = watch.wait(Duration::from_secs(60)).expect("placeholders");
            times.push(seen.shown.duration_since(t).as_secs_f64());
            let panel = stdout(&out).trim().to_string();
            h.home().run_deck(&["close", &panel]);
            wait_for("the placeholders to go", Duration::from_secs(20), || {
                placeholders_on(&h) == 0
            });
            std::thread::sleep(Duration::from_millis(300));
        }
        times
    };
    let small_times = placement(&small);
    let big_times = placement(&big);

    // Keys in a terminal while ten new images arrive in another terminal of the same panel.
    start_cat(&mut h);
    let files: Vec<String> = (0..10)
        .map(|i| {
            let f = dir.join(format!("arrive-{i}.bin"));
            kitty_image_file(&f, 10 + i, (1024, 1024), (90, 50));
            f.display().to_string()
        })
        .collect();
    let (rest, _, rest_lost) = type_keys(&mut h, 30, Duration::from_millis(100));
    h.send_keys("\r");
    std::thread::sleep(Duration::from_millis(300));
    let script = format!(
        "for f in {}; do cat $f; sleep 0.3; done; exec sleep 3600",
        files.join(" ")
    );
    let arriving = new_terminal(h.home(), Some(&first), false, &["sh", "-c", &script]);
    std::thread::sleep(Duration::from_millis(200));
    let (busy, _, busy_lost) = type_keys(&mut h, 30, Duration::from_millis(100));
    wait_quiet(&h, Duration::from_secs(1));
    let _ = h.home().run_deck(&["close", &arriving]);

    // Bytes written on a switch away and back.
    let shown = dir.join("switch.bin");
    kitty_image_file(&shown, 40, (1024, 1024), (90, 50));
    let cmd = format!("cat {}; exec sleep 3600", shown.display());
    let image_panel = new_panel(h.home(), &["sh", "-c", &cmd]);
    let text_panel = new_panel(h.home(), &["sh"]);
    assert!(h.home().run_deck(&["focus", &image_panel]).status.success());
    wait_for("the image", Duration::from_secs(30), || {
        placeholders_on(&h) > 0
    });
    wait_quiet(&h, Duration::from_secs(1));
    let b0 = h.output_bytes();
    assert!(h.home().run_deck(&["focus", &text_panel]).status.success());
    wait_for("the image to go", Duration::from_secs(30), || {
        placeholders_on(&h) == 0
    });
    wait_quiet(&h, Duration::from_secs(1));
    let away = h.output_bytes() - b0;
    let b1 = h.output_bytes();
    assert!(h.home().run_deck(&["focus", &image_panel]).status.success());
    wait_for("the image back", Duration::from_secs(30), || {
        placeholders_on(&h) > 0
    });
    wait_quiet(&h, Duration::from_secs(1));
    let back = h.output_bytes() - b1;
    let image_bytes = 1024u64 * 1024 * 4;

    let report = json!({
        "load_average_start": load_start, "load_average_end": load_average(),
        "image": "RGBA 1024x1024 (4 MiB): 40x20 cells for the placement time, 90x50 for the arriving images and the switch; the small one is 2x2",
        "new_panel_to_placeholders_small_image_ms": summary_ms(&small_times),
        "new_panel_to_placeholders_1024x1024_ms": summary_ms(&big_times),
        "keys_at_rest_ms": { "echo": summary_ms(&rest), "lost": rest_lost },
        "keys_while_ten_images_arrive_ms": { "echo": summary_ms(&busy), "lost": busy_lost },
        "bytes_written_switching_away": away,
        "bytes_written_switching_back": back,
        "image_pixel_bytes": image_bytes,
    });
    record(
        "images",
        &report,
        &format!(
            "image placed after p50 {:.0} ms (small {:.0}); keys p50 {:.1} ms at rest, {:.1} ms with images arriving (max {:.0}); switch away {away} bytes, back {back} bytes",
            median_ms(&big_times),
            median_ms(&small_times),
            median_ms(&rest),
            median_ms(&busy),
            max_ms(&busy)
        ),
    );
    // Measured 41 ms at the worst; five times that.
    assert!(max_ms(&busy) < 200.0);
}

// ---- client memory --------------------------------------------------------------------

fn client_mib(h: &Harness) -> f64 {
    std::thread::sleep(Duration::from_secs(2));
    mib(sys::usage(i32::try_from(h.pid()).unwrap()).unwrap().memory)
}

#[test]
#[ignore = "stress: run with `just stress`"]
fn client_memory() {
    let load_start = load_average();
    let mut points = serde_json::Map::new();
    let mut put = |name: &str, value: f64| {
        eprintln!("stress client_memory {name}: {value:.1} MiB");
        points.insert(name.into(), json!(value));
    };

    // The text tier, 200×60, sidebar on.
    let mut h = Harness::builder((200, 60)).config_unset("sidebar").deck();
    h.wait_for_text("$", Duration::from_secs(10));
    std::thread::sleep(Duration::from_secs(3));
    put("baseline_200x60", client_mib(&h));
    let first = first_panel(h.home());
    let mut panels = vec![first.clone()];
    for _ in 0..19 {
        panels.push(new_panel(h.home(), &["sh"]));
    }
    for panel in &panels {
        assert!(h.home().run_deck(&["focus", panel]).status.success());
        std::thread::sleep(Duration::from_millis(250));
    }
    assert!(h.home().run_deck(&["focus", &first]).status.success());
    put("after_visiting_20_panels", client_mib(&h));

    h.send_keys("seq 1 100000\r");
    // The sidebar is on: the output row is `…│100000`, the command line `…│$ seq 1 100000`.
    h.wait_for_text("│100000", Duration::from_secs(60));
    h.send_bytes(b"\x1b [");
    h.wait_for_text("SCROLL", Duration::from_secs(10));
    for _ in 0..300 {
        h.send_bytes(b"\x1b[5~");
        std::thread::sleep(Duration::from_millis(8));
    }
    h.send_keys("/zzzzzz\r");
    std::thread::sleep(Duration::from_secs(2));
    put("after_paging_copy_mode_and_a_search", client_mib(&h));
    h.send_keys("q");
    put("after_leaving_copy_mode", client_mib(&h));

    for _ in 0..180 {
        panels.push(new_panel(h.home(), &["sh"]));
    }
    put("200_terminals_in_the_sidebar", client_mib(&h));
    drop(h);

    // The image tier: 16 MiB images in eight terminals.
    let h = image_harness();
    std::thread::sleep(Duration::from_secs(3));
    put("image_tier_baseline", client_mib(&h));
    let first = first_panel(h.home());
    let file = h.home().path().join("big.bin");
    kitty_image_file(&file, 7, (2048, 2048), (40, 20));
    let cmd = format!("cat {}; exec sleep 3600", file.display());
    let mut shown = Vec::new();
    for n in 1..=8 {
        let panel = new_panel(h.home(), &["sh", "-c", &cmd]);
        assert!(h.home().run_deck(&["focus", &panel]).status.success());
        wait_for("the image", Duration::from_secs(60), || {
            placeholders_on(&h) > 0
        });
        wait_quiet(&h, Duration::from_secs(1));
        shown.push(panel);
        if n == 1 || n == 8 {
            put(&format!("with_{n}_image_of_16_mib_shown"), client_mib(&h));
        }
        // Leave it, so the next panel's image is the only one placed.
        assert!(h.home().run_deck(&["focus", &first]).status.success());
        wait_for("the image to go", Duration::from_secs(30), || {
            placeholders_on(&h) == 0
        });
    }
    put("after_leaving_the_eight_panels", client_mib(&h));
    for panel in &shown {
        let _ = h.home().run_deck(&["close", panel]);
    }
    wait_for("the panels to close", Duration::from_secs(20), || {
        stats_json(h.home())["terminals"] == 1
    });
    put("after_closing_the_eight_panels", client_mib(&h));

    let report = json!({
        "load_average_start": load_start, "load_average_end": load_average(),
        "unit": "MiB, the client's physical footprint (macOS ri_phys_footprint, Linux VmRSS)",
        "points": Value::Object(points.clone()),
    });
    record("client_memory", &report, "see the file");
    // Text points measured 3.8 to 5.6 MiB, the image points up to 148.7 MiB: five times each.
    for (name, v) in &points {
        let bound = if name.contains("image") || name.contains("eight") {
            750.0
        } else {
            30.0
        };
        assert!(v.as_f64().unwrap() < bound, "{name}: {v}");
    }
}
