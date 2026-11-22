//! The `unsafe` of the daemon: questions to the operating system. Everything else is safe
//! code.

#![allow(unsafe_code)]

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "linux")]
pub use linux::process_cwd;
#[cfg(target_os = "macos")]
pub use macos::process_cwd;

/// The machine's host name, empty when it cannot be read.
pub fn hostname() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: `buf` is valid for `buf.len()` bytes; `gethostname` writes at most that many
    // and `buf` is zeroed, so a name cut at the limit still ends at a NUL below.
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len() - 1) };
    if rc != 0 {
        return String::new();
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(0);
    buf.get(..end)
        .map(|name| String::from_utf8_lossy(name).into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;

    #[test]
    fn the_hostname_matches_the_hostname_command() {
        let expected = if cfg!(target_os = "linux") {
            std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap()
        } else {
            let out = Command::new("hostname").output().unwrap();
            String::from_utf8_lossy(&out.stdout).into_owned()
        };
        assert_eq!(hostname(), expected.trim_end());
    }
}
