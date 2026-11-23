//! The terminal, the tier decision, Nerd Font names and the font-check parsers.

use std::collections::HashMap;

use crate::caps::{
    Run, Terminal, any_nerd_font_file, check_for, fallback_from, fc_list_found, is_nerd_font_name,
    iterm2_font_is_nerd, plan, terminal, terminal_app_font_is_nerd, terminal_from_xtversion,
};
use crate::tier::Tier;

const ITERM2: &str = include_str!("../testdata/iterm2.plist");
const APP_PLAIN: &str = include_str!("../testdata/terminal_app_plain.plist");
const APP_NERD: &str = include_str!("../testdata/terminal_app_nerd.plist");

fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

#[test]
fn xtversion_replies_name_the_terminal() {
    assert_eq!(
        terminal_from_xtversion("kitty(0.36.4)"),
        Terminal::Kitty(Some((0, 36, 4)))
    );
    assert_eq!(terminal_from_xtversion("ghostty 1.2.0"), Terminal::Ghostty);
    assert_eq!(
        terminal_from_xtversion("WezTerm 20240203-110809-5046fc22"),
        Terminal::WezTerm
    );
    assert_eq!(terminal_from_xtversion("iTerm2 3.5.10"), Terminal::ITerm2);
    assert_eq!(
        terminal_from_xtversion("tmux 3.5a"),
        Terminal::Other("tmux 3.5a".into())
    );
}

#[test]
fn without_a_reply_the_environment_names_the_terminal() {
    let from = |pairs: &[(&str, &str)]| terminal(None, &env(pairs));
    assert_eq!(from(&[("TERM_PROGRAM", "ghostty")]), Terminal::Ghostty);
    assert_eq!(from(&[("TERM_PROGRAM", "WezTerm")]), Terminal::WezTerm);
    assert_eq!(from(&[("TERM_PROGRAM", "iTerm.app")]), Terminal::ITerm2);
    assert_eq!(
        from(&[("TERM_PROGRAM", "Apple_Terminal")]),
        Terminal::AppleTerminal
    );
    assert_eq!(from(&[("TERM", "xterm-ghostty")]), Terminal::Ghostty);
    assert_eq!(from(&[("TERM", "xterm-kitty")]), Terminal::Kitty(None));
    assert_eq!(from(&[("KITTY_WINDOW_ID", "3")]), Terminal::Kitty(None));
    assert_eq!(from(&[("GHOSTTY_RESOURCES_DIR", "/x")]), Terminal::Ghostty);
    assert_eq!(from(&[("WEZTERM_EXECUTABLE", "/x")]), Terminal::WezTerm);
    assert_eq!(from(&[("WEZTERM_EXECUTABLE", "")]), Terminal::Unknown);
    assert_eq!(from(&[("TERM", "xterm-256color")]), Terminal::Unknown);
    // A reply wins over the environment.
    assert_eq!(
        terminal(Some("tmux 3.5a"), &env(&[("TERM_PROGRAM", "ghostty")])),
        Terminal::Other("tmux 3.5a".into())
    );
}

/// The tier with the probe's answers as the client gets them and the font check's `font`.
fn tier(
    xtversion: Option<&str>,
    graphics: Option<bool>,
    pairs: &[(&str, &str)],
    truecolor: bool,
    font: bool,
) -> Tier {
    let env = env(pairs);
    let first = plan(xtversion, graphics, &env, truecolor);
    first.tier.with_font(font)
}

