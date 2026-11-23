//! What the outer terminal can show: its name (XTVERSION or the environment), whether it
//! draws images, and whether it has Nerd Font glyphs; and from that, the icon [`Tier`].
//!
//! This is the one module of `deck-tui` that spawns programs (`defaults`, `fc-list`) and lists
//! font folders. Everything that reads their output is a pure function.

use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use crate::graphics;
use crate::tier::Tier;

/// `defaults` by its absolute path, so a `PATH` entry cannot stand in for it.
const DEFAULTS: &str = "/usr/bin/defaults";
/// How long a font-check program may run before it is killed.
const PROGRAM_TIMEOUT: Duration = Duration::from_secs(1);

/// A terminal's version: major, minor, patch.
pub type Version = (u32, u32, u32);

/// The first kitty with Unicode placeholders.
const KITTY_PLACEHOLDERS: Version = (0, 28, 0);
/// The first kitty that bundles the Nerd Font symbols.
const KITTY_NERD: Version = (0, 36, 0);

/// The outer terminal, as far as deck can tell.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::enum_variant_names,
    reason = "Terminal.app's name, as the spec names the variant"
)]
pub enum Terminal {
    Ghostty,
    /// `None` when found from the environment: counts as a current kitty.
    Kitty(Option<Version>),
    WezTerm,
    ITerm2,
    AppleTerminal,
    /// Answered XTVERSION with a name deck does not know (`tmux 3.5a`).
    Other(String),
    Unknown,
}

/// The terminal from its XTVERSION reply: `kitty(0.36.4)`, `ghostty 1.2.0`,
/// `WezTerm 20240203-…`, `iTerm2 3.5.10`, `tmux 3.5a`.
pub fn terminal_from_xtversion(reply: &str) -> Terminal {
    let reply = reply.trim();
    let lower = reply.to_ascii_lowercase();
    if let Some(version) = lower.strip_prefix("kitty(") {
        let version = version.split(')').next().unwrap_or("");
        return Terminal::Kitty(parse_version(version));
    }
    let name = lower.split([' ', '(']).next().unwrap_or("");
    match name {
        "ghostty" => Terminal::Ghostty,
        "kitty" => Terminal::Kitty(None),
        "wezterm" => Terminal::WezTerm,
        "iterm2" => Terminal::ITerm2,
        _ => Terminal::Other(reply.to_string()),
    }
}

fn parse_version(text: &str) -> Option<Version> {
    let mut parts = text.split('.').map(|p| p.parse::<u32>().ok());
    let major = parts.next()??;
    let minor = parts.next().flatten().unwrap_or(0);
    let patch = parts.next().flatten().unwrap_or(0);
    Some((major, minor, patch))
}

/// The terminal from the environment, for when it did not answer XTVERSION.
pub fn terminal_from_env(env: &HashMap<String, String>) -> Terminal {
    let var = |name: &str| env.get(name).map(String::as_str).filter(|v| !v.is_empty());
    match var("TERM_PROGRAM") {
        Some("ghostty") => return Terminal::Ghostty,
        Some("WezTerm") => return Terminal::WezTerm,
        Some("iTerm.app") => return Terminal::ITerm2,
        Some("Apple_Terminal") => return Terminal::AppleTerminal,
        _ => {}
    }
    match var("TERM") {
        Some("xterm-ghostty") => return Terminal::Ghostty,
        Some("xterm-kitty") => return Terminal::Kitty(None),
        _ => {}
    }
    if var("KITTY_WINDOW_ID").is_some() {
        Terminal::Kitty(None)
    } else if var("GHOSTTY_RESOURCES_DIR").is_some() {
        Terminal::Ghostty
    } else if var("WEZTERM_EXECUTABLE").is_some() {
        Terminal::WezTerm
    } else {
        Terminal::Unknown
    }
}

/// The terminal: by XTVERSION when it answered, else by the environment. Terminal.app never
/// answers XTVERSION, so it is found by `TERM_PROGRAM`.
pub fn terminal(xtversion: Option<&str>, env: &HashMap<String, String>) -> Terminal {
    match xtversion {
        Some(reply) => terminal_from_xtversion(reply),
        None => terminal_from_env(env),
    }
}

fn set(env: &HashMap<String, String>, name: &str) -> bool {
    env.get(name).is_some_and(|v| !v.is_empty())
}

/// Whether the terminal draws kitty's Unicode placeholders.
fn has_placeholders(terminal: &Terminal) -> bool {
    match terminal {
        Terminal::Ghostty => true,
        Terminal::Kitty(version) => version.is_none_or(|v| v >= KITTY_PLACEHOLDERS),
        _ => false,
    }
}

