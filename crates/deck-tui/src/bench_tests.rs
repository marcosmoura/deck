//! Ignored benchmarks of what one frame costs the client (the performance review): decode a
//! full 200×60 styled frame, and draw nothing, one cell or the whole screen, with the bytes
//! and write calls each puts on the wire. Run with
//! `cargo nextest run --release -p deck-tui --run-ignored only frame_cost --no-capture`;
//! the numbers go to `target/stress/bench_tui_frame.json`. `image_cost` builds and sends a
//! 64 MiB image canvas (same size and scaled) and a usual 1920×1080 image into 80×24 cells
//! (`bench_tui_image.json`); its bounds are five times what was measured.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "a benchmark fails on error and reports floats"
)]

use std::cell::Cell;
use std::fmt::Write as _;
use std::io::{self, BufWriter, LineWriter, Write};
use std::rc::Rc;
use std::time::{Duration, Instant};

use deck_core::model::{PanelId, ProjectId, SessionId, SessionView, TerminalId};
use deck_core::protocol::{Color, Frame, Line, Run, ServerMsg, Style, decode_payload, encode};
use ratatui::layout::Rect;
use ratatui::{Terminal, TerminalOptions, Viewport};

use crate::app::App;
use crate::backend::DeckBackend;
use crate::mirror::tests::{frame, panel, project};

const SIZE: (u16, u16) = (200, 60);

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
/// The terminal's columns: the window less the sidebar and its border.
const COLS: u16 = 170;

/// Counts what reaches it: bytes and `write` calls.
#[derive(Clone, Default)]
struct Counting {
    bytes: Rc<Cell<u64>>,
    calls: Rc<Cell<u64>>,
}

