//! deck's own desktop notifications (`notify = "system"`): unlike the terminal's OSC 9, a
//! click on one runs `deck focus --raise` on the terminal it is about. macOS shows them with
//! `terminal-notifier`, Linux with `notify-send`; without either, or when the notifier fails,
//! the client falls back to the terminal's notification.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use deck_core::model::{PanelId, TerminalId};

/// An alert to show: its text, and the terminal a click focuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemAlert {
    pub body: String,
    /// The name of the session the terminal is in, for `deck focus --session`.
    pub session: String,
    pub panel: PanelId,
    /// The terminal the agent runs in.
    pub terminal: TerminalId,
    /// The terminal's notification (OSC 9), written when deck cannot show its own.
    pub fallback: Vec<u8>,
}

/// The notification's title.
const TITLE: &str = "deck";

/// The program that shows deck's own notifications here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notifier {
    /// macOS: `terminal-notifier`, whose click runs a command and brings an app forward.
    TerminalNotifier(PathBuf),
    /// Linux: `notify-send`, which waits for a click on its action.
    NotifySend(PathBuf),
}

/// The notifier the client can use, from its environment: none over ssh (the notification
/// would show on the remote machine, or nowhere) or, on Linux, without a desktop session bus,
/// and then the terminal's notification stays in use.
pub fn available() -> Option<Notifier> {
    let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
    pick(
        cfg!(target_os = "macos"),
        cfg!(target_os = "linux"),
        &|name| var(name).is_some(),
        &var("PATH").unwrap_or_default(),
    )
}

pub(crate) fn pick(
    macos: bool,
    linux: bool,
    set: &dyn Fn(&str) -> bool,
    path: &OsString,
) -> Option<Notifier> {
    if set("SSH_CONNECTION") || set("SSH_TTY") {
        return None;
    }
    if macos {
        return find("terminal-notifier", path).map(Notifier::TerminalNotifier);
    }
    if linux && set("DBUS_SESSION_BUS_ADDRESS") {
        return find("notify-send", path).map(Notifier::NotifySend);
    }
    None
}

/// Shows `alert` with `notifier`; a click runs `deck focus --raise` against the daemon at
/// `socket` (the client's, which the command that a click runs could not find from its own
/// environment). `failed` gets the reason when the notification was not shown: the notifier
/// could not be started (at once, on this thread), or it ended with an error (later, on
/// another thread; macOS can refuse notifications to an app: `terminal-notifier` then exits
/// with status 3). The caller writes the terminal's notification instead.
pub fn show(
    notifier: &Notifier,
    alert: &SystemAlert,
    socket: &Path,
    failed: impl FnOnce(String) + Send + 'static,
) {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => return failed(format!("deck does not know its own path: {e}")),
    };
    match notifier {
        Notifier::TerminalNotifier(program) => {
            let bundle = std::env::var("__CFBundleIdentifier").ok();
            let args = terminal_notifier_args(alert, &exe, socket, bundle.as_deref());
            spawn_watched(
                "terminal-notifier",
                Command::new(program).args(args),
                failed,
            );
        }
        Notifier::NotifySend(program) => {
            notify_send(program, alert, exe, socket.to_path_buf(), failed);
        }
    }
}

/// How a notifier ended, for the log: `terminal-notifier ended with status 3`.
fn ended(name: &str, status: std::io::Result<std::process::ExitStatus>) -> String {
    match status {
        Ok(status) => match status.code() {
            Some(code) => format!("{name} ended with status {code}"),
            None => format!("{name} was killed"),
        },
        Err(e) => format!("{name}: {e}"),
    }
}

/// The first `name` on `path`.
fn find(name: &str, path: &OsString) -> Option<PathBuf> {
    std::env::split_paths(path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(name))
        .find(|file| file.is_file())
}

/// Starts `cmd` with no input or output, and waits for it on a thread of its own (so it never
/// lingers as a zombie), which runs `failed` when it ends with an error.
fn spawn_watched(
    name: &'static str,
    cmd: &mut Command,
    failed: impl FnOnce(String) + Send + 'static,
) {
    let started = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match started {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let status = child.wait();
                if !status.as_ref().is_ok_and(std::process::ExitStatus::success) {
                    failed(ended(name, status));
                }
            });
        }
        Err(e) => failed(format!("{name} did not start: {e}")),
    }
}