/// Check 1: the terminal ships the Nerd Font symbols and falls back to them for any font.
/// Inside a deck panel nothing is known about the inner emulator's font.
fn bundles_nerd(terminal: &Terminal, env: &HashMap<String, String>) -> bool {
    if set(env, "DECK") {
        return false;
    }
    match terminal {
        Terminal::Ghostty | Terminal::WezTerm => true,
        Terminal::Kitty(version) => version.is_none_or(|v| v >= KITTY_NERD),
        _ => false,
    }
}

/// The tier, from the probe's answers and the font check's result (`font`: a check found
/// Nerd Font glyphs). Pure.
pub fn decide(
    terminal: &Terminal,
    graphics_ok: bool,
    env: &HashMap<String, String>,
    truecolor: bool,
    font: bool,
) -> Tier {
    if graphics_ok && has_placeholders(terminal) && truecolor && !graphics::multiplexed(env) {
        Tier::Image
    } else if bundles_nerd(terminal, env) || font {
        Tier::Nerd
    } else {
        Tier::Unicode
    }
}

/// The font check of a terminal: its own setting (iTerm2, Terminal.app), then font fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    own: Option<Own>,
    fallback: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Own {
    /// The profile named by `ITERM_PROFILE`, if set.
    ITerm2(Option<String>),
    AppleTerminal,
}

/// Whether a font that has the Nerd glyphs, installed, shows up in the terminal's own font
/// through the system's font fallback (check 4, after the terminal's own check failed).
///
/// From the by-hand check of sub-spec 02 (requirement 7): Terminal.app does not; iTerm2 is
/// taken not to (untested); Alacritty and the other macOS terminals do; on Linux it always
/// applies.
fn fallback_applies(terminal: &Terminal) -> bool {
    !cfg!(target_os = "macos") || !matches!(terminal, Terminal::ITerm2 | Terminal::AppleTerminal)
}

/// The font check to run for `terminal`, `None` when the terminal needs none (it bundles the
/// symbols) or the test seam `DECK_TEST_NO_FONT_CHECK` is set.
pub fn check_for(terminal: &Terminal, env: &HashMap<String, String>) -> Option<Check> {
    if set(env, "DECK_TEST_NO_FONT_CHECK") || bundles_nerd(terminal, env) {
        return None;
    }
    let own = if set(env, "DECK") {
        None
    } else {
        match terminal {
            Terminal::ITerm2 => Some(Own::ITerm2(
                env.get("ITERM_PROFILE").filter(|p| !p.is_empty()).cloned(),
            )),
            Terminal::AppleTerminal => Some(Own::AppleTerminal),
            _ => None,
        }
    };
    let fallback = own.is_none() || fallback_applies(terminal);
    Some(Check { own, fallback })
}

impl Check {
    /// Runs the programs; `true` when a check found Nerd Font glyphs. Blocks for up to a second
    /// per program: run it on a thread.
    pub fn run(&self) -> bool {
        let own = match &self.own {
            Some(Own::ITerm2(profile)) => {
                match run_program(DEFAULTS, &["export", "com.googlecode.iterm2", "-"]) {
                    Run::Out(xml) => iterm2_font_is_nerd(&xml, profile.as_deref()),
                    _ => false,
                }
            }
            Some(Own::AppleTerminal) => {
                match run_program(DEFAULTS, &["export", "com.apple.Terminal", "-"]) {
                    Run::Out(xml) => terminal_app_font_is_nerd(&xml),
                    _ => false,
                }
            }
            None => false,
        };
        own || (self.fallback && fallback_found())
    }
}

/// Check 4: a font with U+F120 and U+EAB4 is installed.
fn fallback_found() -> bool {
    let ran = run_program("fc-list", &[":charset=f120 eab4", "family"]);
    fallback_from(ran, cfg!(target_os = "macos"), nerd_font_file_installed)
}

/// The result of check 4 from how `fc-list` ran. On macOS, when it cannot give an answer (it is
/// missing, failed or timed out), the font folders are looked at instead (`scan`).
pub(crate) fn fallback_from(ran: Run, macos: bool, scan: impl FnOnce() -> bool) -> bool {
    match ran {
        Run::Out(out) => fc_list_found(&out),
        Run::Missing | Run::Failed => macos && scan(),
    }
}

/// `fc-list` printed a family.
pub fn fc_list_found(output: &str) -> bool {
    output.lines().any(|line| !line.trim().is_empty())
}

/// macOS without fontconfig: a Nerd Font file in the font folders.
fn nerd_font_file_installed() -> bool {
    let mut dirs = vec![PathBuf::from("/Library/Fonts")];
    if let Some(home) = std::env::var_os("HOME").filter(|h| !h.is_empty()) {
        dirs.push(PathBuf::from(home).join("Library/Fonts"));
    }
    dirs.iter().any(|dir| {
        std::fs::read_dir(dir).is_ok_and(|entries| {
            any_nerd_font_file(
                entries
                    .flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned()),
            )
        })
    })
}

