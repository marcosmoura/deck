//! Logos as images: detection, colours, the escape sequences, the registry and the cells.

use std::collections::HashMap;

use deck_core::agents::{AgentKind, AgentStatus};
use deck_core::config::Config;
use ratatui::buffer::Buffer;
use ratatui::style::Color;

use deck_core::icons::{Icon, IconKind};

use crate::app::App;
use crate::convert::image_id_color;
use crate::graphics::{
    Images, LAST_ID, Pair, delete, detect, has_image, mask, placeholder, transmit,
};
use crate::icons_tests::{agent, app_with, render, titled};
use crate::tier::Tier;

fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

#[test]
fn detection_table() {
    let auto = |pairs: &[(&str, &str)]| detect(&env(pairs), true);
    // On: Ghostty and kitty.
    assert!(auto(&[
        ("TERM_PROGRAM", "ghostty"),
        ("TERM", "xterm-256color")
    ]));
    assert!(auto(&[("TERM", "xterm-ghostty")]));
    assert!(auto(&[("TERM", "xterm-kitty")]));
    assert!(auto(&[
        ("TERM", "xterm-256color"),
        ("KITTY_WINDOW_ID", "1")
    ]));
    // Off: multiplexers and deck itself, whatever the rest says.
    for var in ["TMUX", "ZELLIJ", "DECK"] {
        assert!(
            !auto(&[("TERM", "xterm-ghostty"), (var, "/tmp/x,1,0")]),
            "{var}"
        );
        // An empty value is not set.
        assert!(auto(&[("TERM", "xterm-ghostty"), (var, "")]), "{var}");
    }
    assert!(!auto(&[
        ("TERM", "screen-256color"),
        ("KITTY_WINDOW_ID", "1")
    ]));
    assert!(!auto(&[
        ("TERM", "tmux-256color"),
        ("TERM_PROGRAM", "ghostty")
    ]));
    // Off: other terminals, and nothing known.
    assert!(!auto(&[("TERM", "xterm-256color")]));
    assert!(!auto(&[
        ("TERM", "xterm-256color"),
        ("KITTY_WINDOW_ID", "")
    ]));
    assert!(!auto(&[]));
    // Off: 256 colours.
    let ghostty = env(&[("TERM_PROGRAM", "ghostty")]);
    assert!(detect(&ghostty, true));
    assert!(!detect(&ghostty, false));
}

fn images() -> Images {
    let mut palette = [None; 16];
    palette[4] = Some((1, 2, 3));
    Images::new(Some((10, 20, 30)), Some((1, 1, 1)), palette)
}

#[test]
fn colours_resolve_to_rgb() {
    let i = images();
    assert_eq!(i.resolve(Color::Rgb(5, 6, 7)), (5, 6, 7));
    // `default` is the outer foreground, or #c0c0c0 without an answer.
    assert_eq!(i.resolve(Color::Reset), (10, 20, 30));
    assert_eq!(
        Images::new(None, None, [None; 16]).resolve(Color::Reset),
        (0xc0, 0xc0, 0xc0)
    );
    // Indexed 0..=15: the terminal's own answer, else the xterm value.
    assert_eq!(i.resolve(Color::Indexed(4)), (1, 2, 3));
    assert_eq!(i.resolve(Color::Blue), (1, 2, 3));
    assert_eq!(
        Images::new(None, None, [None; 16]).resolve(Color::Indexed(4)),
        (0x00, 0x00, 0xee)
    );
    assert_eq!(i.resolve(Color::Indexed(1)), (0xcd, 0, 0));
    assert_eq!(i.resolve(Color::DarkGray), (0x7f, 0x7f, 0x7f));
    // The cube and the grey ramp.
    assert_eq!(i.resolve(Color::Indexed(110)), (135, 175, 215));
    assert_eq!(i.resolve(Color::Indexed(16)), (0, 0, 0));
    assert_eq!(i.resolve(Color::Indexed(231)), (255, 255, 255));
    assert_eq!(i.resolve(Color::Indexed(244)), (128, 128, 128));
    assert_eq!(i.resolve(Color::Indexed(232)), (8, 8, 8));
    assert_eq!(i.resolve(Color::Indexed(255)), (238, 238, 238));
    // The cube is the inverse of `nearest_256`.
    for n in 16..=231u8 {
        let (r, g, b) = i.resolve(Color::Indexed(n));
        assert_eq!(crate::convert::nearest_256(r, g, b), n);
    }
}

