//! Differential compatibility tests: every case in `compat/cases/` is shown on the reference
//! emulator and through deck, and the two screens must be equal, apart from the known
//! differences below.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use support::compat::{Case, Ignore, Run, capture, compare_tampered};

/// A case that differs on purpose, or in part, with the reason. `*` matches every case.
/// An entry that names its case must still be needed: the case must differ without it
/// (`Ignore::Everything` means the case must differ at all). `*` entries are not checked.
/// Entries that deck should fix later name a backlog task.
struct Known {
    case: &'static str,
    ignore: Ignore,
    reason: &'static str,
}

const KNOWN: &[Known] = &[
    Known {
        case: "alt_screen_47",
        ignore: Ignore::Region(1..3, 0..30),
        reason: "alacritty_terminal ignores ?47, so the program's alternate-screen output lands on \
                 the primary screen; xterm and Ghostty switch screens (see deck_switches_screens)",
    },
    Known {
        case: "alt_screen_1047",
        ignore: Ignore::Region(1..4, 0..30),
        reason: "alacritty_terminal ignores ?1047, so the program's alternate-screen output lands \
                 on the primary screen; xterm and Ghostty switch screens (see deck_switches_screens)",
    },
    Known {
        case: "cursor_style_1",
        ignore: Ignore::CursorStyle,
        reason: "deck always sets a steady cursor on the outer terminal: Frame's Cursor carries \
                 the shape but not blinking, so carrying it needs a protocol change (DECK-30)",
    },
    Known {
        case: "cursor_style_3",
        ignore: Ignore::CursorStyle,
        reason: "deck always sets a steady cursor on the outer terminal: Frame's Cursor carries \
                 the shape but not blinking, so carrying it needs a protocol change (DECK-30)",
    },
    Known {
        case: "cursor_style_5",
        ignore: Ignore::CursorStyle,
        reason: "deck always sets a steady cursor on the outer terminal: Frame's Cursor carries \
                 the shape but not blinking, so carrying it needs a protocol change (DECK-30)",
    },
    Known {
        case: "dch_past_the_edge",
        ignore: Ignore::Region(0..1, 21..27),
        reason: "alacritty_terminal's DCH with a count past the end of the line also blanks cells \
                 before the cursor; xterm and Ghostty delete only to the end of the line",
    },
    Known {
        case: "graphemes_vs16",
        ignore: Ignore::Everything,
        reason: "a variation selector 16 widens a text symbol to two cells in Ghostty (deck) and \
                 leaves it one cell in alacritty_terminal; deck follows its own emulator",
    },
    Known {
        case: "graphemes_zwj",
        ignore: Ignore::Everything,
        reason: "Ghostty makes a ZWJ sequence one wide cell, alacritty_terminal draws its parts \
                 side by side; deck follows its own emulator",
    },
    Known {
        case: "origin_mode",
        ignore: Ignore::Cursor,
        reason: "xterm and Ghostty home the cursor when DECOM is reset as well as when it is set; \
                 alacritty_terminal does not home it on reset",
    },
    Known {
        case: "reflow_shrink",
        ignore: Ignore::Everything,
        reason: "when a reflow adds rows, alacritty_terminal pushes the top of the content into \
                 its scrollback although the screen has room; Ghostty keeps it on screen \
                 (see deck_reflows_in_place)",
    },
    Known {
        case: "resize_alt_screen_rows",
        ignore: Ignore::Everything,
        reason: "when the height shrinks, Ghostty (deck) drops the top rows of the alternate \
                 screen and keeps the bottom ones, alacritty_terminal does the reverse; the \
                 program redraws on the resize anyway (see deck_keeps_the_bottom_rows)",
    },
    Known {
        case: "underline_color",
        ignore: Ignore::Region(2..3, 0..24),
        reason: "deck draws no underline colour on text that is not underlined, which looks the \
                 same; the reference keeps SGR 58 on the cell",
    },
    Known {
        case: "underline_color",
        ignore: Ignore::Region(2..3, 39..44),
        reason: "deck draws no underline colour on text that is not underlined, which looks the \
                 same; the reference keeps SGR 58 on the cell",
    },
    Known {
        case: "underline_kinds",
        ignore: Ignore::Region(1..2, 0..6),
        reason: "alacritty_terminal reads SGR 21 as bold off (the Linux console's meaning); xterm's \
                 ctlseqs and Ghostty make it a double underline, which deck shows",
    },
    Known {
        case: "wide_edges",
        ignore: Ignore::Region(3..4, 0..1),
        reason: "erasing the right half of a wide character (ECH) blanks both halves in \
                 xterm and Ghostty; alacritty_terminal leaves the left half",
    },
    Known {
        case: "wide_edges",
        ignore: Ignore::Region(5..6, 2..3),
        reason: "deleting the right half of a wide character (DCH) blanks both halves in \
                 xterm and Ghostty; alacritty_terminal leaves the left half",
    },
];