/// A font file whose name, without its extension, is a Nerd Font name.
pub fn any_nerd_font_file(names: impl IntoIterator<Item = String>) -> bool {
    names.into_iter().any(|name| {
        let stem = name
            .rsplit_once('.')
            .map_or(name.as_str(), |(stem, _)| stem);
        is_nerd_font_name(stem)
    })
}

pub(crate) enum Run {
    Out(String),
    /// The program does not exist.
    Missing,
    /// It failed, timed out or printed something unreadable.
    Failed,
}

/// Runs a program directly, without a shell, with stdin and stderr closed, killed after
/// [`PROGRAM_TIMEOUT`].
fn run_program(program: &str, args: &[&str]) -> Run {
    let mut child = match Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Run::Missing,
        Err(_) => return Run::Failed,
    };
    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Run::Failed;
    };
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        let _ = tx.send(bytes);
    });
    let bytes = rx.recv_timeout(PROGRAM_TIMEOUT);
    if bytes.is_err() {
        let _ = child.kill();
    }
    let status = child.wait();
    match (bytes, status) {
        (Ok(bytes), Ok(status)) if status.success() => {
            String::from_utf8(bytes).map_or(Run::Failed, Run::Out)
        }
        _ => Run::Failed,
    }
}

/// The probe's answers and environment turned into the first tier and the font check that
/// may still upgrade it.
#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    pub tier: Tier,
    pub check: Option<Check>,
}

/// Decides the tier from what the probe found. The graphics query answered `OK`, or, when the
/// terminal answered neither new query, DECK-12's environment rules say Ghostty or kitty.
pub fn plan(
    xtversion: Option<&str>,
    graphics: Option<bool>,
    env: &HashMap<String, String>,
    truecolor: bool,
) -> Plan {
    let terminal = terminal(xtversion, env);
    let graphics_ok = match graphics {
        Some(ok) => ok,
        None => xtversion.is_none() && graphics::detect(env, truecolor),
    };
    let tier = decide(&terminal, graphics_ok, env, truecolor, false);
    let check = if tier == Tier::Unicode {
        check_for(&terminal, env)
    } else {
        None
    };
    Plan { tier, check }
}

/// Whether `name` is a Nerd Font's name: it has `Nerd Font` or `NerdFont` (any case), or `NF`,
/// `NFM` or `NFP` as a word or right before a `-`.
pub fn is_nerd_font_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower.contains("nerd font") || lower.contains("nerdfont") {
        return true;
    }
    let suffix = |s: &str| ["NF", "NFM", "NFP"].iter().any(|n| s.ends_with(n));
    name.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|word| matches!(word, "NF" | "NFM" | "NFP"))
        || name
            .match_indices('-')
            .any(|(i, _)| name.get(..i).is_some_and(suffix))
}

// ---- Property lists -------------------------------------------------------------------

/// The parts of an XML property list the font checks read.
#[derive(Debug, Clone, PartialEq)]
enum Plist {
    Dict(Vec<(String, Plist)>),
    Array(Vec<Plist>),
    Str(String),
    Bool(bool),
    /// The base64 text of a `<data>` element.
    Data(String),
    Other,
}

impl Plist {
    fn get(&self, key: &str) -> Option<&Plist> {
        match self {
            Self::Dict(items) => items.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    fn str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            _ => None,
        }
    }
}

const MAX_DEPTH: usize = 32;

