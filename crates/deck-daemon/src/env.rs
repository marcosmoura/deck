//! The environment of a session's terminals.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::Path;

use deck_core::model::{PanelId, SessionId, TerminalId};

pub type Env = Vec<(OsString, OsString)>;

/// Variables refreshed from the attaching client on every attach.
pub const REFRESHED: &[&str] = &[
    "SSH_AUTH_SOCK",
    "SSH_CONNECTION",
    "SSH_TTY",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XAUTHORITY",
];

fn dropped(key: &OsStr) -> bool {
    let key = key.as_bytes();
    key.starts_with(b"DECK_")
        || key == b"DECK"
        || key == b"TMUX"
        || key == b"TMUX_PANE"
        || key == b"COLUMNS"
        || key == b"LINES"
        || key.starts_with(b"TERM_PROGRAM")
        || key == b"TERM"
        || key == b"COLORTERM"
}

pub struct TerminalIds<'a> {
    pub socket: &'a Path,
    pub session: SessionId,
    pub panel: PanelId,
    pub terminal: TerminalId,
}

/// The environment of one terminal: the session's, minus what deck owns, plus what deck
/// sets.
pub fn terminal_env(session_env: &[(OsString, OsString)], ids: &TerminalIds<'_>) -> Env {
    let mut env: Env = session_env
        .iter()
        .filter(|(k, _)| !dropped(k))
        .cloned()
        .collect();
    let mut set = |k: &str, v: OsString| env.push((OsString::from(k), v));
    set("TERM", "xterm-256color".into());
    set("COLORTERM", "truecolor".into());
    set("DECK", "1".into());
    set(
        "DECK_SOCKET",
        OsString::from_vec(ids.socket.as_os_str().as_bytes().to_vec()),
    );
    set("DECK_SESSION_ID", ids.session.to_string().into());
    set("DECK_PANEL_ID", ids.panel.to_string().into());
    set("DECK_TERMINAL_ID", ids.terminal.to_string().into());
    env
}

/// Replaces the refreshed variables of `session_env` with those of `hello_env`; a variable
/// the client lacks is removed.
pub fn refresh(session_env: &mut Env, hello_env: &[(OsString, OsString)]) {
    session_env.retain(|(k, _)| !REFRESHED.iter().any(|r| k.as_bytes() == r.as_bytes()));
    session_env.extend(
        hello_env
            .iter()
            .filter(|(k, _)| REFRESHED.iter().any(|r| k.as_bytes() == r.as_bytes()))
            .cloned(),
    );
}

pub fn lookup<'a>(env: &'a [(OsString, OsString)], key: &str) -> Option<&'a OsStr> {
    env.iter()
        .rev()
        .find(|(k, _)| k.as_bytes() == key.as_bytes())
        .map(|(_, v)| v.as_os_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(pairs: &[(&str, &str)]) -> Env {
        pairs
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect()
    }

    fn ids(socket: &Path) -> TerminalIds<'_> {
        TerminalIds {
            socket,
            session: SessionId(1),
            panel: PanelId(3),
            terminal: TerminalId(4),
        }
    }

    #[test]
    fn terminal_env_filters_and_adds() {
        let session = e(&[
            ("FOO", "bar"),
            ("DECK_TERMINAL_ID", "99"),
            ("DECK_X", "y"),
            ("TMUX", "t"),
            ("TMUX_PANE", "%1"),
            ("COLUMNS", "80"),
            ("LINES", "24"),
            ("TERM_PROGRAM", "iTerm"),
            ("TERM_PROGRAM_VERSION", "3"),
            ("TERM", "dumb"),
            ("COLORTERM", "no"),
            ("LANG", "en_US.UTF-8"),
        ]);
        let env = terminal_env(&session, &ids(Path::new("/s/deck.sock")));
        let get = |k: &str| lookup(&env, k).map(|v| v.to_str().unwrap().to_string());
        assert_eq!(get("FOO").as_deref(), Some("bar"));
        assert_eq!(get("LANG").as_deref(), Some("en_US.UTF-8"));
        assert_eq!(get("DECK_TERMINAL_ID").as_deref(), Some("4"));
        assert_eq!(get("DECK_SESSION_ID").as_deref(), Some("1"));
        assert_eq!(get("DECK_PANEL_ID").as_deref(), Some("3"));
        assert_eq!(get("DECK_SOCKET").as_deref(), Some("/s/deck.sock"));
        assert_eq!(get("DECK").as_deref(), Some("1"));
        assert_eq!(get("TERM").as_deref(), Some("xterm-256color"));
        assert_eq!(get("COLORTERM").as_deref(), Some("truecolor"));
        for gone in [
            "DECK_X",
            "TMUX",
            "TMUX_PANE",
            "COLUMNS",
            "LINES",
            "TERM_PROGRAM",
            "TERM_PROGRAM_VERSION",
        ] {
            assert_eq!(get(gone), None, "{gone}");
        }
        // Each key once.
        let mut keys: Vec<_> = env.iter().map(|(k, _)| k.clone()).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), env.len());
    }

    #[test]
    fn refresh_replaces_and_removes_the_allowlist_only() {
        let mut session = e(&[("FOO", "bar"), ("SSH_AUTH_SOCK", "/old"), ("DISPLAY", ":0")]);
        refresh(
            &mut session,
            &e(&[("SSH_AUTH_SOCK", "/new"), ("FOO", "other")]),
        );
        assert_eq!(lookup(&session, "SSH_AUTH_SOCK"), Some(OsStr::new("/new")));
        assert_eq!(lookup(&session, "DISPLAY"), None);
        assert_eq!(lookup(&session, "FOO"), Some(OsStr::new("bar")));
    }
}
