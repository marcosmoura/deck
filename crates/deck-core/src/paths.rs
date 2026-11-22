//! Where deck keeps its config, state and socket, resolved from an injected environment.

use std::path::PathBuf;

/// Environment lookup, injected so tests never touch the process environment.
pub type EnvLookup<'a> = &'a dyn Fn(&str) -> Option<String>;

/// Socket paths of this many bytes or more are refused (the macOS limit is 104).
pub const SOCKET_PATH_LIMIT: usize = 100;

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum PathError {
    #[error("cannot find the home directory: HOME is not set")]
    NoHome,
    #[error(
        "socket path {} is {len} bytes, the limit is under {SOCKET_PATH_LIMIT}; \
         set DECK_HOME to a shorter directory",
        path.display()
    )]
    SocketPathTooLong { path: PathBuf, len: usize },
}

/// A variable that is set and not empty.
fn var(env: EnvLookup<'_>, name: &str) -> Option<PathBuf> {
    env(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

fn home(env: EnvLookup<'_>) -> Result<PathBuf, PathError> {
    var(env, "HOME").ok_or(PathError::NoHome)
}

/// The config file.
pub fn config_file(env: EnvLookup<'_>) -> Result<PathBuf, PathError> {
    if let Some(dir) = var(env, "DECK_HOME") {
        return Ok(dir.join("config.toml"));
    }
    let base = match var(env, "XDG_CONFIG_HOME") {
        Some(dir) => dir,
        None => home(env)?.join(".config"),
    };
    Ok(base.join("deck").join("config.toml"))
}

/// The directory of `state.json`, `state.lock` and `daemon.log`.
pub fn state_dir(env: EnvLookup<'_>) -> Result<PathBuf, PathError> {
    if let Some(dir) = var(env, "DECK_HOME") {
        return Ok(dir.join("state"));
    }
    let base = match var(env, "XDG_STATE_HOME") {
        Some(dir) => dir,
        None => home(env)?.join(".local").join("state"),
    };
    Ok(base.join("deck"))
}

/// The directory of `deck.sock` and `deck.lock`.
pub fn runtime_dir(env: EnvLookup<'_>, uid: u32) -> PathBuf {
    if let Some(dir) = var(env, "DECK_HOME") {
        return dir.join("run");
    }
    if let Some(dir) = var(env, "XDG_RUNTIME_DIR") {
        return dir.join("deck");
    }
    let tmp = var(env, "TMPDIR").unwrap_or_else(|| PathBuf::from("/tmp"));
    tmp.join(format!("deck-{uid}"))
}

/// The daemon's socket; an error when the path is 100 bytes or longer.
pub fn socket_path(env: EnvLookup<'_>, uid: u32) -> Result<PathBuf, PathError> {
    let path = runtime_dir(env, uid).join("deck.sock");
    let len = path.as_os_str().len();
    if len >= SOCKET_PATH_LIMIT {
        return Err(PathError::SocketPathTooLong { path, len });
    }
    Ok(path)
}

/// The lock file holding the daemon's pid.
pub fn runtime_lock_path(env: EnvLookup<'_>, uid: u32) -> PathBuf {
    runtime_dir(env, uid).join("deck.lock")
}

pub fn state_file(env: EnvLookup<'_>) -> Result<PathBuf, PathError> {
    Ok(state_dir(env)?.join("state.json"))
}

pub fn state_lock_path(env: EnvLookup<'_>) -> Result<PathBuf, PathError> {
    Ok(state_dir(env)?.join("state.lock"))
}

pub fn log_file(env: EnvLookup<'_>) -> Result<PathBuf, PathError> {
    Ok(state_dir(env)?.join("daemon.log"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            vars.iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| (*v).to_string())
        }
    }

    type Case = (
        &'static [(&'static str, &'static str)],
        Result<&'static str, PathError>,
    );

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn config_table() {
        let cases: &[Case] = &[
            (&[("HOME", "/h")], Ok("/h/.config/deck/config.toml")),
            (
                &[("HOME", "/h"), ("XDG_CONFIG_HOME", "/x")],
                Ok("/x/deck/config.toml"),
            ),
            (
                &[("HOME", "/h"), ("XDG_CONFIG_HOME", "")],
                Ok("/h/.config/deck/config.toml"),
            ),
            (
                &[
                    ("HOME", "/h"),
                    ("XDG_CONFIG_HOME", "/x"),
                    ("DECK_HOME", "/d"),
                ],
                Ok("/d/config.toml"),
            ),
            (&[("DECK_HOME", "/d")], Ok("/d/config.toml")),
            (&[], Err(PathError::NoHome)),
        ];
        for (vars, want) in cases {
            let got = config_file(&lookup(vars));
            assert_eq!(got, want.clone().map(p), "{vars:?}");
        }
    }

    #[test]
    fn state_table() {
        let cases: &[Case] = &[
            (&[("HOME", "/h")], Ok("/h/.local/state/deck")),
            (&[("HOME", "/h"), ("XDG_STATE_HOME", "/x")], Ok("/x/deck")),
            (
                &[
                    ("HOME", "/h"),
                    ("XDG_STATE_HOME", "/x"),
                    ("DECK_HOME", "/d"),
                ],
                Ok("/d/state"),
            ),
            (&[("DECK_HOME", "/d")], Ok("/d/state")),
            (&[], Err(PathError::NoHome)),
        ];
        for (vars, want) in cases {
            let got = state_dir(&lookup(vars));
            assert_eq!(got, want.clone().map(p), "{vars:?}");
        }
        let env = lookup(&[("DECK_HOME", "/d")]);
        assert_eq!(state_file(&env).unwrap(), p("/d/state/state.json"));
        assert_eq!(state_lock_path(&env).unwrap(), p("/d/state/state.lock"));
        assert_eq!(log_file(&env).unwrap(), p("/d/state/daemon.log"));
    }

    #[test]
    fn runtime_table() {
        let cases: &[(&[(&str, &str)], &str)] = &[
            (&[], "/tmp/deck-501"),
            (&[("TMPDIR", "/var/t/")], "/var/t/deck-501"),
            (&[("TMPDIR", "")], "/tmp/deck-501"),
            (
                &[("TMPDIR", "/var/t"), ("XDG_RUNTIME_DIR", "/run/user/501")],
                "/run/user/501/deck",
            ),
            (&[("XDG_RUNTIME_DIR", "/run/u")], "/run/u/deck"),
            (
                &[
                    ("TMPDIR", "/t"),
                    ("XDG_RUNTIME_DIR", "/r"),
                    ("DECK_HOME", "/d"),
                ],
                "/d/run",
            ),
        ];
        for (vars, want) in cases {
            let env = lookup(vars);
            assert_eq!(runtime_dir(&env, 501), p(want), "{vars:?}");
            assert_eq!(socket_path(&env, 501).unwrap(), p(want).join("deck.sock"));
            assert_eq!(runtime_lock_path(&env, 501), p(want).join("deck.lock"));
        }
    }

    #[test]
    fn socket_path_limit() {
        // "/" + dir + "/run/deck.sock" where the suffix is 14 bytes.
        let build = |total: usize| {
            let dir = format!("/{}", "a".repeat(total - "/run/deck.sock".len() - 1));
            let path = PathBuf::from(&dir).join("run").join("deck.sock");
            assert_eq!(path.as_os_str().len(), total);
            dir
        };
        let ok = build(99);
        assert!(socket_path(&lookup(&[("DECK_HOME", &ok)]), 1).is_ok());
        let long = build(100);
        let err = socket_path(&lookup(&[("DECK_HOME", &long)]), 1).unwrap_err();
        let PathError::SocketPathTooLong { path, len } = &err else {
            panic!("{err:?}")
        };
        assert_eq!(*len, 100);
        let msg = err.to_string();
        assert!(
            msg.contains(path.to_str().unwrap()) && msg.contains("100"),
            "{msg}"
        );
    }
}