#[test]
fn every_icon_but_the_drop_caret_has_a_mask_of_64_by_64_that_is_not_empty() {
    for icon in Icon::ALL {
        let Some(mask) = mask(icon) else {
            assert_eq!(icon, Icon::DropTarget);
            continue;
        };
        assert_eq!(mask.len(), 4096);
        assert!(mask.iter().any(|&a| a != 0), "{icon:?}");
    }
    // Codex uses OpenAI's mark; the masks of different icons differ.
    assert_eq!(
        mask(Icon::Kind(IconKind::Codex)),
        Some(include_bytes!("../assets/icons/openai.mask"))
    );
    assert_ne!(mask(Icon::Idle), mask(Icon::Quiet));
    assert_ne!(mask(Icon::Quiet), mask(Icon::Unseen));
    assert!(has_image(Icon::Waiting) && has_image(Icon::Spinner) && !has_image(Icon::DropTarget));
}

fn decode(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for c in text.bytes().filter(|&c| c != b'=') {
        let v = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
            .iter()
            .position(|&a| a == c)
            .unwrap();
        acc = (acc << 6) | u32::try_from(v).unwrap();
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((acc >> bits) & 0xff).unwrap());
        }
    }
    out
}

#[test]
fn the_transmit_sequence_is_chunked_and_decodes_to_the_tinted_image() {
    let pair = Pair {
        icon: Icon::Kind(IconKind::Claude),
        rgb: (0x12, 0x34, 0x56),
        frame: 0,
    };
    let bytes = transmit(0xDEC001, &pair);
    let text = String::from_utf8(bytes).unwrap();
    let commands: Vec<&str> = text.split("\x1b\\").filter(|c| !c.is_empty()).collect();
    // 16384 bytes of RGBA are 21848 base64 bytes: 5 full chunks and a short one.
    assert_eq!(commands.len(), 6);
    let mut payload = String::new();
    for (n, command) in commands.iter().enumerate() {
        let (header, data) = command
            .strip_prefix("\x1b_G")
            .unwrap()
            .split_once(';')
            .unwrap();
        assert!(data.len() <= 4096);
        let last = n + 1 == commands.len();
        let more = if last { "m=0" } else { "m=1" };
        if n == 0 {
            assert_eq!(
                header,
                format!("a=T,U=1,f=32,s=64,v=64,c=2,r=1,i=14598145,q=2,{more}")
            );
        } else {
            assert_eq!(header, format!("{more},q=2"));
        }
        payload.push_str(data);
    }
    assert_eq!(decode(&payload), pair.rgba());
    let rgba = pair.rgba();
    let first = rgba.chunks(4).find(|p| p.get(3) != Some(&0)).unwrap();
    assert_eq!(&first[..3], &[0x12, 0x34, 0x56]);
}

#[test]
fn the_delete_sequence() {
    assert_eq!(delete(7), b"\x1b_Ga=d,d=I,i=7,q=2\x1b\\");
}

#[test]
fn ids_count_up_from_a_fixed_start_and_each_pair_is_sent_once() {
    let mut i = images();
    let a = Pair {
        icon: Icon::Kind(IconKind::Claude),
        rgb: (1, 1, 1),
        frame: 0,
    };
    let b = Pair {
        icon: Icon::Kind(IconKind::Claude),
        rgb: (2, 2, 2),
        frame: 0,
    };
    assert_eq!(i.id(&a), None);
    assert!(i.register(a));
    assert!(i.register(b));
    assert!(i.register(a));
    assert_eq!(i.id(&a), Some(0xDEC001));
    assert_eq!(i.id(&b), Some(0xDEC002));
    let mut out = Vec::new();
    i.send_pending(&mut out).unwrap();
    let first = out.len();
    assert!(first > 0);
    // Nothing more goes out for what was sent.
    i.send_pending(&mut out).unwrap();
    assert_eq!(out.len(), first);
    let c = Pair {
        icon: Icon::Kind(IconKind::Codex),
        rgb: (1, 1, 1),
        frame: 0,
    };
    i.register(c);
    i.send_pending(&mut out).unwrap();
    assert_eq!(out.len(), first + transmit(0xDEC003, &c).len());
    let mut gone = Vec::new();
    i.delete_all(&mut gone).unwrap();
    let want = [delete(0xDEC001), delete(0xDEC002), delete(0xDEC003)].concat();
    assert_eq!(gone, want);
}