fn known_for(case: &str) -> Vec<&'static Known> {
    KNOWN
        .iter()
        .filter(|k| k.case == "*" || k.case == case)
        .collect()
}

/// Checks a captured run against the table: equal without the known differences, and every
/// entry that names the case still needed (the run differs without it), so a fix is noticed
/// and the entry removed. `*` entries are only applied.
fn verify(run: &Run, known: &[&Known]) -> Result<(), String> {
    let report = |d: Box<support::compat::Diff>| {
        d.save();
        d.to_string()
    };
    if known.is_empty() {
        return run.diff(&[]).map_err(report);
    }
    let gone = |k: &Known| {
        let why = if matches!(k.ignore, Ignore::Everything) {
            String::new()
        } else {
            format!(" ({})", k.reason)
        };
        format!("known difference no longer differs: {}{why}", run.case)
    };
    if let Some(k) = known
        .iter()
        .find(|k| matches!(k.ignore, Ignore::Everything))
    {
        return match run.diff(&[]) {
            Err(_) => Ok(()),
            Ok(()) => Err(gone(k)),
        };
    }
    let ignore = |skip: Option<usize>| -> Vec<Ignore> {
        known
            .iter()
            .enumerate()
            .filter(|(i, _)| Some(*i) != skip)
            .map(|(_, k)| k.ignore.clone())
            .collect()
    };
    run.diff(&ignore(None)).map_err(|d| {
        let reasons: Vec<&str> = known.iter().map(|k| k.reason).collect();
        format!("{}\n  known differences ignored: {reasons:?}", report(d))
    })?;
    for (i, k) in known.iter().enumerate() {
        if k.case != "*" && run.diff(&ignore(Some(i))).is_ok() {
            return Err(gone(k));
        }
    }
    Ok(())
}

fn run_case(name: &str) {
    verify_case(Case::named(name));
}

/// A recording of a real program, replayed like a case.
fn run_recording(name: &str) {
    verify_case(Case::recording(name));
}

fn verify_case(case: Case) {
    let name = case.name.clone();
    let run = capture(&case, None);
    if let Err(report) = verify(&run, &known_for(&name)) {
        panic!("{report}");
    }
}

macro_rules! cases {
    ($($name:ident),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                run_case(stringify!($name));
            }
        )*

        /// Every file in `compat/cases/` is in the list above.
        #[test]
        fn every_case_file_is_listed() {
            let listed = [$(stringify!($name)),*];
            let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/compat/cases");
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.extension().is_some_and(|e| e == "vt") {
                    let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
                    assert!(listed.contains(&stem.as_str()), "case {stem} is not in compat.rs");
                }
            }
        }
    };
}

cases!(
    plain,
    sgr_16,
    sgr_256,
    sgr_truecolor,
    sgr_attributes,
    cursor_move,
    erase,
    long_line,
    wide_char,
    combining,
    resize,
    region_ind_ri,
    region_su_sd,
    region_il_dl,
    region_reset,
    ich_dch_ech_rep,
    tab_stops,
    autowrap,
    origin_mode,
    save_restore,
    alt_screen_47,
    alt_screen_1047,
    alt_screen_1049,
    dec_graphics,
    wide_edges,
    graphemes_vs16,
    graphemes_zwj,
    graphemes_cjk,
    dch_past_the_edge,
    resize_alt_screen_rows,
    cursor_style,
    cursor_style_hidden,
    cursor_style_0,
    cursor_style_1,
    cursor_style_2,
    cursor_style_3,
    cursor_style_4,
    cursor_style_5,
    cursor_style_6,
    cursor_style_default_after_reset,
    sync_redraw,
    newlines,
    erase_scrollback,
    erase_scrollback_after,
    reflow_grow,
    reflow_shrink,
    reflow_cursor,
    reflow_cursor_end,
    resize_alt_screen,
    resize_alt_screen_grow,
    last_row,
    reflow_shrink_full,
    underline_kinds,
    underline_color,
);

