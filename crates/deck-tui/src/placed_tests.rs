//! Program images in the client: receiving pixels, building outer images, drawing placeholder
//! cells.

use std::time::Instant;

use deck_core::icons::{Icon, IconKind};
use deck_core::model::TerminalId;
use deck_core::protocol::{
    CellPx, ImageFormat, ImageScreen, MAX_IMAGE_PIECE, Placement, PlacementKind, ServerMsg,
    SourceRect,
};
use ratatui::crossterm::event as ct;
use ratatui::style::Modifier;

use crate::app::App;
use crate::convert::image_id_color;
use crate::diacritics::{DIACRITICS, PLACEHOLDER};
use crate::graphics::{Images, Pair, delete};
use crate::icons_tests::render;
use crate::mirror::tests::{frame, line, no_sidebar, view};
use crate::placed::{Canvas, MAX_CANVAS, Piece, Pixels, Program, canvas};

const CELL: CellPx = CellPx {
    width: 8,
    height: 16,
};
const T: TerminalId = TerminalId(4);

fn slot(image: u32) -> crate::placed::Slot {
    (T, ImageScreen::Primary, image)
}

fn piece(image: u32, generation: u64, offset: u32, total: u32, data: &[u8]) -> Piece {
    Piece {
        slot: slot(image),
        generation,
        width: 2,
        height: 2,
        format: ImageFormat::Rgba,
        offset,
        total,
        data: data.to_vec(),
    }
}

/// A 2×2 image: red, green / blue, white, all opaque.
const RGBA: [u8; 16] = [
    255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
];

fn pixels(width: u32, height: u32, rgba: &[u8]) -> Pixels {
    Pixels {
        width,
        height,
        format: ImageFormat::Rgba,
        data: rgba.to_vec(),
    }
}

fn rect(x: u32, y: u32, width: u32, height: u32) -> SourceRect {
    SourceRect {
        x,
        y,
        width,
        height,
    }
}

// Receiving pixels.

#[test]
fn pieces_assemble_into_pixels() {
    let mut p = Program::default();
    p.piece(piece(1, 1, 0, 16, &RGBA[..6]));
    assert!(p.assembling(slot(1)));
    assert!(!p.has_pixels(slot(1), 1));
    p.piece(piece(1, 1, 6, 16, &RGBA[6..]));
    assert!(!p.assembling(slot(1)));
    assert!(p.has_pixels(slot(1), 1));
}

#[test]
fn an_offset_of_zero_restarts_the_assembly() {
    let mut p = Program::default();
    p.piece(piece(1, 1, 0, 16, &RGBA[..6]));
    p.piece(piece(1, 1, 0, 16, &RGBA[..4]));
    // Continuing where the restarted assembly ended completes it.
    p.piece(piece(1, 1, 4, 16, &RGBA[4..]));
    assert!(p.has_pixels(slot(1), 1));
}

#[test]
fn a_gap_discards_the_assembly() {
    let mut p = Program::default();
    p.piece(piece(1, 1, 0, 16, &RGBA[..6]));
    p.piece(piece(1, 1, 8, 16, &RGBA[8..]));
    assert!(!p.assembling(slot(1)));
    assert!(!p.has_pixels(slot(1), 1));
    // The rest of it finds nothing to continue.
    p.piece(piece(1, 1, 6, 16, &RGBA[6..]));
    assert!(!p.has_pixels(slot(1), 1));
}

#[test]
fn a_piece_of_an_older_generation_is_discarded() {
    let mut p = Program::default();
    p.piece(piece(1, 2, 0, 16, &RGBA[..6]));
    p.piece(piece(1, 1, 0, 16, &RGBA));
    assert!(p.assembling(slot(1)));
    assert!(!p.has_pixels(slot(1), 1));
    p.piece(piece(1, 2, 6, 16, &RGBA[6..]));
    assert!(p.has_pixels(slot(1), 2));
    // Complete pixels of a newer generation also win.
    p.piece(piece(1, 1, 0, 16, &RGBA));
    assert!(p.has_pixels(slot(1), 2));
    assert!(!p.has_pixels(slot(1), 1));
}

#[test]
fn a_newer_generation_replaces_the_pixels() {
    let mut p = Program::default();
    p.piece(piece(1, 1, 0, 16, &RGBA));
    p.piece(piece(1, 2, 0, 16, &RGBA));
    assert!(p.has_pixels(slot(1), 2));
    assert!(!p.has_pixels(slot(1), 1));
}

#[test]
fn a_size_that_does_not_fit_the_total_is_refused() {
    let mut p = Program::default();
    p.piece(piece(1, 1, 0, 12, &RGBA[..12]));
    assert!(!p.assembling(slot(1)));
    let mut big = piece(2, 1, 0, 16, &RGBA);
    big.width = 1 << 15;
    big.height = 1 << 15;
    big.total = u32::MAX;
    p.piece(big);
    assert!(!p.assembling(slot(2)));
}

#[test]
fn gone_and_leaving_the_session_forget_the_pixels() {
    let mut p = Program::default();
    p.piece(piece(1, 1, 0, 16, &RGBA));
    p.piece(piece(2, 1, 0, 16, &RGBA));
    p.gone(slot(1), 0);
    assert!(p.has_pixels(slot(1), 1), "another generation is not it");
    p.gone(slot(1), 1);
    assert!(!p.has_pixels(slot(1), 1));
    p.retain_terminals(|t| t != T);
    assert!(!p.has_pixels(slot(2), 1));
}

