//! Ignored benchmarks of what one frame costs the daemon (the performance review): build a
//! full 200×60 styled frame, an incremental one, clone it, encode it. Run with
//! `cargo nextest run --release -p deck-daemon --run-ignored only frame_cost --no-capture`;
//! the numbers go to `target/stress/bench_daemon_frame.json`. `png_cost` times PNG commands that
//! declare a large image (`bench_png.json`); `snapshot_cost` times terminal snapshots for a live
//! upgrade, feeding one back and writing and reading 30 handoff records (`bench_snapshot.json`).
//! The bounds in the last two are five times what was measured.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "a benchmark fails on error and reports floats"
)]

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use deck_core::model::TerminalId;
use deck_core::protocol::{
    ImageFormat, ImageScreen, MAX_IMAGE_PIECE, ServerMsg, decode_payload, encode,
};

use super::*;

const ITERATIONS: usize = 1000;

/// The load average, as the machine prints it.
fn load_average() -> String {
    std::fs::read_to_string("/proc/loadavg").unwrap_or_else(|_| {
        std::process::Command::new("sysctl")
            .args(["-n", "vm.loadavg"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    })
}

/// 60 rows of 200 columns: a colour and a weight per word, some wide characters.
fn styled_screen(e: &mut Emu) {
    for row in 0..60 {
        let mut line = format!("\x1b[{};1H", row + 1);
        for word in 0..28 {
            let _ = write!(
                line,
                "\x1b[{};{}mw{:03}{} \x1b[0m",
                31 + (row + word) % 7,
                if word % 3 == 0 { 1 } else { 22 },
                (row * 31 + word * 7) % 1000,
                if word % 9 == 0 { "日本" } else { "abc" },
            );
        }
        e.feed(line.as_bytes());
    }
}

/// (median, mean) of the durations, in microseconds.
fn stats(mut times: Vec<Duration>) -> (f64, f64) {
    times.sort();
    let micros = |d: Duration| d.as_secs_f64() * 1e6;
    let mean = times.iter().copied().map(micros).sum::<f64>() / times.len() as f64;
    (micros(times[times.len() / 2]), mean)
}

fn timed<T>(mut f: impl FnMut() -> T) -> (f64, f64) {
    let mut keep = Vec::new();
    let times = (0..ITERATIONS)
        .map(|_| {
            let start = Instant::now();
            let out = f();
            let spent = start.elapsed();
            // Dropped outside the timing.
            keep.push(out);
            keep.truncate(1);
            spent
        })
        .collect();
    stats(times)
}

#[test]
#[ignore = "benchmark: run with `--release --run-ignored only frame_cost`"]
fn frame_cost() {
    let load_start = load_average();
    let id = TerminalId(1);
    let mut e = Emu::new(200, 60, 1000).expect("emulator");
    styled_screen(&mut e);
    let full = e.full_frame(id);
    let runs: usize = full.lines.iter().map(|(_, l)| l.runs.len()).sum();
    let msg = ServerMsg::Frame(full.clone());
    let bytes = encode(&msg).unwrap();

    let (full_p50, full_mean) = timed(|| e.full_frame(id));
    let (clone_p50, clone_mean) = timed(|| full.clone());
    let (encode_p50, encode_mean) = timed(|| encode(&msg).unwrap());
    let (decode_p50, decode_mean) = timed(|| decode_payload::<ServerMsg>(&bytes[4..]).unwrap());
    // One row changed: feed it (not timed), then build the damage frame.
    e.reset_damage();
    let mut damage_times = Vec::new();
    let mut damage_bytes = 0;
    for i in 0..ITERATIONS {
        e.feed(format!("\x1b[10;1H{}", i % 10).as_bytes());
        let start = Instant::now();
        let frame = e.damage_frame(id, Duration::ZERO).expect("a frame");
        damage_times.push(start.elapsed());
        damage_bytes = encode(&ServerMsg::Frame(frame)).unwrap().len();
    }
    let (damage_p50, damage_mean) = stats(damage_times);

    // One 4 MiB piece of an image.
    let piece = ServerMsg::Image {
        terminal: id,
        screen: ImageScreen::Primary,
        image: 1,
        generation: 1,
        width: 1024,
        height: 1024,
        format: ImageFormat::Rgba,
        offset: 0,
        total: MAX_IMAGE_PIECE as u32,
        data: vec![0x5a; MAX_IMAGE_PIECE],
    };
    let mut piece_times = Vec::new();
    for _ in 0..50 {
        let start = Instant::now();
        let out = encode(&piece).unwrap();
        piece_times.push(start.elapsed());
        assert!(out.len() > MAX_IMAGE_PIECE);
    }
    let (piece_p50, piece_mean) = stats(piece_times);

    let report = serde_json::json!({
        "frame": "200x60, a colour and a weight per word, wide characters",
        "iterations": ITERATIONS,
        "load_average_start": load_start,
        "load_average_end": load_average(),
        "runs_in_full_frame": runs,
        "full_frame_encoded_bytes": bytes.len(),
        "damage_frame_one_row_encoded_bytes": damage_bytes,
        "microseconds": {
            "full_frame_build": { "p50": full_p50, "mean": full_mean },
            "damage_frame_one_changed_row": { "p50": damage_p50, "mean": damage_mean },
            "frame_clone": { "p50": clone_p50, "mean": clone_mean },
            "encode_full_frame": { "p50": encode_p50, "mean": encode_mean },
            "decode_full_frame": { "p50": decode_p50, "mean": decode_mean },
            "encode_4mib_image_piece_50_iterations": { "p50": piece_p50, "mean": piece_mean },
        },
    });
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/stress");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("bench_daemon_frame.json"),
        serde_json::to_string_pretty(&report).unwrap() + "\n",
    )
    .unwrap();
    eprintln!("{report:#}");
    // Bounds: 5 times the medians of the first run (macOS, a loaded machine).
    assert!(full_p50 < 5.0 * 840.0, "full frame {full_p50:.0} µs");
    assert!(encode_p50 < 5.0 * 30.0, "encode {encode_p50:.0} µs");
}

// ---------------------------------------------------------------------------------------
// Second round: tiny PNG commands, snapshots for a live upgrade, the handoff file.
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

fn percentiles_ms(mut times: Vec<Duration>) -> serde_json::Value {
    times.sort();
    let ms = |d: Duration| d.as_secs_f64() * 1e3;
    serde_json::json!({
        "n": times.len(),
        "min": ms(times[0]),
        "p50": ms(times[times.len() / 2]),
        "max": ms(times[times.len() - 1]),
        "total": times.iter().copied().map(ms).sum::<f64>(),
    })
}

fn rss_mib() -> f64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<f64>()
        .unwrap_or(f64::NAN)
        / 1024.0
}