#[test]
fn ids_run_out_at_24_bits() {
    let mut i = images();
    i.next = LAST_ID;
    let a = Pair {
        icon: Icon::Kind(IconKind::Claude),
        rgb: (1, 1, 1),
        frame: 0,
    };
    let b = Pair {
        icon: Icon::Kind(IconKind::Claude),
        rgb: (2, 2, 2),
        frame: 0,
    };
    assert!(i.register(a));
    assert!(!i.register(b));
    assert_eq!(i.id(&a), Some(LAST_ID));
    assert_eq!(i.id(&b), None);
}

/// An app in Ghostty: images on, 24-bit colour.
fn ghostty(config: &str) -> App {
    let mut view = titled();
    // A second Claude panel that is not the active one.
    view.projects[0].panels[3].terminals[0].agent =
        Some(agent(AgentKind::Claude, AgentStatus::Idle));
    let mut a = app_with(config, view);
    a.truecolor = true;
    a.images = Some(images());
    a.tier = Tier::Image;
    a
}

fn placeholder_cell(buf: &Buffer, x: u16, y: u16) -> Option<u32> {
    let cell = buf.cell((x, y))?;
    // Both cells of the image: row 0, columns 0 and 1.
    let second = buf.cell((x + 1, y))?;
    if cell.symbol() != "\u{10EEEE}\u{0305}\u{0305}"
        || second.symbol() != "\u{10EEEE}\u{0305}\u{030D}"
        || second.fg != cell.fg
        || second.bg != cell.bg
        || !second.modifier.is_empty()
    {
        return None;
    }
    match cell.fg {
        Color::Rgb(r, g, b) => Some(u32::from_be_bytes([0, r, g, b])),
        _ => None,
    }
}

#[test]
fn agent_rows_draw_placeholders() {
    let mut a = ghostty("");
    let mut sink = Vec::new();
    a.send_images(&mut sink).unwrap();
    let (buf, _) = render(&a);
    // The Claude row's icon cell: the placeholder with two diacritics, the id as colour.
    let active = placeholder_cell(&buf, 1, 3).expect("placeholder on the active row");
    assert_eq!(buf[(1, 3)].symbol(), "\u{10EEEE}\u{0305}\u{0305}");
    // No attribute but the colours is left on an image cell, bold included.
    assert!(buf[(1, 3)].modifier.is_empty());
    // The second Claude row (inactive: `sidebar_fg`) has the same image: Claude's logo is
    // drawn in its own colour on every row.
    let inactive = placeholder_cell(&buf, 1, 6).expect("placeholder on the second row");
    assert_eq!(active, inactive);
    // Every program icon is an image of its own: Codex, the editor, git.
    let codex = placeholder_cell(&buf, 1, 5).expect("Codex");
    let editor = placeholder_cell(&buf, 1, 4).expect("editor");
    let git = placeholder_cell(&buf, 1, 7).expect("git");
    assert!(codex != editor && editor != git && codex != git);
    // The status marks: the spinner is an image of its own (frame 0), the waiting mark is an
    // image and not the row's icon; the space's caret is an image too.
    let spinner = placeholder_cell(&buf, 25, 3).expect("spinner");
    assert!(spinner != codex && spinner != active);
    let waiting = placeholder_cell(&buf, 25, 5).expect("waiting mark");
    assert!(waiting != codex && waiting != active);
    // The idle mark of the sixth row is an image of its own, not the waiting mark nor the
    // row's icon.
    let idle = placeholder_cell(&buf, 25, 6).expect("idle mark");
    assert!(idle != waiting && idle != inactive);
    placeholder_cell(&buf, 25, 2).expect("caret");
    // The names did not move.
    assert_eq!(buf[(4, 3)].symbol(), "F");
    // Every id drawn was sent.
    for id in [spinner, active, inactive, codex, editor, git, waiting, idle] {
        assert!((0xDEC001..0xDEC001 + 80).contains(&id), "{id:x}");
        assert!(
            a.images
                .as_ref()
                .unwrap()
                .order
                .iter()
                .any(|(i, _)| *i == id)
        );
    }
    assert_eq!(buf[(1, 3)].fg, image_id_color(active));
}