#[test]
fn the_tier_decision_table() {
    use Tier::{Image, Nerd, Unicode};
    let ok = Some(true);
    assert_eq!(tier(Some("ghostty 1.2.0"), ok, &[], true, false), Image);
    assert_eq!(tier(Some("kitty(0.36.4)"), ok, &[], true, false), Image);
    // Without truecolor the bundled symbols still give Nerd.
    assert_eq!(tier(Some("kitty(0.36.4)"), ok, &[], false, false), Nerd);
    // kitty 0.27: no placeholders, no bundled symbols: Nerd only when a font is found.
    assert_eq!(tier(Some("kitty(0.27.1)"), ok, &[], true, false), Unicode);
    assert_eq!(tier(Some("kitty(0.27.1)"), ok, &[], true, true), Nerd);
    // kitty 0.30: images, from the version where placeholders exist.
    assert_eq!(tier(Some("kitty(0.30.0)"), ok, &[], true, false), Image);
    assert_eq!(tier(Some("WezTerm 20240203"), ok, &[], true, false), Nerd);
    assert_eq!(tier(Some("iTerm2 3.5.10"), None, &[], true, true), Nerd);
    assert_eq!(tier(Some("iTerm2 3.5.10"), None, &[], true, false), Unicode);
    // Behind tmux, the font check alone.
    let tmux = [("TMUX", "/t,1,0")];
    assert_eq!(tier(Some("tmux 3.5a"), None, &tmux, true, false), Unicode);
    assert_eq!(tier(Some("tmux 3.5a"), None, &tmux, true, true), Nerd);
    // An error reply to the graphics query is not images.
    assert_eq!(
        tier(Some("ghostty 1.2.0"), Some(false), &[], true, false),
        Nerd
    );
    // The environment is the fallback when neither query was answered.
    assert_eq!(
        tier(None, None, &[("TERM_PROGRAM", "ghostty")], true, false),
        Image
    );
    assert_eq!(
        tier(None, None, &[("KITTY_WINDOW_ID", "1")], true, false),
        Image
    );
    assert_eq!(
        tier(None, None, &[("KITTY_WINDOW_ID", "1")], false, false),
        Nerd
    );
    assert_eq!(
        tier(None, None, &[("TERM_PROGRAM", "WezTerm")], true, false),
        Nerd
    );
    assert_eq!(
        tier(
            None,
            None,
            &[("TERM", "xterm-ghostty"), ("TMUX", "/t,1,0")],
            true,
            false
        ),
        Nerd
    );
    assert_eq!(tier(None, None, &[], true, false), Unicode);
    // XTVERSION answered but not the graphics query: no guessing from the environment.
    assert_eq!(
        tier(
            Some("iTerm2 3.5.10"),
            None,
            &[("TERM_PROGRAM", "ghostty")],
            true,
            false
        ),
        Unicode
    );
    // In a deck panel an inner Ghostty counts for nothing: only the font check.
    let deck = [("DECK", "1")];
    assert_eq!(tier(Some("ghostty 1.2.0"), ok, &deck, true, false), Unicode);
    assert_eq!(tier(Some("ghostty 1.2.0"), ok, &deck, true, true), Nerd);
}

#[test]
fn which_font_check_runs() {
    let check = |t: &Terminal, pairs: &[(&str, &str)]| check_for(t, &env(pairs));
    assert!(check(&Terminal::Ghostty, &[]).is_none());
    assert!(check(&Terminal::Kitty(None), &[]).is_none());
    assert!(check(&Terminal::Kitty(Some((0, 36, 0))), &[]).is_none());
    assert!(check(&Terminal::Kitty(Some((0, 35, 9))), &[]).is_some());
    assert!(check(&Terminal::ITerm2, &[]).is_some());
    assert!(check(&Terminal::Unknown, &[]).is_some());
    // The test seam skips every check but the bundled one.
    assert!(check(&Terminal::ITerm2, &[("DECK_TEST_NO_FONT_CHECK", "1")]).is_none());
    assert!(check(&Terminal::Unknown, &[("DECK_TEST_NO_FONT_CHECK", "1")]).is_none());
    // Nothing to run once the images or the bundled symbols decided.
    assert!(
        plan(Some("ghostty 1.2.0"), Some(true), &env(&[]), true)
            .check
            .is_none()
    );
    assert!(
        plan(Some("iTerm2 3.5.10"), None, &env(&[]), true)
            .check
            .is_some()
    );
    // Inside a deck panel only the fallback applies, whatever the inner emulator is.
    assert!(check(&Terminal::Ghostty, &[("DECK", "1")]).is_some());
}

#[test]
fn nerd_font_names() {
    for yes in [
        "JetBrainsMono Nerd Font",
        "JetBrainsMonoNFM-Regular",
        "Hack NF",
        "Symbols Nerd Font Mono",
        "FiraCodeNerdFontMono-Retina",
        "JetBrainsMono NFP",
        "Hack nerd font",
    ] {
        assert!(is_nerd_font_name(yes), "{yes}");
    }
    for no in [
        "Monaco",
        "SF Mono",
        "Menlo-Regular",
        "NFL Sans",
        "SFMono-Regular",
        "",
    ] {
        assert!(!is_nerd_font_name(no), "{no}");
    }
}