fn write_report(name: &str, report: &serde_json::Value) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/stress");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(name),
        serde_json::to_string_pretty(report).unwrap() + "\n",
    )
    .unwrap();
    eprintln!("{report:#}");
}

/// 100 small PNG commands that declare a large image, fed one at a time, against 100 that
/// declare a small one; and the same 100 in one feed of about 14 KiB.
#[test]
#[ignore = "benchmark: run with `--release --run-ignored only png_cost`"]
fn png_cost() {
    let load_start = load_average();
    let mut e = Emu::new(200, 60, 1000).expect("emulator");
    let time_each = |e: &mut Emu, png: &[u8]| -> Vec<Duration> {
        (0..100)
            .map(|i| {
                let command = png_command(i + 1, png);
                let start = Instant::now();
                e.feed(&command);
                start.elapsed()
            })
            .collect()
    };
    let small = percentiles_ms(time_each(&mut e, &tiny_png(4, 4)));
    let rss_before = rss_mib();
    // 4096 × 4096 RGBA is exactly the 64 MiB the daemon accepts.
    let big_png = tiny_png(4096, 4096);
    let big = percentiles_ms(time_each(&mut e, &big_png));
    let rss_after = rss_mib();
    let one_feed: Vec<u8> = (0..100)
        .flat_map(|i| png_command(i + 1, &big_png))
        .collect();
    let start = Instant::now();
    e.feed(&one_feed);
    let one_feed_ms = start.elapsed().as_secs_f64() * 1e3;
    let report = serde_json::json!({
        "load_average_start": load_start,
        "load_average_end": load_average(),
        "command_bytes": png_command(1, &big_png).len(),
        "commands": 100,
        "declares_4x4_ms": small,
        "declares_4096x4096_ms": big,
        "hundred_commands_in_one_feed_ms": one_feed_ms,
        "rss_mib_before_after_big": [rss_before, rss_after],
    });
    write_report("bench_png.json", &report);
    // Before the size check: 59 ms and 64 MiB of rss. After it: 0.4 us a command, no growth.
    // The bounds are well past the new figures and well under the old ones.
    assert!(
        one_feed_ms < 5.0,
        "100 big-declaring PNGs took {one_feed_ms:.1} ms"
    );
    assert!(
        rss_after - rss_before < 16.0,
        "rss grew {:.0} MiB",
        rss_after - rss_before
    );
}