#[test]
fn pieces_of_the_largest_size_fit_the_protocol() {
    // The daemon splits at 4 MiB; the client takes any split.
    assert_eq!(MAX_IMAGE_PIECE, 4 << 20);
}

// Ids.

#[test]
fn one_allocator_serves_icon_images_and_programs_and_reuses_freed_ids() {
    let mut i = Images::new(None, None, [None; 16]);
    let logo = Pair {
        icon: Icon::Kind(IconKind::Claude),
        rgb: (1, 2, 3),
        frame: 0,
    };
    assert!(i.register(logo));
    let a = i.alloc().unwrap();
    let b = i.alloc().unwrap();
    assert_ne!(Some(a), i.id(&logo));
    assert_ne!(a, b);
    assert!(a != 0 && a <= crate::graphics::LAST_ID);
    i.release(a);
    assert_eq!(i.alloc(), Some(a));
    i.next = crate::graphics::LAST_ID + 1;
    assert_eq!(i.alloc(), None);
    i.release(b);
    assert_eq!(i.alloc(), Some(b));
}

// Canvas.

fn px(c: &Canvas, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * c.width + x) * 4) as usize;
    c.rgba[at..at + 4].try_into().unwrap()
}

#[test]
fn a_canvas_is_the_cells_in_pixels_with_the_image_at_the_top_left() {
    let c = canvas(&pixels(2, 2, &RGBA), rect(0, 0, 2, 2), (2, 2), 2, 1, CELL).unwrap();
    assert_eq!((c.width, c.height), (16, 16));
    assert_eq!(px(&c, 0, 0), [255, 0, 0, 255]);
    assert_eq!(px(&c, 1, 0), [0, 255, 0, 255]);
    assert_eq!(px(&c, 0, 1), [0, 0, 255, 255]);
    assert_eq!(px(&c, 1, 1), [255, 255, 255, 255]);
    // Everything else is transparent.
    assert_eq!(px(&c, 2, 0), [0; 4]);
    assert_eq!(px(&c, 0, 2), [0; 4]);
    assert_eq!(px(&c, 15, 15), [0; 4]);
}

#[test]
fn a_canvas_crops_to_the_source_rectangle() {
    let c = canvas(&pixels(2, 2, &RGBA), rect(1, 0, 1, 2), (1, 2), 1, 1, CELL).unwrap();
    assert_eq!(px(&c, 0, 0), [0, 255, 0, 255]);
    assert_eq!(px(&c, 0, 1), [255, 255, 255, 255]);
    assert_eq!(px(&c, 1, 0), [0; 4]);
    // A rectangle past the image is cut to it.
    let c = canvas(&pixels(2, 2, &RGBA), rect(1, 1, 9, 9), (9, 9), 1, 1, CELL).unwrap();
    assert_eq!(px(&c, 0, 0), [255, 255, 255, 255]);
}

#[test]
fn a_canvas_scales_the_source_to_the_rendered_size() {
    // 2×2 up to 4×4: the corners keep their colours, the middle is a mix.
    let c = canvas(&pixels(2, 2, &RGBA), rect(0, 0, 2, 2), (4, 4), 1, 1, CELL).unwrap();
    assert_eq!(px(&c, 0, 0), [255, 0, 0, 255]);
    assert_eq!(px(&c, 3, 0), [0, 255, 0, 255]);
    assert_eq!(px(&c, 0, 3), [0, 0, 255, 255]);
    assert_eq!(px(&c, 3, 3), [255, 255, 255, 255]);
    let mid = px(&c, 1, 0);
    assert!(
        mid[0] > 0 && mid[0] < 255 && mid[1] > 0 && mid[1] < 255,
        "{mid:?}"
    );
    assert_eq!(mid[3], 255);
    assert_eq!(px(&c, 4, 0), [0; 4]);
}

#[test]
fn a_canvas_scales_down_and_blends_by_alpha() {
    // 4×1 transparent/opaque pairs down to 2×1: a transparent pixel adds no colour.
    let src = [
        255, 0, 0, 0, 0, 0, 255, 255, //
        255, 0, 0, 0, 0, 0, 255, 255,
    ];
    let c = canvas(&pixels(4, 1, &src), rect(0, 0, 4, 1), (2, 1), 1, 1, CELL).unwrap();
    let a = px(&c, 0, 0);
    assert_eq!(&a[..3], &[0, 0, 255], "{a:?}");
    assert!(a[3] > 0 && a[3] < 255);
}

#[test]
fn a_canvas_only_computes_what_lands_on_it() {
    // A 100 000-pixel-wide rendering on a one-cell canvas: the canvas is all there is.
    let c = canvas(
        &pixels(1, 1, &[9, 8, 7, 255]),
        rect(0, 0, 1, 1),
        (100_000, 100_000),
        1,
        1,
        CELL,
    )
    .unwrap();
    assert_eq!((c.width, c.height), (8, 16));
    assert_eq!(px(&c, 7, 15), [9, 8, 7, 255]);
}

#[test]
fn an_rgb_image_is_opaque() {
    let image = Pixels {
        width: 1,
        height: 1,
        format: ImageFormat::Rgb,
        data: vec![1, 2, 3],
    };
    let c = canvas(&image, rect(0, 0, 1, 1), (1, 1), 1, 1, CELL).unwrap();
    assert_eq!(px(&c, 0, 0), [1, 2, 3, 255]);
}

