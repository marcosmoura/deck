//! Logos as images: detection, colours, the escape sequences, the registry and the cells.

use std::collections::HashMap;

use deck_core::agents::{AgentKind, AgentStatus};
use deck_core::config::Config;
use deck_core::theme::IconImages;
use ratatui::buffer::Buffer;
use ratatui::style::Color;

use deck_core::icons::IconKind;

use crate::app::App;
use crate::convert::image_id_color;
use crate::graphics::{Images, LAST_ID, Logo, Pair, delete, detect, placeholder, transmit};
use crate::icons_tests::{agent, app_with, render, titled};

fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

#[test]
fn detection_table() {
    let auto = |pairs: &[(&str, &str)]| detect(&env(pairs), IconImages::Auto, true);
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
    // Off: the config key and 256 colours.
    let ghostty = env(&[("TERM_PROGRAM", "ghostty")]);
    assert!(detect(&ghostty, IconImages::Auto, true));
    assert!(!detect(&ghostty, IconImages::Off, true));
    assert!(!detect(&ghostty, IconImages::Auto, false));
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
fn the_masks_are_64_by_64_and_not_empty() {
    for logo in [Logo::Claude, Logo::Openai, Logo::Opencode] {
        let mask = logo.mask();
        assert_eq!(mask.len(), 4096);
        assert!(mask.iter().any(|&a| a != 0), "{logo:?}");
        // The 4-pixel margin is empty.
        for y in 0..64usize {
            for x in 0..64usize {
                if !(4..60).contains(&y) || !(4..60).contains(&x) {
                    assert_eq!(mask.get(y * 64 + x), Some(&0), "{logo:?} at {x},{y}");
                }
            }
        }
    }
    assert_eq!(Logo::of(IconKind::Codex), Some(Logo::Openai));
    assert_eq!(Logo::of(IconKind::Terminal), None);
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
        logo: Logo::Claude,
        rgb: (0x12, 0x34, 0x56),
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
        logo: Logo::Claude,
        rgb: (1, 1, 1),
    };
    let b = Pair {
        logo: Logo::Claude,
        rgb: (2, 2, 2),
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
        logo: Logo::Openai,
        rgb: (1, 1, 1),
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
        logo: Logo::Claude,
        rgb: (1, 1, 1),
    };
    let b = Pair {
        logo: Logo::Claude,
        rgb: (2, 2, 2),
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
    let (buf, rows) = render(&a);
    // The Claude row's icon cell: the placeholder with two diacritics, the id as colour.
    let active = placeholder_cell(&buf, 3, 3).expect("placeholder on the active row");
    assert_eq!(buf[(3, 3)].symbol(), "\u{10EEEE}\u{0305}\u{0305}");
    // No attribute but the colours is left on an image cell, bold included.
    assert!(buf[(3, 3)].modifier.is_empty());
    // The second Claude row (inactive: `sidebar_fg`) has the same image: Claude's logo is
    // drawn in its own colour on every row.
    let inactive = placeholder_cell(&buf, 3, 6).expect("placeholder on the second row");
    assert_eq!(active, inactive);
    // Codex has its own logo, a shell keeps its glyph, an editor too.
    assert!(placeholder_cell(&buf, 3, 5).is_some());
    assert_eq!(buf[(3, 4)].symbol(), "✎");
    assert_eq!(buf[(3, 7)].symbol(), "±");
    // The names did not move.
    assert!(rows[3].starts_with(" ⠋ "), "{}", rows[3]);
    assert_eq!(buf[(6, 3)].symbol(), "F");
    // Every id drawn was sent.
    for id in [active, inactive] {
        assert!((0xDEC001..0xDEC001 + 6).contains(&id), "{id:x}");
        assert!(
            a.images
                .as_ref()
                .unwrap()
                .order
                .iter()
                .any(|(i, _)| *i == id)
        );
    }
    assert_eq!(buf[(3, 3)].fg, image_id_color(active));
}

#[test]
fn a_pair_not_sent_is_drawn_as_text() {
    // `send_images` was not called: the draw never shows an image the terminal lacks.
    let a = ghostty("");
    let (_, rows) = render(&a);
    assert!(rows[3].starts_with(" ⠋ ✻  "), "{}", rows[3]);
}

#[test]
fn images_off_draws_text_icons_byte_for_byte() {
    let mut plain = app_with("", titled());
    plain.truecolor = true;
    let (want, _) = render(&plain);
    // With the key at `off` the frame is the plain one and nothing is sent.
    let mut a = app_with("[theme]\nicon_images = \"off\"", titled());
    a.truecolor = true;
    a.images = Some(images());
    let mut sink = Vec::new();
    a.send_images(&mut sink).unwrap();
    assert!(sink.is_empty());
    assert_eq!(render(&a).0, want);
    // `icons = "none"`: no icon and no image.
    let mut a = app_with("[theme]\nicons = \"none\"", titled());
    a.images = Some(images());
    let mut sink = Vec::new();
    a.send_images(&mut sink).unwrap();
    assert!(sink.is_empty());
    let mut b = app_with("[theme]\nicons = \"none\"", titled());
    b.images = None;
    assert_eq!(render(&a).0, render(&b).0);
    // Without truecolor the images are off.
    let mut a = ghostty("");
    a.truecolor = false;
    assert!(a.logos().is_none());
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
    // Both Claude rows (Claude's colour) and Codex (sidebar_fg).
    let first = a.images.as_ref().unwrap().order.len();
    assert_eq!(first, 2);
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
    assert!(placeholder_cell(&buf, 3, 6).is_some());

    // `off` draws text icons from the next frame; what was sent stays registered.
    let sent = out.len();
    reload(&mut a, "[theme]\nicon_images = \"off\"");
    a.send_images(&mut out).unwrap();
    assert_eq!(out.len(), sent);
    let (_, rows) = render(&a);
    assert!(rows[3].starts_with(" ⠋ ✻  "), "{}", rows[3]);

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
    let cell = &buf[(3, 6)];
    // The default selection is `REVERSED` over `default` colours: the row shows the outer
    // foreground (10, 20, 30) as its background; Claude's logo keeps its own colour on it.
    let id = placeholder_cell(&buf, 3, 6).expect("placeholder on the selected row");
    assert!(cell.modifier.is_empty(), "{:?}", cell.modifier);
    assert_eq!(cell.bg, Color::Rgb(10, 20, 30));
    let images = a.images.as_ref().unwrap();
    let pair = images.order.iter().find(|(i, _)| *i == id).unwrap().1;
    assert_eq!(pair.rgb, (0xd9, 0x77, 0x57));
    // The row's own cells are still reversed.
    assert!(
        buf[(5, 6)]
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
    let (_, text) = render(&ghostty("[theme]\nicon_images = \"off\""));
    let columns = name_column(&text);
    assert!(columns.iter().all(|c| *c == Some(6)), "{columns:?}");
    let mut a = ghostty("");
    let mut sink = Vec::new();
    a.send_images(&mut sink).unwrap();
    let (buf, _) = render(&a);
    // Logo rows and glyph rows: the first letter of each name is at column 6.
    for (y, first) in [(3, "F"), (4, "n"), (5, "R"), (7, "a")] {
        assert_eq!(buf[(6, y)].symbol(), first, "row {y}");
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
        (3, 6, crate::sidebar::dimmed_style(&a.theme, base)),
        (
            5,
            3,
            crate::sidebar::panel_style(&a.theme, base, true, true),
        ),
    ] {
        let id = placeholder_cell(&buf, column, row).expect("the logo is drawn");
        let pair = images.order.iter().find(|(i, _)| *i == id).unwrap().1;
        // The logo is drawn in its provider colour over the row's style (`icon_style`).
        let kind = [IconKind::Claude, IconKind::Codex, IconKind::OpenCode]
            .into_iter()
            .find(|k| Logo::of(*k) == Some(pair.logo))
            .expect("an agent logo");
        let style = crate::sidebar::icon_style(&a.theme, kind, style);
        assert_eq!(pair.rgb, images.cell_for(style).0, "row {row}");
    }
}