/// `lines` lines of `cols` columns, plain or with an SGR colour per word.
fn fill(e: &mut Emu, cols: usize, lines: usize, styled: bool) {
    let mut chunk = String::new();
    for n in 0..lines {
        let mut line = String::new();
        let mut word = 0;
        while line.len() < cols - 8 {
            if styled {
                let _ = write!(line, "\x1b[{}mw{:04}\x1b[0m ", 31 + (n + word) % 7, word);
            } else {
                let _ = write!(line, "w{:04} ", word);
            }
            word += 1;
        }
        // The visible width stops short of the margin: no wrap.
        line.push_str("\r\n");
        chunk.push_str(&line);
        if chunk.len() > 256 * 1024 {
            e.feed(chunk.as_bytes());
            chunk.clear();
        }
    }
    e.feed(chunk.as_bytes());
}

#[test]
#[ignore = "benchmark: run with `--release --run-ignored only snapshot_cost`"]
fn snapshot_cost() {
    let load_start = load_average();
    let mut cases = Vec::new();
    let mut biggest = Vec::new();
    for (cols, styled, lines) in [
        (80, false, 10_000),
        (80, true, 10_000),
        (200, false, 10_000),
        (200, true, 10_000),
        // The budget is bytes: this many lines fill it at 80 columns.
        (80, false, 100_000),
        (200, true, 100_000),
    ] {
        let mut e = Emu::new(cols as u16, 60, 10_000).expect("emulator");
        fill(&mut e, cols, lines, styled);
        let history = e.history();
        let mut times = Vec::new();
        let mut snapshot = Vec::new();
        for _ in 0..5 {
            let start = Instant::now();
            snapshot = e.snapshot();
            times.push(start.elapsed());
        }
        let mut fed = Vec::new();
        for _ in 0..5 {
            let mut fresh = Emu::new(cols as u16, 60, 10_000).expect("emulator");
            let start = Instant::now();
            fresh.feed(&snapshot);
            fed.push(start.elapsed());
        }
        cases.push(serde_json::json!({
            "columns": cols, "styled": styled, "lines_printed": lines, "history_lines": history,
            "snapshot_bytes": snapshot.len(),
            "snapshot_ms": percentiles_ms(times),
            "feed_snapshot_back_ms": percentiles_ms(fed),
        }));
        if cols == 200 && styled && lines == 100_000 {
            biggest = snapshot;
        }
    }

    // 30 handoff records of the biggest snapshot: write, read, size.
    let mut file = crate::handoff::file::tests::sample();
    let template = file.terminals[0].clone();
    file.terminals = (0..30)
        .map(|i| {
            let mut t = template.clone();
            t.id = TerminalId(i + 1);
            t.snapshot.clone_from(&biggest);
            t
        })
        .collect();
    let dir = tempfile::tempdir().unwrap();
    let (mut writes, mut reads) = (Vec::new(), Vec::new());
    let mut size = 0;
    for _ in 0..3 {
        let start = Instant::now();
        let out = crate::handoff::file::write(dir.path(), &file).unwrap();
        writes.push(start.elapsed());
        size = out.metadata().unwrap().len();
        let start = Instant::now();
        let back = crate::handoff::file::read(out).unwrap();
        reads.push(start.elapsed());
        assert_eq!(back.terminals.len(), 30);
    }
    // Measured 0.30 s to write and 0.36 s to read; five times each.
    let slowest = |v: &[Duration]| v.iter().max().unwrap().as_secs_f64();
    assert!(
        slowest(&writes) < 1.5,
        "writing took {:.2} s",
        slowest(&writes)
    );
    assert!(
        slowest(&reads) < 1.8,
        "reading took {:.2} s",
        slowest(&reads)
    );
    let report = serde_json::json!({
        "load_average_start": load_start,
        "load_average_end": load_average(),
        "emulator": "60 rows, 10,000 lines of scrollback",
        "cases": cases,
        "handoff_file_30_records": {
            "snapshot_bytes_each": biggest.len(),
            "file_bytes": size,
            "write_ms": percentiles_ms(writes),
            "read_ms": percentiles_ms(reads),
        },
    });
    write_report("bench_snapshot.json", &report);
}