#[test]
fn a_canvas_over_64_mib_is_refused() {
    let image = pixels(1, 1, &[0, 0, 0, 255]);
    // 297 × 297 cells of 8×16: 45 MB, fine.
    assert!(canvas(&image, rect(0, 0, 1, 1), (1, 1), 297, 297, CELL).is_some());
    // The same cells of 12×24: 4·(3564·7128) = 101 MB.
    let big = CellPx {
        width: 12,
        height: 24,
    };
    assert!(u64::from(297u32 * 12) * u64::from(297u32 * 24) * 4 > MAX_CANVAS);
    assert!(canvas(&image, rect(0, 0, 1, 1), (1, 1), 297, 297, big).is_none());
    assert!(canvas(&image, rect(0, 0, 1, 1), (1, 1), 0, 1, CELL).is_none());
}

// Outer images and drawing.

fn placement(image: u32, col: i32, row: i32, cols: u16, rows: u16) -> Placement {
    Placement {
        screen: ImageScreen::Primary,
        image,
        generation: 1,
        kind: PlacementKind::Placed { col, row },
        cols,
        rows,
        source: rect(0, 0, 2, 2),
        pixels: (2, 2),
    }
}

fn image_msg(image: u32, generation: u64, data: &[u8]) -> ServerMsg {
    ServerMsg::Image {
        terminal: T,
        screen: ImageScreen::Primary,
        image,
        generation,
        width: 2,
        height: 2,
        format: ImageFormat::Rgba,
        offset: 0,
        total: 16,
        data: data.to_vec(),
    }
}

/// An app with one terminal (78×21), images on in Ghostty's way, a cell of 8×16 and the images
/// received.
fn app(images: &[u32]) -> App {
    let mut a = App::new(&no_sidebar(), (80, 24), &[]);
    a.cell = Some(CELL);
    a.images = Some(Images::new(None, None, [None; 16]));
    a.on_server(ServerMsg::SessionView(view(None)));
    for image in images {
        a.on_server(image_msg(*image, 1, &RGBA));
    }
    a
}

fn show(a: &mut App, text: &[&str], placements: Vec<Placement>) {
    let rows = text
        .iter()
        .enumerate()
        .map(|(i, l)| (u16::try_from(i).unwrap(), line(l)))
        .collect();
    let mut f = frame(true, (78, 21), rows);
    f.images = placements;
    a.on_server(ServerMsg::Frame(f));
}

/// The bytes the client writes before a draw, then after it.
fn cycle(a: &mut App) -> (String, String) {
    let mut before = Vec::new();
    a.send_images(&mut before).unwrap();
    let mut after = Vec::new();
    a.finish_images(&mut after).unwrap();
    (
        String::from_utf8_lossy(&before).into_owned(),
        String::from_utf8_lossy(&after).into_owned(),
    )
}

fn placeholder_at(buf: &ratatui::buffer::Buffer, x: u16, y: u16) -> Option<(u32, usize, usize)> {
    let cell = &buf[(x, y)];
    let mut chars = cell.symbol().chars();
    if chars.next()? != PLACEHOLDER {
        return None;
    }
    let index = |c: Option<char>| DIACRITICS.iter().position(|d| Some(*d) == c).unwrap();
    let (row, col) = (index(chars.next()), index(chars.next()));
    assert_eq!(chars.next(), None);
    let ratatui::style::Color::Rgb(r, g, b) = cell.fg else {
        panic!("not an id colour: {:?}", cell.fg);
    };
    Some((u32::from_be_bytes([0, r, g, b]), row, col))
}

#[test]
fn a_placed_image_is_sent_once_as_a_canvas_and_a_virtual_placement() {
    let mut a = app(&[7]);
    show(&mut a, &[], vec![placement(7, 1, 2, 3, 2)]);
    let (before, after) = cycle(&mut a);
    let id = a
        .images
        .as_ref()
        .unwrap()
        .placed_id(T, &placement(7, 1, 2, 3, 2), a.cell);
    let (id, cols, rows) = id.unwrap();
    assert_eq!((cols, rows), (3, 2));
    // 3×2 cells of 8×16 is 24×32 pixels.
    assert!(
        before.starts_with(&format!("\x1b_Ga=t,f=32,s=24,v=32,i={id},q=2,m=")),
        "{before:?}"
    );
    assert!(
        before.ends_with(&format!("\x1b_Ga=p,U=1,i={id},c=3,r=2,q=2\x1b\\")),
        "{before:?}"
    );
    assert_eq!(after, "");
    // Nothing changed: nothing is sent again.
    let (before, after) = cycle(&mut a);
    assert_eq!((before.as_str(), after.as_str()), ("", ""));
    assert_ne!(id, 0);
    assert!(id <= crate::graphics::LAST_ID);
}

#[test]
fn a_large_canvas_goes_in_chunks_of_4096() {
    let mut a = app(&[7]);
    show(&mut a, &[], vec![placement(7, 0, 0, 20, 10)]);
    let (before, _) = cycle(&mut a);
    // 160×160 pixels is 102 400 bytes: 34 chunks of 3072 raw bytes.
    let parts: Vec<&str> = before.split("\x1b\\").collect();
    assert_eq!(parts.len(), 34 + 1 + 1, "{}", parts.len());
    assert!(parts[0].contains(",m=1;"));
    assert!(parts[1].starts_with("\x1b_Gm=1,q=2;"));
    assert!(parts[33].starts_with("\x1b_Gm=0,q=2;"));
    assert!(parts[34].starts_with("\x1b_Ga=p,U=1,"));
}