macro_rules! recordings {
    ($($name:ident),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                run_recording(stringify!($name));
            }
        )*

        /// Every file in `compat/recordings/` is in the list above.
        #[test]
        fn every_recording_is_listed() {
            let listed = [$(stringify!($name)),*];
            let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/compat/recordings");
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.extension().is_some_and(|e| e == "vt") {
                    let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
                    assert!(listed.contains(&stem.as_str()), "recording {stem} is not in compat.rs");
                }
            }
        }
    };
}

// Real programs, recorded with `just record-compat`; the programs are listed in
// docs/features/compatibility.md.
recordings!(nvim, less, lazygit, fzf, git_log, git_diff, bat);

#[test]
fn escapes_become_bytes() {
    let case = Case::parse(
        "t",
        "#! size 10x5\n# comment\nA\\e[1m\\x41\\\\\\r\\n\\t\nB\n",
    );
    assert_eq!(case.size, (10, 5));
    assert_eq!(case.bytes, b"A\x1b[1mA\\\r\n\tB");
}

#[test]
fn a_case_built_to_differ_fails_naming_the_cell() {
    let case = Case::named("plain");
    let err = compare_tampered(&case, &|deck| {
        deck.cell_mut(1, 3).ch = 'X';
    })
    .unwrap_err();
    assert_eq!(err.cells(), [(1, 3)]);
    let report = err.to_string();
    assert!(report.contains("(1, 3) ref='o'"), "{report}");
    assert!(report.contains("deck='X'"), "{report}");
}

#[test]
fn a_known_difference_that_stops_differing_fails_the_run() {
    let case = Case::named("plain");
    let run = capture(&case, None);
    let stale = Known {
        case: "plain",
        ignore: Ignore::Everything,
        reason: "a stale entry",
    };
    let err = verify(&run, &[&stale]).unwrap_err();
    assert_eq!(err, "known difference no longer differs: plain");
}

#[test]
fn an_unneeded_entry_fails_but_a_wildcard_does_not() {
    use alacritty_terminal::term::cell::Flags;
    let run = capture(&Case::named("plain"), None);
    let unneeded = Known {
        case: "plain",
        ignore: Ignore::Attr(Flags::BOLD),
        reason: "not needed",
    };
    let err = verify(&run, &[&unneeded]).unwrap_err();
    assert_eq!(
        err,
        "known difference no longer differs: plain (not needed)"
    );
    let wildcard = Known {
        case: "*",
        ignore: Ignore::Attr(Flags::BOLD),
        reason: "applies everywhere",
    };
    verify(&run, &[&wildcard]).unwrap();
}

#[test]
fn the_floating_bar_does_not_cover_the_last_row() {
    // The corpus runs with no mode and no flash, so the bar is not drawn.
    let run = capture(&Case::named("last_row"), None);
    let phase = &run.phases[0];
    let last = phase.deck.text().lines().last().unwrap().to_string();
    assert!(last.starts_with("LAST ROW reverse"), "{last}");
    assert!(last.ends_with('!'), "{last}");
    verify(&run, &known_for("last_row")).unwrap();
}

/// What deck alone shows after a case, rows joined, trailing empty rows dropped, for the cases
/// where the reference is wrong and a known difference stands in for the comparison.
fn deck_text(name: &str) -> String {
    let run = capture(&Case::named(name), None);
    run.phases
        .last()
        .unwrap()
        .deck
        .text()
        .trim_end()
        .to_string()
}

#[test]
fn deck_switches_screens() {
    for name in ["alt_screen_47", "alt_screen_1047"] {
        assert_eq!(
            deck_text(name),
            "primary one\nprimary two\n\nback",
            "{name}"
        );
    }
}

#[test]
fn deck_reflows_in_place() {
    let line = "0123456789abcdefghijklmnopqrst\nuvwxyz0123456789ABCDEFGHIJKLMN\n\
                OPQRSTUVWXYZ0123456789-0123456\n789-0123456789-";
    assert_eq!(deck_text("reflow_shrink"), format!("{line}\nshort\n{line}"));
}

#[test]
fn deck_keeps_the_bottom_rows() {
    assert_eq!(
        deck_text("resize_alt_screen_rows"),
        "\n         mid\n\n\n\nbottom"
    );
}

#[test]
fn deck_draws_what_ghostty_measures() {
    assert_eq!(
        deck_text("graphemes_vs16"),
        "a\u{2764}\u{fe0f} b\n\u{2714}\u{fe0f} g"
    );
    assert_eq!(deck_text("graphemes_zwj"), "c\u{1F469}\u{200D}d");
}
