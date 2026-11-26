//! Bringing the outer terminal's window forward (`ServerMsg::Raise`): a notification of this
//! session was clicked, and its terminal is focused inside deck, but deck's window may be
//! behind another one, in a tab that is not selected or in a split that has no focus. Only
//! the terminal app can change that, and the client is the one process that runs inside it.
//!
//! Ghostty on macOS is asked through its AppleScript (`focus`, by the client's tty). The
//! client's `osascript` belongs to Ghostty as far as macOS is concerned, so Ghostty talks to
//! itself and macOS asks no permission. Other terminals have no such door: there the
//! notifier's own "bring the app forward" is all there is.

use std::process::{Command, Stdio};

/// Ghostty's bundle id on macOS.
const GHOSTTY: &str = "com.mitchellh.ghostty";

/// How the client can bring its outer terminal's window forward.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Raiser {
    /// Ghostty on macOS, through AppleScript.
    Ghostty,
}

/// The way to raise the window here, from the client's environment: none over ssh (the
/// window is on another machine).
pub fn available() -> Option<Raiser> {
    let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    pick(
        cfg!(target_os = "macos"),
        var("__CFBundleIdentifier").as_deref(),
        var("SSH_CONNECTION").is_some() || var("SSH_TTY").is_some(),
    )
}

pub(crate) fn pick(macos: bool, bundle: Option<&str>, ssh: bool) -> Option<Raiser> {
    (macos && !ssh && bundle == Some(GHOSTTY)).then_some(Raiser::Ghostty)
}

/// Raises the window and waits for it: what happened, for the log. Call it off the client's
/// loop.
pub fn run(raiser: Option<Raiser>) -> String {
    match raiser {
        None => "raise: nothing here can bring the window forward".into(),
        Some(Raiser::Ghostty) => match ghostty() {
            Ok(tty) => format!("raise: Ghostty focused {tty}"),
            Err(why) => format!("raise: failed: {why}"),
        },
    }
}

/// The client's tty, as `tty` prints it for the client's own input.
fn tty() -> Result<String, String> {
    let out = Command::new("/usr/bin/tty")
        .stdin(Stdio::inherit())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| format!("tty: {e}"))?;
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.status.success() && is_tty_path(&name) {
        Ok(name)
    } else {
        Err("the client's input is not a terminal".into())
    }
}

/// A device path that is safe inside the script's quotes.
pub(crate) fn is_tty_path(name: &str) -> bool {
    name.strip_prefix("/dev/")
        .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric()))
}

/// The script: focus the Ghostty terminal on `tty` (its window comes to the front, its tab is
/// selected, its split gets the focus), then bring Ghostty forward.
pub(crate) fn ghostty_script(tty: &str) -> Vec<String> {
    vec![
        format!("tell application id \"{GHOSTTY}\""),
        "repeat with t in terminals".into(),
        format!("if (tty of t) is \"{tty}\" then"),
        "focus t".into(),
        "activate".into(),
        "return".into(),
        "end if".into(),
        "end repeat".into(),
        format!("error \"no Ghostty terminal on {tty}\""),
        "end tell".into(),
    ]
}

fn ghostty() -> Result<String, String> {
    let tty = tty()?;
    let mut cmd = Command::new("/usr/bin/osascript");
    for line in ghostty_script(&tty) {
        cmd.args(["-e", &line]);
    }
    let out = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .output()
        .map_err(|e| format!("osascript: {e}"))?;
    if out.status.success() {
        Ok(tty)
    } else {
        let said = String::from_utf8_lossy(&out.stderr);
        Err(format!(
            "osascript: {}",
            said.lines().next().unwrap_or("").trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_ghostty_on_this_mac_is_raised() {
        assert_eq!(pick(true, Some(GHOSTTY), false), Some(Raiser::Ghostty));
        // Another terminal, another system, or a window on another machine.
        assert_eq!(pick(true, Some("com.apple.Terminal"), false), None);
        assert_eq!(pick(true, None, false), None);
        assert_eq!(pick(false, Some(GHOSTTY), false), None);
        assert_eq!(pick(true, Some(GHOSTTY), true), None);
    }

    #[test]
    fn the_script_focuses_the_terminal_on_the_clients_tty() {
        let script = ghostty_script("/dev/ttys004").join("\n");
        assert!(script.starts_with("tell application id \"com.mitchellh.ghostty\"\n"));
        assert!(script.contains("if (tty of t) is \"/dev/ttys004\" then\nfocus t\nactivate\n"));
        assert!(script.ends_with("error \"no Ghostty terminal on /dev/ttys004\"\nend tell"));
    }

    #[test]
    fn only_a_plain_device_path_goes_into_the_script() {
        assert!(is_tty_path("/dev/ttys004"));
        assert!(is_tty_path("/dev/ttyp1"));
        for bad in [
            "",
            "/dev/",
            "not a tty",
            "/dev/tty\" & (do shell script \"x\")",
            "/tmp/x",
        ] {
            assert!(!is_tty_path(bad), "{bad}");
        }
    }

    #[test]
    fn without_a_raiser_the_log_says_so() {
        assert_eq!(
            run(None),
            "raise: nothing here can bring the window forward"
        );
    }
}
