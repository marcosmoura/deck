//! Recording real programs for the compat suite: scenario files, the scratch environment the
//! program runs in, and the privacy check every recording must pass.
//!
//! A scenario (`compat/scenarios/<name>.scenario`) has one step per line, `#` lines are
//! comments:
//!
//! - `size <cols>x<rows>` (default 100x30) and `setup <sh command>` come before `run`; setup
//!   commands run in the scratch `HOME`, with `$DECK_SCENARIOS` naming the scenarios directory
//!   and fixed git dates.
//! - `run <argv…>` starts the program (quotes group words).
//! - `wait <text>`, `send <text with escapes>`, `key <name>` and `sleep <ms>` drive it.
//! - `stop` ends the recording there and terminates the program, so the replay's last screen
//!   is the interesting one rather than the blank screen a quitting full-screen program
//!   leaves behind. Without it the recording ends when the program exits.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use deck_core::keys::{KeySpec, encode};
use deck_core::protocol::Modes;

use super::Harness;
use super::compat::unescape;

/// Where the recorded programs run: their `HOME` is below it, and it is the only path root a
/// recording may show.
pub const SCRATCH: &str = "/tmp/deck-record";
pub const DEFAULT_SIZE: (u16, u16) = (100, 30);
const WAIT: Duration = Duration::from_secs(20);
/// Fixed dates for the commits that setup steps make, so a re-record shows the same log.
const GIT_DATE: &str = "2026-01-15T10:00:00+00:00";

#[derive(Debug, Clone)]
pub enum Step {
    Wait(String),
    Send(Vec<u8>),
    Sleep(Duration),
    Stop,
}

#[derive(Debug, Clone)]
pub struct Scenario {
    pub name: String,
    pub size: (u16, u16),
    pub setup: Vec<String>,
    pub argv: Vec<String>,
    pub steps: Vec<Step>,
}

pub fn scenarios_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/compat/scenarios")
}

pub fn recordings_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/compat/recordings")
}

impl Scenario {
    pub fn named(name: &str) -> Self {
        let path = scenarios_dir().join(format!("{name}.scenario"));
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        Self::parse(name, &text)
    }

    pub fn parse(name: &str, text: &str) -> Self {
        let mut scenario = Self {
            name: name.into(),
            size: DEFAULT_SIZE,
            setup: Vec::new(),
            argv: Vec::new(),
            steps: Vec::new(),
        };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
            let started = !scenario.argv.is_empty();
            match word {
                "size" | "setup" if started => panic!("{name}: `{word}` comes before `run`"),
                "run" if started => panic!("{name}: more than one `run`"),
                "size" => {
                    scenario.size = rest
                        .split_once('x')
                        .and_then(|(c, r)| Some((c.parse().ok()?, r.parse().ok()?)))
                        .unwrap_or_else(|| panic!("{name}: bad size {rest:?}"));
                }
                "setup" => scenario.setup.push(rest.into()),
                "run" => scenario.argv = split_words(name, rest),
                _ if !started => panic!("{name}: `{word}` comes before `run`"),
                "wait" => scenario.steps.push(Step::Wait(rest.into())),
                "send" => scenario.steps.push(Step::Send(unescape(name, rest))),
                "key" => {
                    let spec = KeySpec::parse(rest).unwrap_or_else(|e| panic!("{name}: {e}"));
                    let bytes = encode(&spec.to_event(), Modes(0));
                    scenario.steps.push(Step::Send(bytes));
                }
                "sleep" => {
                    let ms = rest
                        .parse()
                        .unwrap_or_else(|_| panic!("{name}: bad sleep {rest:?}"));
                    scenario.steps.push(Step::Sleep(Duration::from_millis(ms)));
                }
                "stop" => scenario.steps.push(Step::Stop),
                other => panic!("{name}: unknown step {other:?}"),
            }
        }
        assert!(!scenario.argv.is_empty(), "{name}: no `run` step");
        scenario
    }
}

