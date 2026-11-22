//! Linux: the cwd of a process through `/proc`.

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