#[test]
fn the_outer_image_waits_for_the_pixels() {
    let mut a = app(&[]);
    show(&mut a, &[], vec![placement(7, 0, 0, 2, 1)]);
    assert_eq!(cycle(&mut a), (String::new(), String::new()));
    let (_, buf_rows) = render(&a);
    assert!(buf_rows.iter().all(|r| !r.contains(PLACEHOLDER)));
    a.on_server(image_msg(7, 1, &RGBA));
    assert!(a.is_dirty());
    let (before, _) = cycle(&mut a);
    assert!(before.contains("a=t"));
}

#[test]
fn an_image_that_is_no_longer_placed_is_deleted_after_the_draw_and_its_id_is_reused() {
    let mut a = app(&[7, 8]);
    show(&mut a, &[], vec![placement(7, 0, 0, 2, 1)]);
    cycle(&mut a);
    let (id, ..) = a
        .images
        .as_ref()
        .unwrap()
        .placed_id(T, &placement(7, 0, 0, 2, 1), a.cell)
        .unwrap();
    show(&mut a, &[], vec![placement(8, 0, 0, 2, 1)]);
    let (before, after) = cycle(&mut a);
    // The new image goes out first, under another id; the old one goes after the draw.
    assert!(before.contains("a=t"));
    assert!(!before.contains(&format!("i={id},")));
    assert_eq!(after.as_bytes(), delete(id).as_slice());
    // Placing image 7 again uses the freed id.
    show(
        &mut a,
        &[],
        vec![placement(8, 0, 0, 2, 1), placement(7, 0, 5, 2, 1)],
    );
    let (before, _) = cycle(&mut a);
    assert!(before.contains(&format!("i={id},")), "{before:?}");
}

#[test]
fn another_size_or_cell_makes_another_outer_image() {
    let mut a = app(&[7]);
    show(&mut a, &[], vec![placement(7, 0, 0, 2, 1)]);
    cycle(&mut a);
    // Moving it is no new image.
    show(&mut a, &[], vec![placement(7, 3, 4, 2, 1)]);
    assert_eq!(cycle(&mut a), (String::new(), String::new()));
    // A font zoom is.
    a.cell = Some(CellPx {
        width: 9,
        height: 18,
    });
    let (before, after) = cycle(&mut a);
    assert!(before.contains("s=18,v=18"), "{before:?}");
    assert!(after.contains("a=d,d=I"), "{after:?}");
}

#[test]
fn leaving_the_panel_deletes_the_outer_images_and_coming_back_builds_them() {
    let mut a = app(&[7]);
    show(&mut a, &[], vec![placement(7, 0, 0, 2, 1)]);
    cycle(&mut a);
    // Another panel is active: the terminal is not on screen.
    let mut v = view(None);
    v.active_panel = None;
    v.projects[0].active_panel = None;
    a.on_server(ServerMsg::SessionView(v));
    let (before, after) = cycle(&mut a);
    assert_eq!(before, "");
    assert!(after.contains("a=d,d=I"));
    a.on_server(ServerMsg::SessionView(view(None)));
    let (before, _) = cycle(&mut a);
    assert!(before.contains("a=t"), "built again from the kept pixels");
}

#[test]
fn image_gone_deletes_the_outer_image() {
    let mut a = app(&[7]);
    show(&mut a, &[], vec![placement(7, 0, 0, 2, 1)]);
    cycle(&mut a);
    a.on_server(ServerMsg::ImageGone {
        terminal: T,
        screen: ImageScreen::Primary,
        image: 7,
        generation: 1,
    });
    show(&mut a, &[], vec![]);
    let (_, after) = cycle(&mut a);
    assert!(after.contains("a=d,d=I"));
}

#[test]
fn a_huge_placement_is_cut_to_297_cells_each_way_and_never_to_64_mib() {
    let mut a = app(&[7]);
    let mut p = placement(7, 0, 0, 60_000, 60_000);
    p.pixels = (60_000 * 8, 60_000 * 16);
    show(&mut a, &[], vec![p]);
    let (before, _) = cycle(&mut a);
    assert!(
        before.contains(&format!("s={},v={}", 297 * 8, 297 * 16)),
        "{}",
        &before[..80]
    );
    assert!(before.contains(",c=297,r=297,"));
    // With a bigger cell the canvas would pass 64 MiB: nothing is built or sent.
    let mut b = app(&[7]);
    b.cell = Some(CellPx {
        width: 12,
        height: 24,
    });
    show(&mut b, &[], vec![p]);
    assert_eq!(cycle(&mut b), (String::new(), String::new()));
    assert_eq!(cycle(&mut b), (String::new(), String::new()));
}

#[test]
fn nothing_is_sent_without_images_or_a_cell_size() {
    let mut a = app(&[7]);
    a.images = None;
    show(&mut a, &[], vec![placement(7, 0, 0, 2, 1)]);
    assert_eq!(cycle(&mut a), (String::new(), String::new()));
    assert!(!a.program_images());
    let mut b = app(&[7]);
    assert!(b.program_images());
    b.cell = None;
    assert!(!b.program_images());
    show(&mut b, &[], vec![placement(7, 0, 0, 2, 1)]);
    assert_eq!(cycle(&mut b), (String::new(), String::new()));
}

