//! Program images end to end: a program in a panel transmits and places an image with the
//! kitty graphics protocol (a fixed 2×2 RGBA picture, written with `printf`), and the client in
//! a "Ghostty" harness draws it as Unicode placeholder cells over an outer image it sends.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use std::time::Duration;

use support::{Color, Harness, TIMEOUT, wait_until};

const PLACEHOLDER: char = '\u{10EEEE}';
/// The first diacritics of the kitty table: the numbers 0 to 7.
const DIACRITICS: [char; 8] = [
    '\u{0305}', '\u{030D}', '\u{030E}', '\u{0310}', '\u{0312}', '\u{033D}', '\u{033E}', '\u{033F}',
];
/// A 2×2 RGBA image: red, green / blue, white.
const RGBA_2X2: &str = "/wAA/wD/AP8AAP///////w==";
/// A 1×1 red pixel.
const RGBA_1X1: &str = "/wAA/w==";

const SIZE: (u16, u16) = (80, 24);

fn ghostty(size: (u16, u16)) -> Harness {
    let h = Harness::builder(size).kitty().deck();
    h.wait_for_text("$", TIMEOUT);
    h
}

/// Has the program write one kitty graphics command.
fn send(h: &mut Harness, params: &str, payload: &str) {
    h.send_keys(&format!("printf '\\033_G{params};{payload}\\033\\\\'\r"));
}

