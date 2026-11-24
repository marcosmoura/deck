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

fn is_deck_var(key: &OsStr) -> bool {
    let key = key.as_bytes();
    key.starts_with(b"DECK_") || key == b"DECK"
}

/// The environment of one terminal: the session's, minus what deck owns, plus what deck
/// sets, with `extra` (a launch spec's `--env`) between: an `extra` value replaces the
/// session's, but `DECK` and `DECK_*` are set last and can never be overridden.
pub fn terminal_env(
    session_env: &[(OsString, OsString)],
    extra: &[(OsString, OsString)],
    ids: &TerminalIds<'_>,
) -> Env {
    let mut env: Env = session_env
        .iter()
        .filter(|(k, _)| !dropped(k))
        .cloned()
        .collect();
    let mut set = |k: &OsStr, v: OsString| {
        env.retain(|(have, _)| have != k);
        env.push((k.to_os_string(), v));
    };
    set(OsStr::new("TERM"), "xterm-256color".into());
    set(OsStr::new("COLORTERM"), "truecolor".into());
    set(OsStr::new("TERM_PROGRAM"), "deck".into());
    set(
        OsStr::new("TERM_PROGRAM_VERSION"),
        env!("CARGO_PKG_VERSION").into(),
    );
    for (k, v) in extra.iter().filter(|(k, _)| !is_deck_var(k)) {
        set(k, v.clone());
    }
    let mut set = |k: &str, v: OsString| set(OsStr::new(k), v);
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
        let env = terminal_env(&session, &[], &ids(Path::new("/s/deck.sock")));
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
        // The client's own TERM_PROGRAM (iTerm 3) is replaced by deck's.
        assert_eq!(get("TERM_PROGRAM").as_deref(), Some("deck"));
        assert_eq!(
            get("TERM_PROGRAM_VERSION").as_deref(),
            Some(env!("CARGO_PKG_VERSION"))
        );
        for gone in ["DECK_X", "TMUX", "TMUX_PANE", "COLUMNS", "LINES"] {
            assert_eq!(get(gone), None, "{gone}");
        }
        // Each key once.
        let mut keys: Vec<_> = env.iter().map(|(k, _)| k.clone()).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), env.len());
    }

    #[test]
    fn extra_variables_override_the_session_but_never_deck_ones() {
        let session = e(&[("FOO", "bar"), ("KEEP", "1")]);
        let extra = e(&[
            ("FOO", "new"),
            ("TERM", "dumb"),
            ("DECK_TERMINAL_ID", "1"),
            ("DECK", "0"),
            ("EXTRA", "x"),
        ]);
        let env = terminal_env(&session, &extra, &ids(Path::new("/s/deck.sock")));
        let get = |k: &str| lookup(&env, k).map(|v| v.to_str().unwrap().to_string());
        assert_eq!(get("FOO").as_deref(), Some("new"));
        assert_eq!(get("KEEP").as_deref(), Some("1"));
        assert_eq!(get("EXTRA").as_deref(), Some("x"));
        assert_eq!(get("TERM").as_deref(), Some("dumb"));
        assert_eq!(get("DECK_TERMINAL_ID").as_deref(), Some("4"));
        assert_eq!(get("DECK").as_deref(), Some("1"));
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