fn virtual_placement(image: u32, generation: u64, cols: u16, rows: u16) -> Placement {
    Placement {
        kind: PlacementKind::Virtual,
        generation,
        cols,
        rows,
        ..placement(image, 0, 0, cols, rows)
    }
}

#[test]
fn a_virtual_placement_sends_the_images_own_pixels_with_the_programs_size() {
    let mut a = app(&[7]);
    let p = virtual_placement(7, 1, 3, 2);
    show(&mut a, &[], vec![p]);
    let (before, after) = cycle(&mut a);
    let id = a.images.as_ref().unwrap().virtual_id(T, &p).unwrap();
    assert!(
        before.starts_with(&format!("\x1b_Ga=t,f=32,s=2,v=2,i={id},q=2,m=0;")),
        "{before:?}"
    );
    assert!(
        before.ends_with(&format!("\x1b_Ga=p,U=1,i={id},c=3,r=2,q=2\x1b\\")),
        "{before:?}"
    );
    assert_eq!(after, "");
    // Sent once; deleted when no placement is left.
    assert_eq!(cycle(&mut a), (String::new(), String::new()));
    show(&mut a, &[], vec![]);
    let (before, after) = cycle(&mut a);
    assert_eq!(before, "");
    assert_eq!(after.as_bytes(), delete(id).as_slice());
}

/// A mirror row of `runs`: each is its column, text and foreground.
fn row_of(runs: &[(u16, String, deck_core::protocol::Color)]) -> deck_core::protocol::Line {
    deck_core::protocol::Line {
        runs: runs
            .iter()
            .map(|(col, text, fg)| deck_core::protocol::Run {
                col: *col,
                text: text.clone(),
                style: deck_core::protocol::Style {
                    fg: *fg,
                    ..deck_core::protocol::Style::default()
                },
            })
            .collect(),
    }
}

/// A program placeholder cell: the character and the diacritics of the numbers.
fn cell(marks: &[usize]) -> String {
    std::iter::once(PLACEHOLDER)
        .chain(marks.iter().map(|n| DIACRITICS[*n]))
        .collect()
}

fn rgb(id: u32) -> deck_core::protocol::Color {
    let [_, r, g, b] = id.to_be_bytes();
    deck_core::protocol::Color::Rgb(r, g, b)
}

fn show_row(a: &mut App, line: deck_core::protocol::Line, placements: Vec<Placement>) {
    let mut f = frame(true, (78, 21), vec![(0, line)]);
    f.images = placements;
    a.on_server(ServerMsg::Frame(f));
}

/// The (row, column) of the cells of row 0, each `None` when it is not drawn as an image.
fn drawn(a: &App, from: u16, count: u16) -> (Option<u32>, Vec<Option<(usize, usize)>>) {
    let (buf, _) = render(a);
    let cells: Vec<_> = (from..from + count)
        .map(|x| placeholder_at(&buf, x, 0))
        .collect();
    let id = cells.iter().flatten().map(|c| c.0).next();
    (id, cells.iter().map(|c| c.map(|c| (c.1, c.2))).collect())
}

#[test]
fn a_program_placeholder_is_drawn_with_the_outer_id_and_explicit_diacritics() {
    let mut a = app(&[7]);
    let p = virtual_placement(7, 1, 4, 3);
    let text = cell(&[2, 3]);
    show_row(&mut a, row_of(&[(5, text, rgb(7))]), vec![p]);
    cycle(&mut a);
    let outer = a.images.as_ref().unwrap().virtual_id(T, &p).unwrap();
    let (id, got) = drawn(&a, 5, 1);
    assert_eq!(id, Some(outer));
    assert_eq!(got, vec![Some((2, 3))]);
}

#[test]
fn an_indexed_foreground_is_an_8_bit_id() {
    let mut a = app(&[7]);
    let p = virtual_placement(7, 1, 4, 3);
    let line = row_of(&[(0, cell(&[1, 1]), deck_core::protocol::Color::Indexed(7))]);
    show_row(&mut a, line, vec![p]);
    cycle(&mut a);
    assert_eq!(drawn(&a, 0, 1).1, vec![Some((1, 1))]);
}