/// Transmits and places the 2×2 image `id` at the cursor over `cols × rows` cells.
fn show(h: &mut Harness, id: u32, cols: u32, rows: u32) {
    send(
        h,
        &format!("a=T,f=32,s=2,v=2,i={id},c={cols},r={rows},t=d,q=2"),
        RGBA_2X2,
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Cell {
    row: usize,
    col: usize,
    id: u32,
    /// The image row and column the diacritics say.
    at: (usize, usize),
}

fn diacritic(c: Option<&char>) -> usize {
    DIACRITICS
        .iter()
        .position(|d| Some(d) == c)
        .unwrap_or(usize::MAX)
}

/// Every placeholder cell on the screen.
fn placeholders(h: &Harness, size: (u16, u16)) -> Vec<Cell> {
    let mut out = Vec::new();
    for row in 0..usize::from(size.1) {
        for col in 0..usize::from(size.0) {
            let cell = h.cell(row, col);
            if cell.ch != PLACEHOLDER {
                continue;
            }
            let rgb = match cell.fg {
                Color::Spec(rgb) => rgb,
                other => panic!("a placeholder without an id colour: {other:?}"),
            };
            out.push(Cell {
                row,
                col,
                id: u32::from_be_bytes([0, rgb.r, rgb.g, rgb.b]),
                at: (
                    diacritic(cell.zerowidth.first()),
                    diacritic(cell.zerowidth.get(1)),
                ),
            });
        }
    }
    out
}

/// Waits until at least `count` placeholder cells are on the screen.
fn wait_for_placeholders(h: &Harness, size: (u16, u16), count: usize) -> Vec<Cell> {
    wait_until("placeholder cells", TIMEOUT, || {
        placeholders(h, size).len() >= count
    });
    placeholders(h, size)
}

fn raw(h: &Harness) -> String {
    String::from_utf8_lossy(&h.raw_output()).into_owned()
}

fn wait_for_raw(h: &Harness, what: &str) {
    wait_until(what, TIMEOUT, || raw(h).contains(what));
}

fn prefixed(h: &mut Harness, key: &str) {
    h.send_bytes(b"\x1b ");
    h.send_keys(key);
}

#[test]
fn a_placed_image_shows_as_placeholder_cells_over_an_outer_image() {
    let mut h = ghostty(SIZE);
    show(&mut h, 7, 3, 2);
    let cells = wait_for_placeholders(&h, SIZE, 6);
    assert_eq!(cells.len(), 6, "{cells:?}");
    let id = cells[0].id;
    assert!(id != 0 && id <= 0xFF_FFFF);
    for c in &cells {
        assert_eq!(c.id, id);
        assert!(c.row < 24);
        // Row-major in a block of 2 rows by 3 columns, from its top left.
        assert_eq!(c.at, (c.row - cells[0].row, c.col - cells[0].col), "{c:?}");
    }
    assert_eq!(cells[0].at, (0, 0));
    assert_eq!(cells[5].at, (1, 2));
    // The canvas is 3×2 cells of 8×16 pixels, its virtual placement 3×2 cells.
    wait_for_raw(&h, &format!("\x1b_Ga=t,f=32,s=24,v=32,i={id},q=2,m="));
    wait_for_raw(&h, &format!("\x1b_Ga=p,U=1,i={id},c=3,r=2,q=2\x1b\\"));
    // The program's own commands never reach the outer terminal.
    assert!(!raw(&h).contains("\x1b_Ga=T"), "{}", raw(&h));
    let path = h.capture("image");
    assert!(path.exists());
}

#[test]
fn a_placement_is_clipped_at_the_terminals_border() {
    let size = (100, 24);
    let mut h = ghostty(size);
    prefixed(&mut h, "t");
    wait_until("the second terminal", TIMEOUT, || {
        h.screen_text().contains("╭─ 2 · ")
    });
    h.wait_for_text("$", TIMEOUT);
    // Wider than the terminal's 48 columns.
    show(&mut h, 7, 60, 2);
    let cells = wait_for_placeholders(&h, size, 96);
    let first = cells[0];
    let border = 99;
    let last = cells.iter().map(|c| c.col).max().unwrap();
    assert_eq!(last, border - 1, "the image stops at the border");
    assert!(cells.iter().all(|c| c.col < border));
    for row in first.row..first.row + 2 {
        assert_eq!(h.cell(row, border).ch, '│', "row {row}");
        assert_eq!(h.cell(row, border).fg, h.cell(first.row - 1, border).fg);
    }
    // The other terminal's border is untouched too.
    assert!(cells.iter().all(|c| c.col > 50));
    let path = h.capture("clipped");
    assert!(path.exists());
}

#[test]
fn scrolling_moves_the_cells_with_the_text_and_deletes_the_outer_image_once_gone() {
    let mut h = ghostty(SIZE);
    show(&mut h, 7, 3, 4);
    let cells = wait_for_placeholders(&h, SIZE, 12);
    assert_eq!(cells.len(), 12);
    let id = cells[0].id;

    // Output until the image's first rows are above the screen.
    let mut half = false;
    for n in 0..60 {
        let now = placeholders(&h, SIZE);
        // The image's first rows are above the screen: what is left shows from its top.
        if !now.is_empty() && now.iter().all(|c| c.at.0 >= 1) {
            half = true;
            break;
        }
        h.send_keys(&format!("echo line{n}\r"));
        h.wait_for_row_start(&format!("line{n}"), TIMEOUT);
    }
    assert!(half, "{}", h.screen_text());
    let now = placeholders(&h, SIZE);
    assert!(now.len() < 12 && !now.is_empty());
    assert!(now.iter().all(|c| c.id == id));
    let path = h.capture("scrolled-half");
    assert!(path.exists());

    // Until it is all out of view: the outer image is deleted.
    for n in 100..160 {
        if placeholders(&h, SIZE).is_empty() {
            break;
        }
        h.send_keys(&format!("echo line{n}\r"));
        h.wait_for_row_start(&format!("line{n}"), TIMEOUT);
    }
    wait_until("the cells to be gone", TIMEOUT, || {
        placeholders(&h, SIZE).is_empty()
    });
    wait_for_raw(&h, &format!("\x1b_Ga=d,d=I,i={id},q=2\x1b\\"));
}

#[test]
fn the_program_deleting_the_image_removes_the_cells_and_the_outer_image() {
    let mut h = ghostty(SIZE);
    show(&mut h, 7, 3, 2);
    let id = wait_for_placeholders(&h, SIZE, 6)[0].id;
    send(&mut h, "a=d,d=I,i=7,q=2", "");
    wait_until("the cells to be gone", TIMEOUT, || {
        placeholders(&h, SIZE).is_empty()
    });
    wait_for_raw(&h, &format!("\x1b_Ga=d,d=I,i={id},q=2\x1b\\"));
}

#[test]
fn a_huge_placement_is_cut_and_the_client_stays_bounded() {
    // A small cell keeps the 297×297-cell canvas (a few MB) quick for the harness's emulator.
    let mut h = Harness::builder(SIZE).cell_px(2, 4).kitty().deck();
    h.wait_for_text("$", TIMEOUT);
    send(
        &mut h,
        "a=T,f=32,s=1,v=1,i=9,c=60000,r=60000,t=d,q=2",
        RGBA_1X1,
    );
    // The image covers the screen: its cells are what shows. The canvas is megabytes on the
    // wire, slow to reach the harness on a busy machine.
    wait_until("placeholder cells", Duration::from_secs(30), || {
        !placeholders(&h, SIZE).is_empty()
    });
    let cells = placeholders(&h, SIZE);
    assert!(cells.len() <= 297 * 297);
    {
        // What shows is cut at the diacritics table, and the canvas stays under 64 MiB.
        let out = raw(&h);
        let canvas = out.split("\x1b_Ga=t,f=32,s=").nth(1).unwrap();
        let (w, rest) = canvas.split_once(",v=").unwrap();
        let (hgt, _) = rest.split_once(',').unwrap();
        let (w, hgt): (u64, u64) = (w.parse().unwrap(), hgt.parse().unwrap());
        assert!(w <= 297 * 2 && hgt <= 297 * 4);
        assert!(w * hgt * 4 <= 64 << 20);
        assert!(out.contains(",c=297,r=297,q=2"));
    }
    assert!(raw(&h).len() < 128 << 20);
    // The client still answers.
    prefixed(&mut h, "d");
    assert!(h.wait_for_exit(TIMEOUT).success());
}

#[test]
fn a_dialog_over_the_image_covers_its_cells_and_closing_it_shows_them_again() {
    let mut h = ghostty(SIZE);
    show(&mut h, 7, 78, 20);
    let before = wait_for_placeholders(&h, SIZE, 1560).len();
    assert!(before > 1000, "{before}");
    prefixed(&mut h, "?");
    h.wait_for_text("Help", TIMEOUT);
    let covered = placeholders(&h, SIZE).len();
    assert!(covered < before, "{covered} of {before}");
    h.capture("under-help");
    h.send_bytes(b"\x1b");
    wait_until("the cells back", TIMEOUT, || {
        placeholders(&h, SIZE).len() == before
    });
}

#[test]
fn detaching_deletes_every_program_image() {
    let mut h = ghostty(SIZE);
    show(&mut h, 7, 3, 2);
    show(&mut h, 8, 3, 2);
    let ids: std::collections::BTreeSet<u32> = {
        wait_until("both images", TIMEOUT, || {
            placeholders(&h, SIZE)
                .iter()
                .map(|c| c.id)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == 2
        });
        placeholders(&h, SIZE).iter().map(|c| c.id).collect()
    };
    prefixed(&mut h, "d");
    assert!(h.wait_for_exit(TIMEOUT).success());
    let out = raw(&h);
    let leave = out
        .rfind("\x1b[?1049l")
        .expect("leaves the alternate screen");
    for id in ids {
        let delete = format!("\x1b_Ga=d,d=I,i={id},q=2\x1b\\");
        let at = out
            .rfind(&delete)
            .unwrap_or_else(|| panic!("no delete of {id}"));
        assert!(
            at < leave,
            "{id} is deleted before the alternate screen ends"
        );
    }
}

#[test]
fn without_the_kitty_environment_no_graphics_command_is_written() {
    let mut h = Harness::builder(SIZE).cell_px(8, 16).deck();
    h.wait_for_text("$", TIMEOUT);
    show(&mut h, 7, 3, 2);
    h.send_keys("echo alive\r");
    h.wait_for_row_start("alive", TIMEOUT);
    assert!(placeholders(&h, SIZE).is_empty());
    // The start-up probe's own graphics query (`ESC _ G i=31,..`) is not an image command.
    assert!(!raw(&h).contains("\x1b_Ga="), "{}", raw(&h));
}

#[test]
fn a_program_written_placeholder_is_drawn_blank_with_images_on_and_off() {
    for kitty in [true, false] {
        let mut h = if kitty {
            ghostty(SIZE)
        } else {
            let h = Harness::builder(SIZE).cell_px(8, 16).deck();
            h.wait_for_text("$", TIMEOUT);
            h
        };
        // U+10EEEE and two diacritics, in octal UTF-8.
        h.send_keys("printf 'a\\364\\216\\273\\256\\314\\205\\315\\215b\\n'\r");
        h.wait_for_row_start("a b", TIMEOUT);
        assert!(placeholders(&h, SIZE).is_empty(), "kitty {kitty}");
        assert!(!raw(&h).contains(PLACEHOLDER), "kitty {kitty}");
    }
}

// Program-made Unicode placeholders (virtual placements).

/// The cell text for `printf`: U+10EEEE (octal UTF-8) and the diacritics of the numbers 0 to 3.
fn octal_cell(marks: &[usize]) -> String {
    const OCTAL: [&str; 4] = ["\\314\\205", "\\314\\215", "\\314\\216", "\\315\\220"];
    let marks: String = marks
        .iter()
        .map(|n| OCTAL.get(*n).copied().unwrap())
        .collect();
    format!("\\364\\216\\273\\256{marks}")
}

/// Has the program transmit image `id`, place it virtually over `cols × rows` and write a block
/// of `rows` rows of `cols` cells in the colour of its low 24 bits: the first cell of a row has
/// its row and column diacritics (and `third` if given), the others none.
fn program_placeholders(h: &mut Harness, id: u32, third: Option<usize>, (cols, rows): (u32, u32)) {
    send(h, &format!("a=t,f=32,s=2,v=2,i={id},t=d,q=2"), RGBA_2X2);
    send(h, &format!("a=p,U=1,i={id},c={cols},r={rows},q=2"), "");
    let [_, r, g, b] = id.to_be_bytes();
    let mut text = format!("\\033[38;2;{r};{g};{b}m");
    for row in 0..rows {
        let marks: Vec<usize> = [row as usize, 0].into_iter().chain(third).collect();
        text.push_str(&octal_cell(&marks));
        for _ in 1..cols {
            text.push_str(&octal_cell(&[]));
        }
        text.push_str("\\n");
    }
    h.send_keys(&format!("printf '{text}\\033[0m'\r"));
}

#[test]
fn a_program_made_placeholder_block_is_drawn_over_an_outer_image() {
    let mut h = ghostty(SIZE);
    program_placeholders(&mut h, 11, None, (3, 2));
    let cells = wait_for_placeholders(&h, SIZE, 6);
    assert_eq!(cells.len(), 6, "{cells:?}");
    let outer = cells[0].id;
    assert!(outer != 11 && outer != 0 && outer <= 0xFF_FFFF);
    for c in &cells {
        assert_eq!(c.id, outer);
        // Explicit diacritics on every cell, the inherited ones included.
        assert_eq!(
            c.at,
            (c.row - cells[0].row, c.col - cells[0].col),
            "{c:?} of {cells:?}"
        );
    }
    assert_eq!(cells[5].at, (1, 2));
    // The image's own pixels, with the program's size; no canvas.
    wait_for_raw(&h, &format!("\x1b_Ga=t,f=32,s=2,v=2,i={outer},q=2,m=0;"));
    wait_for_raw(&h, &format!("\x1b_Ga=p,U=1,i={outer},c=3,r=2,q=2\x1b\\"));
    assert!(!raw(&h).contains("t=d,q=2\x1b"), "{}", raw(&h));
    let path = h.capture("program-placeholders");
    assert!(path.exists());
}

#[test]
fn a_32_bit_id_with_a_third_diacritic_is_drawn() {
    let mut h = ghostty(SIZE);
    // 0x01000012: the low 24 bits in the colour, the high byte (1) as the third diacritic.
    program_placeholders(&mut h, 0x0100_0012, Some(1), (2, 2));
    let cells = wait_for_placeholders(&h, SIZE, 4);
    assert_eq!(cells.len(), 4, "{cells:?}");
    let outer = cells[0].id;
    assert_eq!(cells[3].at, (1, 1));
    assert!(cells.iter().all(|c| c.id == outer));
    wait_for_raw(&h, &format!("\x1b_Ga=p,U=1,i={outer},c=2,r=2,q=2\x1b\\"));
    h.capture("program-placeholders-32-bit");
}

#[test]
fn without_the_kitty_environment_program_placeholders_are_blank_and_write_nothing() {
    let mut h = Harness::builder(SIZE).cell_px(8, 16).deck();
    h.wait_for_text("$", TIMEOUT);
    program_placeholders(&mut h, 11, None, (3, 2));
    h.send_keys("echo alive\r");
    h.wait_for_row_start("alive", TIMEOUT);
    assert!(placeholders(&h, SIZE).is_empty());
    // The start-up probe's own graphics query (`ESC _ G i=31,..`) is not an image command.
    assert!(!raw(&h).contains("\x1b_Ga="), "{}", raw(&h));
    assert!(!raw(&h).contains(PLACEHOLDER));
}

/// A 4×4 RGBA PNG.
const PNG_4X4: &str = "iVBORw0KGgoAAAANSUhEUgAAAAQAAAAECAYAAACp8Z5+AAAAFUlEQVR4nGP4z8DwHwwZ/oMBA+kCAA97J9lJHDRvAAAAAElFTkSuQmCC";

#[test]
fn claude_codes_chunked_png_thumbnail_is_drawn() {
    let mut h = ghostty(SIZE);
    // What Claude Code writes: `a=T,U=1` with a PNG in two chunks (the first with the full
    // keys and `m=1`, the next with `m=0,q=2` only), then bare cells after each row's first.
    let id: u32 = 0x0100_0021;
    let (first, second) = PNG_4X4.split_at(64);
    send(
        &mut h,
        &format!("a=T,U=1,q=2,f=100,i={id},c=3,r=2,m=1"),
        first,
    );
    h.send_keys(&format!("printf '\\033_Gm=0,q=2;{second}\\033\\\\'\r"));
    let [_, r, g, b] = id.to_be_bytes();
    let mut text = format!("\\033[38;2;{r};{g};{b}m");
    for row in 0..2 {
        text.push_str(&octal_cell(&[row, 0, 1]));
        text.push_str(&octal_cell(&[]));
        text.push_str(&octal_cell(&[]));
        text.push_str("\\n");
    }
    h.send_keys(&format!("printf '{text}\\033[0m'\r"));
    let cells = wait_for_placeholders(&h, SIZE, 6);
    assert_eq!(cells.len(), 6, "{cells:?}\n{}", h.screen_text());
    let outer = cells[0].id;
    assert!(outer != 0 && outer <= 0xFF_FFFF);
    for c in &cells {
        assert_eq!(c.id, outer);
        assert_eq!(c.at, (c.row - cells[0].row, c.col - cells[0].col), "{c:?}");
    }
    assert_eq!(cells[5].at, (1, 2));
    wait_for_raw(&h, &format!("\x1b_Ga=t,f=32,s=4,v=4,i={outer},q=2,m=0;"));
    wait_for_raw(&h, &format!("\x1b_Ga=p,U=1,i={outer},c=3,r=2,q=2\x1b\\"));
    h.capture("claude-code-thumbnail");
}

#[test]
fn claude_codes_file_path_png_is_drawn() {
    let mut h = ghostty(SIZE);
    // What Claude Code writes without probing: `a=T,U=1,q=1,f=100,t=f` with the base64 of the
    // path of a PNG, then bare placeholder cells.
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("thumb.png");
    let path_text = path.to_str().expect("utf-8").to_string();
    h.send_keys(&format!(
        "printf %s '{PNG_4X4}' | base64 -d > '{path_text}'\r"
    ));
    wait_until("the PNG file", TIMEOUT, || {
        std::fs::metadata(&path).is_ok_and(|m| m.len() > 50)
    });
    let id: u32 = 0x0100_0022;
    let encoded = path_text
        .bytes()
        .collect::<Vec<u8>>()
        .chunks(3)
        .map(|c| {
            const ABC: &[u8; 64] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let n = c
                .iter()
                .enumerate()
                .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
            (0..4)
                .map(|i| {
                    if i <= c.len() {
                        char::from(ABC[((n >> (18 - 6 * i)) & 63) as usize])
                    } else {
                        '='
                    }
                })
                .collect::<String>()
        })
        .collect::<String>();
    send(
        &mut h,
        &format!("a=T,U=1,q=1,f=100,t=f,i={id},c=3,r=2"),
        &encoded,
    );
    let [_, r, g, b] = id.to_be_bytes();
    let mut text = format!("\\033[38;2;{r};{g};{b}m");
    for row in 0..2 {
        text.push_str(&octal_cell(&[row, 0, 1]));
        text.push_str(&octal_cell(&[]));
        text.push_str(&octal_cell(&[]));
        text.push_str("\\n");
    }
    h.send_keys(&format!("printf '{text}\\033[0m'\r"));
    let cells = wait_for_placeholders(&h, SIZE, 6);
    assert_eq!(cells.len(), 6, "{cells:?}\n{}", h.screen_text());
    let outer = cells[0].id;
    assert!(outer != 0 && outer <= 0xFF_FFFF);
    wait_for_raw(&h, &format!("\x1b_Ga=t,f=32,s=4,v=4,i={outer},q=2,m=0;"));
    wait_for_raw(&h, &format!("\x1b_Ga=p,U=1,i={outer},c=3,r=2,q=2\x1b\\"));
    h.capture("claude-code-file-path");
}

/// Asks XTVERSION and reads the answer, waiting at most 1 s.
const READ_VERSION: &str = "set +m; stty raw -echo min 0 time 10; printf '\\033[>q'; \
    x=$(dd bs=1 count=30 2>/dev/null | tr -d '\\033'); stty sane; echo \"VER=${#x}:$x\"";

#[test]
fn a_program_asking_xtversion_gets_the_outer_terminals_name() {
    // The harness's terminal answers `ghostty 1.2.0`; deck passes it to the panel's programs.
    let mut h = Harness::builder(SIZE)
        .kitty()
        .xtversion("ghostty 1.2.0")
        .deck();
    h.wait_for_text("$", TIMEOUT);
    h.send_keys(&format!("{READ_VERSION}\r"));
    h.wait_for_text("VER=17:P>|ghostty 1.2.0\\", TIMEOUT);
    h.capture("xtversion");
}

#[test]
fn xtversion_gets_no_answer_without_images_or_without_a_name() {
    // The outer terminal names itself, but the client shows no images (no kitty environment).
    let mut h = Harness::builder(SIZE)
        .cell_px(8, 16)
        .xtversion("ghostty 1.2.0")
        .deck();
    h.wait_for_text("$", TIMEOUT);
    h.send_keys(&format!("{READ_VERSION}\r"));
    h.wait_for_text("VER=0:", TIMEOUT);
    // An image-capable client whose terminal gave no name.
    let mut h = ghostty(SIZE);
    h.send_keys(&format!("{READ_VERSION}\r"));
    h.wait_for_text("VER=0:", TIMEOUT);
}