/// Splits a `run` line into words; single or double quotes group a word.
fn split_words(name: &str, text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quote = None;
    for c in text.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => word.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                in_word = true;
            }
            (None, c) if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            (None, c) => {
                word.push(c);
                in_word = true;
            }
        }
    }
    assert!(quote.is_none(), "{name}: an unclosed quote in `run`");
    if in_word {
        words.push(word);
    }
    words
}

/// The environment the program runs in: nothing of the recorder's own but a `PATH` without
/// entries below its home, everything else pointing into the scratch directory.
fn environment(home: &Path) -> Vec<(String, String)> {
    let real_home = std::env::var("HOME").unwrap_or_default();
    let path = std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .filter(|p| real_home.is_empty() || !p.starts_with(&real_home))
        .collect::<Vec<_>>()
        .join(":");
    let home = home.to_string_lossy();
    [
        ("PATH", path),
        ("TERM", "xterm-256color".into()),
        ("COLORTERM", "truecolor".into()),
        ("LANG", "en_US.UTF-8".into()),
        ("HOME", home.to_string()),
        ("XDG_CONFIG_HOME", format!("{home}/.config")),
        ("XDG_DATA_HOME", format!("{home}/.local/share")),
        ("XDG_STATE_HOME", format!("{home}/.local/state")),
        ("XDG_CACHE_HOME", format!("{home}/.cache")),
        ("USER", "deck".into()),
        ("LOGNAME", "deck".into()),
        ("HOSTNAME", "deck".into()),
        // A system gitconfig must not change what a recording shows.
        ("GIT_CONFIG_NOSYSTEM", "1".into()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

/// Waits until the program has written nothing for a moment.
fn quiet(h: &Harness) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut len = h.raw_output().len();
    let mut since = Instant::now();
    while since.elapsed() < Duration::from_millis(600) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
        let now = h.raw_output().len();
        if now != len {
            len = now;
            since = Instant::now();
        }
    }
}

/// Plays a scenario with the program straight on the harness's terminal and returns the
/// program's raw output.
pub fn play(scenario: &Scenario) -> Vec<u8> {
    let name = &scenario.name;
    let root = Path::new(SCRATCH).join(name);
    let _ = fs::remove_dir_all(&root);
    // The scratch directory is the home itself: `/home/` is on the privacy denylist.
    let home = root.clone();
    for dir in [".config", ".local/share", ".local/state", ".cache"] {
        fs::create_dir_all(home.join(dir)).expect("scratch directories");
    }
    fs::write(
        home.join(".gitconfig"),
        "[user]\n\tname = deck\n\temail = deck@example.invalid\n[init]\n\tdefaultBranch = main\n",
    )
    .expect(".gitconfig");
    let env = environment(&home);

    for command in &scenario.setup {
        let status = Command::new("/bin/sh")
            .args(["-c", command])
            .env_clear()
            .envs(env.iter().map(|(k, v)| (k, v)))
            .env("DECK_SCENARIOS", scenarios_dir())
            .env("GIT_AUTHOR_DATE", GIT_DATE)
            .env("GIT_COMMITTER_DATE", GIT_DATE)
            .current_dir(&home)
            .stdin(Stdio::null())
            .status()
            .expect("run a setup command");
        assert!(status.success(), "{name}: setup failed: {command}");
    }

    // `env -i` clears the harness's own environment too; the shell enters the scratch home.
    let mut argv: Vec<String> = vec!["/usr/bin/env".into(), "-i".into()];
    argv.extend(env.iter().map(|(k, v)| format!("{k}={v}")));
    argv.extend(["/bin/sh".into(), "-c".into()]);
    argv.push("cd \"$HOME\" && exec \"$0\" \"$@\"".into());
    argv.extend(scenario.argv.iter().cloned());
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
    let mut h = Harness::spawn(scenario.size, &argv);

    let mut stopped = false;
    for step in &scenario.steps {
        match step {
            Step::Wait(text) => h.wait_for_text(text, WAIT),
            Step::Send(bytes) => h.send_bytes(bytes),
            Step::Sleep(d) => std::thread::sleep(*d),
            Step::Stop => {
                stopped = true;
                break;
            }
        }
    }
    if stopped {
        quiet(&h);
    } else {
        h.wait_for_exit(WAIT);
    }
    let out = h.raw_output();
    if stopped {
        h.terminate();
    }
    let _ = fs::remove_dir_all(&root);
    out
}

/// Roots a recording may show an absolute path under: the scratch directory and system
/// directories that are the same on every machine.
const PATH_ROOTS: &[&str] = &[
    "/tmp/deck-record",
    "/private/tmp/deck-record",
    "/usr/",
    "/opt/",
    "/bin/",
    "/sbin/",
    "/etc/",
    "/lib/",
    "/dev/",
    "/System/",
    "/Library/",
    "/Applications/",
];

/// Absolute paths in `text` (a `/` at the start of a word, followed by a name) that are not
/// under an allowed root.
fn stray_paths(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let name = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-');
    let mut found = Vec::new();
    for (i, &c) in chars.iter().enumerate() {
        let starts = c == '/'
            && chars.get(i + 1).is_some_and(|&n| name(n))
            && (i == 0 || chars[i - 1].is_whitespace() || "\'\"=(:[".contains(chars[i - 1]));
        if !starts {
            continue;
        }
        let path: String = chars[i..]
            .iter()
            .take_while(|&&c| name(c) || c == '/')
            .collect();
        if !PATH_ROOTS.iter().any(|root| path.starts_with(root)) {
            found.push(path);
        }
    }
    found
}

/// What makes a recording unfit to commit: the machine's home or host name, a path under
/// any home directory, or any absolute path outside the scratch directory and the system
/// roots.
pub fn privacy_violations(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    let mut found = Vec::new();
    let mut deny = |what: &str, needle: &str| {
        if !needle.is_empty() && text.contains(needle) {
            found.push(format!("{what} ({needle:?})"));
        }
    };
    for path in ["/Users/", "/home/", "/var/folders/"] {
        deny("a home or temp path", path);
    }
    let home = std::env::var("HOME").unwrap_or_default();
    if home.len() >= 4 {
        deny("the real $HOME", &home);
    }
    for flag in [None, Some("-s")] {
        let mut command = Command::new("hostname");
        command.args(flag);
        if let Ok(out) = command.output() {
            let host = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if host.len() >= 4 && host != "deck" {
                deny("the host name", &host);
                deny("the host name", host.split('.').next().unwrap_or_default());
            }
        }
    }
    for path in stray_paths(&text) {
        found.push(format!("a path outside the scratch directory ({path:?})"));
    }
    found
}

/// Writes a recording after the privacy check; refuses to replace one without `force`.
pub fn write_recording(name: &str, bytes: &[u8], force: bool) -> Result<PathBuf, String> {
    let path = recordings_dir().join(format!("{name}.vt"));
    if path.exists() && !force {
        return Err(format!(
            "{} exists; run with FORCE=1 to replace it",
            path.display()
        ));
    }
    let mut found = privacy_violations(bytes);
    // The user name is checked only here: on another machine it can be a word the programs
    // print (`deck`, `main`), so the committed test leaves it out.
    if let Ok(user) = std::env::var("USER")
        && user.len() >= 4
        && String::from_utf8_lossy(bytes).contains(&user)
    {
        found.push(format!("the real $USER ({user:?})"));
    }
    if !found.is_empty() {
        return Err(format!(
            "{name}: the recording shows private details: {}",
            found.join(", ")
        ));
    }
    fs::create_dir_all(recordings_dir()).map_err(|e| e.to_string())?;
    fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path)
}