#[test]
fn iterm2_profile_fonts() {
    // The default profile (by `Default Bookmark Guid`) has a Nerd Font.
    assert!(iterm2_font_is_nerd(ITERM2, None));
    // `ITERM_PROFILE` wins; a profile with Monaco has no Nerd glyphs, even though its
    // (unused) non-ASCII font is a Nerd Font.
    assert!(!iterm2_font_is_nerd(ITERM2, Some("Plain")));
    assert!(iterm2_font_is_nerd(ITERM2, Some("Nerd & Co")));
    // A non-ASCII font in use decides alone: Nerd when it is, not when only the normal one is.
    assert!(iterm2_font_is_nerd(ITERM2, Some("Split")));
    assert!(!iterm2_font_is_nerd(ITERM2, Some("Mixed")));
    // An unknown profile name falls back to the default one.
    assert!(iterm2_font_is_nerd(ITERM2, Some("Nope")));
    // Nothing exported (iTerm2 not configured here), or not a plist.
    let empty = "<?xml version=\"1.0\"?>\n<plist version=\"1.0\">\n<dict/>\n</plist>\n";
    assert!(!iterm2_font_is_nerd(empty, None));
    assert!(!iterm2_font_is_nerd("", None));
    assert!(!iterm2_font_is_nerd("<plist><dict><key>x", None));
}

#[test]
fn terminal_app_profile_fonts() {
    // Basic uses SFMono-Regular (archived data from a real `defaults export`).
    assert!(!terminal_app_font_is_nerd(APP_PLAIN));
    // The default profile's font data names JetBrainsMonoNFM-Regular.
    assert!(terminal_app_font_is_nerd(APP_NERD));
    assert!(!terminal_app_font_is_nerd("<plist><dict/></plist>"));
}

#[test]
fn fc_list_output_and_font_files() {
    assert!(fc_list_found("JetBrainsMono Nerd Font\nHack\n"));
    assert!(!fc_list_found(""));
    assert!(!fc_list_found("\n  \n"));
    let files = |names: &[&str]| any_nerd_font_file(names.iter().map(|n| (*n).to_string()));
    assert!(files(&["Monaco.ttf", "JetBrainsMonoNerdFont-Regular.ttf"]));
    assert!(files(&["HackNFM-Regular.otf"]));
    assert!(!files(&["Monaco.ttf", "SFMono-Regular.otf"]));
    assert!(!files(&[]));
}

#[test]
fn a_late_font_answer_upgrades_the_tier_once_and_redraws() {
    let mut app = crate::app::App::new(&deck_core::config::Config::default(), (100, 24), &[]);
    app.tier = Tier::Unicode;
    assert!(app.take_dirty());
    assert!(!app.take_dirty());
    app.on_font_answer(true);
    assert_eq!(app.tier, Tier::Nerd);
    assert!(app.take_dirty());
    // A second answer changes nothing, and nothing ever downgrades.
    app.on_font_answer(false);
    assert_eq!(app.tier, Tier::Nerd);
    assert!(!app.take_dirty());

    // No answer leaves Unicode; a negative one too, and a later positive one is ignored.
    let mut app = crate::app::App::new(&deck_core::config::Config::default(), (100, 24), &[]);
    app.tier = Tier::Unicode;
    app.on_font_answer(false);
    app.on_font_answer(true);
    assert_eq!(app.tier, Tier::Unicode);

    // The image tier stays.
    let mut app = crate::app::App::new(&deck_core::config::Config::default(), (100, 24), &[]);
    app.tier = Tier::Image;
    app.on_font_answer(true);
    assert_eq!(app.tier, Tier::Image);
}

#[test]
fn macos_looks_at_the_font_folders_when_fc_list_gives_no_answer() {
    let scan = |found: bool| move || found;
    // Missing, failed or timed out: the folders decide on macOS, nothing on Linux.
    for ran in [|| Run::Missing, || Run::Failed] {
        assert!(fallback_from(ran(), true, scan(true)));
        assert!(!fallback_from(ran(), true, scan(false)));
        assert!(!fallback_from(ran(), false, scan(true)));
    }
    // An answer is final: no folder scan, even an empty one.
    assert!(fallback_from(
        Run::Out("Hack NF\n".into()),
        true,
        || panic!("scanned")
    ));
    assert!(!fallback_from(Run::Out(String::new()), true, || panic!(
        "scanned"
    )));
}