#[test]
fn a_pair_not_sent_is_drawn_as_text() {
    // `send_images` was not called: the draw never shows an image the terminal lacks.
    let a = ghostty("");
    let (_, rows) = render(&a);
    assert!(rows[3].starts_with(" \u{ec82}  "), "{}", rows[3]);
}

#[test]
fn outside_the_image_tier_text_icons_are_drawn_byte_for_byte() {
    let mut plain = app_with("", titled());
    plain.truecolor = true;
    let (want, _) = render(&plain);
    for tier in [Tier::Unicode, Tier::Nerd] {
        // Images may exist but the tier says text: nothing is sent, the frame is the plain one.
        let mut a = app_with("", titled());
        a.truecolor = true;
        a.images = Some(images());
        a.tier = tier;
        assert!(a.live_images().is_none());
        let mut sink = Vec::new();
        a.send_images(&mut sink).unwrap();
        assert!(sink.is_empty());
        if tier == Tier::Unicode {
            assert_eq!(render(&a).0, want);
        }
    }
    // Without truecolor the images are off.
    let mut a = ghostty("");
    a.truecolor = false;
    assert!(a.live_images().is_none());
    let mut sink = Vec::new();
    a.send_images(&mut sink).unwrap();
    assert!(sink.is_empty());
}

fn reload(a: &mut App, config: &str) {
    a.apply_config(Config::parse(config).unwrap().config);
}

#[test]
fn images_are_sent_once_before_the_frame_and_follow_a_reload() {
    let mut a = ghostty("");
    let mut out = Vec::new();
    a.send_images(&mut out).unwrap();
    // Claude in its colour (both rows), then in `sidebar_fg` the editor, Codex and git, the
    // waiting and idle marks and the caret, and the two bar chips in `muted`: nine pairs, none twice, and the 60 frames of the
    // loader in its one colour.
    let first = a.images.as_ref().unwrap().order.len();
    assert_eq!(first, 9 + 60);
    assert!(out.starts_with(b"\x1b_Ga=T,U=1,f=32,s=64,v=64,c=2,r=1,i=14598145,q=2,m=1;"));
    let len = out.len();
    for _ in 0..3 {
        a.send_images(&mut out).unwrap();
    }
    assert_eq!(out.len(), len, "nothing more is sent for the same pairs");

    // A colour change sends the new pairs only.
    reload(&mut a, "[theme]\nsidebar_fg = \"#ff0000\"");
    a.send_images(&mut out).unwrap();
    assert!(out.len() > len);
    let after = a.images.as_ref().unwrap().order.len();
    assert!(after > first && after < 2 * first + 1, "{after}");
    let (buf, _) = render(&a);
    assert!(placeholder_cell(&buf, 1, 6).is_some());

    // Another tier draws text icons from the next frame; what was sent stays registered.
    let sent = out.len();
    a.tier = Tier::Unicode;
    a.send_images(&mut out).unwrap();
    assert_eq!(out.len(), sent);
    let (_, rows) = render(&a);
    assert!(rows[3].starts_with(" ✻  Fix"), "{}", rows[3]);
    a.tier = Tier::Image;

    // On exit, one delete per id sent.
    let mut gone = Vec::new();
    a.delete_images(&mut gone).unwrap();
    let text = String::from_utf8(gone).unwrap();
    assert_eq!(text.matches("a=d,d=I,i=").count(), after);
    assert!(text.ends_with("q=2\x1b\\"));
}

#[test]
fn the_placeholder_is_two_cells_wide() {
    assert_eq!(crate::draw::width(&placeholder()), 2);
}