/// The next tag of `rest`: its name (`/dict` for a closing tag), whether it closes itself, and
/// the text before it. Skips the XML declaration, the doctype and comments.
fn next_tag<'a>(rest: &mut &'a str) -> Option<(&'a str, bool, &'a str)> {
    loop {
        let open = rest.find('<')?;
        let text = rest.get(..open)?;
        let after = rest.get(open + 1..)?;
        let close = after.find('>')?;
        let tag = after.get(..close)?;
        *rest = after.get(close + 1..)?;
        if tag.starts_with('?') || tag.starts_with('!') {
            continue;
        }
        let self_closing = tag.ends_with('/');
        let name = tag
            .trim_end_matches('/')
            .split_whitespace()
            .next()
            .unwrap_or("");
        return Some((name, self_closing, text));
    }
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// The text of a leaf element, up to its closing tag.
fn leaf_text(rest: &mut &str, name: &str, self_closing: bool) -> Option<String> {
    if self_closing {
        return Some(String::new());
    }
    let (close, _, text) = next_tag(rest)?;
    (close.strip_prefix('/') == Some(name)).then(|| unescape(text))
}

fn parse_value(rest: &mut &str, name: &str, self_closing: bool, depth: usize) -> Option<Plist> {
    if depth > MAX_DEPTH {
        return None;
    }
    match name {
        "dict" => {
            let mut items = Vec::new();
            if !self_closing {
                loop {
                    let (tag, closed, _) = next_tag(rest)?;
                    match tag {
                        "/dict" => break,
                        "key" => {
                            let key = leaf_text(rest, "key", closed)?;
                            let (vname, vclosed, _) = next_tag(rest)?;
                            items.push((key, parse_value(rest, vname, vclosed, depth + 1)?));
                        }
                        _ => return None,
                    }
                }
            }
            Some(Plist::Dict(items))
        }
        "array" => {
            let mut items = Vec::new();
            if !self_closing {
                loop {
                    let (tag, closed, _) = next_tag(rest)?;
                    if tag == "/array" {
                        break;
                    }
                    items.push(parse_value(rest, tag, closed, depth + 1)?);
                }
            }
            Some(Plist::Array(items))
        }
        "string" => leaf_text(rest, name, self_closing).map(Plist::Str),
        "data" => leaf_text(rest, name, self_closing).map(Plist::Data),
        "true" => Some(Plist::Bool(true)),
        "false" => Some(Plist::Bool(false)),
        _ => {
            leaf_text(rest, name, self_closing)?;
            Some(Plist::Other)
        }
    }
}

fn parse_plist(xml: &str) -> Option<Plist> {
    let mut rest = xml;
    loop {
        let (name, closed, _) = next_tag(&mut rest)?;
        if name != "plist" {
            return parse_value(&mut rest, name, closed, 0);
        }
    }
}

/// Check 2: the iTerm2 profile in use (the one named `profile`, else the one whose `Guid` is
/// `Default Bookmark Guid`) has a Nerd Font: its `Non Ascii Font` when `Use Non-ASCII Font` is
/// true (it alone decides), else its `Normal Font`. `xml` is `defaults export com.googlecode.iterm2 -`.
pub fn iterm2_font_is_nerd(xml: &str, profile: Option<&str>) -> bool {
    let Some(root) = parse_plist(xml) else {
        return false;
    };
    let Some(Plist::Array(profiles)) = root.get("New Bookmarks") else {
        return false;
    };
    let by = |key: &str, value: &str| {
        profiles
            .iter()
            .find(|p| p.get(key).and_then(Plist::str) == Some(value))
    };
    let Some(profile) = profile.and_then(|name| by("Name", name)).or_else(|| {
        let guid = root.get("Default Bookmark Guid").and_then(Plist::str)?;
        by("Guid", guid)
    }) else {
        return false;
    };
    let font = |key: &str| {
        profile
            .get(key)
            .and_then(Plist::str)
            .is_some_and(is_nerd_font_name)
    };
    // iTerm2 draws the private-use glyphs with the non-ASCII font when it is in use.
    if profile.get("Use Non-ASCII Font") == Some(&Plist::Bool(true)) {
        font("Non Ascii Font")
    } else {
        font("Normal Font")
    }
}

/// Check 3: the Terminal.app profile named by `Default Window Settings` has a Nerd Font. Its
/// `Font` is an archived `NSFont` whose bytes hold the PostScript name as text. `xml` is
/// `defaults export com.apple.Terminal -`.
pub fn terminal_app_font_is_nerd(xml: &str) -> bool {
    let Some(root) = parse_plist(xml) else {
        return false;
    };
    let Some(name) = root.get("Default Window Settings").and_then(Plist::str) else {
        return false;
    };
    let Some(Plist::Data(font)) = root
        .get("Window Settings")
        .and_then(|profiles| profiles.get(name))
        .and_then(|profile| profile.get("Font"))
    else {
        return false;
    };
    base64_decode(font).is_some_and(|bytes| font_data_is_nerd(&bytes))
}

/// Whether a text run of the archived font is a Nerd Font name. A run may start with the
/// binary plist's length marker (`^SFMono-Regular`), so it is tried with and without its
/// first character.
pub fn font_data_is_nerd(bytes: &[u8]) -> bool {
    bytes
        .split(|b| !(0x20..=0x7e).contains(b))
        .filter(|run| run.len() >= 4)
        .filter_map(|run| std::str::from_utf8(run).ok())
        .any(|run| is_nerd_font_name(run) || run.get(1..).is_some_and(is_nerd_font_name))
}

fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text
        .bytes()
        .filter(|b| !b.is_ascii_whitespace() && *b != b'=')
    {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((acc >> bits) & 0xff).ok()?);
        }
    }
    Some(out)
}