#[test]
fn a_third_diacritic_is_the_high_byte_of_a_32_bit_id() {
    let mut a = app(&[0x0100_0007, 7]);
    let big = virtual_placement(0x0100_0007, 1, 2, 2);
    let small = virtual_placement(7, 1, 2, 2);
    // The same colour: the third diacritic (number 1) picks the 32-bit image.
    let line = row_of(&[(0, cell(&[0, 0, 1]), rgb(7)), (1, cell(&[0, 1]), rgb(7))]);
    show_row(&mut a, line, vec![big, small]);
    cycle(&mut a);
    let images = a.images.as_ref().unwrap();
    let (big_id, small_id) = (
        images.virtual_id(T, &big).unwrap(),
        images.virtual_id(T, &small).unwrap(),
    );
    assert_ne!(big_id, small_id);
    let (buf, _) = render(&a);
    assert_eq!(placeholder_at(&buf, 0, 0), Some((big_id, 0, 0)));
    assert_eq!(placeholder_at(&buf, 1, 0), Some((big_id, 0, 1)));
    // A cell continuing the 32-bit one (next column; or the row alone) keeps its high byte;
    // another column, or another row, is the 24-bit image.
    let line = row_of(&[
        (0, cell(&[0, 0, 1]), rgb(7)),
        (1, cell(&[0, 1]), rgb(7)),
        (2, cell(&[0]), rgb(7)),
        (3, cell(&[0, 7]), rgb(7)),
        (4, cell(&[1]), rgb(7)),
    ]);
    show_row(&mut a, line, vec![big, small]);
    cycle(&mut a);
    let (buf, _) = render(&a);
    assert_eq!(placeholder_at(&buf, 1, 0), Some((big_id, 0, 1)));
    assert_eq!(placeholder_at(&buf, 2, 0), Some((big_id, 0, 2)));
    assert_eq!(placeholder_at(&buf, 3, 0), Some((small_id, 0, 7)));
    assert_eq!(placeholder_at(&buf, 4, 0), Some((small_id, 1, 0)));
    // A bare cell after a 32-bit cell is the same image, one column on.
    let line = row_of(&[(0, cell(&[0, 0, 1]), rgb(7)), (1, cell(&[]), rgb(7))]);
    show_row(&mut a, line, vec![big, small]);
    cycle(&mut a);
    let (buf, _) = render(&a);
    assert_eq!(placeholder_at(&buf, 1, 0), Some((big_id, 0, 1)));
}

#[test]
fn cells_without_diacritics_inherit_the_row_and_the_next_column() {
    let mut a = app(&[7]);
    let p = virtual_placement(7, 1, 6, 6);
    let id = rgb(7);
    let line = row_of(&[
        // No left neighbour: row 0, column 0.
        (0, cell(&[]), id),
        // Row only, with the left cell in row 0: column 1.
        (1, cell(&[0]), id),
        // Row only, another row than the left one: column 0.
        (2, cell(&[4]), id),
        // Nothing: the left's row, its column + 1; then again.
        (3, cell(&[]), id),
        (4, cell(&[]), id),
        // Another id breaks the chain.
        (5, cell(&[]), rgb(9)),
        // A gap breaks it too.
        (8, cell(&[]), id),
    ]);
    show_row(&mut a, line, vec![p]);
    cycle(&mut a);
    let (_, got) = drawn(&a, 0, 9);
    assert_eq!(
        got,
        vec![
            Some((0, 0)),
            Some((0, 1)),
            Some((4, 0)),
            Some((4, 1)),
            Some((4, 2)),
            None,
            None,
            None,
            Some((0, 0)),
        ]
    );
}

#[test]
fn a_run_of_cells_split_across_mirror_runs_still_inherits() {
    let mut a = app(&[7]);
    let p = virtual_placement(7, 1, 6, 6);
    let line = row_of(&[
        (0, cell(&[2, 3]), rgb(7)),
        (1, format!("{}{}", cell(&[]), cell(&[])), rgb(7)),
    ]);
    show_row(&mut a, line, vec![p]);
    cycle(&mut a);
    assert_eq!(
        drawn(&a, 0, 3).1,
        vec![Some((2, 3)), Some((2, 4)), Some((2, 5))]
    );
}

#[test]
fn the_newest_generation_of_an_image_is_the_one_drawn() {
    let mut a = app(&[]);
    a.on_server(image_msg(7, 2, &RGBA));
    let (old, new) = (virtual_placement(7, 1, 2, 2), virtual_placement(7, 2, 3, 3));
    show_row(
        &mut a,
        row_of(&[(0, cell(&[0, 0]), rgb(7))]),
        vec![old, new],
    );
    cycle(&mut a);
    let images = a.images.as_ref().unwrap();
    assert_eq!(images.virtual_id(T, &old), None);
    let outer = images.virtual_id(T, &new).unwrap();
    assert_eq!(drawn(&a, 0, 1).0, Some(outer));
}

#[test]
fn an_unknown_or_unready_image_and_another_screen_stay_blank() {
    let line = || row_of(&[(0, format!("{}x", cell(&[0, 0])), rgb(7))]);
    let blank = |a: &App| {
        let (buf, rows) = render(a);
        assert_eq!(placeholder_at(&buf, 0, 0), None);
        assert!(rows[0].starts_with(" x"), "{:?}", rows[0]);
    };
    // No virtual placement.
    let mut a = app(&[7]);
    show_row(&mut a, line(), vec![]);
    cycle(&mut a);
    blank(&a);
    // Another image's placement.
    let mut a = app(&[7, 8]);
    show_row(&mut a, line(), vec![virtual_placement(8, 1, 2, 2)]);
    cycle(&mut a);
    blank(&a);
    // Pixels not in yet: nothing was sent.
    let mut a = app(&[]);
    show_row(&mut a, line(), vec![virtual_placement(7, 1, 2, 2)]);
    assert_eq!(cycle(&mut a), (String::new(), String::new()));
    blank(&a);
    // A placement of the other screen.
    let mut a = app(&[7]);
    let mut f = frame(true, (78, 21), vec![(0, line())]);
    f.images = vec![virtual_placement(7, 1, 2, 2)];
    f.modes = deck_core::protocol::Modes(deck_core::protocol::Modes::ALTERNATE_SCREEN);
    a.on_server(ServerMsg::Frame(f));
    cycle(&mut a);
    blank(&a);
    // Images off: no cell, no command.
    let mut a = app(&[7]);
    a.images = None;
    show_row(&mut a, line(), vec![virtual_placement(7, 1, 2, 2)]);
    assert_eq!(cycle(&mut a), (String::new(), String::new()));
    blank(&a);
}