impl Write for Counting {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.bytes.set(self.bytes.get() + buf.len() as u64);
        self.calls.set(self.calls.get() + 1);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// `runs` styled runs per line: `runs` = 1 is one colour per line, 200 a colour change per
/// column. `seed` shifts every colour so that two seeds differ in every cell.
fn styled_lines(runs: usize, seed: usize) -> Vec<(u16, Line)> {
    let per = usize::from(COLS).div_ceil(runs);
    (0..SIZE.1)
        .map(|row| {
            let runs = (0..runs)
                .map(|r| {
                    let col = r * per;
                    let width = per.min(usize::from(COLS) - col);
                    let mut text = String::new();
                    for i in 0..width {
                        let _ = write!(text, "{}", char::from(b'a' + ((i + r + seed) % 26) as u8));
                    }
                    Run {
                        col: col as u16,
                        text,
                        style: Style {
                            fg: Color::Rgb(
                                (30 + (usize::from(row) + seed * 40) % 200) as u8,
                                100,
                                (r * 9 % 200) as u8 + 30,
                            ),
                            bg: Color::Indexed(((r + seed) % 8) as u8),
                            flags: if r % 3 == 0 { Style::BOLD } else { 0 },
                            ..Style::default()
                        },
                    }
                })
                .filter(|run| !run.text.is_empty())
                .collect();
            (
                row,
                Line {
                    runs,
                    wrapped: false,
                },
            )
        })
        .collect()
}

fn full_frame(runs: usize, seed: usize) -> Frame {
    frame(true, (COLS, SIZE.1), styled_lines(runs, seed))
}

/// An app of the default config (sidebar on) with one terminal showing `frame`.
fn app() -> App {
    let p = panel(1, "bench", 1);
    let view = SessionView {
        id: SessionId(1),
        name: "main".into(),
        active_project: Some(ProjectId(1)),
        active_panel: Some(PanelId(1)),
        layout_viewport: Some(SIZE),
        attached_clients: 0,
        projects: vec![project(1, "bench", false, vec![p])],
    };
    let mut a = App::new(&deck_core::config::Config::default(), SIZE, &[]);
    a.on_server(ServerMsg::SessionView(view));
    let mut f = full_frame(8, 0);
    f.terminal = TERMINAL;
    a.on_server(ServerMsg::Frame(f));
    a
}

fn terminal<W: Write>(writer: W) -> Terminal<DeckBackend<W>> {
    Terminal::with_options(
        DeckBackend::new(writer),
        TerminalOptions {
            viewport: Viewport::Fixed(Rect::new(0, 0, SIZE.0, SIZE.1)),
        },
    )
    .unwrap()
}

fn micros(d: Duration) -> f64 {
    d.as_secs_f64() * 1e6
}

/// (p50, mean) of the durations in µs.
fn stats(mut times: Vec<Duration>) -> (f64, f64) {
    times.sort();
    let mean = times.iter().copied().map(micros).sum::<f64>() / times.len() as f64;
    (micros(times[times.len() / 2]), mean)
}

/// One draw of `change` kind, `iterations` times: (p50 µs, mean µs, bytes per draw, writes
/// per draw) through the given writer.
fn measure<W: Write>(
    writer: W,
    counting: &Counting,
    iterations: usize,
    mut change: impl FnMut(&mut App, usize),
) -> (f64, f64, f64, f64) {
    let mut a = app();
    let mut t = terminal(writer);
    // The first draw writes the whole screen; it is not a sample.
    a.draw_frame(&mut t).unwrap();
    let (b0, c0) = (counting.bytes.get(), counting.calls.get());
    let mut times = Vec::new();
    for i in 0..iterations {
        change(&mut a, i);
        let start = Instant::now();
        a.draw_frame(&mut t).unwrap();
        times.push(start.elapsed());
    }
    let (p50, mean) = stats(times);
    (
        p50,
        mean,
        (counting.bytes.get() - b0) as f64 / iterations as f64,
        (counting.calls.get() - c0) as f64 / iterations as f64,
    )
}

/// The only terminal of `app()`: panel 1's first.
const TERMINAL: TerminalId = TerminalId(11);

fn one_cell(a: &mut App, i: usize) {
    let mut f = frame(
        false,
        (COLS, SIZE.1),
        vec![(10, crate::mirror::tests::line(&format!("{}", i % 10)))],
    );
    f.terminal = TERMINAL;
    a.on_server(ServerMsg::Frame(f));
}

fn whole_screen(a: &mut App, i: usize) {
    let mut f = full_frame(8, 1 + i % 2);
    f.terminal = TERMINAL;
    a.on_server(ServerMsg::Frame(f));
}

#[test]
#[ignore = "benchmark: run with `--release --run-ignored only frame_cost`"]
fn frame_cost() {
    let load_start = load_average();
    let mut json = String::from("{\n");
    // Decode: the client's work for one message, from the bytes of a whole frame.
    for runs in [1usize, 8, 200] {
        let mut f = full_frame(runs, 0);
        f.terminal = TERMINAL;
        let bytes = encode(&ServerMsg::Frame(f)).unwrap();
        let mut times = Vec::new();
        for _ in 0..1000 {
            let mut reader = deck_core::protocol::FrameReader::new();
            reader.push(&bytes);
            let start = Instant::now();
            let payload = reader.next_payload().unwrap().unwrap();
            let msg = decode_payload::<ServerMsg>(&payload).unwrap();
            times.push(start.elapsed());
            drop(msg);
        }
        let (p50, mean) = stats(times);
        eprintln!(
            "decode_full_frame runs_per_line={runs}: {} bytes, p50 {p50:.1} µs, mean {mean:.1} µs",
            bytes.len()
        );
        let _ = writeln!(
            json,
            "  \"decode_full_frame_{runs}_runs_per_line\": {{ \"bytes\": {}, \"p50_us\": {p50:.1}, \"mean_us\": {mean:.1} }},",
            bytes.len()
        );
    }

    // Draws, behind the 1 KiB `LineWriter` of `io::stdout()` and behind a 64 KiB `BufWriter`.
    let mut report = |name: &str, iterations: usize, change: fn(&mut App, usize)| -> f64 {
        let c = Counting::default();
        let line = measure(
            LineWriter::with_capacity(1024, c.clone()),
            &c,
            iterations,
            change,
        );
        let c = Counting::default();
        let buffered = measure(
            BufWriter::with_capacity(64 * 1024, c.clone()),
            &c,
            iterations,
            change,
        );
        eprintln!(
            "{name}: LineWriter(1 KiB) p50 {:.0} µs, {:.0} bytes, {:.1} writes; BufWriter(64 KiB) p50 {:.0} µs, {:.0} bytes, {:.1} writes",
            line.0, line.2, line.3, buffered.0, buffered.2, buffered.3
        );
        let _ = writeln!(
            json,
            "  \"{name}\": {{ \"iterations\": {iterations}, \"line_writer_1k\": {{ \"p50_us\": {:.1}, \"mean_us\": {:.1}, \"bytes\": {:.0}, \"writes\": {:.1} }}, \"buf_writer_64k\": {{ \"p50_us\": {:.1}, \"mean_us\": {:.1}, \"bytes\": {:.0}, \"writes\": {:.1} }} }},",
            line.0, line.1, line.2, line.3, buffered.0, buffered.1, buffered.2, buffered.3
        );
        line.0
    };
    let nothing = report("draw_nothing_changed", 1000, |_, _| {});
    let one = report("draw_one_cell", 1000, one_cell);
    let whole = report("draw_whole_screen", 200, whole_screen);
    let _ = writeln!(json, "  \"load_average_start\": \"{load_start}\",");
    let _ = writeln!(json, "  \"load_average_end\": \"{}\"", load_average());
    json.push_str("}\n");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/stress");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("bench_tui_frame.json"), json).unwrap();
    // Bounds: 5 times the medians of the first run (macOS, a loaded machine).
    assert!(
        nothing < 5.0 * 500.0,
        "a draw of nothing took {nothing:.0} µs"
    );
    assert!(one < 5.0 * 500.0, "a draw of one cell took {one:.0} µs");
    assert!(whole < 5.0 * 770.0, "a full draw took {whole:.0} µs");
}

// ---------------------------------------------------------------------------------------
// Second round: what a program's image costs the client on its UI thread.
// ---------------------------------------------------------------------------------------

/// An image of a gradient and noise, so nothing is a copy of one pixel.
fn picture(width: u32, height: u32) -> crate::placed::Pixels {
    let mut data = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            data.extend_from_slice(&[
                (x * 255 / width.max(1)) as u8,
                (y * 255 / height.max(1)) as u8,
                ((x ^ y) & 255) as u8,
                255,
            ]);
        }
    }
    crate::placed::Pixels {
        width,
        height,
        format: deck_core::protocol::ImageFormat::Rgba,
        data,
    }
}