#[test]
fn a_selected_row_draws_its_visible_colours_without_reverse() {
    use deck_core::model::{PanelId, ProjectId};
    let mut a = ghostty("");
    a.sidebar_focus = true;
    a.selection = Some(crate::sidebar::Sel::Panel(ProjectId(1), PanelId(6)));
    let mut sink = Vec::new();
    a.send_images(&mut sink).unwrap();
    let (buf, _) = render(&a);
    let cell = &buf[(1, 6)];
    // The default selection is `REVERSED` over `default` colours: the row shows the outer
    // foreground (10, 20, 30) as its background; Claude's logo keeps its own colour on it.
    let id = placeholder_cell(&buf, 1, 6).expect("placeholder on the selected row");
    assert!(cell.modifier.is_empty(), "{:?}", cell.modifier);
    assert_eq!(cell.bg, Color::Rgb(10, 20, 30));
    let images = a.images.as_ref().unwrap();
    let pair = images.order.iter().find(|(i, _)| *i == id).unwrap().1;
    assert_eq!(pair.rgb, (0xd9, 0x77, 0x57));
    // The row's own cells are still reversed.
    assert!(
        buf[(4, 6)]
            .modifier
            .contains(ratatui::style::Modifier::REVERSED)
    );
}

#[test]
fn names_start_at_the_same_column_with_logos_and_with_text_icons() {
    let name_column = |rows: &[String]| {
        // Panel rows of the first project: Claude, nvim, Codex, shell, git.
        (3..8)
            .map(|y| {
                rows[y]
                    .chars()
                    .position(|c| c.is_alphanumeric() || c == '~' || c == '.')
            })
            .collect::<Vec<_>>()
    };
    let mut text_icons = ghostty("");
    text_icons.tier = Tier::Nerd;
    let (_, text) = render(&text_icons);
    let columns = name_column(&text);
    assert!(columns.iter().all(|c| *c == Some(4)), "{columns:?}");
    let mut a = ghostty("");
    let mut sink = Vec::new();
    a.send_images(&mut sink).unwrap();
    let (buf, _) = render(&a);
    // Logo rows and glyph rows: the first letter of each name is at column 4.
    for (y, first) in [(3, "F"), (4, "n"), (5, "R"), (7, "a")] {
        assert_eq!(buf[(4, y)].symbol(), first, "row {y}");
    }
}

#[test]
fn the_logos_of_the_dragged_and_target_rows_are_sent_in_their_colours() {
    use crate::sidebar::{RowDrag, Sel};
    use deck_core::model::{PanelId, ProjectId};
    let mut a = ghostty("");
    // Panel 6 (row 6) is dragged to the place of the first panel (row 3, the active one).
    a.row_drag = Some(RowDrag {
        item: Sel::Panel(ProjectId(1), PanelId(6)),
        to: 0,
    });
    assert!(a.drag_target_sel().is_some());
    let mut sink = Vec::new();
    a.send_images(&mut sink).unwrap();
    let (buf, _) = render(&a);
    let base = crate::sidebar::base_style(&a.theme);
    let images = a.images.as_ref().unwrap();
    // The dragged row is dimmed in place; the target's logo moved two columns right and has
    // its normal (band, active) colours.
    for (column, row, style) in [
        (1, 6, crate::sidebar::dimmed_style(&a.theme, base)),
        (
            3,
            3,
            crate::sidebar::panel_style(&a.theme, base, true, true),
        ),
    ] {
        let id = placeholder_cell(&buf, column, row).expect("the logo is drawn");
        let pair = images.order.iter().find(|(i, _)| *i == id).unwrap().1;
        // The logo is drawn in its provider colour over the row's style (`icon_style`).
        let Icon::Kind(kind) = pair.icon else {
            panic!("a program icon")
        };
        let style = crate::sidebar::icon_style(kind, style);
        assert_eq!(pair.rgb, images.cell_for(style).0, "row {row}");
    }
}