#[test]
fn rows_and_columns_past_the_table_draw_blank() {
    let mut a = app(&[7]);
    let p = virtual_placement(7, 1, 6, 6);
    let id = rgb(7);
    let line = row_of(&[
        (0, cell(&[296, 295]), id),
        // Inherited: column 296, then 297, which has no diacritic.
        (1, cell(&[]), id),
        (2, cell(&[]), id),
    ]);
    show_row(&mut a, line, vec![p]);
    cycle(&mut a);
    assert_eq!(
        drawn(&a, 0, 3).1,
        vec![Some((296, 295)), Some((296, 296)), None]
    );
}

#[test]
fn the_cells_hold_placeholders_with_their_diacritics_and_the_id() {
    let mut a = app(&[7]);
    show(&mut a, &["abcdefgh"], vec![placement(7, 1, 0, 3, 2)]);
    cycle(&mut a);
    let (id, ..) = a
        .images
        .as_ref()
        .unwrap()
        .placed_id(T, &placement(7, 1, 0, 3, 2), a.cell)
        .unwrap();
    let (buf, _) = render(&a);
    assert_eq!(buf[(0, 0)].symbol(), "a");
    for row in 0..2u16 {
        for col in 0..3u16 {
            let got = placeholder_at(&buf, 1 + col, row).unwrap();
            assert_eq!(got, (id, usize::from(row), usize::from(col)));
            let cell = &buf[(1 + col, row)];
            assert_eq!(cell.fg, image_id_color(id));
            assert!(cell.modifier.is_empty());
        }
    }
    assert_eq!(buf[(4, 0)].symbol(), "e");
    assert_eq!(placeholder_at(&buf, 1, 2), None);
}

#[test]
fn the_cells_keep_the_background_underneath() {
    let mut a = app(&[7]);
    let mut f = frame(true, (78, 21), vec![]);
    f.lines = vec![(
        0,
        deck_core::protocol::Line {
            runs: vec![deck_core::protocol::Run {
                col: 0,
                text: "xyz".into(),
                style: deck_core::protocol::Style {
                    bg: deck_core::protocol::Color::Rgb(1, 2, 3),
                    ..deck_core::protocol::Style::default()
                },
            }],
        },
    )];
    f.images = vec![placement(7, 0, 0, 1, 1)];
    a.on_server(ServerMsg::Frame(f));
    cycle(&mut a);
    let (buf, _) = render(&a);
    assert!(placeholder_at(&buf, 0, 0).is_some());
    assert_eq!(buf[(0, 0)].bg, ratatui::style::Color::Rgb(1, 2, 3));
    assert_eq!(buf[(1, 0)].symbol(), "y");
}

#[test]
fn placements_are_clipped_at_the_terminal_and_negative_offsets_cut_the_top_left() {
    let mut a = app(&[7]);
    // Starts 1 column and 1 row above and left: its cell (1, 1) is the first one shown.
    show(&mut a, &[], vec![placement(7, -1, -1, 3, 3)]);
    cycle(&mut a);
    let (buf, _) = render(&a);
    assert_eq!(placeholder_at(&buf, 0, 0).map(|p| (p.1, p.2)), Some((1, 1)));
    assert_eq!(placeholder_at(&buf, 1, 1).map(|p| (p.1, p.2)), Some((2, 2)));
    // Cells 1 and 2 of the image's rows 1 and 2 are the last ones.
    assert_eq!(placeholder_at(&buf, 1, 0).map(|p| (p.1, p.2)), Some((1, 2)));
    assert_eq!(placeholder_at(&buf, 0, 1).map(|p| (p.1, p.2)), Some((2, 1)));
    assert_eq!(placeholder_at(&buf, 2, 2), None);

    // At the right and bottom edge of the 80×24 area.
    let mut b = app(&[7]);
    show(&mut b, &[], vec![placement(7, 78, 22, 5, 5)]);
    cycle(&mut b);
    let (buf, _) = render(&b);
    assert!(placeholder_at(&buf, 79, 23).is_some());
    assert!(placeholder_at(&buf, 78, 22).is_some());
}

#[test]
fn a_later_placement_covers_an_earlier_one() {
    let mut a = app(&[7, 8]);
    show(
        &mut a,
        &[],
        vec![placement(7, 0, 0, 3, 1), placement(8, 1, 0, 3, 1)],
    );
    cycle(&mut a);
    let (buf, _) = render(&a);
    let (first, ..) = placeholder_at(&buf, 0, 0).unwrap();
    let (second, row, col) = placeholder_at(&buf, 1, 0).unwrap();
    assert_ne!(first, second);
    assert_eq!((row, col), (0, 0));
    assert_eq!(placeholder_at(&buf, 3, 0).unwrap().0, second);
}

