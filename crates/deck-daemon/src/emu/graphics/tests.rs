use deck_core::protocol::CellPx;
use deck_core::protocol::ScrollTo;

use super::*;
use crate::emu::files::Scanner;

const CELL: CellPx = CellPx {
    width: 9,
    height: 18,
};

fn emu() -> Emu {
    let mut e = Emu::new(80, 24, 100).expect("emulator");
    e.resize_with_cell(80, 24, Some(CELL));
    e
}

fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ABC[(n >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A kitty graphics command: `keys` and a payload.
fn apc(keys: &str, payload: &[u8]) -> Vec<u8> {
    format!("\x1b_G{keys};{}\x1b\\", base64(payload)).into_bytes()
}

fn replies(e: &mut Emu, input: &[u8]) -> Vec<String> {
    e.feed(input)
        .into_iter()
        .filter_map(|ev| match ev {
            super::super::EmuEvent::Reply(s) => Some(s),
            _ => None,
        })
        .collect()
}

/// Feeds the emulator what the PTY reader would: the output after the file scanner.
fn scanned_replies(e: &mut Emu, files: &mut Scanner, input: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    files.scan(input, |piece| out.extend(raw_replies(e, piece)));
    out
}

fn rgba(width: u32, height: u32) -> Vec<u8> {
    (0..width * height)
        .flat_map(|i| [i as u8, 10, 20, 255])
        .collect()
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for b in bytes {
        crc ^= u32::from(*b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                crc >> 1 ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn chunk(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = (body.len() as u32).to_be_bytes().to_vec();
    let mut typed = kind.to_vec();
    typed.extend_from_slice(body);
    out.extend_from_slice(&typed);
    out.extend_from_slice(&crc32(&typed).to_be_bytes());
    out
}

/// A PNG of `width` by `height` whose scanlines (without the filter bytes) are `rows`.
fn png(width: u32, height: u32, depth: u8, color: u8, palette: &[u8], rows: &[u8]) -> Vec<u8> {
    let mut header = width.to_be_bytes().to_vec();
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[depth, color, 0, 0, 0]);
    let stride = rows.len() / height as usize;
    let mut raw = Vec::new();
    for line in rows.chunks(stride) {
        raw.push(0);
        raw.extend_from_slice(line);
    }
    // A zlib stream of one stored block.
    let len = raw.len() as u16;
    let mut zlib = vec![0x78, 0x01, 1];
    zlib.extend_from_slice(&len.to_le_bytes());
    zlib.extend_from_slice(&(!len).to_le_bytes());
    zlib.extend_from_slice(&raw);
    let (mut a, mut b) = (1u32, 0u32);
    for byte in &raw {
        a = (a + u32::from(*byte)) % 65521;
        b = (b + a) % 65521;
    }
    zlib.extend_from_slice(&(b << 16 | a).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    out.extend(chunk(b"IHDR", &header));
    if !palette.is_empty() {
        out.extend(chunk(b"PLTE", palette));
    }
    out.extend(chunk(b"IDAT", &zlib));
    out.extend(chunk(b"IEND", &[]));
    out
}

fn only(e: &mut Emu) -> Placement {
    let all = e.placements();
    assert_eq!(all.len(), 1, "{all:?}");
    all[0]
}

fn placed(col: i32, row: i32) -> PlacementKind {
    PlacementKind::Placed { col, row }
}

#[test]
fn a_support_query_gets_an_ok_answer() {
    let mut e = emu();
    let got = replies(&mut e, &apc("a=q,i=31,s=1,v=1,f=24", &[1, 2, 3]));
    assert_eq!(got, ["\x1b_Gi=31;OK\x1b\\"]);
}

#[test]
fn a_transmitted_rgba_image_is_placed_with_its_grid_and_pixels() {
    let mut e = emu();
    let pixels = rgba(18, 36);
    // At the cursor (column 3, row 2).
    e.feed(b"\x1b[3;4H");
    e.feed(&apc("a=T,f=32,s=18,v=36,i=7,C=1", &pixels));
    let p = only(&mut e);
    assert_eq!(p.screen, ImageScreen::Primary);
    assert_eq!(p.image, 7);
    assert_eq!(p.kind, placed(3, 2));
    // 18x36 pixels in 9x18 cells.
    assert_eq!((p.cols, p.rows), (2, 2));
    assert_eq!(
        p.source,
        SourceRect {
            x: 0,
            y: 0,
            width: 18,
            height: 36
        }
    );
    assert_eq!(p.pixels, (18, 36));
    let got = e.image(ImageScreen::Primary, 7).expect("pixels");
    assert_eq!(got.generation, p.generation);
    assert_eq!((got.width, got.height), (18, 36));
    assert_eq!(got.format, ImageFormat::Rgba);
    assert_eq!(got.data, pixels);
}

#[test]
fn an_rgb_image_keeps_three_bytes_per_pixel() {
    let mut e = emu();
    let pixels: Vec<u8> = (0..2 * 2 * 3).collect();
    e.feed(&apc("a=T,f=24,s=2,v=2,i=1,c=1,r=1", &pixels));
    let got = e.image(ImageScreen::Primary, 1).expect("pixels");
    assert_eq!(got.format, ImageFormat::Rgb);
    assert_eq!(got.data, pixels);
}

#[test]
fn a_png_is_decoded_to_rgba() {
    let mut e = emu();
    let rows = [255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 9, 9, 9, 255];
    e.feed(&apc("a=T,f=100,i=2", &png(2, 2, 8, 6, &[], &rows)));
    let p = only(&mut e);
    // 2x2 pixels in 9x18 cells: one cell.
    assert_eq!(p.kind, placed(0, 0));
    assert_eq!((p.cols, p.rows), (1, 1));
    assert_eq!(p.source.width, 2);
    let got = e.image(ImageScreen::Primary, 2).expect("pixels");
    assert_eq!(got.format, ImageFormat::Rgba);
    assert_eq!((got.width, got.height), (2, 2));
    assert_eq!(got.data, rows);
}

/// Feeds the emulator without the rewriting of file commands, as Ghostty alone would see them.
fn raw_replies(e: &mut Emu, input: &[u8]) -> Vec<String> {
    e.feed(input)
        .into_iter()
        .filter_map(|ev| match ev {
            super::super::EmuEvent::Reply(s) => Some(s),
            _ => None,
        })
        .collect()
}

#[test]
fn the_emulators_own_file_temp_file_and_shared_memory_mediums_are_off() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("pixel.png");
    std::fs::write(&path, png(1, 1, 8, 6, &[], &[1, 2, 3, 4])).expect("write");
    let path = path.to_str().expect("utf-8").as_bytes();
    for (medium, target) in [
        ("f", path),
        ("t", path),
        ("s", b"/deck-test-shm".as_slice()),
    ] {
        let mut e = emu();
        let got = raw_replies(
            &mut e,
            &apc(&format!("a=T,f=100,t={medium},i=3,c=1,r=1"), target),
        );
        assert_eq!(got.len(), 1, "t={medium}: {got:?}");
        assert!(got[0].starts_with("\x1b_Gi=3;E"), "t={medium}: {got:?}");
        assert!(e.placements().is_empty(), "t={medium}");
        assert!(e.image(ImageScreen::Primary, 3).is_none(), "t={medium}");
    }
}

#[test]
fn a_png_read_from_a_file_is_stored_and_listed() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("pixel.png");
    std::fs::write(&path, png(1, 1, 8, 6, &[], &[1, 2, 3, 4])).expect("write");
    let mut e = emu();
    let mut files = Scanner::default();
    let got = scanned_replies(
        &mut e,
        &mut files,
        &apc(
            "a=T,f=100,t=f,i=3,c=1,r=1",
            path.to_str().expect("utf-8").as_bytes(),
        ),
    );
    assert_eq!(got, ["\x1b_Gi=3;OK\x1b\\"]);
    assert_eq!(only(&mut e).image, 3);
    assert_eq!(
        e.image(ImageScreen::Primary, 3).expect("pixels").data,
        [1, 2, 3, 4]
    );
    // The same command split in two chunks of output.
    let mut e = emu();
    let mut files = Scanner::default();
    let cmd = apc(
        "a=T,f=100,t=f,i=4,c=1,r=1",
        path.to_str().expect("utf-8").as_bytes(),
    );
    let (a, b) = cmd.split_at(cmd.len() / 2);
    assert!(scanned_replies(&mut e, &mut files, a).is_empty());
    assert_eq!(
        scanned_replies(&mut e, &mut files, b),
        ["\x1b_Gi=4;OK\x1b\\"]
    );
}

#[test]
fn a_path_to_a_fifo_does_not_block_the_emulator() {
    let dir = tempfile::tempdir().expect("temp dir");
    let fifo = dir.path().join("pipe");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("mkfifo");
    assert!(made.success());
    let target = fifo.to_str().expect("utf-8").to_string();
    let (done, finished) = std::sync::mpsc::channel();
    // On its own thread: a regression would block here for good, and must not hang the suite.
    std::thread::spawn(move || {
        let mut e = emu();
        let got = replies(&mut e, &apc("a=T,f=100,t=f,i=3", target.as_bytes()));
        let _ = done.send((got, e.placements().len()));
    });
    let (got, placed) = finished
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the emulator returned");
    assert!(got.iter().all(|r| !r.contains(";OK")), "{got:?}");
    assert_eq!(placed, 0);
}

/// The crate's decoder expands palette and 16-bit PNGs to RGBA8 but leaves greyscale ones at
/// one or two bytes per pixel, which Ghostty rejects; deck's wrapper expands those too.
#[test]
fn grey_palette_and_sixteen_bit_pngs_come_out_as_rgba() {
    let cases: [(&str, Vec<u8>, Vec<u8>); 5] = [
        (
            "grey",
            png(2, 1, 8, 0, &[], &[10, 200]),
            vec![10, 10, 10, 255, 200, 200, 200, 255],
        ),
        (
            "grey alpha",
            png(2, 1, 8, 4, &[], &[10, 255, 200, 7]),
            vec![10, 10, 10, 255, 200, 200, 200, 7],
        ),
        (
            "palette",
            png(2, 1, 8, 3, &[255, 0, 0, 0, 0, 255], &[0, 1]),
            vec![255, 0, 0, 255, 0, 0, 255, 255],
        ),
        (
            "rgb16",
            png(1, 1, 16, 2, &[], &[0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc]),
            vec![0x12, 0x56, 0x9a, 255],
        ),
        (
            "grey16",
            png(1, 1, 16, 0, &[], &[0x80, 0x00]),
            vec![0x80, 0x80, 0x80, 255],
        ),
    ];
    for (name, bytes, expected) in cases {
        let mut e = emu();
        let got = replies(&mut e, &apc("a=T,f=100,i=1", &bytes));
        assert_eq!(got, ["\x1b_Gi=1;OK\x1b\\"], "{name}");
        let p = only(&mut e);
        assert_eq!((p.kind, p.cols, p.rows), (placed(0, 0), 1, 1), "{name}");
        let pixels = e
            .image(ImageScreen::Primary, 1)
            .unwrap_or_else(|| panic!("{name}: no pixels"));
        assert_eq!(pixels.format, ImageFormat::Rgba, "{name}");
        assert_eq!(pixels.data, expected, "{name}");
    }
}

#[test]
fn grey_formats_expand_to_rgb_and_rgba() {
    assert_eq!(
        pixels_of(kg::ImageFormat::Gray, 2, 1, &[5, 6]),
        Ok((ImageFormat::Rgb, vec![5, 5, 5, 6, 6, 6]))
    );
    assert_eq!(
        pixels_of(kg::ImageFormat::GrayAlpha, 1, 2, &[5, 1, 6, 2]),
        Ok((ImageFormat::Rgba, vec![5, 5, 5, 1, 6, 6, 6, 2]))
    );
    assert!(pixels_of(kg::ImageFormat::Gray, 2, 2, &[5, 6]).is_err());
    assert!(pixels_of(kg::ImageFormat::Png, 1, 1, &[0; 4]).is_err());
}

/// A PNG that is a header declaring `width` by `height`, four bytes of data and the end.
fn png_header_only(width: u32, height: u32) -> Vec<u8> {
    let mut header = width.to_be_bytes().to_vec();
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    out.extend(chunk(b"IHDR", &header));
    out.extend(chunk(b"IDAT", &[0x78, 0x01, 0x01, 0x00]));
    out.extend(chunk(b"IEND", &[]));
    out
}

fn resident_bytes() -> u64 {
    crate::platform::self_stats().rss_bytes.expect("rss")
}

#[test]
fn a_png_too_short_for_the_size_it_declares_is_refused_before_decoding() {
    // 111 bytes of output declaring 4096 by 4096: within the storage limit, but deflate
    // cannot make 64 MiB of scanlines out of a few bytes.
    let tiny = png_header_only(4096, 4096);
    assert!(!png_can_hold_its_size(&tiny));
    assert!(!png_can_hold_its_size(b"not a png, however long it is"));
    // What a real, small or flat image declares is let through.
    assert!(png_can_hold_its_size(&png(2, 2, 8, 6, &[], &rgba(2, 2))));
    let mut e = emu();
    // Without the check the decoder allocates and fills the declared 64 MiB before it finds
    // the data short, and keeps the buffer.
    let before = resident_bytes();
    let got = replies(&mut e, &apc("a=T,f=100,i=3", &tiny));
    assert!(got[0].starts_with("\x1b_Gi=3;EINVAL"), "{got:?}");
    let grown = resident_bytes().saturating_sub(before);
    assert!(grown < 32 << 20, "resident memory grew by {grown} bytes");
    // One bit per pixel: the scanlines are 1/32 of the RGBA, so they are what counts.
    let mut flat = png_header_only(1024, 1024);
    flat[24..26].copy_from_slice(&[1, 0]);
    assert_eq!(declared_scanline_bytes(&flat), Some(1024 * 129));
}

#[test]
fn a_png_declaring_more_than_the_storage_limit_is_refused_before_decoding() {
    let huge = png_header_only(16384, 16384);
    assert_eq!(declared_png_bytes(&huge), Some(1 << 30));
    assert_eq!(
        declared_png_bytes(&png_header_only(4096, 4096)),
        Some(STORAGE_LIMIT)
    );
    assert_eq!(
        declared_png_bytes(&png_header_only(u32::MAX, u32::MAX)),
        None
    );
    assert_eq!(declared_png_bytes(&huge[..23]), None);
    assert_eq!(declared_png_bytes(b"not a png, however long it is"), None);

    // 107 bytes of output: without the gate the decoder zero-fills 1 GiB and keeps it.
    let mut e = emu();
    let before = resident_bytes();
    let got = replies(&mut e, &apc("a=T,f=100,i=7", &huge));
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(got[0].starts_with("\x1b_Gi=7;EINVAL"), "{got:?}");
    let grown = resident_bytes().saturating_sub(before);
    assert!(grown < 256 << 20, "resident memory grew by {grown} bytes");
    assert!(e.image(ImageScreen::Primary, 7).is_none());

    // One pixel row over the limit is refused; a PNG within it still decodes.
    let got = replies(&mut e, &apc("a=T,f=100,i=8", &png_header_only(4096, 4097)));
    assert!(got[0].starts_with("\x1b_Gi=8;EINVAL"), "{got:?}");
    let small = png(2, 2, 8, 6, &[], &rgba(2, 2));
    assert_eq!(
        replies(&mut e, &apc("a=T,f=100,i=9", &small)),
        ["\x1b_Gi=9;OK\x1b\\"]
    );
    assert!(e.image(ImageScreen::Primary, 9).is_some());
}

#[test]
fn a_malformed_png_is_rejected_with_an_error_answer() {
    let mut e = emu();
    let got = replies(&mut e, &apc("a=T,f=100,i=5", b"not a png at all"));
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(got[0].starts_with("\x1b_Gi=5;E"), "{got:?}");
    assert!(e.placements().is_empty());
    assert!(e.image(ImageScreen::Primary, 5).is_none());
    // A PNG cut short.
    let whole = png(2, 2, 8, 6, &[], &rgba(2, 2));
    let got = replies(&mut e, &apc("a=T,f=100,i=6", &whole[..whole.len() - 20]));
    assert!(got.iter().all(|r| !r.contains(";OK")), "{got:?}");
    // The emulator still works.
    assert_eq!(
        replies(&mut e, &apc("a=q,i=9,s=1,v=1,f=24", &[1, 2, 3])),
        ["\x1b_Gi=9;OK\x1b\\"]
    );
}

#[test]
fn an_image_half_scrolled_out_has_a_negative_row_and_scrollback_moves_it_back() {
    let mut e = emu();
    // Four rows tall at the top of the screen; the cursor stays.
    e.feed(&apc("a=T,f=32,s=9,v=72,i=1,c=1,r=4,C=1", &rgba(9, 72)));
    assert_eq!(only(&mut e).kind, placed(0, 0));
    // Two lines scroll off the top.
    e.feed(b"\x1b[24;1H\n\n");
    let p = only(&mut e);
    assert_eq!(p.kind, placed(0, -2));
    assert_eq!(p.rows, 4);
    // The display back by one line shows it one row lower, and by two at its first row.
    e.scroll(ScrollTo::Lines(1));
    assert_eq!(only(&mut e).kind, placed(0, -1));
    e.scroll(ScrollTo::Lines(1));
    assert_eq!(only(&mut e).kind, placed(0, 0));
    e.scroll(ScrollTo::Bottom);
    assert_eq!(only(&mut e).kind, placed(0, -2));
    // Scrolled all the way off the top, nothing touches the viewport.
    e.feed(b"\n\n\n\n");
    assert!(e.placements().is_empty());
}

#[test]
fn a_virtual_placement_is_listed_once_as_virtual() {
    let mut e = emu();
    e.feed(&apc("a=T,f=32,s=9,v=36,i=4,U=1,c=3,r=2", &rgba(9, 36)));
    let p = only(&mut e);
    assert_eq!(p.kind, PlacementKind::Virtual);
    assert_eq!((p.cols, p.rows), (3, 2));
    assert_eq!(p.image, 4);
    // A second virtual placement of the image is not listed on its own.
    e.feed(&apc("a=p,i=4,p=2,U=1,c=5,r=4", &[]));
    let p = only(&mut e);
    assert_eq!(p.kind, PlacementKind::Virtual);
}

#[test]
fn an_image_on_the_alternate_screen_is_listed_with_that_screen() {
    let mut e = emu();
    e.feed(&apc("a=T,f=32,s=1,v=1,i=1,c=1,r=1", &[1, 2, 3, 4]));
    e.feed(b"\x1b[?1049h");
    // Each screen has its own images: the primary one is not listed here.
    assert!(e.placements().is_empty());
    assert!(e.image(ImageScreen::Primary, 1).is_none());
    e.feed(&apc("a=T,f=32,s=1,v=1,i=1,c=1,r=1", &[5, 6, 7, 8]));
    let p = only(&mut e);
    assert_eq!(p.screen, ImageScreen::Alternate);
    assert_eq!(
        e.image(ImageScreen::Alternate, 1).expect("pixels").data,
        [5, 6, 7, 8]
    );
    e.feed(b"\x1b[?1049l");
    assert_eq!(only(&mut e).screen, ImageScreen::Primary);
    assert_eq!(
        e.image(ImageScreen::Primary, 1).expect("pixels").data,
        [1, 2, 3, 4]
    );
}

#[test]
fn a_huge_column_count_saturates() {
    let mut e = emu();
    e.feed(&apc("a=T,f=32,s=1,v=1,i=1,c=70000,r=1", &[1, 2, 3, 4]));
    let p = only(&mut e);
    assert_eq!(p.cols, u16::MAX);
}

#[test]
fn placements_are_listed_lower_z_first() {
    let mut e = emu();
    e.feed(&apc("a=T,f=32,s=1,v=1,i=1,c=1,r=1,z=5,C=1", &[1, 2, 3, 4]));
    e.feed(&apc("a=T,f=32,s=1,v=1,i=2,c=1,r=1,z=-3,C=1", &[1, 2, 3, 4]));
    e.feed(&apc("a=T,f=32,s=1,v=1,i=3,c=1,r=1,z=0,C=1", &[1, 2, 3, 4]));
    let order: Vec<u32> = e.placements().iter().map(|p| p.image).collect();
    assert_eq!(order, [2, 3, 1]);
}

#[test]
fn deleting_a_placement_removes_it() {
    let mut e = emu();
    e.feed(&apc("a=T,f=32,s=1,v=1,i=1,c=1,r=1", &[1, 2, 3, 4]));
    assert_eq!(e.placements().len(), 1);
    e.feed(b"\x1b_Ga=d,d=i,i=1\x1b\\");
    assert!(e.placements().is_empty());
    e.feed(&apc("a=T,f=32,s=1,v=1,i=2,c=1,r=1", &[1, 2, 3, 4]));
    e.feed(b"\x1b_Ga=d,d=A\x1b\\");
    assert!(e.placements().is_empty());
    assert!(e.image(ImageScreen::Primary, 2).is_none());
}

#[test]
fn transmitting_an_image_again_changes_its_generation() {
    let mut e = emu();
    e.feed(&apc("a=T,f=32,s=1,v=1,i=1,c=1,r=1", &[1, 2, 3, 4]));
    let first = only(&mut e).generation;
    e.feed(&apc("a=T,f=32,s=1,v=1,i=1,c=1,r=1", &[9, 9, 9, 9]));
    let p = only(&mut e);
    assert_ne!(p.generation, first);
    assert_eq!(
        e.image(ImageScreen::Primary, 1).expect("pixels").data,
        [9, 9, 9, 9]
    );
}

#[test]
fn the_storage_limit_evicts_the_oldest_image() {
    let mut e = emu();
    e.term.set_kitty_image_storage_limit(900).expect("limit");
    let pixels = rgba(10, 10);
    for id in 1..=3 {
        e.feed(&apc(&format!("a=t,f=32,s=10,v=10,i={id}"), &pixels));
    }
    assert!(e.image(ImageScreen::Primary, 1).is_none());
    assert!(e.image(ImageScreen::Primary, 3).is_some());
}

#[test]
fn a_zlib_compressed_image_is_read_back_uncompressed() {
    let mut e = emu();
    let pixels = rgba(2, 2);
    // A zlib stream of one stored block.
    let len = pixels.len() as u16;
    let mut z = vec![0x78, 0x01, 1];
    z.extend_from_slice(&len.to_le_bytes());
    z.extend_from_slice(&(!len).to_le_bytes());
    z.extend_from_slice(&pixels);
    let (mut a, mut b) = (1u32, 0u32);
    for byte in &pixels {
        a = (a + u32::from(*byte)) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&(b << 16 | a).to_be_bytes());
    e.feed(&apc("a=T,f=32,s=2,v=2,i=1,o=z,c=1,r=1", &z));
    assert_eq!(
        e.image(ImageScreen::Primary, 1).expect("pixels").data,
        pixels
    );
}

#[test]
fn change_detection_sees_images_and_their_movement() {
    let mut e = emu();
    assert!(!e.images_changed());
    e.feed(&apc("a=T,f=32,s=9,v=36,i=1,c=1,r=2,C=1", &rgba(9, 36)));
    assert!(e.images_changed());
    assert!(!e.images_changed());
    // Text that does not touch the image.
    e.feed(b"hello");
    assert!(!e.images_changed());
    // The placement moves when the screen scrolls.
    e.feed(b"\x1b[24;1H\n");
    assert!(e.images_changed());
    assert!(!e.images_changed());
    e.feed(b"\x1b_Ga=d,d=A\x1b\\");
    assert!(e.images_changed());
    assert!(!e.images_changed());
}

#[test]
fn the_png_decoder_installs_on_every_emulator_thread() {
    let threads: Vec<_> = (0..4)
        .map(|_| {
            std::thread::spawn(|| {
                let mut e = emu();
                let got = replies(
                    &mut e,
                    &apc(
                        "a=T,f=100,i=1,c=1,r=1",
                        &png(1, 1, 8, 6, &[], &[1, 2, 3, 4]),
                    ),
                );
                got == ["\x1b_Gi=1;OK\x1b\\"]
            })
        })
        .collect();
    for t in threads {
        assert!(t.join().expect("thread"));
    }
}

#[test]
fn placing_an_image_over_blank_cells_gives_a_frame_that_lists_it() {
    use deck_core::model::TerminalId;
    use std::time::Duration;

    let mut e = emu();
    let id = TerminalId(1);
    e.feed(&apc("a=t,f=32,s=1,v=1,i=1,q=2", &[1, 2, 3, 4]));
    assert!(e.damage_frame(id, Duration::ZERO).is_some());
    assert!(
        e.damage_frame(id, Duration::ZERO)
            .unwrap()
            .images
            .is_empty()
    );
    // The cursor stays and no cell is written: only the placement is new.
    e.feed(b"\x1b_Ga=p,i=1,c=2,r=1,C=1,q=2\x1b\\");
    let frame = e.damage_frame(id, Duration::ZERO).expect("a frame");
    assert!(!frame.full);
    assert_eq!(frame.images.len(), 1);
    assert_eq!((frame.images[0].cols, frame.images[0].rows), (2, 1));
}