#[test]
fn the_unseen_marker_after_the_name_is_an_image_with_a_sent_id() {
    let mut view = titled();
    view.projects[0].panels[1].unseen = true;
    let mut a = app_with("", view);
    a.truecolor = true;
    a.images = Some(images());
    a.tier = Tier::Image;
    let mut sink = Vec::new();
    a.send_images(&mut sink).unwrap();
    let (buf, rows) = render(&a);
    // ` nvim` then a space: the name starts at column 4.
    let x = 4 + u16::try_from("nvim".len()).unwrap() + 1;
    let id = placeholder_cell(&buf, x, 4).unwrap_or_else(|| panic!("no unseen image: {}", rows[4]));
    let images = a.images.as_ref().unwrap();
    let (_, pair) = *images
        .order
        .iter()
        .find(|(i, _)| *i == id)
        .expect("a sent id");
    assert_eq!(pair.icon, Icon::Unseen);
}

/// A byte sink the test can read while a terminal owns it.
#[derive(Clone, Default)]
struct Shared(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);

impl std::io::Write for Shared {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A terminal over a byte sink, to read what a frame writes.
fn byte_terminal(
    a: &App,
    sink: &Shared,
) -> ratatui::Terminal<ratatui::backend::CrosstermBackend<Shared>> {
    use ratatui::layout::Rect;
    use ratatui::{TerminalOptions, Viewport};
    ratatui::Terminal::with_options(
        ratatui::backend::CrosstermBackend::new(sink.clone()),
        TerminalOptions {
            viewport: Viewport::Fixed(Rect::new(0, 0, a.size.0, a.size.1)),
        },
    )
    .unwrap()
}

#[test]
fn a_frame_sends_its_new_images_before_its_cells_and_then_draws_once() {
    let mut a = ghostty("");
    let sink = Shared::default();
    let mut t = byte_terminal(&a, &sink);
    a.draw_frame(&mut t).unwrap();
    let first = String::from_utf8_lossy(&sink.0.borrow()).into_owned();
    let sent = first.find("\x1b_Ga=T").expect("images were sent");
    let cell = first.find('\u{10EEEE}').expect("placeholders were drawn");
    assert!(sent < cell);
    // The glyph was never shown first.
    assert!(!first.contains('\u{f040}'), "the editor glyph was drawn");
    assert_eq!(a.images.as_ref().unwrap().order.len(), 9 + 60);
    // The same state again: nothing is sent.
    let len = sink.0.borrow().len();
    a.draw_frame(&mut t).unwrap();
    let again = String::from_utf8_lossy(&sink.0.borrow()[len..]).into_owned();
    assert!(!again.contains("\x1b_G"));
    assert_eq!(a.images.as_ref().unwrap().order.len(), 9 + 60);
}

#[test]
fn spinner_frames_are_the_mask_turned_by_six_degrees_each() {
    use crate::graphics::{SPINNER_FRAMES, spinner_frame};
    let source = mask(Icon::Spinner).unwrap();
    assert_eq!(&spinner_frame(0), source);
    // A quarter turn clockwise: the pixel at (x, y) comes from (y, 63 - x).
    let quarter = spinner_frame(15);
    for y in 0..64usize {
        for x in 0..64usize {
            let want = i32::from(source[(63 - x) * 64 + y]);
            let got = i32::from(quarter[y * 64 + x]);
            assert!((want - got).abs() <= 2, "({x}, {y}): {got} vs {want}");
        }
    }
    // A full revolution wraps.
    assert_eq!(spinner_frame(SPINNER_FRAMES), spinner_frame(0));
    // Every frame keeps the ink of the mask within 5%.
    let total = |m: &[u8]| m.iter().map(|&a| u32::from(a)).sum::<u32>();
    let want = total(source);
    for k in 0..SPINNER_FRAMES {
        let got = total(&spinner_frame(k));
        assert!(
            got.abs_diff(want) * 20 <= want,
            "frame {k}: {got} vs {want}"
        );
    }
    // And the frames differ from one another.
    assert_ne!(spinner_frame(0), spinner_frame(1));
}

/// A client in Ghostty whose spinner timer is armed and which started at `a.started`.
fn spinning() -> App {
    let mut a = ghostty("");
    a.spin_at = None;
    a.on_server(deck_core::protocol::ServerMsg::SessionView(titled()));
    a
}

#[test]
fn the_frame_comes_from_the_clock() {
    use std::time::Duration;
    let mut a = spinning();
    let at = |a: &mut App, d: Duration| {
        a.spin_at = Some(a.started);
        let now = a.started + d;
        a.tick(now);
        a.spin_frame
    };
    assert_eq!(at(&mut a, Duration::ZERO), 0);
    assert_eq!(at(&mut a, Duration::from_micros(16_700)), 1);
    assert_eq!(at(&mut a, Duration::from_secs(1)), 0);
    assert_eq!(at(&mut a, Duration::from_millis(1500)), 30);
    // A late tick shows the frame for its time: no frame is owed.
    assert_eq!(at(&mut a, Duration::from_millis(500)), 30);
    assert_eq!(at(&mut a, Duration::from_millis(2_000 + 100)), 6);
}

#[test]
fn every_spinner_shows_the_same_frame() {
    let mut view = titled();
    view.projects[0].panels[3].terminals[0].agent =
        Some(agent(AgentKind::Claude, AgentStatus::Working));
    let mut a = ghostty("");
    a.on_server(deck_core::protocol::ServerMsg::SessionView(view));
    a.spin_frame = 7;
    let mut sink = Vec::new();
    a.send_images(&mut sink).unwrap();
    let (buf, rows) = render(&a);
    let ids: Vec<u32> = (0..24u16)
        .filter(|&y| rows[usize::from(y)].starts_with(' '))
        .filter_map(|y| placeholder_cell(&buf, 25, y))
        .collect();
    // The two working agents' marks (the others are the waiting and idle marks).
    let images = a.images.as_ref().unwrap();
    let spinners: Vec<u32> = images
        .order
        .iter()
        .filter(|(_, p)| p.icon == Icon::Spinner && p.frame == 7)
        .map(|(id, _)| *id)
        .collect();
    assert!(!spinners.is_empty());
    let shown = ids.iter().filter(|id| spinners.contains(id)).count();
    assert!(shown >= 2, "{ids:?}");
}

#[test]
fn the_timer_runs_at_60_hz_only_in_the_image_tier() {
    use std::time::Duration;
    let third = Duration::from_nanos(1_000_000_000 / 60 + 1);
    let a = spinning();
    let d = a.wake_deadline().expect("a deadline");
    let now = std::time::Instant::now();
    assert!(d <= now + third, "{:?}", d.saturating_duration_since(now));
    // On a boundary of 1/60 s since the start.
    let nanos = d.duration_since(a.started).as_nanos();
    let rest = (nanos * 60) % 1_000_000_000;
    assert!(rest < 60, "{rest}");
    // The other tiers keep 100 ms.
    for tier in [Tier::Nerd, Tier::Unicode] {
        let mut a = spinning();
        a.tier = tier;
        a.spin_at = None;
        a.on_server(deck_core::protocol::ServerMsg::SessionView(titled()));
        let d = a.wake_deadline().expect("a deadline");
        let ahead = d.saturating_duration_since(std::time::Instant::now());
        assert!(ahead > Duration::from_millis(50), "{tier:?} {ahead:?}");
        assert!(ahead <= Duration::from_millis(100), "{tier:?} {ahead:?}");
    }
    // Nothing spins: no timer.
    let mut view = titled();
    view.projects[0].panels[0].terminals[0].agent = None;
    let mut a = ghostty("");
    a.spin_at = None;
    a.on_server(deck_core::protocol::ServerMsg::SessionView(view));
    assert_eq!(a.wake_deadline(), None);
}

#[test]
fn the_deadline_moves_on_from_a_late_tick_without_drifting() {
    use std::time::Duration;
    let mut a = spinning();
    a.spin_at = Some(a.started);
    let late = a.started + Duration::from_millis(100);
    a.tick(late);
    let d = a.wake_deadline().expect("next");
    assert!(d > late && d <= late + Duration::from_millis(17));
    assert_eq!(a.spin_frame, 6);
}

#[test]
fn all_frames_of_a_colour_go_out_before_the_first_is_drawn_and_never_again() {
    let mut a = ghostty("");
    let sink = Shared::default();
    let mut t = byte_terminal(&a, &sink);
    a.draw_frame(&mut t).unwrap();
    let out = String::from_utf8_lossy(&sink.0.borrow()).into_owned();
    let spinner_ids: Vec<u32> = a
        .images
        .as_ref()
        .unwrap()
        .order
        .iter()
        .filter(|(_, p)| p.icon == Icon::Spinner)
        .map(|(id, _)| *id)
        .collect();
    assert_eq!(spinner_ids.len(), 60);
    let first_cell = out.find('\u{10EEEE}').unwrap();
    for id in &spinner_ids {
        let at = out
            .find(&format!(",i={id},"))
            .unwrap_or_else(|| panic!("{id} not sent"));
        assert!(at < first_cell);
    }
    // A later frame of the loader sends nothing.
    let len = sink.0.borrow().len();
    a.spin_frame = 31;
    a.draw_frame(&mut t).unwrap();
    let again = String::from_utf8_lossy(&sink.0.borrow()[len..]).into_owned();
    assert!(!again.contains("\x1b_G"));
    // A second colour sends its own 60, once.
    reload(
        &mut a,
        "[theme]\nsidebar_fg = \"#ff0000\"\nsidebar_bg = \"#00ff00\"",
    );
    let len = sink.0.borrow().len();
    a.draw_frame(&mut t).unwrap();
    let more = String::from_utf8_lossy(&sink.0.borrow()[len..]).into_owned();
    let sent_after = a
        .images
        .as_ref()
        .unwrap()
        .order
        .iter()
        .filter(|(_, p)| p.icon == Icon::Spinner)
        .count();
    assert_eq!(sent_after, 120);
    assert!(more.matches("a=T").count() >= 60);
    // Exit deletes them all.
    let mut gone = Vec::new();
    a.delete_images(&mut gone).unwrap();
    let text = String::from_utf8(gone).unwrap();
    for id in &spinner_ids {
        assert!(text.contains(&format!("i={id},")));
    }
}

#[test]
fn two_draws_one_frame_apart_differ_only_in_the_spinner_cells() {
    let mut a = ghostty("");
    let mut sink = Vec::new();
    a.send_images(&mut sink).unwrap();
    a.spin_frame = 3;
    let (before, _) = render(&a);
    a.spin_frame = 4;
    let (after, _) = render(&a);
    let mut differing = 0;
    for y in 0..24u16 {
        for x in 0..100u16 {
            if before[(x, y)] != after[(x, y)] {
                differing += 1;
                assert!(
                    before[(x, y)].symbol().starts_with('\u{10EEEE}')
                        && after[(x, y)].symbol().starts_with('\u{10EEEE}'),
                    "({x}, {y}) {:?} {:?}",
                    before[(x, y)],
                    after[(x, y)]
                );
            }
        }
    }
    assert!(differing > 0 && differing <= 8, "{differing}");
}

#[test]
fn the_glyph_fallback_of_the_loader_steps_every_100_ms_in_the_image_tier() {
    let mut a = app_with("", titled());
    a.tier = Tier::Image;
    a.truecolor = true;
    let glyph = |a: &mut App, frame: u8| {
        a.spin_frame = frame;
        render(a).0[(25, 3)].symbol().to_string()
    };
    let first = glyph(&mut a, 0);
    assert_eq!(glyph(&mut a, 5), first);
    assert_ne!(glyph(&mut a, 6), first);
}

#[test]
fn the_sidebar_chips_draw_images_in_the_image_tier() {
    let mut a = ghostty("");
    let mut sink = Vec::new();
    a.send_images(&mut sink).unwrap();
    let (buf, rows) = render(&a);
    // Header ` +  New space `: the slot at columns 14 and 15 (the sidebar is 28 wide).
    let new_space = placeholder_cell(&buf, 14, 0).unwrap_or_else(|| panic!("{}", rows[0]));
    // Bottom bar: `New` at column 2, `Resume` at 17.
    let bar = usize::from(a.size.1) - 1;
    let new = placeholder_cell(&buf, 2, u16::try_from(bar).unwrap()).expect("New chip");
    let resume = placeholder_cell(&buf, 17, u16::try_from(bar).unwrap()).expect("Resume chip");
    assert_eq!(new_space, new, "the same icon in the same colour");
    assert_ne!(new, resume);
    assert!(
        rows[bar].contains("New") && rows[bar].contains("Resume"),
        "{}",
        rows[bar]
    );
}
