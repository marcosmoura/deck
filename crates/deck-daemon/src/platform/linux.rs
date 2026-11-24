//! Linux: the cwd and the arguments of a process, and the daemon's own figures, through
//! `/proc`.

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

/// The resident memory in bytes: the second field of `/proc/self/statm`, in pages.
pub fn rss_bytes() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    // SAFETY: `sysconf` only reads a system constant.
    let page = u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).ok()?;
    rss_pages(&statm)?.checked_mul(page)
}

fn rss_pages(statm: &str) -> Option<u64> {
    statm.split_whitespace().nth(1)?.parse().ok()
}

/// The open descriptors: the entries of `/proc/self/fd`, less the one that listing opens.
pub fn fds() -> Option<u32> {
    let count = std::fs::read_dir("/proc/self/fd").ok()?.count();
    u32::try_from(count.saturating_sub(1)).ok()
}

/// The threads: the `Threads:` line of `/proc/self/status`.
pub fn threads() -> Option<u32> {
    threads_in(&std::fs::read_to_string("/proc/self/status").ok()?)
}

fn threads_in(status: &str) -> Option<u32> {
    status
        .lines()
        .find_map(|l| l.strip_prefix("Threads:"))?
        .trim()
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_figures_are_read_from_proc_text() {
        assert_eq!(rss_pages("5000 1234 300 1 0 900 0\n"), Some(1234));
        assert_eq!(rss_pages("5000"), None);
        assert_eq!(
            threads_in("Name:\tdeck\nThreads:\t38\nVmRSS: 5 kB\n"),
            Some(38)
        );
        assert_eq!(threads_in("Name:\tdeck\n"), None);
    }

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