/// `notify-send` waits for a click on its `default` action and prints the action's name;
/// then the terminal is focused. Linux has no portable way to raise the terminal's window.
fn notify_send(
    notifier: &Path,
    alert: &SystemAlert,
    exe: PathBuf,
    socket: PathBuf,
    failed: impl FnOnce(String) + Send + 'static,
) {
    let started = Command::new(notifier)
        .args(notify_send_args(alert))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let child = match started {
        Ok(child) => child,
        Err(e) => return failed(format!("notify-send did not start: {e}")),
    };
    let focus = focus_args(alert);
    std::thread::spawn(move || {
        let out = match child.wait_with_output() {
            Ok(out) => out,
            Err(e) => return failed(format!("notify-send: {e}")),
        };
        // An older libnotify rejects `--action` or `--wait` and exits with an error.
        if !out.status.success() {
            return failed(ended("notify-send", Ok(out.status)));
        }
        if String::from_utf8_lossy(&out.stdout).trim() == "default" {
            let _ = Command::new(exe)
                .env("DECK_SOCKET", socket)
                .args(focus)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    });
}

/// `deck focus` on the alert's terminal, in its session; `--raise` has the client bring its
/// window forward.
pub(crate) fn focus_args(alert: &SystemAlert) -> Vec<String> {
    vec![
        "focus".into(),
        "--raise".into(),
        "--session".into(),
        alert.session.clone(),
        alert.terminal.0.to_string(),
    ]
}

/// A word for `sh`: single-quoted, with each `'` closed, escaped and reopened.
fn shell_quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', r"'\''"))
}

/// `terminal-notifier`'s arguments: the text, one notification per terminal (a newer one
/// replaces it), the command a click runs (through `sh`, so every word is quoted; it runs in
/// launchd's environment, so it names the daemon's socket) and, when known, the terminal's app
/// to bring forward.
pub(crate) fn terminal_notifier_args(
    alert: &SystemAlert,
    exe: &Path,
    socket: &Path,
    bundle: Option<&str>,
) -> Vec<String> {
    let words = std::iter::once(exe.to_string_lossy().into_owned())
        .chain(focus_args(alert))
        .map(|word| shell_quote(&word))
        .collect::<Vec<_>>()
        .join(" ");
    let command = format!(
        "DECK_SOCKET={} {words}",
        shell_quote(&socket.to_string_lossy())
    );
    let mut args = vec![
        "-title".into(),
        TITLE.into(),
        "-message".into(),
        alert.body.clone(),
        "-group".into(),
        format!("deck-{}-{}", alert.session, alert.terminal.0),
        "-execute".into(),
        command,
    ];
    if let Some(bundle) = bundle.filter(|b| !b.is_empty()) {
        args.extend(["-activate".into(), bundle.to_string()]);
    }
    args
}

