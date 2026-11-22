//! Linux: the cwd and the arguments of a process through `/proc`.

use std::path::PathBuf;

/// The working directory of the process `pid`, `None` on any failure and for a path that
/// is not valid UTF-8.
pub fn process_cwd(pid: u32) -> Option<PathBuf> {
    let path = std::fs::read_link(format!("/proc/{pid}/cwd")).ok()?;
    let text = path.to_str()?;
    // A deleted directory reads as "<path> (deleted)".
    if text.ends_with(" (deleted)") && !path.exists() {
        return None;
    }
    Some(path)
}

/// The arguments of the process `pid` (`argv[0]` first), from `/proc/<pid>/cmdline`.
pub fn process_argv(pid: u32) -> Option<Vec<String>> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    Some(split_cmdline(&raw))
}

/// The NUL-separated arguments of a `cmdline` file.
fn split_cmdline(raw: &[u8]) -> Vec<String> {
    let raw = raw.strip_suffix(&[0]).unwrap_or(raw);
    if raw.is_empty() {
        return Vec::new();
    }
    raw.split(|&b| b == 0)
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cmdline_splits_at_nuls() {
        assert_eq!(
            split_cmdline(b"node\0/x/codex.js\0"),
            ["node", "/x/codex.js"]
        );
        assert_eq!(split_cmdline(b"claude\0"), ["claude"]);
        assert!(split_cmdline(b"").is_empty());
    }
}