#[test]
fn a_wide_character_cut_by_the_rectangle_leaves_a_blank_cell() {
    let mut a = app(&[7]);
    // `界` takes columns 0 and 1; the image starts at column 1.
    show(&mut a, &["界x"], vec![placement(7, 1, 0, 1, 1)]);
    cycle(&mut a);
    let (buf, _) = render(&a);
    assert_eq!(buf[(0, 0)].symbol(), " ");
    assert!(placeholder_at(&buf, 1, 0).is_some());
    // One cut on its first half: the second is blank.
    let mut b = app(&[7]);
    show(&mut b, &["界x"], vec![placement(7, 0, 0, 1, 1)]);
    cycle(&mut b);
    let (buf, _) = render(&b);
    assert!(placeholder_at(&buf, 0, 0).is_some());
    assert_eq!(buf[(1, 0)].symbol(), " ");
    assert_eq!(buf[(2, 0)].symbol(), "x");
}

#[test]
fn an_image_without_its_outer_image_draws_nothing_and_images_off_draws_the_text() {
    let mut a = app(&[]);
    show(&mut a, &["text"], vec![placement(7, 0, 0, 2, 1)]);
    let (_, rows) = render(&a);
    assert!(rows[0].starts_with("text"));
    let mut b = app(&[7]);
    b.images = None;
    show(&mut b, &["text"], vec![placement(7, 0, 0, 2, 1)]);
    let (_, rows) = render(&b);
    assert!(rows[0].starts_with("text"));
}

#[test]
fn a_program_made_placeholder_is_drawn_blank_with_images_on_and_off() {
    let text = format!("a{PLACEHOLDER}\u{0305}\u{030D}b{PLACEHOLDER}c");
    for on in [true, false] {
        let mut a = app(&[]);
        if !on {
            a.images = None;
        }
        show(&mut a, &[&text], vec![]);
        let (buf, rows) = render(&a);
        assert!(rows[0].starts_with("a b c"), "{on}: {:?}", rows[0]);
        assert!(
            (0..80).all(|x| !buf[(x, 0)].symbol().contains(PLACEHOLDER)),
            "{on}"
        );
    }
}

#[test]
fn selecting_over_an_image_shows_the_image_without_the_highlight() {
    let mut a = app(&[7]);
    show(&mut a, &["hello world"], vec![placement(7, 2, 0, 2, 1)]);
    cycle(&mut a);
    let now = Instant::now();
    let mut mouse = |kind, column| {
        a.on_event(
            ct::Event::Mouse(ct::MouseEvent {
                kind,
                column,
                row: 0,
                modifiers: ct::KeyModifiers::NONE,
            }),
            now,
        )
    };
    mouse(ct::MouseEventKind::Down(ct::MouseButton::Left), 0);
    mouse(ct::MouseEventKind::Drag(ct::MouseButton::Left), 5);
    mouse(ct::MouseEventKind::Up(ct::MouseButton::Left), 5);
    let (buf, _) = render(&a);
    assert!(buf[(0, 0)].modifier.contains(Modifier::REVERSED));
    assert!(buf[(5, 0)].modifier.contains(Modifier::REVERSED));
    for x in 2..4 {
        assert!(placeholder_at(&buf, x, 0).is_some());
        assert!(!buf[(x, 0)].modifier.contains(Modifier::REVERSED), "{x}");
    }
}

#[test]
fn delete_all_covers_every_program_image() {
    let mut a = app(&[7, 8]);
    show(
        &mut a,
        &[],
        vec![placement(7, 0, 0, 2, 1), placement(8, 0, 3, 2, 1)],
    );
    cycle(&mut a);
    let mut out = Vec::new();
    a.delete_images(&mut out).unwrap();
    let out = String::from_utf8(out).unwrap();
    assert_eq!(out.matches("a=d,d=I").count(), 2, "{out:?}");
    // A new session forgets them: the next draw deletes them.
    a.reset_session();
    let (_, after) = cycle(&mut a);
    assert_eq!(after.matches("a=d,d=I").count(), 2);
}

#[test]
fn a_selected_image_cell_keeps_the_background_under_the_highlight() {
    let config = deck_core::config::Config {
        sidebar: false,
        theme: deck_core::theme::Theme {
            selection_bg: deck_core::protocol::Color::Rgb(9, 9, 9),
            ..deck_core::theme::Theme::default()
        },
        ..deck_core::config::Config::default()
    };
    let mut a = App::new(&config, (80, 24), &[]);
    a.cell = Some(CELL);
    a.images = Some(Images::new(None, None, [None; 16]));
    a.on_server(ServerMsg::SessionView(view(None)));
    a.on_server(image_msg(7, 1, &RGBA));
    show(&mut a, &["hello world"], vec![placement(7, 2, 0, 2, 1)]);
    cycle(&mut a);
    let now = Instant::now();
    for (kind, column) in [
        (ct::MouseEventKind::Down(ct::MouseButton::Left), 0),
        (ct::MouseEventKind::Drag(ct::MouseButton::Left), 5),
        (ct::MouseEventKind::Up(ct::MouseButton::Left), 5),
    ] {
        a.on_event(
            ct::Event::Mouse(ct::MouseEvent {
                kind,
                column,
                row: 0,
                modifiers: ct::KeyModifiers::NONE,
            }),
            now,
        );
    }
    let (buf, _) = render(&a);
    let lit = ratatui::style::Color::Rgb(9, 9, 9);
    assert_eq!(buf[(0, 0)].bg, lit);
    assert_eq!(buf[(5, 0)].bg, lit);
    for x in 2..4 {
        assert!(placeholder_at(&buf, x, 0).is_some());
        assert_ne!(buf[(x, 0)].bg, lit, "{x}");
    }
}