/// `notify-send`'s arguments: a `default` action (a click on the notification) that it waits
/// for and prints. Notification daemons read the body as markup, so `&`, `<` and `>` in a
/// name are escaped.
pub(crate) fn notify_send_args(alert: &SystemAlert) -> Vec<String> {
    let body = alert
        .body
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    vec![
        "--app-name=deck".into(),
        "--action=default=Open".into(),
        "--wait".into(),
        TITLE.into(),
        body,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alert(session: &str) -> SystemAlert {
        SystemAlert {
            body: "claude needs you · deck › Ping".into(),
            session: session.into(),
            panel: PanelId(42),
            terminal: TerminalId(43),
            fallback: b"\x1b]9;x\x07".to_vec(),
        }
    }

    #[test]
    fn terminal_notifier_runs_deck_focus_and_brings_the_terminal_forward() {
        let args = terminal_notifier_args(
            &alert("main"),
            Path::new("/opt/deck/bin/deck"),
            Path::new("/run/deck.sock"),
            Some("com.mitchellh.ghostty"),
        );
        assert_eq!(
            args,
            [
                "-title",
                "deck",
                "-message",
                "claude needs you · deck › Ping",
                "-group",
                "deck-main-43",
                "-execute",
                "DECK_SOCKET='/run/deck.sock' '/opt/deck/bin/deck' 'focus' '--raise' '--session' 'main' '43'",
                "-activate",
                "com.mitchellh.ghostty",
            ]
        );
        // Without a bundle id there is nothing to bring forward.
        let args = terminal_notifier_args(&alert("main"), Path::new("/d"), Path::new("/s"), None);
        assert!(!args.contains(&"-activate".to_string()));
    }

    #[test]
    fn a_session_name_cannot_break_out_of_the_command() {
        let args = terminal_notifier_args(
            &alert("it's; rm -rf ~"),
            Path::new("/d"),
            Path::new("/s'x"),
            None,
        );
        let command = &args[args.iter().position(|a| a == "-execute").unwrap() + 1];
        assert_eq!(
            command,
            r"DECK_SOCKET='/s'\''x' '/d' 'focus' '--raise' '--session' 'it'\''s; rm -rf ~' '43'"
        );
    }

    #[test]
    fn notify_send_waits_for_a_click_on_its_default_action() {
        assert_eq!(
            notify_send_args(&alert("main")),
            [
                "--app-name=deck",
                "--action=default=Open",
                "--wait",
                "deck",
                "claude needs you · deck › Ping",
            ]
        );
        assert_eq!(
            focus_args(&alert("main")),
            ["focus", "--raise", "--session", "main", "43"]
        );
    }

    #[test]
    fn a_notifier_that_ends_with_an_error_is_reported() {
        let run = |program: &str| {
            let (tx, rx) = std::sync::mpsc::channel();
            spawn_watched("notifier", &mut Command::new(program), move |why| {
                let _ = tx.send(why);
            });
            rx.recv_timeout(std::time::Duration::from_secs(5)).ok()
        };
        assert_eq!(
            run("false").as_deref(),
            Some("notifier ended with status 1")
        );
        assert_eq!(run("true"), None);
        // One that cannot be started is reported at once.
        let why = run("/nonexistent/deck-notifier").unwrap();
        assert!(why.starts_with("notifier did not start: "), "{why}");
    }

    #[test]
    fn notify_send_escapes_markup() {
        let mut a = alert("main");
        a.body = "claude finished · R&D › <api>".into();
        assert_eq!(
            notify_send_args(&a)[4],
            "claude finished · R&amp;D › &lt;api&gt;"
        );
    }

    #[test]
    fn the_notifier_depends_on_the_platform_and_the_session() {
        let dir = std::env::temp_dir().join(format!("deck-pick-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["terminal-notifier", "notify-send"] {
            std::fs::write(dir.join(name), "").unwrap();
        }
        let path = std::env::join_paths([&dir]).unwrap();
        let with = |vars: &'static [&'static str]| move |name: &str| vars.contains(&name);
        assert_eq!(
            pick(true, false, &with(&[]), &path),
            Some(Notifier::TerminalNotifier(dir.join("terminal-notifier")))
        );
        // Linux needs a desktop session bus.
        assert_eq!(pick(false, true, &with(&[]), &path), None);
        assert_eq!(
            pick(false, true, &with(&["DBUS_SESSION_BUS_ADDRESS"]), &path),
            Some(Notifier::NotifySend(dir.join("notify-send")))
        );
        // Over ssh, never: the terminal's notification reaches the local machine.
        for ssh in ["SSH_CONNECTION", "SSH_TTY"] {
            let vars: &'static [&'static str] = if ssh == "SSH_TTY" {
                &["SSH_TTY", "DBUS_SESSION_BUS_ADDRESS"]
            } else {
                &["SSH_CONNECTION", "DBUS_SESSION_BUS_ADDRESS"]
            };
            assert_eq!(pick(true, false, &with(vars), &path), None);
            assert_eq!(pick(false, true, &with(vars), &path), None);
        }
        // Neither platform: none.
        assert_eq!(pick(false, false, &with(&[]), &path), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_notifier_is_found_on_the_path_only() {
        let dir = std::env::temp_dir().join(format!("deck-notify-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tool = dir.join("terminal-notifier");
        std::fs::write(&tool, "").unwrap();
        let path = std::env::join_paths([Path::new("relative"), &dir]).unwrap();
        assert_eq!(find("terminal-notifier", &path), Some(tool));
        assert_eq!(find("notify-send", &path), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