/// Builds the canvas of `image` shown at `pixels` over `cols × rows` cells of `cell`, and
/// builds and sends it through `Images::ensure_placed` into a counting sink.
fn image_case(
    name: &str,
    image: &crate::placed::Pixels,
    pixels: (u32, u32),
    (cols, rows): (u16, u16),
    cell: deck_core::protocol::CellPx,
    iterations: usize,
    max_ms: f64,
) -> String {
    use deck_core::protocol::{ImageFormat, ImageScreen, Placement, PlacementKind, SourceRect};
    let source = SourceRect {
        x: 0,
        y: 0,
        width: image.width,
        height: image.height,
    };
    let mut build = Vec::new();
    let mut canvas_bytes = 0;
    for _ in 0..iterations {
        let start = Instant::now();
        let canvas = crate::placed::canvas(image, source, pixels, cols, rows, cell).unwrap();
        build.push(start.elapsed());
        canvas_bytes = canvas.rgba.len();
    }
    let (build_p50, _) = stats(build);

    let terminal = TerminalId(1);
    let mut send = Vec::new();
    let (mut bytes, mut calls) = (0, 0);
    for _ in 0..iterations {
        let mut images = crate::graphics::Images::new(None, None, [None; 16]);
        images.program.piece(crate::placed::Piece {
            slot: (terminal, ImageScreen::Primary, 1),
            generation: 1,
            width: image.width,
            height: image.height,
            format: ImageFormat::Rgba,
            offset: 0,
            total: image.data.len() as u32,
            data: image.data.clone(),
        });
        let placement = Placement {
            screen: ImageScreen::Primary,
            image: 1,
            generation: 1,
            kind: PlacementKind::Placed { col: 0, row: 0 },
            cols,
            rows,
            source,
            pixels,
        };
        let sink = Counting::default();
        let mut out = sink.clone();
        let start = Instant::now();
        images
            .ensure_placed(&mut out, &[(terminal, placement)], Some(cell))
            .unwrap();
        send.push(start.elapsed());
        bytes = sink.bytes.get();
        calls = sink.calls.get();
    }
    let (send_p50, _) = stats(send);
    assert!(
        build_p50 / 1e3 < max_ms && send_p50 / 1e3 < max_ms,
        "{name}: canvas {:.1} ms, ensure_placed {:.1} ms",
        build_p50 / 1e3,
        send_p50 / 1e3
    );
    eprintln!(
        "{name}: canvas {canvas_bytes} bytes in {:.1} ms; ensure_placed {:.1} ms, {bytes} bytes in {calls} writes",
        build_p50 / 1e3,
        send_p50 / 1e3
    );
    format!(
        "    {{ \"case\": \"{name}\", \"image\": [{}, {}], \"shown_at_pixels\": [{}, {}], \"cells\": [{cols}, {rows}], \"canvas_bytes\": {canvas_bytes}, \"iterations\": {iterations}, \"canvas_build_p50_ms\": {:.2}, \"ensure_placed_p50_ms\": {:.2}, \"bytes_written\": {bytes}, \"write_calls\": {calls} }}",
        image.width,
        image.height,
        pixels.0,
        pixels.1,
        build_p50 / 1e3,
        send_p50 / 1e3
    )
}

#[test]
#[ignore = "benchmark: run with `--release --run-ignored only image_cost`"]
fn image_cost() {
    use deck_core::protocol::CellPx;
    let load_start = load_average();
    // 297 cells is the most the diacritics table numbers; 12×15-pixel cells make the largest
    // canvas the client builds (63.5 MB of the 64 MiB it allows).
    let big = CellPx {
        width: 12,
        height: 15,
    };
    let (w, h) = (297 * 12, 297 * 15);
    let same = picture(w, h);
    let scaled = picture(2048, 2048);
    let usual = picture(1920, 1080);
    let cases = [
        image_case(
            "canvas_64mib_same_size",
            &same,
            (w, h),
            (297, 297),
            big,
            3,
            400.0,
        ),
        image_case(
            "canvas_64mib_scaled_from_2048x2048",
            &scaled,
            (w, h),
            (297, 297),
            big,
            3,
            1000.0,
        ),
        image_case(
            "1920x1080_into_80x24_cells",
            &usual,
            (640, 384),
            (80, 24),
            CellPx {
                width: 8,
                height: 16,
            },
            20,
            20.0,
        ),
    ];
    let report = format!(
        "{{\n  \"load_average_start\": \"{load_start}\",\n  \"load_average_end\": \"{}\",\n  \"cases\": [\n{}\n  ]\n}}\n",
        load_average(),
        cases.join(",\n")
    );
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/stress");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("bench_tui_image.json"), report).unwrap();
}
